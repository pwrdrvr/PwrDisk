//! How a scanned tree is held: in a few buffers per worker, by `u32`
//! index.
//!
//! Every node is a 64-byte record, each directory's children a contiguous
//! run of one vector, every name in a string, and the allocation and
//! sharing records, which most nodes lack, in side tables.
//!
//! The walk builds on many workers at once, and each appends to a
//! [`Store`] of its own, so none waits on another. A finished tree keeps
//! the stores as they are instead of joining them: joining writes a second
//! copy of every node at the end of a scan, and macOS does not take back
//! the memory of a store freed as it is copied, so a 19.9-million-node
//! scan peaked at 6.1 to 6.4 GiB that way against 3.6 to 3.8 GiB in one
//! buffer. A node records which store holds its name, side records and
//! children, in a byte of its own.
//!
//! A tree of separately allocated nodes is freed one `free()` at a time,
//! each write reading a page back in, from the compressor if macOS has
//! packed it away: 0.8 to 0.9 s for a 20-million-node tree in a fresh
//! process, 15 s in a window under memory pressure. Here a drop frees a
//! few buffers per worker without reading them. Nothing here needs
//! `unsafe`: an index is checked where a pointer would have to be trusted,
//! and a name is a slice of a `String`.
//!
//! A pass that writes splits the work by store, since each is its own
//! memory: files are visited store by store, each store's directories are
//! sorted by one worker, and a directory's totals are worked out on every
//! worker by reading, then written by position.
//!
//! Reading goes through [`Node`], a cursor that pairs a stored record with
//! the tree it indexes into; writing goes through [`NodeMut`], or a pass
//! over the whole tree.

use std::cmp::Ordering;
use std::fmt;
use std::marker::PhantomData;
use std::num::NonZeroU64;
use std::ops::{Deref, Range};
use std::sync::{Mutex, MutexGuard, PoisonError};

use rayon::prelude::*;

use crate::classify::{Category, Reclaim};
use crate::sharing::{Allocation, Sharing};
use crate::tree::{Draft, Metric, NodeKind};

/// The longest name a tree holds.
///
/// A local file system allows a few hundred bytes, even decoded lossily;
/// anything read from elsewhere, such as an SSH host's stream, is refused
/// past this before it is built.
pub const MAX_NAME_BYTES: usize = 4096;

/// Everything about a node that is plain data, and so free to write.
///
/// `bytes` and `files` are the subtree totals the treemap draws. A disk
/// holds tens of millions of nodes, so every field is paid that many
/// times: the size of a stored node is held by a test.
#[derive(Clone, Copy, Debug)]
pub struct Facts {
    pub kind: NodeKind,
    /// Subtree total: direct contents plus every descendant. A file's own
    /// size, or nothing for a second name of a file already charged.
    pub bytes: u64,
    /// Files at or beneath this node; `1` for a file.
    pub files: u64,
    /// Directories at or beneath this node; `1` for a directory. Four
    /// billion is more than any volume holds.
    pub dirs: u32,
    /// `(device, inode)` for files, used to de-duplicate hardlinks. No file
    /// has inode zero, which lets `None` cost nothing.
    pub inode: Option<(u64, NonZeroU64)>,
    /// Some entries here or below could not be read; totals are incomplete.
    pub read_error: bool,
    /// Newest write time at or beneath this node, in Unix seconds; `0` when
    /// unknown. Derived for directories by [`crate::tree::aggregate`].
    pub modified: i64,
    /// What kind of data this is, for colour. Set by [`crate::classify`].
    pub category: Category,
    /// Why this space can be had back, if it can. Set by
    /// [`crate::classify`]; inherited by everything beneath.
    pub reclaim: Option<Reclaim>,
}

impl Facts {
    /// A directory with nothing counted yet.
    pub const fn directory() -> Self {
        Self {
            kind: NodeKind::Directory,
            bytes: 0,
            files: 0,
            dirs: 1,
            inode: None,
            read_error: false,
            modified: 0,
            category: Category::Other,
            reclaim: None,
        }
    }

    /// A leaf entry of `bytes`.
    pub fn entry(kind: NodeKind, bytes: u64) -> Self {
        Self {
            kind,
            bytes,
            files: u64::from(kind == NodeKind::File),
            dirs: 0,
            ..Self::directory()
        }
    }

    /// The value a treemap should weight this node by.
    pub const fn value(&self, metric: Metric) -> u64 {
        match metric {
            Metric::Bytes => self.bytes,
            Metric::Files => self.files,
        }
    }
}

/// Which side table a node's [`Extra`] points into, in its top two bits.
///
/// A node carries one index instead of one per table: an allocation is
/// kept for few files and sharing figures for some directories, so two
/// indices would cost every node four bytes for what most never use, and
/// take a stored node from 64 bytes to 72.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Extra(u32);

impl Extra {
    const NONE: Self = Self(0);
    const SHIFT: u32 = 30;
    const ALLOCATION: u32 = 1;
    const SHARING: u32 = 2;
    const BOTH: u32 = 3;

    fn new(tag: u32, index: usize) -> Self {
        let index = u32::try_from(index)
            .ok()
            .filter(|index| *index < 1 << Self::SHIFT)
            .expect("fewer than a billion side records");
        Self(tag << Self::SHIFT | index)
    }

    const fn tag(self) -> u32 {
        self.0 >> Self::SHIFT
    }

    const fn index(self) -> usize {
        (self.0 & ((1 << Self::SHIFT) - 1)) as usize
    }
}

/// One node as stored: its facts and where its name, children and side
/// records are, all in the store that made it.
#[derive(Clone, Copy, Debug)]
struct Slot {
    facts: Facts,
    /// Offset of the name's length prefix in its store's names.
    name: u32,
    /// Index of the first child in its store's nodes; zero when there are
    /// none.
    first: u32,
    len: u32,
    extra: Extra,
}

/// Where a node is: the store whose nodes hold it, and its index there.
/// Good until the tree is next changed, since sorting moves nodes within
/// their blocks.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct NodeId {
    store: u8,
    index: u32,
}

/// One worker's buffers, or a whole tree built on one thread.
#[derive(Default)]
struct Store {
    nodes: Vec<Slot>,
    /// For each of `nodes`, the store that made it, which holds its name,
    /// side records and children: a directory's children can come from
    /// any worker, and a slot has no room left to say which.
    homes: Vec<u8>,
    /// Every name, each after its length written as one `char`: a `String`
    /// can be sliced without checking its bytes again, which `&[u8]` could
    /// only skip with `unsafe`, and a length that is a `char` keeps the
    /// buffer valid UTF-8. A name is at most a few hundred bytes, well
    /// under the surrogate range a `char` cannot hold.
    names: String,
    allocations: Vec<Allocation>,
    sharings: Vec<Sharing>,
    /// A node with both, which only a followed directory link has.
    both: Vec<(Allocation, Sharing)>,
}

/// A copy made on every worker: copying a large tree is gigabytes of
/// writes to pages touched for the first time, which one thread would
/// fault in one at a time.
impl Clone for Store {
    fn clone(&self) -> Self {
        fn copied<T: Copy + Send + Sync>(source: &[T]) -> Vec<T> {
            let mut copy = Vec::with_capacity(source.len());
            copy.par_extend(source.par_iter().copied());
            copy
        }
        let ((nodes, homes), (names, (allocations, sharings))) = rayon::join(
            || rayon::join(|| copied(&self.nodes), || copied(&self.homes)),
            || {
                rayon::join(
                    || self.names.clone(),
                    || {
                        rayon::join(
                            || copied(&self.allocations),
                            || copied(&self.sharings),
                        )
                    },
                )
            },
        );
        Self {
            nodes,
            homes,
            names,
            allocations,
            sharings,
            both: self.both.clone(),
        }
    }
}

fn name_at(names: &str, at: u32) -> &str {
    let rest = &names[at as usize..];
    let mut chars = rest.chars();
    let len = chars.next().map_or(0, |len| len as usize);
    let start = rest.len() - chars.as_str().len();
    &rest[start..start + len]
}

impl Store {
    fn index(len: usize) -> u32 {
        u32::try_from(len).expect("fewer than four billion nodes")
    }

    fn name(&self, at: u32) -> &str {
        name_at(&self.names, at)
    }

    fn push_name(&mut self, name: &str) -> u32 {
        let at = u32::try_from(self.names.len())
            .expect("a store's names fit in four gigabytes");
        // `MAX_NAME_BYTES` is far below the first length that is not a
        // `char`, 55,296, where the surrogates begin.
        assert!(name.len() <= MAX_NAME_BYTES, "a name of {}", name.len());
        let len = u32::try_from(name.len())
            .ok()
            .and_then(char::from_u32)
            .expect("a length below the surrogates");
        self.names.push(len);
        self.names.push_str(name);
        at
    }

    fn allocation(&self, extra: Extra) -> Option<&Allocation> {
        match extra.tag() {
            Extra::ALLOCATION => Some(&self.allocations[extra.index()]),
            Extra::BOTH => Some(&self.both[extra.index()].0),
            _ => None,
        }
    }

    fn sharing(&self, extra: Extra) -> Option<&Sharing> {
        match extra.tag() {
            Extra::SHARING => Some(&self.sharings[extra.index()]),
            Extra::BOTH => Some(&self.both[extra.index()].1),
            _ => None,
        }
    }

    fn extra(
        &mut self,
        allocation: Option<Allocation>,
        sharing: Option<Sharing>,
    ) -> Extra {
        match (allocation, sharing) {
            (None, None) => Extra::NONE,
            (Some(allocation), None) => {
                self.allocations.push(allocation);
                Extra::new(Extra::ALLOCATION, self.allocations.len() - 1)
            }
            (None, Some(sharing)) => {
                self.sharings.push(sharing);
                Extra::new(Extra::SHARING, self.sharings.len() - 1)
            }
            (Some(allocation), Some(sharing)) => {
                self.both.push((allocation, sharing));
                Extra::new(Extra::BOTH, self.both.len() - 1)
            }
        }
    }

    /// Replace a node's sharing figures, in place when it has some, so
    /// aggregating again does not grow the tables. A record left behind
    /// when a node changes table stays until the tree is dropped; only
    /// a change in what is kept does that, which re-ranking never makes.
    fn set_sharing(&mut self, extra: Extra, sharing: Option<Sharing>) -> Extra {
        let index = extra.index();
        match (extra.tag(), sharing) {
            (Extra::SHARING, Some(value)) => {
                self.sharings[index] = value;
                extra
            }
            (Extra::BOTH, Some(value)) => {
                self.both[index].1 = value;
                extra
            }
            (Extra::ALLOCATION, Some(value)) => {
                let allocation = self.allocations[index];
                self.extra(Some(allocation), Some(value))
            }
            (Extra::BOTH, None) => {
                let allocation = self.both[index].0;
                self.extra(Some(allocation), None)
            }
            (Extra::SHARING, None) => Extra::NONE,
            (_, None) => extra,
            (_, value) => self.extra(None, value),
        }
    }

    /// A node for `name` with these facts and records, holding `children`,
    /// each with the store that made it, which become one block here.
    fn place(
        &mut self,
        name: &str,
        facts: Facts,
        allocation: Option<Allocation>,
        sharing: Option<Sharing>,
        children: impl ExactSizeIterator<Item = (Slot, u8)>,
    ) -> Slot {
        let len = Self::index(children.len());
        let first = if len == 0 {
            0
        } else {
            let first = Self::index(self.nodes.len());
            self.nodes.reserve(children.len());
            self.homes.reserve(children.len());
            for (child, home) in children {
                self.nodes.push(child);
                self.homes.push(home);
            }
            first
        };
        Slot {
            facts,
            name: self.push_name(name),
            first,
            len,
            extra: self.extra(allocation, sharing),
        }
    }

    /// A copy of `node` from any tree, and everything beneath it, made in
    /// this store, which is store `me` of its build.
    fn copy(&mut self, node: Node<'_>, name: &str, me: u8) -> Slot {
        let children: Vec<Slot> = node
            .children()
            .iter()
            .map(|child| self.copy(child, child.name(), me))
            .collect();
        self.place(
            name,
            *node.facts(),
            node.allocation().copied(),
            node.stored_sharing().copied(),
            children.into_iter().map(|child| (child, me)),
        )
    }

    /// A copy of all of `source` under `name`: each of its stores'
    /// buffers appended in turn, every index moved by where its home
    /// store's buffers landed. One sequential pass instead of a walk with
    /// a list per directory: the worker that meets a reused tree copies
    /// all of it before it lists anything else. `source`'s own root is
    /// appended too, and nothing reaches it, which keeps every index a
    /// plain shift.
    fn absorb(&mut self, source: &Tree, name: &str, me: u8) -> Slot {
        let mut bases = Vec::with_capacity(source.stores.len());
        let mut next = Bases {
            nodes: self.nodes.len(),
            names: self.names.len(),
            allocations: self.allocations.len(),
            sharings: self.sharings.len(),
            both: self.both.len(),
        };
        for part in &source.stores {
            bases.push(next);
            next.nodes += part.nodes.len();
            next.names += part.names.len();
            next.allocations += part.allocations.len();
            next.sharings += part.sharings.len();
            next.both += part.both.len();
        }
        u32::try_from(next.names)
            .expect("a store's names fit in four gigabytes");
        for part in &source.stores {
            self.names.push_str(&part.names);
            self.allocations.extend_from_slice(&part.allocations);
            self.sharings.extend_from_slice(&part.sharings);
            self.both.extend_from_slice(&part.both);
            // Not in parallel: this runs with the worker's store held, and
            // a rayon worker waiting on its own jobs can pick up a walk
            // task that asks for the same store.
            self.nodes.extend(
                part.nodes
                    .iter()
                    .zip(&part.homes)
                    .map(|(slot, &home)| bases[usize::from(home)].moved(slot)),
            );
        }
        self.homes.resize(self.nodes.len(), me);
        let root = source.root();
        Slot {
            name: self.push_name(name),
            ..bases[usize::from(root.home())].moved(root.slot)
        }
    }

    fn draft(&mut self, draft: &Draft, me: u8) -> Slot {
        let children: Vec<Slot> = draft
            .children
            .iter()
            .map(|child| self.draft(child, me))
            .collect();
        self.place(
            &draft.name,
            draft.facts,
            draft.allocation.as_deref().copied(),
            draft.sharing.as_deref().copied(),
            children.into_iter().map(|child| (child, me)),
        )
    }

    const fn bytes(&self) -> StorageBytes {
        StorageBytes {
            used: self.nodes.len() * size_of::<Slot>()
                + self.homes.len()
                + self.names.len()
                + self.allocations.len() * size_of::<Allocation>()
                + self.sharings.len() * size_of::<Sharing>()
                + self.both.len() * size_of::<(Allocation, Sharing)>(),
            reserved: self.nodes.capacity() * size_of::<Slot>()
                + self.homes.capacity()
                + self.names.capacity()
                + self.allocations.capacity() * size_of::<Allocation>()
                + self.sharings.capacity() * size_of::<Sharing>()
                + self.both.capacity() * size_of::<(Allocation, Sharing)>(),
        }
    }
}

/// Where one store's buffers start once appended to another.
#[derive(Clone, Copy, Default)]
struct Bases {
    nodes: usize,
    names: usize,
    allocations: usize,
    sharings: usize,
    both: usize,
}

impl Bases {
    /// `slot`, whose home store these bases are for, as the store it is
    /// appended to reads it.
    fn moved(self, slot: &Slot) -> Slot {
        let index = slot.extra.index();
        let extra = match slot.extra.tag() {
            Extra::ALLOCATION => {
                Extra::new(Extra::ALLOCATION, self.allocations + index)
            }
            Extra::SHARING => Extra::new(Extra::SHARING, self.sharings + index),
            Extra::BOTH => Extra::new(Extra::BOTH, self.both + index),
            _ => Extra::NONE,
        };
        let shift = |at: u32, base: usize| Store::index(at as usize + base);
        Slot {
            name: shift(slot.name, self.names),
            first: if slot.len == 0 {
                0
            } else {
                shift(slot.first, self.nodes)
            },
            extra,
            ..*slot
        }
    }
}

/// What a tree's buffers hold.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StorageBytes {
    /// Bytes holding nodes, names and side records.
    pub used: usize,
    /// Bytes the buffers have room for, including growth room that is
    /// never touched and so never resident.
    pub reserved: usize,
}

/// One entry in a scanned tree, read through the tree that holds it.
/// Its facts read through `Deref`.
#[derive(Clone, Copy)]
pub struct Node<'t> {
    tree: &'t Tree,
    slot: &'t Slot,
    id: NodeId,
}

impl Deref for Node<'_> {
    type Target = Facts;

    fn deref(&self) -> &Facts {
        &self.slot.facts
    }
}

impl<'t> Node<'t> {
    pub const fn facts(self) -> &'t Facts {
        &self.slot.facts
    }

    /// The store that holds this node's name, records and children.
    fn home(self) -> u8 {
        self.tree.stores[usize::from(self.id.store)].homes
            [self.id.index as usize]
    }

    fn records(self) -> &'t Store {
        &self.tree.stores[usize::from(self.home())]
    }

    pub fn name(self) -> &'t str {
        self.records().name(self.slot.name)
    }

    /// Children, ordered by [`crate::tree::Metric`] value, descending.
    pub fn children(self) -> Children<'t> {
        let store = self.home();
        let first = self.slot.first;
        Children {
            tree: self.tree,
            store,
            first,
            slots: &self.tree.stores[usize::from(store)].nodes
                [first as usize..first as usize + self.slot.len as usize],
        }
    }

    pub fn allocation(self) -> Option<&'t Allocation> {
        self.records().allocation(self.slot.extra)
    }

    /// Derived clone figures, sparse: see [`Node::sharing`].
    pub fn stored_sharing(self) -> Option<&'t Sharing> {
        self.records().sharing(self.slot.extra)
    }

    /// Where this node is in its tree, for writing to it later: a pass
    /// that only reads can work out figures on every worker, then write
    /// them by position.
    pub(crate) const fn id(self) -> NodeId {
        self.id
    }

    /// Whether this is its tree's root, and so the whole tree.
    fn is_root(self) -> bool {
        self.id == self.tree.root
    }
}

impl fmt::Debug for Node<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Node")
            .field("name", &self.name())
            .field("facts", self.facts())
            .field("allocation", &self.allocation())
            .field("sharing", &self.stored_sharing())
            .field("children", &self.children())
            .finish()
    }
}

/// A directory's children: a contiguous run of one store's nodes.
#[derive(Clone, Copy)]
pub struct Children<'t> {
    tree: &'t Tree,
    store: u8,
    first: u32,
    slots: &'t [Slot],
}

impl<'t> Children<'t> {
    pub const fn len(self) -> usize {
        self.slots.len()
    }

    pub const fn is_empty(self) -> bool {
        self.slots.is_empty()
    }

    fn node(self, offset: usize, slot: &'t Slot) -> Node<'t> {
        Node {
            tree: self.tree,
            slot,
            id: NodeId {
                store: self.store,
                index: self.first + Store::index(offset),
            },
        }
    }

    pub fn get(self, index: usize) -> Option<Node<'t>> {
        self.slots.get(index).map(|slot| self.node(index, slot))
    }

    /// The child at `index`, which must exist.
    pub fn at(self, index: usize) -> Node<'t> {
        self.node(index, &self.slots[index])
    }

    pub fn iter(self) -> Iter<'t> {
        let first = self.first;
        Iter {
            tree: self.tree,
            store: self.store,
            slots: (first..first + Store::index(self.slots.len()))
                .zip(self.slots.iter()),
        }
    }

    pub fn par_iter(self) -> impl IndexedParallelIterator<Item = Node<'t>> {
        self.slots
            .par_iter()
            .enumerate()
            .map(move |(offset, slot)| self.node(offset, slot))
    }
}

impl<'t> IntoIterator for Children<'t> {
    type Item = Node<'t>;
    type IntoIter = Iter<'t>;

    fn into_iter(self) -> Iter<'t> {
        self.iter()
    }
}

impl fmt::Debug for Children<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list().entries(self.iter()).finish()
    }
}

/// The children of one directory, in order.
#[derive(Clone)]
pub struct Iter<'t> {
    tree: &'t Tree,
    store: u8,
    slots: std::iter::Zip<Range<u32>, std::slice::Iter<'t, Slot>>,
}

impl<'t> Iter<'t> {
    const fn node(&self, (index, slot): (u32, &'t Slot)) -> Node<'t> {
        Node {
            tree: self.tree,
            slot,
            id: NodeId {
                store: self.store,
                index,
            },
        }
    }
}

impl fmt::Debug for Iter<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Iter")
            .field("remaining", &self.slots.len())
            .finish()
    }
}

impl<'t> Iterator for Iter<'t> {
    type Item = Node<'t>;

    fn next(&mut self) -> Option<Node<'t>> {
        let next = self.slots.next()?;
        Some(self.node(next))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.slots.size_hint()
    }

    fn nth(&mut self, n: usize) -> Option<Node<'t>> {
        let next = self.slots.nth(n)?;
        Some(self.node(next))
    }
}

impl DoubleEndedIterator for Iter<'_> {
    fn next_back(&mut self) -> Option<Self::Item> {
        let next = self.slots.next_back()?;
        Some(self.node(next))
    }
}

impl ExactSizeIterator for Iter<'_> {}

impl std::iter::FusedIterator for Iter<'_> {}

/// What a sort of one directory's children compares.
#[derive(Clone, Copy, Debug)]
pub struct Sibling<'a> {
    facts: &'a Facts,
    pub name: &'a str,
}

impl Deref for Sibling<'_> {
    type Target = Facts;

    fn deref(&self) -> &Facts {
        self.facts
    }
}

/// Write access to one node of a tree, and through it to its subtree.
#[derive(Debug)]
pub struct NodeMut<'t> {
    tree: &'t mut Tree,
    id: NodeId,
}

impl fmt::Debug for Store {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Store")
            .field("nodes", &self.nodes.len())
            .field("names", &self.names.len())
            .finish_non_exhaustive()
    }
}

impl Deref for NodeMut<'_> {
    type Target = Facts;

    fn deref(&self) -> &Facts {
        &self.slot().facts
    }
}

impl NodeMut<'_> {
    fn slot(&self) -> &Slot {
        &self.tree.stores[usize::from(self.id.store)].nodes
            [self.id.index as usize]
    }

    fn slot_mut(&mut self) -> &mut Slot {
        &mut self.tree.stores[usize::from(self.id.store)].nodes
            [self.id.index as usize]
    }

    fn home(&self) -> u8 {
        self.tree.stores[usize::from(self.id.store)].homes
            [self.id.index as usize]
    }

    /// This node, to read.
    pub fn node(&self) -> Node<'_> {
        Node {
            tree: self.tree,
            slot: self.slot(),
            id: self.id,
        }
    }

    pub fn facts_mut(&mut self) -> &mut Facts {
        &mut self.slot_mut().facts
    }

    /// This node again, for a shorter borrow.
    pub const fn reborrow(&mut self) -> NodeMut<'_> {
        NodeMut {
            tree: self.tree,
            id: self.id,
        }
    }

    pub fn child_count(&self) -> usize {
        self.slot().len as usize
    }

    pub fn child_mut(&mut self, index: usize) -> Option<NodeMut<'_>> {
        let slot = *self.slot();
        let index = u32::try_from(index).ok().filter(|i| *i < slot.len)?;
        Some(NodeMut {
            id: NodeId {
                store: self.home(),
                index: slot.first + index,
            },
            tree: self.tree,
        })
    }

    /// Visit each child in order, one at a time.
    pub fn for_each_child_mut(&mut self, mut visit: impl FnMut(NodeMut<'_>)) {
        let slot = *self.slot();
        let store = self.home();
        for index in slot.first..slot.first + slot.len {
            visit(NodeMut {
                tree: self.tree,
                id: NodeId { store, index },
            });
        }
    }

    /// Reorder the children in place. Each child's own children stay
    /// where they are: a child records where its block is, and moves
    /// with that record.
    pub fn sort_children_by(
        &mut self,
        compare: impl Fn(Sibling<'_>, Sibling<'_>) -> Ordering,
    ) {
        let slot = *self.slot();
        let home = usize::from(self.home());
        let (names, mut parts) = self.tree.split();
        let block = slot.first as usize..(slot.first + slot.len) as usize;
        let Part { nodes, homes } = &mut parts[home];
        sort_block(
            &mut nodes[block.clone()],
            &mut homes[block],
            &names,
            &compare,
            &mut Scratch::default(),
        );
    }

    pub fn set_sharing(&mut self, sharing: Option<Sharing>) {
        let extra = self.slot().extra;
        let home = usize::from(self.home());
        let extra = self.tree.stores[home].set_sharing(extra, sharing);
        self.slot_mut().extra = extra;
    }
}

/// Reused by one worker across the blocks it sorts.
#[derive(Default)]
struct Scratch {
    order: Vec<u32>,
    sorted: Vec<(Slot, u8)>,
}

/// Sort one block of children and their homes together. The order is
/// worked out on indices and then applied, since the two live in separate
/// vectors; a block already in order, as after a re-rank that changes
/// nothing, is left unwritten.
fn sort_block(
    slots: &mut [Slot],
    homes: &mut [u8],
    names: &[&str],
    compare: &impl Fn(Sibling<'_>, Sibling<'_>) -> Ordering,
    scratch: &mut Scratch,
) {
    if slots.len() < 2 {
        return;
    }
    let sibling = |offset: u32| {
        let slot = &slots[offset as usize];
        Sibling {
            facts: &slot.facts,
            name: name_at(
                names[usize::from(homes[offset as usize])],
                slot.name,
            ),
        }
    };
    let order = &mut scratch.order;
    order.clear();
    order.extend(0..Store::index(slots.len()));
    order.sort_unstable_by(|&left, &right| {
        compare(sibling(left), sibling(right))
    });
    if order
        .iter()
        .enumerate()
        .all(|(at, &from)| from as usize == at)
    {
        return;
    }
    let sorted = &mut scratch.sorted;
    sorted.clear();
    sorted.extend(
        order
            .iter()
            .map(|&from| (slots[from as usize], homes[from as usize])),
    );
    for ((slot, home), &(sorted_slot, sorted_home)) in
        slots.iter_mut().zip(homes.iter_mut()).zip(sorted.iter())
    {
        *slot = sorted_slot;
        *home = sorted_home;
    }
}

/// One store's nodes and homes, to write, while every store's names are
/// read.
struct Part<'a> {
    nodes: &'a mut [Slot],
    homes: &'a mut [u8],
}

/// Sort the blocks of one store, which start at `blocks`' firsts, in
/// order: halves of the store go to separate workers until each has a
/// few hundred blocks, so a tree built on one thread, as an SSH scan's
/// is, still sorts on every worker.
fn sort_blocks(
    nodes: &mut [Slot],
    homes: &mut [u8],
    base: u32,
    blocks: &[(u32, u32)],
    names: &[&str],
    compare: &(impl Fn(Sibling<'_>, Sibling<'_>) -> Ordering + Sync),
) {
    const ALONE: usize = 256;
    if blocks.len() <= ALONE {
        let mut scratch = Scratch::default();
        for &(first, len) in blocks {
            let start = (first - base) as usize;
            let end = start + len as usize;
            sort_block(
                &mut nodes[start..end],
                &mut homes[start..end],
                names,
                compare,
                &mut scratch,
            );
        }
        return;
    }
    let (left, right) = blocks.split_at(blocks.len() / 2);
    let middle = right[0].0;
    let at = (middle - base) as usize;
    let (left_nodes, right_nodes) = nodes.split_at_mut(at);
    let (left_homes, right_homes) = homes.split_at_mut(at);
    rayon::join(
        || sort_blocks(left_nodes, left_homes, base, left, names, compare),
        || sort_blocks(right_nodes, right_homes, middle, right, names, compare),
    );
}

/// Updates a read-only pass leaves for each run of its work, so that none
/// is copied into another.
type Updates<T> = Mutex<Vec<Vec<(NodeId, T)>>>;

/// One store's part of a [`Tree::par_write`]: nodes by index, and blocks
/// whose files all take a value.
struct Targets<T> {
    nodes: Vec<(u32, T)>,
    files: Vec<(u32, u32, T)>,
}

impl<T> Default for Targets<T> {
    fn default() -> Self {
        Self {
            nodes: Vec::new(),
            files: Vec::new(),
        }
    }
}

/// `node`'s facts once `derive` has worked out every directory beneath
/// it, each directory's new facts left in `updates`.
fn derive_at<D>(
    node: Node<'_>,
    depth: usize,
    derive: &D,
    updates: &mut Vec<(NodeId, Facts)>,
    sink: &Updates<Facts>,
) -> Facts
where
    D: Fn(&mut Facts, &mut dyn Iterator<Item = Facts>) + Sync,
{
    let mut facts = *node.facts();
    if !facts.kind.is_dir() {
        return facts;
    }
    let children = node.children();
    if depth < crate::tree::PARALLEL_LEVELS && children.len() > 1 {
        // Subdirectories near the top are independent work.
        let derived: Vec<Facts> = children
            .par_iter()
            .map(|child| {
                let mut mine = Vec::new();
                let facts =
                    derive_at(child, depth + 1, derive, &mut mine, sink);
                if !mine.is_empty() {
                    lock(sink).push(mine);
                }
                facts
            })
            .collect();
        derive(&mut facts, &mut derived.into_iter());
    } else {
        let mut derived = children
            .iter()
            .map(|child| derive_at(child, depth + 1, derive, updates, sink));
        derive(&mut facts, &mut derived);
        // Every subdirectory is derived, even past where `derive` stopped.
        derived.for_each(drop);
    }
    updates.push((node.id(), facts));
    facts
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Where a tree is built, by any number of workers at once.
///
/// Each worker appends to a store of its own, so none waits on another,
/// and the finished tree keeps the stores. `'h` brands what it builds
/// with the build it belongs to.
pub struct Builder {
    /// One per worker of the pool the build starts on, and one more for
    /// any other thread. Each has a lock only so that a thread of another
    /// pool, whose worker number can match, cannot write to it at the same
    /// time; a pool's own workers never meet on one.
    stores: Box<[Mutex<Store>]>,
}

impl Default for Builder {
    fn default() -> Self {
        // A store's number is a byte.
        let workers = rayon::current_num_threads().min(usize::from(u8::MAX));
        Self {
            stores: std::iter::repeat_with(Mutex::default)
                .take(workers + 1)
                .collect(),
        }
    }
}

impl fmt::Debug for Builder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Builder")
            .field("stores", &self.stores.len())
            .finish()
    }
}

/// A node one build has made, not yet placed in its tree.
#[must_use]
pub struct Built<'h> {
    slot: Slot,
    /// The store that made it.
    home: u8,
    /// Invariant: a node of one build cannot be taken for another's.
    brand: PhantomData<fn(&'h ()) -> &'h ()>,
}

impl fmt::Debug for Built<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.slot.fmt(f)
    }
}

impl Deref for Built<'_> {
    type Target = Facts;

    fn deref(&self) -> &Facts {
        &self.slot.facts
    }
}

impl Built<'_> {
    pub const fn facts_mut(&mut self) -> &mut Facts {
        &mut self.slot.facts
    }
}

impl Builder {
    /// This thread's store and its number.
    fn store(&self) -> (MutexGuard<'_, Store>, u8) {
        let other = self.stores.len() - 1;
        let home = rayon::current_thread_index().map_or(other, |w| w % other);
        (
            lock(&self.stores[home]),
            u8::try_from(home).expect("at most 256 stores"),
        )
    }

    /// Run `make` on this thread's store, every node it adds made there.
    fn make(&self, make: impl FnOnce(&mut Store, u8) -> Slot) -> Built<'_> {
        let (mut store, home) = self.store();
        let slot = make(&mut store, home);
        let len = store.nodes.len();
        store.homes.resize(len, home);
        Self::built(slot, home)
    }

    const fn built<'h>(slot: Slot, home: u8) -> Built<'h> {
        Built {
            slot,
            home,
            brand: PhantomData,
        }
    }

    /// A leaf, or a directory that holds nothing.
    pub fn leaf<'h>(
        &'h self,
        name: &str,
        facts: Facts,
        allocation: Option<Allocation>,
    ) -> Built<'h> {
        self.make(|store, _| {
            store.place(name, facts, allocation, None, std::iter::empty())
        })
    }

    /// Several leaves at once: how a listing adds its files.
    pub fn leaves<'h, 'a>(
        &'h self,
        leaves: impl IntoIterator<Item = (&'a str, Facts, Option<Allocation>)>,
    ) -> Vec<Built<'h>> {
        let (mut store, home) = self.store();
        leaves
            .into_iter()
            .map(|(name, facts, allocation)| {
                let slot = store.place(
                    name,
                    facts,
                    allocation,
                    None,
                    std::iter::empty(),
                );
                Self::built(slot, home)
            })
            .collect()
    }

    /// A directory holding `children`, which become one block of the tree
    /// in the order given.
    pub fn directory<'h>(
        &'h self,
        name: &str,
        facts: Facts,
        allocation: Option<Allocation>,
        children: Vec<Built<'h>>,
    ) -> Built<'h> {
        let (mut store, home) = self.store();
        // Each child keeps the store it was made in; the block is here.
        let slot = store.place(
            name,
            facts,
            allocation,
            None,
            children.into_iter().map(|child| (child.slot, child.home)),
        );
        Self::built(slot, home)
    }

    /// A deep copy of `node`, from any tree, into this build: how a
    /// subtree moves between trees.
    pub fn copy<'h>(&'h self, node: Node<'_>) -> Built<'h> {
        self.copy_named(node, node.name())
    }

    /// [`Self::copy`], under another name.
    pub fn copy_named<'h>(&'h self, node: Node<'_>, name: &str) -> Built<'h> {
        self.make(|store, me| {
            // A whole tree, as a wider scan reuses one, goes in as its
            // buffers.
            if node.is_root() {
                store.absorb(node.tree, name, me)
            } else {
                store.copy(node, name, me)
            }
        })
    }

    /// `draft`, with everything beneath it.
    pub fn draft<'h>(&'h self, draft: &Draft) -> Built<'h> {
        self.make(|store, me| store.draft(draft, me))
    }
}

/// A scanned tree and the buffers its nodes live in.
pub struct Tree {
    /// One per worker that built it, and one for any other thread.
    stores: Box<[Store]>,
    root: NodeId,
}

impl Clone for Tree {
    fn clone(&self) -> Self {
        Self {
            stores: self.stores.par_iter().map(Store::clone).collect(),
            root: self.root,
        }
    }
}

impl fmt::Debug for Tree {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Tree")
            .field("root", &self.root())
            .finish_non_exhaustive()
    }
}

impl From<Draft> for Tree {
    fn from(draft: Draft) -> Self {
        Self::from(&draft)
    }
}

impl From<&Draft> for Tree {
    fn from(draft: &Draft) -> Self {
        Self::build(|builder| builder.draft(draft))
    }
}

impl Tree {
    /// Build a tree: `build` makes nodes with the builder it is given and
    /// returns the root. The brand keeps every node it makes in this tree.
    ///
    /// A node of one build cannot become part of another tree, where its
    /// indices would name other nodes:
    ///
    /// ```compile_fail
    /// use disktree_core::tree::{Facts, NodeKind, Tree};
    /// let mut smuggled = None;
    /// let _first = Tree::build(|builder| {
    ///     smuggled =
    ///         Some(builder.leaf("x", Facts::entry(NodeKind::File, 1), None));
    ///     builder.leaf("root", Facts::directory(), None)
    /// });
    /// let _second = Tree::build(|_| smuggled.take().unwrap());
    /// ```
    pub fn build(build: impl for<'h> FnOnce(&'h Builder) -> Built<'h>) -> Self {
        let builder = Builder::default();
        let Built { slot, home, .. } = build(&builder);
        Self::finish(builder, slot, home)
    }

    /// [`Self::build`], for a build that can fail.
    pub fn try_build<E>(
        build: impl for<'h> FnOnce(&'h Builder) -> Result<Built<'h>, E>,
    ) -> Result<Self, E> {
        let builder = Builder::default();
        let Built { slot, home, .. } = build(&builder)?;
        Ok(Self::finish(builder, slot, home))
    }

    /// The builder's stores as they are, with the root in the store that
    /// made it.
    fn finish(builder: Builder, root: Slot, home: u8) -> Self {
        let mut stores: Box<[Store]> = builder
            .stores
            .into_vec()
            .into_iter()
            .map(|store| {
                store.into_inner().unwrap_or_else(PoisonError::into_inner)
            })
            .collect();
        let store = &mut stores[usize::from(home)];
        let index = Store::index(store.nodes.len());
        store.nodes.push(root);
        store.homes.push(home);
        Self {
            stores,
            root: NodeId { store: home, index },
        }
    }

    /// A tree of its own holding a copy of `node` and everything beneath:
    /// every store at once when `node` is a root.
    pub fn copy_of(node: Node<'_>) -> Self {
        if node.is_root() {
            return node.tree.clone();
        }
        Self::build(|builder| builder.copy(node))
    }

    pub fn root(&self) -> Node<'_> {
        Node {
            tree: self,
            slot: &self.stores[usize::from(self.root.store)].nodes
                [self.root.index as usize],
            id: self.root,
        }
    }

    pub const fn root_mut(&mut self) -> NodeMut<'_> {
        NodeMut {
            id: self.root,
            tree: self,
        }
    }

    /// The node at `id`, which [`Node::id`] gave on this tree, unchanged
    /// since.
    pub(crate) const fn node_mut(&mut self, id: NodeId) -> NodeMut<'_> {
        NodeMut { tree: self, id }
    }

    pub fn storage_bytes(&self) -> StorageBytes {
        self.stores.iter().map(Store::bytes).fold(
            StorageBytes::default(),
            |sum, part| StorageBytes {
                used: sum.used + part.used,
                reserved: sum.reserved + part.reserved,
            },
        )
    }

    /// Replace the sharing figures of the nodes at these positions, which
    /// [`Node::id`] gave on this tree, unchanged since.
    pub(crate) fn set_sharing(
        &mut self,
        figures: impl IntoIterator<Item = (NodeId, Option<Sharing>)>,
    ) {
        for (id, sharing) in figures {
            self.node_mut(id).set_sharing(sharing);
        }
    }

    /// Every store's nodes and homes to write, beside every store's names
    /// to read.
    fn split(&mut self) -> (Vec<&str>, Vec<Part<'_>>) {
        let mut names = Vec::with_capacity(self.stores.len());
        let mut parts = Vec::with_capacity(self.stores.len());
        for store in &mut self.stores {
            let Store {
                nodes,
                homes,
                names: text,
                ..
            } = store;
            names.push(text.as_str());
            parts.push(Part { nodes, homes });
        }
        (names, parts)
    }

    /// Write `value` with `write` to each node these name, and, where the
    /// flag is set, to every child of it that is not a directory: what a
    /// pass that decided by reading leaves to write. Each store's writes
    /// run on their own worker, which is why a directory's files are named
    /// once by their directory rather than one by one.
    pub(crate) fn par_write<T: Copy + Send + Sync>(
        &mut self,
        writes: Vec<Vec<(NodeId, T, bool)>>,
        write: impl Fn(&mut Facts, T) + Sync,
    ) {
        let stores = self.stores.len();
        // Sorted into the stores the writes land in, a node in the store
        // that holds it and its files in the store of its block: one set
        // of lists per run of work, however many lists came in.
        let sorted: Vec<Vec<Targets<T>>> = writes
            .into_par_iter()
            .flatten_iter()
            .fold(
                || {
                    std::iter::repeat_with(Targets::default)
                        .take(stores)
                        .collect::<Vec<_>>()
                },
                |mut by_store, (id, value, files)| {
                    by_store[usize::from(id.store)]
                        .nodes
                        .push((id.index, value));
                    if files {
                        let node = self.node(id);
                        by_store[usize::from(node.home())].files.push((
                            node.slot.first,
                            node.slot.len,
                            value,
                        ));
                    }
                    by_store
                },
            )
            .collect();
        self.stores
            .par_iter_mut()
            .enumerate()
            .for_each(|(at, store)| {
                for targets in &sorted {
                    let targets = &targets[at];
                    for &(index, value) in &targets.nodes {
                        write(&mut store.nodes[index as usize].facts, value);
                    }
                    for &(first, len, value) in &targets.files {
                        let block = first as usize..(first + len) as usize;
                        store.nodes[block]
                            .iter_mut()
                            .filter(|slot| !slot.facts.kind.is_dir())
                            .for_each(|slot| write(&mut slot.facts, value));
                    }
                }
            });
    }

    /// Every node, on every worker, ordered by [`Node::id`]. A node that
    /// nothing reaches, such as the root of a tree a build absorbed, is
    /// among them.
    pub(crate) fn par_nodes(&self) -> impl ParallelIterator<Item = Node<'_>> {
        self.stores
            .par_iter()
            .enumerate()
            .flat_map(move |(at, store)| {
                let at = u8::try_from(at).expect("at most 256 stores");
                store
                    .nodes
                    .par_iter()
                    .enumerate()
                    .map(move |(index, slot)| Node {
                        tree: self,
                        slot,
                        id: NodeId {
                            store: at,
                            index: Store::index(index),
                        },
                    })
            })
    }

    fn node(&self, id: NodeId) -> Node<'_> {
        Node {
            tree: self,
            slot: &self.stores[usize::from(id.store)].nodes[id.index as usize],
            id,
        }
    }

    /// Every node that is not a directory, with `leaf`, on every worker.
    pub fn par_leaves(&mut self, leaf: impl Fn(&mut Facts) + Sync) {
        self.stores.par_iter_mut().for_each(|store| {
            store
                .nodes
                .par_iter_mut()
                .filter(|slot| !slot.facts.kind.is_dir())
                .for_each(|slot| leaf(&mut slot.facts));
        });
    }

    /// Each directory's facts from its children's, bottom-up: `derive`
    /// gets a directory's facts to change and its children's, already
    /// derived. Worked out on every worker by reading, then written.
    pub fn par_derive(
        &mut self,
        derive: impl Fn(&mut Facts, &mut dyn Iterator<Item = Facts>) + Sync,
    ) {
        let sink = Updates::default();
        let mut updates = Vec::new();
        derive_at(self.root(), 0, &derive, &mut updates, &sink);
        let mut lists =
            sink.into_inner().unwrap_or_else(PoisonError::into_inner);
        lists.push(updates);
        for (id, facts) in lists.into_iter().flatten() {
            *self.node_mut(id).facts_mut() = facts;
        }
    }

    /// Reorder every directory's children by `compare`, each store's
    /// directories on their own workers. Each child's own children stay
    /// where they are: a child records where its block is, and moves with
    /// that record.
    pub fn sort_children(
        &mut self,
        compare: impl Fn(Sibling<'_>, Sibling<'_>) -> Ordering + Sync,
    ) {
        // Every block worth sorting, by the store that holds it: found by
        // reading each store's directories, wherever they are held.
        let found: Vec<Vec<(u8, u32, u32)>> = self
            .stores
            .par_iter()
            .map(|store| {
                store
                    .nodes
                    .iter()
                    .zip(&store.homes)
                    .filter(|(slot, _)| slot.len > 1)
                    .map(|(slot, &home)| (home, slot.first, slot.len))
                    .collect()
            })
            .collect();
        let mut blocks: Vec<Vec<(u32, u32)>> =
            vec![Vec::new(); self.stores.len()];
        for (home, first, len) in found.into_iter().flatten() {
            blocks[usize::from(home)].push((first, len));
        }
        blocks.par_iter_mut().for_each(|list| list.sort_unstable());
        let (names, parts) = self.split();
        parts.into_par_iter().zip(blocks).for_each(|(part, list)| {
            sort_blocks(part.nodes, part.homes, 0, &list, &names, &compare);
        });
    }

    /// Records in each side table, for measuring what they cost.
    pub fn side_records(&self) -> (usize, usize, usize) {
        self.stores.iter().fold((0, 0, 0), |(a, s, b), store| {
            (
                a + store.allocations.len(),
                s + store.sharings.len(),
                b + store.both.len(),
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(bytes: u64) -> Facts {
        Facts::entry(NodeKind::File, bytes)
    }

    fn names(node: Node<'_>) -> Vec<&str> {
        node.children().iter().map(Node::name).collect()
    }

    fn sample() -> Tree {
        Tree::build(|builder| {
            let deep = builder.leaf("deep", file(7), None);
            let nested = builder.directory(
                "child",
                Facts::directory(),
                None,
                vec![deep],
            );
            let direct = builder.leaf(
                "direct",
                file(5),
                Some(Allocation {
                    bytes: 5,
                    identity: None,
                    links: 1,
                    followed: false,
                    clone: None,
                }),
            );
            let empty = builder.leaf("", file(0), None);
            builder.directory(
                "root",
                Facts::directory(),
                None,
                vec![nested, direct, empty],
            )
        })
    }

    /// A home directory is tens of millions of nodes: every byte here is
    /// tens of megabytes there.
    #[test]
    fn a_node_stays_small() {
        // Facts, then a name offset, the first child, the child count and
        // one side-table index.
        assert_eq!(size_of::<Facts>(), 48);
        assert_eq!(size_of::<Slot>(), 64);
    }

    #[test]
    fn a_built_tree_reads_back() {
        let tree = sample();
        let root = tree.root();
        assert_eq!(root.name(), "root");
        assert_eq!(names(root), ["child", "direct", ""]);
        let deep = root.children().at(0).children().at(0);
        assert_eq!(deep.name(), "deep");
        assert_eq!(deep.bytes, 7);
        assert_eq!(
            root.children().at(1).allocation().map(|a| a.bytes),
            Some(5)
        );
        assert!(root.children().at(2).children().is_empty());
    }

    #[test]
    fn a_long_name_and_many_children_survive() {
        let long = "é".repeat(300);
        let tree = Tree::build(|builder| {
            let children = (0..1000)
                .map(|index| {
                    builder.leaf(&format!("{index}"), file(index), None)
                })
                .chain(std::iter::once(builder.leaf(&long, file(1), None)))
                .collect();
            builder.directory("root", Facts::directory(), None, children)
        });
        let children = tree.root().children();
        assert_eq!(children.len(), 1001);
        assert_eq!(children.at(999).name(), "999");
        assert_eq!(children.at(999).bytes, 999);
        assert_eq!(children.at(1000).name(), long);
    }

    #[test]
    fn writes_reach_the_tree_and_children_reorder_in_place() {
        let mut tree = sample();
        let mut root = tree.root_mut();
        root.facts_mut().bytes = 12;
        root.for_each_child_mut(|mut child| {
            child.facts_mut().read_error = true;
        });
        root.sort_children_by(|left, right| left.name.cmp(right.name));
        root.child_mut(0).expect("a child").facts_mut().modified = 9;
        let root = tree.root();
        assert_eq!(root.bytes, 12);
        assert_eq!(names(root), ["", "child", "direct"]);
        assert_eq!(root.children().at(0).modified, 9);
        assert!(root.children().iter().all(|child| child.read_error));
        assert_eq!(root.children().at(1).children().at(0).name(), "deep");
    }

    #[test]
    fn sharing_is_rewritten_in_place() {
        let mut tree = sample();
        let mut root = tree.root_mut();
        root.set_sharing(Some(Sharing::default()));
        let first = root.node().stored_sharing().map(std::ptr::from_ref);
        root.set_sharing(Some(Sharing {
            files: 3,
            ..Sharing::default()
        }));
        assert_eq!(root.node().stored_sharing().map(std::ptr::from_ref), first);
        assert_eq!(tree.root().stored_sharing().map(|s| s.files), Some(3));
        tree.root_mut().set_sharing(None);
        assert!(tree.root().stored_sharing().is_none());
    }

    #[test]
    fn a_node_can_hold_an_allocation_and_sharing() {
        let mut tree = sample();
        let mut root = tree.root_mut();
        let mut direct = root.child_mut(1).expect("direct");
        // What aggregating does to every file: no figures to keep must
        // not cost the file its allocation.
        direct.set_sharing(None);
        assert_eq!(direct.node().allocation().map(|a| a.bytes), Some(5));
        direct.set_sharing(Some(Sharing {
            files: 2,
            ..Sharing::default()
        }));
        let node = direct.node();
        assert_eq!(node.allocation().map(|a| a.bytes), Some(5));
        assert_eq!(node.stored_sharing().map(|s| s.files), Some(2));
        direct.set_sharing(None);
        assert_eq!(direct.node().allocation().map(|a| a.bytes), Some(5));
        assert!(direct.node().stored_sharing().is_none());
    }

    #[test]
    fn a_copy_outlives_its_source() {
        let source = sample();
        let copy = source.clone();
        drop(source);
        assert_eq!(copy.root().children().len(), 3);
        assert_eq!(
            copy.root().children().at(0).children().at(0).name(),
            "deep"
        );
        let subtree = {
            let source = sample();
            Tree::copy_of(source.root().children().at(0))
        };
        assert_eq!(subtree.root().name(), "child");
        assert_eq!(subtree.root().children().len(), 1);
    }

    /// Everything below two nodes reads the same, names included.
    fn assert_same(left: Node<'_>, right: Node<'_>) {
        assert_eq!(left.bytes, right.bytes, "{}", left.name());
        assert_eq!(left.kind, right.kind, "{}", left.name());
        assert_eq!(
            left.allocation().map(|a| a.bytes),
            right.allocation().map(|a| a.bytes),
            "{}",
            left.name()
        );
        assert_eq!(
            left.stored_sharing().map(|s| s.files),
            right.stored_sharing().map(|s| s.files),
            "{}",
            left.name()
        );
        assert_eq!(names(left), names(right));
        for (left, right) in left.children().iter().zip(right.children()) {
            assert_same(left, right);
        }
    }

    #[test]
    fn a_whole_tree_copied_into_a_build_keeps_its_records() {
        let mut source = sample();
        let shared = |files| {
            Some(Sharing {
                files,
                ..Sharing::default()
            })
        };
        let mut root = source.root_mut();
        root.set_sharing(shared(1));
        // `direct` already has an allocation: this puts it in `both`.
        root.child_mut(1).expect("direct").set_sharing(shared(2));
        root.child_mut(0).expect("child").set_sharing(shared(3));
        let allocation = Allocation {
            bytes: 9,
            identity: None,
            links: 1,
            followed: false,
            clone: None,
        };
        let tree = Tree::build(|builder| {
            // Records already in every table, so the copy's must move.
            let mut before = builder.leaf("before", file(9), Some(allocation));
            before.slot.extra =
                builder.store().0.set_sharing(before.slot.extra, shared(4));
            let other = builder.leaf("other", file(1), None);
            let other = builder.directory(
                "other-dir",
                Facts::directory(),
                None,
                vec![other],
            );
            let reused = builder.copy_named(source.root(), "reused");
            builder.directory(
                "top",
                Facts::directory(),
                None,
                vec![before, other, reused],
            )
        });
        let reused = tree.root().children().at(2);
        assert_eq!(reused.name(), "reused");
        assert_same(reused, source.root());
        let before = tree.root().children().at(0);
        assert_eq!(before.allocation().map(|a| a.bytes), Some(9));
        assert_eq!(before.stored_sharing().map(|s| s.files), Some(4));
        // The source's five, its root included though nothing reaches
        // it, then `top`'s block, the block
        // in `other-dir` and `top` itself.
        let nodes: usize =
            tree.stores.iter().map(|store| store.nodes.len()).sum();
        assert_eq!(nodes, 5 + 3 + 1 + 1);
    }

    #[test]
    fn a_tree_moves_to_another_thread() {
        let tree = sample();
        let count = std::thread::spawn(move || tree.root().children().len())
            .join()
            .expect("joined");
        assert_eq!(count, 3);
    }

    /// A directory's children are made on whichever workers listed them,
    /// and the directory on another: every index has to survive the join.
    #[test]
    fn nodes_from_every_worker_join_into_one_tree() {
        let known = sample();
        let tree = Tree::build(|builder| {
            let files: Vec<Built<'_>> = (0..2000_u64)
                .into_par_iter()
                .map(|index| {
                    let allocation =
                        index.is_multiple_of(3).then_some(Allocation {
                            bytes: index,
                            identity: None,
                            links: 1,
                            followed: false,
                            clone: None,
                        });
                    builder.leaf(&format!("f{index}"), file(index), allocation)
                })
                .collect();
            let mut files = files.into_iter();
            let groups: Vec<Vec<Built<'_>>> = (0..20)
                .map(|_| files.by_ref().take(100).collect())
                .collect();
            let mut dirs: Vec<Built<'_>> = groups
                .into_par_iter()
                .rev()
                .enumerate()
                .map(|(dir, group)| {
                    builder.directory(
                        &format!("d{}", 19 - dir),
                        Facts::directory(),
                        None,
                        group,
                    )
                })
                .collect();
            dirs.reverse();
            let (copy, ()) = rayon::join(
                || builder.copy_named(known.root(), "known"),
                || {},
            );
            dirs.push(copy);
            builder.directory("root", Facts::directory(), None, dirs)
        });
        let root = tree.root();
        assert_eq!(root.children().len(), 21);
        for (dir, node) in root.children().iter().take(20).enumerate() {
            assert_eq!(node.name(), format!("d{dir}"));
            for (offset, file) in node.children().iter().enumerate() {
                let index = (dir * 100 + offset) as u64;
                assert_eq!(file.name(), format!("f{index}"));
                assert_eq!(file.bytes, index);
                assert_eq!(
                    file.allocation().map(|a| a.bytes),
                    index.is_multiple_of(3).then_some(index)
                );
            }
        }
        let copy = root.children().at(20);
        assert_eq!(copy.name(), "known");
        assert_same(copy, known.root());
    }

    #[test]
    fn a_tree_built_on_every_worker_holds_every_node() {
        let tree = Tree::build(|builder| {
            let children = (0..64)
                .into_par_iter()
                .map(|dir| {
                    let names: Vec<String> = (0..100)
                        .map(|index| format!("{dir}-{index}"))
                        .collect();
                    let files = builder.leaves(
                        names.iter().map(|name| (name.as_str(), file(1), None)),
                    );
                    builder.directory(
                        &format!("{dir}"),
                        Facts::directory(),
                        None,
                        files,
                    )
                })
                .collect();
            builder.directory("root", Facts::directory(), None, children)
        });
        let root = tree.root();
        assert_eq!(root.children().len(), 64);
        assert!(root.children().iter().enumerate().all(|(dir, child)| {
            child.children().len() == 100
                && child.children().at(99).name() == format!("{dir}-99")
        }));
        let bytes = tree.storage_bytes();
        assert!(bytes.used > 0 && bytes.used <= bytes.reserved);
    }
}
