//! How a scanned tree is held: every node's name, child list and boxes
//! live in bump arenas that the [`Tree`] owns.
//!
//! A tree of separately allocated nodes is freed one `free()` at a time:
//! forty million calls for a 20-million-node home directory, each writing
//! to the page it frees, so a drop reads the whole tree back in, from the
//! compressor if macOS has packed it away. On such a tree that took 0.8 to
//! 0.9 s in a fresh process and 15 s in a window under memory pressure.
//! Nodes here own nothing that needs a destructor, so a drop releases a
//! few hundred large chunks without reading one of them: 0.04 s, with no
//! page faults. The same tree also took a sixth less memory, since a name
//! or a child list is sized to the byte instead of to malloc's size
//! classes, and copying it for the metric toggle took 0.1 s instead of
//! 1.3 s. `examples/tree_memory.rs` measures all three.
//!
//! Soundness rests on three rules, each held by the types:
//!
//! * A [`Node`] is only reachable by reference through the [`Tree`] that
//!   owns its arena, so nothing can read its pointers after the arena is
//!   gone. [`Name`], [`Children`] and [`Boxed`] are neither `Copy` nor
//!   `Clone`, so a field cannot be taken out of a borrowed node.
//! * Nodes are made only inside [`Tree::build`], as [`Built`] values
//!   branded with that build's lifetime, so one tree's nodes cannot be
//!   placed in another's: a subtree moves between trees only as a copy.
//! * Writes go through [`NodeMut`], which changes facts and the order of
//!   children but never hands out `&mut Node`; with one, `mem::swap` could
//!   trade nodes between two trees.

#![allow(
    unsafe_code,
    reason = "nodes hold raw pointers into their tree's arena; each block \
              says why it is sound"
)]

use std::alloc::Layout;
use std::fmt;
use std::marker::PhantomData;
use std::num::NonZeroU64;
use std::ops::Deref;
use std::ptr::NonNull;
use std::sync::Mutex;

use bumpalo::Bump;
use rayon::prelude::*;

use crate::classify::{Category, Reclaim};
use crate::sharing::{Allocation, Sharing};
use crate::tree::{Draft, NodeKind, PARALLEL_LEVELS};

/// Everything about a node that is plain data, and so free to write.
///
/// `bytes` and `files` are the subtree totals the treemap draws. A disk
/// holds tens of millions of nodes, so every field is paid that many
/// times: the size of [`Node`] is held by a test.
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
}

/// A node's name: UTF-8 in its tree's arena, just after its length.
///
/// Keeping the length beside the bytes makes a name one pointer in the
/// node instead of two words.
pub struct Name(NonNull<u8>);

/// The length before a name's bytes. A file name is at most a few hundred
/// bytes, even decoded lossily.
const NAME_HEADER: usize = size_of::<u32>();

impl Deref for Name {
    type Target = str;

    fn deref(&self) -> &str {
        // SAFETY: `alloc_name` wrote a `u32` length and then that many bytes
        // of a `&str`, in this tree's arena, which outlives `&self`.
        unsafe {
            let bytes = self.0.as_ptr();
            let len = bytes.sub(NAME_HEADER).cast::<u32>().read_unaligned();
            std::str::from_utf8_unchecked(std::slice::from_raw_parts(
                bytes,
                len as usize,
            ))
        }
    }
}

impl AsRef<str> for Name {
    fn as_ref(&self) -> &str {
        self
    }
}

impl fmt::Display for Name {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&**self, f)
    }
}

impl fmt::Debug for Name {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&**self, f)
    }
}

impl PartialEq for Name {
    fn eq(&self, other: &Self) -> bool {
        **self == **other
    }
}

impl Eq for Name {}

impl PartialEq<str> for Name {
    fn eq(&self, other: &str) -> bool {
        &**self == other
    }
}

impl PartialEq<&str> for Name {
    fn eq(&self, other: &&str) -> bool {
        &**self == *other
    }
}

impl PartialOrd for Name {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Name {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (**self).cmp(&**other)
    }
}

/// A directory's children: nodes in its tree's arena, just after their
/// count. Empty, which nearly every node is, costs no allocation.
pub struct Children(Option<NonNull<Node>>);

/// The count before a child list, padded so the nodes after it are
/// aligned.
const CHILDREN_HEADER: usize = if align_of::<Node>() > size_of::<usize>() {
    align_of::<Node>()
} else {
    size_of::<usize>()
};

impl Children {
    #[allow(
        clippy::cast_ptr_alignment,
        reason = "the count sits `CHILDREN_HEADER` bytes before an aligned \
                  node, and that is a multiple of a `usize`'s alignment"
    )]
    const fn as_ptr_len(&self) -> (*mut Node, usize) {
        match self.0 {
            None => (NonNull::dangling().as_ptr(), 0),
            // SAFETY: `alloc_children` wrote the count `CHILDREN_HEADER`
            // bytes before the first node, aligned for a `usize`.
            Some(first) => unsafe {
                let len = first
                    .as_ptr()
                    .cast::<u8>()
                    .sub(CHILDREN_HEADER)
                    .cast::<usize>()
                    .read();
                (first.as_ptr(), len)
            },
        }
    }
}

impl Deref for Children {
    type Target = [Node];

    fn deref(&self) -> &[Node] {
        let (first, len) = self.as_ptr_len();
        // SAFETY: `len` initialised nodes from one allocation in this tree's
        // arena, which outlives `&self`; or empty, from a dangling pointer.
        unsafe { std::slice::from_raw_parts(first, len) }
    }
}

impl<'a> IntoIterator for &'a Children {
    type Item = &'a Node;
    type IntoIter = std::slice::Iter<'a, Node>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl fmt::Debug for Children {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list().entries(self.iter()).finish()
    }
}

/// A value in its tree's arena, as `Box` would hold it on the heap.
pub struct Boxed<T>(NonNull<T>);

impl<T> Deref for Boxed<T> {
    type Target = T;

    fn deref(&self) -> &T {
        // SAFETY: an initialised value in this tree's arena, which outlives
        // `&self`, and is only written through `NodeMut`, which borrows the
        // tree exclusively.
        unsafe { self.0.as_ref() }
    }
}

impl<T: fmt::Debug> fmt::Debug for Boxed<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&**self, f)
    }
}

/// One entry in the scanned tree. Read its facts through `Deref`; reach it
/// only through its [`Tree`].
pub struct Node {
    facts: Facts,
    pub name: Name,
    /// Children, ordered by [`crate::tree::Metric`] value, descending.
    pub children: Children,
    pub allocation: Option<Boxed<Allocation>>,
    /// Derived clone figures, sparse: see [`Node::sharing`].
    pub sharing: Option<Boxed<Sharing>>,
}

// SAFETY: a node's pointers name memory in its tree's arena, which lives
// as long as the tree and carries no thread affinity. It is read through
// `&Node` and written only through `NodeMut`, which borrows the tree
// exclusively, so sharing or sending a tree is sharing or sending plain
// data.
unsafe impl Send for Node {}
// SAFETY: as above; shared access only reads.
unsafe impl Sync for Node {}
// SAFETY: these are parts of a node, readable from any thread for the
// reasons above.
unsafe impl Send for Name {}
// SAFETY: as for `Name: Send`.
unsafe impl Sync for Name {}
// SAFETY: as for `Name: Send`.
unsafe impl Send for Children {}
// SAFETY: as for `Name: Send`.
unsafe impl Sync for Children {}
// SAFETY: as for `Name: Send`, for any value that may itself be shared.
unsafe impl<T: Sync> Send for Boxed<T> {}
// SAFETY: as for `Name: Send`.
unsafe impl<T: Sync> Sync for Boxed<T> {}

impl Deref for Node {
    type Target = Facts;

    fn deref(&self) -> &Facts {
        &self.facts
    }
}

impl Node {
    /// What `Deref` reads, for a `const fn`.
    pub const fn facts(&self) -> &Facts {
        &self.facts
    }
}

impl fmt::Debug for Node {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Node")
            .field("name", &self.name)
            .field("facts", &self.facts)
            .field("allocation", &self.allocation)
            .field("sharing", &self.sharing)
            .field("children", &self.children)
            .finish()
    }
}

/// Write access to one node: its facts, its sharing figures and the
/// order of its children, which stay its own.
#[derive(Debug)]
pub struct NodeMut<'t> {
    node: &'t mut Node,
    herd: &'t Herd,
}

impl Deref for NodeMut<'_> {
    type Target = Node;

    fn deref(&self) -> &Node {
        self.node
    }
}

impl NodeMut<'_> {
    pub const fn facts_mut(&mut self) -> &mut Facts {
        &mut self.node.facts
    }

    /// This node again, for a shorter borrow.
    pub const fn reborrow(&mut self) -> NodeMut<'_> {
        NodeMut {
            node: self.node,
            herd: self.herd,
        }
    }

    #[allow(
        clippy::needless_pass_by_ref_mut,
        reason = "the slice is written through a pointer, and borrowing \
                  `self` exclusively is what makes that sound"
    )]
    const fn children_slice(&mut self) -> &mut [Node] {
        let (first, len) = self.node.children.as_ptr_len();
        // SAFETY: a child list belongs to one parent alone, since every
        // builder allocates a fresh one, so exclusive access to the parent
        // is exclusive access to its children.
        unsafe { std::slice::from_raw_parts_mut(first, len) }
    }

    pub fn children_mut(
        &mut self,
    ) -> impl ExactSizeIterator<Item = NodeMut<'_>> + DoubleEndedIterator {
        let herd = self.herd;
        self.children_slice()
            .iter_mut()
            .map(move |node| NodeMut { node, herd })
    }

    pub fn par_children_mut(
        &mut self,
    ) -> impl IndexedParallelIterator<Item = NodeMut<'_>> {
        let herd = self.herd;
        self.children_slice()
            .par_iter_mut()
            .map(move |node| NodeMut { node, herd })
    }

    pub fn child_mut(&mut self, index: usize) -> Option<NodeMut<'_>> {
        let herd = self.herd;
        self.children_slice()
            .get_mut(index)
            .map(|node| NodeMut { node, herd })
    }

    /// Reorder the children in place.
    pub fn sort_children_by(
        &mut self,
        compare: impl FnMut(&Node, &Node) -> std::cmp::Ordering,
    ) {
        self.children_slice().sort_unstable_by(compare);
    }

    /// Replace the derived sharing figures; in place when there are some
    /// already, so aggregating a tree again does not grow its arena.
    pub fn set_sharing(&mut self, sharing: Option<Sharing>) {
        match (&mut self.node.sharing, sharing) {
            // SAFETY: this node's own value, and `self` borrows the tree
            // exclusively.
            (Some(slot), Some(value)) => unsafe { *slot.0.as_mut() = value },
            (slot @ None, Some(value)) => {
                *slot = Some(self.herd.get().boxed(value));
            }
            (slot, None) => *slot = None,
        }
    }
}

/// A pool of bump arenas, one taken by each worker while it builds.
///
/// A bump arena is not `Sync`, and the walk builds directories on every
/// worker at once, so each takes one from the pool and puts it back when
/// done. Allocations stay put: a `Bump` moves, its chunks do not.
pub struct Herd {
    /// One pool per worker index, so workers taking and returning arenas
    /// millions of times never wait on each other: behind one lock,
    /// building a 20-million-node tree took twice as long.
    spare: Box<[Mutex<Vec<Bump>>]>,
}

impl fmt::Debug for Herd {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Herd").finish_non_exhaustive()
    }
}

impl Herd {
    /// More pools than the machines this runs on have cores; a worker index
    /// past it shares a pool, which is slower but still correct.
    const POOLS: usize = 64;

    fn new() -> Self {
        Self {
            spare: std::iter::repeat_with(Mutex::default)
                .take(Self::POOLS)
                .collect(),
        }
    }

    /// An arena for this thread to build with.
    pub fn get(&self) -> Member<'_> {
        let pool = rayon::current_thread_index()
            .map_or(0, |index| index % Self::POOLS);
        let bump = crate::scan::lock(&self.spare[pool])
            .pop()
            .unwrap_or_default();
        Member {
            bump: Some(bump),
            pool,
            herd: self,
            brand: PhantomData,
        }
    }

    fn bytes(&self) -> ArenaBytes {
        let mut bytes = ArenaBytes::default();
        for pool in &self.spare {
            for bump in crate::scan::lock(pool).iter_mut() {
                bytes.reserved += bump.allocated_bytes();
                bytes.used +=
                    bump.iter_allocated_chunks().map(<[_]>::len).sum::<usize>();
            }
        }
        bytes
    }
}

/// What a tree's arenas hold.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ArenaBytes {
    /// Bytes handed out to nodes, names and boxes.
    pub used: usize,
    /// Bytes of address space the arenas took, including the unused end
    /// of each one's newest chunk, which is never touched.
    pub reserved: usize,
}

/// One worker's arena, back in its herd on drop. `'h` brands what it
/// builds with the build it belongs to.
pub struct Member<'h> {
    bump: Option<Bump>,
    pool: usize,
    herd: &'h Herd,
    /// Invariant: a node of one build cannot be taken for another's.
    brand: PhantomData<fn(&'h ()) -> &'h ()>,
}

impl fmt::Debug for Member<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Member").finish_non_exhaustive()
    }
}

impl Drop for Member<'_> {
    fn drop(&mut self) {
        if let Some(bump) = self.bump.take() {
            crate::scan::lock(&self.herd.spare[self.pool]).push(bump);
        }
    }
}

/// A node one build has made, not yet placed in its tree.
#[must_use]
pub struct Built<'h> {
    node: Node,
    brand: PhantomData<fn(&'h ()) -> &'h ()>,
}

impl fmt::Debug for Built<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.node.fmt(f)
    }
}

impl Deref for Built<'_> {
    type Target = Node;

    fn deref(&self) -> &Node {
        &self.node
    }
}

impl Built<'_> {
    pub const fn facts_mut(&mut self) -> &mut Facts {
        &mut self.node.facts
    }
}

impl<'h> Member<'h> {
    const fn bump(&self) -> &Bump {
        self.bump.as_ref().expect("present until drop")
    }

    fn boxed<T>(&self, value: T) -> Boxed<T> {
        Boxed(NonNull::from(self.bump().alloc(value)))
    }

    fn name(&self, name: &str) -> Name {
        let len = u32::try_from(name.len()).expect("a file name fits");
        let layout = Layout::from_size_align(NAME_HEADER + name.len(), 1)
            .expect("a file name fits");
        let base = self.bump().alloc_layout(layout).as_ptr();
        // SAFETY: `base` is a fresh allocation of `NAME_HEADER` plus the
        // name's length, so both writes stay inside it; `read_unaligned`
        // in `Name::deref` matches the unaligned write.
        unsafe {
            base.cast::<u32>().write_unaligned(len);
            let bytes = base.add(NAME_HEADER);
            std::ptr::copy_nonoverlapping(name.as_ptr(), bytes, name.len());
            Name(NonNull::new_unchecked(bytes))
        }
    }

    #[allow(
        clippy::cast_ptr_alignment,
        reason = "the allocation is aligned for a `Node` and a `usize`, and \
                  `CHILDREN_HEADER` is a multiple of both alignments"
    )]
    fn children(&self, children: Vec<Built<'h>>) -> Children {
        if children.is_empty() {
            return Children(None);
        }
        let len = children.len();
        let layout = Layout::array::<Node>(len)
            .and_then(|nodes| {
                Layout::from_size_align(
                    CHILDREN_HEADER + nodes.size(),
                    align_of::<Node>().max(align_of::<usize>()),
                )
            })
            .expect("a directory's children fit in memory");
        let base = self.bump().alloc_layout(layout).as_ptr();
        // SAFETY: `base` is aligned for both a `usize` and a `Node` and
        // holds the header plus `len` nodes; `CHILDREN_HEADER` is a
        // multiple of `Node`'s alignment, so the first node is aligned. Each
        // of the `len` children is moved in exactly once.
        unsafe {
            base.cast::<usize>().write(len);
            let first = base.add(CHILDREN_HEADER).cast::<Node>();
            for (index, child) in children.into_iter().enumerate() {
                first.add(index).write(child.node);
            }
            Children(Some(NonNull::new_unchecked(first)))
        }
    }

    /// A leaf, or a directory that holds nothing.
    pub fn leaf(
        &self,
        name: &str,
        facts: Facts,
        allocation: Option<Allocation>,
    ) -> Built<'h> {
        Built {
            node: Node {
                facts,
                name: self.name(name),
                children: Children(None),
                allocation: allocation.map(|value| self.boxed(value)),
                sharing: None,
            },
            brand: PhantomData,
        }
    }

    /// A directory holding `children`, which move into this arena in the
    /// order given.
    pub fn directory(
        &self,
        name: &str,
        facts: Facts,
        allocation: Option<Allocation>,
        children: Vec<Built<'h>>,
    ) -> Built<'h> {
        let mut built = self.leaf(name, facts, allocation);
        built.node.children = self.children(children);
        built
    }

    /// A deep copy of `node`, from this tree or any other, into this
    /// build: how a subtree moves between trees.
    pub fn copy(&self, node: &Node) -> Built<'h> {
        self.copy_named(node, &node.name)
    }

    /// [`Self::copy`], under another name.
    pub fn copy_named(&self, node: &Node, name: &str) -> Built<'h> {
        copy_at(self.herd, self, node, name, 0)
    }

    /// `draft`, with everything beneath it.
    pub fn draft(&self, draft: &Draft) -> Built<'h> {
        let children = draft
            .children
            .iter()
            .map(|child| self.draft(child))
            .collect();
        let mut built = self.directory(
            &draft.name,
            draft.facts(),
            draft.allocation.as_deref().copied(),
            children,
        );
        built.node.sharing =
            draft.sharing.as_deref().map(|value| self.boxed(*value));
        built
    }
}

fn copy_at<'h>(
    herd: &'h Herd,
    member: &Member<'h>,
    node: &Node,
    name: &str,
    depth: usize,
) -> Built<'h> {
    let children = if depth < PARALLEL_LEVELS {
        node.children
            .par_iter()
            .map(|child| {
                copy_at(herd, &herd.get(), child, &child.name, depth + 1)
            })
            .collect()
    } else {
        node.children
            .iter()
            .map(|child| copy_at(herd, member, child, &child.name, depth + 1))
            .collect()
    };
    let mut built = member.directory(
        name,
        node.facts,
        node.allocation.as_deref().copied(),
        children,
    );
    built.node.sharing =
        node.sharing.as_deref().map(|value| member.boxed(*value));
    built
}

/// A scanned tree and the arenas its nodes live in. Derefs to its root.
pub struct Tree {
    root: Node,
    herd: Herd,
}

impl fmt::Debug for Tree {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Tree")
            .field("root", &self.root)
            .finish_non_exhaustive()
    }
}

impl Deref for Tree {
    type Target = Node;

    fn deref(&self) -> &Node {
        &self.root
    }
}

/// A deep copy into fresh arenas, made in parallel.
impl Clone for Tree {
    fn clone(&self) -> Self {
        Self::copy_of(&self.root)
    }
}

impl From<Draft> for Tree {
    fn from(draft: Draft) -> Self {
        Self::from(&draft)
    }
}

impl From<&Draft> for Tree {
    fn from(draft: &Draft) -> Self {
        Self::build(|herd| herd.get().draft(draft))
    }
}

impl Tree {
    /// Build a tree: `build` takes arenas from the herd it is given and
    /// returns the root. The brand keeps every node it makes in this tree.
    ///
    /// A node of one build cannot become part of another tree, which
    /// would leave it pointing into arenas freed with the first:
    ///
    /// ```compile_fail
    /// use disktree_core::tree::{Facts, NodeKind, Tree};
    /// let mut smuggled = None;
    /// let _first = Tree::build(|herd| {
    ///     let member = herd.get();
    ///     smuggled =
    ///         Some(member.leaf("x", Facts::entry(NodeKind::File, 1), None));
    ///     member.leaf("root", Facts::directory(), None)
    /// });
    /// let _second = Tree::build(|_| smuggled.take().unwrap());
    /// ```
    ///
    /// Nor can a build return a node made in another tree's arenas:
    ///
    /// ```compile_fail
    /// use disktree_core::tree::{Facts, Tree};
    /// let first =
    ///     Tree::build(|herd| herd.get().leaf("root", Facts::directory(), None));
    /// let _second = Tree::build(|_| {
    ///     first.herd().get().leaf("root", Facts::directory(), None)
    /// });
    /// ```
    pub fn build(build: impl for<'h> FnOnce(&'h Herd) -> Built<'h>) -> Self {
        let herd = Herd::new();
        let root = build(&herd).node;
        Self { root, herd }
    }

    /// [`Self::build`], for a build that can fail.
    pub fn try_build<E>(
        build: impl for<'h> FnOnce(&'h Herd) -> Result<Built<'h>, E>,
    ) -> Result<Self, E> {
        let herd = Herd::new();
        let root = build(&herd)?.node;
        Ok(Self { root, herd })
    }

    /// A tree of its own holding a copy of `node` and everything beneath.
    pub fn copy_of(node: &Node) -> Self {
        Self::build(|herd| herd.get().copy(node))
    }

    pub const fn root(&self) -> &Node {
        &self.root
    }

    pub const fn root_mut(&mut self) -> NodeMut<'_> {
        NodeMut {
            node: &mut self.root,
            herd: &self.herd,
        }
    }

    /// The arenas this tree's nodes live in. Only good for building
    /// nothing: what a member makes here cannot join this tree.
    pub const fn herd(&self) -> &Herd {
        &self.herd
    }

    pub fn arena_bytes(&self) -> ArenaBytes {
        self.herd.bytes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(bytes: u64) -> Facts {
        Facts::entry(NodeKind::File, bytes)
    }

    fn sample() -> Tree {
        Tree::build(|herd| {
            let member = herd.get();
            let deep = member.leaf("deep", file(7), None);
            let nested =
                member.directory("child", Facts::directory(), None, vec![deep]);
            let direct = member.leaf(
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
            let empty = member.leaf("", file(0), None);
            member.directory(
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
    #[cfg(target_pointer_width = "64")]
    fn a_node_stays_small() {
        // Facts, then one pointer each for the name, the children and the
        // two sparse boxes.
        assert_eq!(size_of::<Facts>(), 48);
        assert_eq!(size_of::<Node>(), 80);
    }

    #[test]
    fn a_built_tree_reads_back() {
        let tree = sample();
        assert_eq!(&*tree.name, "root");
        let names: Vec<&str> =
            tree.children.iter().map(|child| &*child.name).collect();
        assert_eq!(names, ["child", "direct", ""]);
        assert_eq!(&*tree.children[0].children[0].name, "deep");
        assert_eq!(tree.children[0].children[0].bytes, 7);
        assert_eq!(
            tree.children[1].allocation.as_deref().map(|a| a.bytes),
            Some(5)
        );
        assert!(tree.children[2].children.is_empty());
    }

    #[test]
    fn a_long_name_and_many_children_survive() {
        let long = "é".repeat(300);
        let tree = Tree::build(|herd| {
            let member = herd.get();
            let children = (0..1000)
                .map(|index| {
                    member.leaf(&format!("{index}"), file(index), None)
                })
                .chain(std::iter::once(member.leaf(&long, file(1), None)))
                .collect();
            member.directory("root", Facts::directory(), None, children)
        });
        assert_eq!(tree.children.len(), 1001);
        assert_eq!(&*tree.children[999].name, "999");
        assert_eq!(tree.children[999].bytes, 999);
        assert_eq!(&*tree.children[1000].name, long);
    }

    #[test]
    fn writes_reach_the_tree_and_children_reorder_in_place() {
        let mut tree = sample();
        let mut root = tree.root_mut();
        root.facts_mut().bytes = 12;
        for mut child in root.children_mut() {
            child.facts_mut().read_error = true;
        }
        root.sort_children_by(|left, right| left.name.cmp(&right.name));
        root.child_mut(0).expect("a child").facts_mut().modified = 9;
        assert_eq!(tree.bytes, 12);
        let names: Vec<&str> =
            tree.children.iter().map(|child| &*child.name).collect();
        assert_eq!(names, ["", "child", "direct"]);
        assert_eq!(tree.children[0].modified, 9);
        assert!(tree.children.iter().all(|child| child.read_error));
        assert_eq!(&*tree.children[1].children[0].name, "deep");
    }

    #[test]
    fn sharing_is_rewritten_in_place() {
        let mut tree = sample();
        let mut root = tree.root_mut();
        root.set_sharing(Some(Sharing::default()));
        let first = root.sharing.as_deref().map(std::ptr::from_ref);
        root.set_sharing(Some(Sharing {
            files: 3,
            ..Sharing::default()
        }));
        assert_eq!(root.sharing.as_deref().map(std::ptr::from_ref), first);
        assert_eq!(tree.sharing.as_deref().map(|s| s.files), Some(3));
        tree.root_mut().set_sharing(None);
        assert!(tree.sharing.is_none());
    }

    #[test]
    fn a_copy_outlives_its_source() {
        let source = sample();
        let copy = source.clone();
        drop(source);
        assert_eq!(copy.children.len(), 3);
        assert_eq!(&*copy.children[0].children[0].name, "deep");
        let subtree = {
            let source = sample();
            Tree::copy_of(&source.children[0])
        };
        assert_eq!(&*subtree.name, "child");
        assert_eq!(subtree.children.len(), 1);
    }

    #[test]
    fn a_tree_moves_to_another_thread() {
        let tree = sample();
        let count = std::thread::spawn(move || tree.children.len())
            .join()
            .expect("joined");
        assert_eq!(count, 3);
    }

    #[test]
    fn a_tree_built_on_every_worker_holds_every_node() {
        let tree = Tree::build(|herd| {
            let children = (0..64)
                .into_par_iter()
                .map(|dir| {
                    let member = herd.get();
                    let files = (0..100)
                        .map(|index| {
                            member.leaf(
                                &format!("{dir}-{index}"),
                                file(1),
                                None,
                            )
                        })
                        .collect();
                    member.directory(
                        &format!("{dir}"),
                        Facts::directory(),
                        None,
                        files,
                    )
                })
                .collect();
            herd.get()
                .directory("root", Facts::directory(), None, children)
        });
        assert_eq!(tree.children.len(), 64);
        assert!(tree.children.iter().enumerate().all(|(dir, child)| {
            child.children.len() == 100
                && *child.children[99].name == *format!("{dir}-99")
        }));
        let bytes = tree.arena_bytes();
        assert!(bytes.used > 0 && bytes.used <= bytes.reserved);
    }
}
