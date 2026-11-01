//! APFS sharing, reported separately from the treemap's allocated bytes.
//!
//! Harold Hunt's clone accounting, ported from huntharo/disktree#3. He
//! first wrote it for diskhound (huntharo/diskhound#4, contributed as
//! tzarebczan/diskhound#40). Clone IDs describe full data-fork clones, not
//! every shared extent of modified files.

use rayon::prelude::*;
use rustc_hash::FxHashMap;

use crate::tree::{Node, NodeMut};

mod reclaim;
pub use reclaim::{Estimate, estimate};

/// Sparse allocation facts survive hardlink weight de-duplication.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Allocation {
    pub bytes: u64,
    pub identity: Option<(u64, u64)>,
    pub links: u64,
    pub followed: bool,
    pub clone: Option<CloneInfo>,
}

/// Metadata retained only for files that may share allocation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CloneInfo {
    pub device: u64,
    pub id: u64,
    pub references: u32,
    /// Private data-fork allocation; other forks can remain unmeasured.
    pub private_bytes: Option<u64>,
    /// Clone identity describes the data fork, not resource forks or xattrs.
    pub data_bytes: u64,
}

/// Derived figures; none changes `Node::bytes` or a tile's weight.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Sharing {
    pub files: u64,
    pub bytes: u64,
    pub private_bytes: u64,
    pub duplicate_bytes: u64,
    pub unknown_files: u64,
    /// More than one million distinct groups: repeated-byte reporting is partial.
    pub groups_truncated: bool,
    pub estimate: Option<Estimate>,
}

impl Sharing {
    /// Lower bound before snapshots or changes since the scan.
    pub fn reclaimable(self, listed: u64) -> u64 {
        self.estimate.map_or(listed, |estimate| estimate.lower)
    }

    pub fn possible(self, listed: u64) -> u64 {
        self.estimate.map_or(listed, |estimate| estimate.upper)
    }

    /// Allocation in clone files not known to be private. Other clones,
    /// selected or not and inside the scan or not, may hold these blocks,
    /// so deleting this alone does not free them. An unknown private size
    /// counts as shared: the conservative side for a saving.
    pub const fn shared_bytes(self) -> u64 {
        self.bytes.saturating_sub(self.private_bytes)
    }
}

/// Leaf metadata is already retained; do not allocate another box per clone.
pub(crate) fn leaf_summary(node: &Node) -> Sharing {
    let Some(allocation) = node.allocation.as_deref() else {
        return Sharing::default();
    };
    let mut summary = Sharing {
        estimate: Some(allocation.estimate()),
        ..Sharing::default()
    };
    own(node, &mut summary);
    summary
}

impl Allocation {
    fn estimate(self) -> Estimate {
        if self.followed || (self.links > 1 && self.identity.is_some()) {
            return Estimate::default();
        }
        if self.links > 1 {
            return Estimate {
                upper: self.bytes,
                partial: true,
                ..Estimate::default()
            };
        }
        let lower = self.clone.map_or(self.bytes, |info| {
            info.private_bytes.unwrap_or(0).min(self.bytes)
        });
        let full = self.clone.is_some_and(|info| {
            info.device != 0
                && info.id != 0
                && info.references >= 2
                && info.private_bytes == Some(0)
        });
        Estimate {
            lower,
            upper: if full {
                self.bytes.saturating_sub(
                    self.clone.map_or(0, |info| info.data_bytes),
                )
            } else {
                self.bytes
            },
            partial: self.clone.is_some_and(CloneInfo::incomplete),
        }
    }
}

impl CloneInfo {
    const fn incomplete(self) -> bool {
        self.device == 0
            || self.id == 0
            || self.references == 0
            || self.private_bytes.is_none()
    }
}

#[derive(Debug)]
struct Group {
    bytes: u64,
    count: u64,
}

impl Group {
    const fn duplicates(&self) -> u64 {
        self.bytes.saturating_mul(self.count - 1)
    }
}

type Groups = FxHashMap<(u64, u64), Group>;
// Bound temporary grouping memory on volumes with millions of clone IDs.
const MAX_CLONE_GROUPS: usize = 1_000_000;

/// Small-to-large merging avoids walking every subtree for every ancestor.
/// Hardlink duplicates already have zero weight when this pass runs.
pub(crate) fn refresh(node: &mut NodeMut<'_>) {
    collect(node, MAX_CLONE_GROUPS);
}

fn collect(
    node: &mut NodeMut<'_>,
    limit: usize,
) -> (Groups, reclaim::Accumulator) {
    collect_at(node, limit, 0)
}

/// A file's clone facts in a directory's figures: what [`collect`] of the
/// file would return, merged, without the file's own maps. Most files of
/// a large tree are leaves, and a map apiece was most of this pass's time.
fn fold_leaf(
    leaf: &mut NodeMut<'_>,
    limit: usize,
    groups: &mut Groups,
    allocation: &mut reclaim::Accumulator,
    summary: &mut Sharing,
) {
    // Only a directory keeps figures; clear a file's only if it has some,
    // so a pass reads its millions of files without writing to them.
    if leaf.sharing.is_some() {
        leaf.set_sharing(None);
    }
    allocation.add_leaf(leaf);
    if let Some((key, group)) = own(leaf, summary) {
        join(groups, key, group, limit, summary);
    }
}

/// A node's own clone figures, and its group when it can repeat bytes.
fn own(node: &Node, summary: &mut Sharing) -> Option<((u64, u64), Group)> {
    let info = node
        .allocation
        .as_deref()
        .and_then(|allocation| allocation.clone)
        .filter(|_| node.bytes > 0)?;
    summary.files += 1;
    summary.bytes = summary.bytes.saturating_add(node.bytes);
    summary.private_bytes = summary
        .private_bytes
        .saturating_add(info.private_bytes.unwrap_or(0).min(node.bytes));
    summary.unknown_files += u64::from(info.private_bytes.is_none());
    (info.device != 0 && info.id != 0 && info.references >= 2).then(|| {
        (
            (info.device, info.id),
            Group {
                // A modified clone repeats only what it still shares; its
                // private blocks are its own. An unknown private size keeps
                // the whole data fork, as before.
                bytes: info.data_bytes.min(
                    node.bytes.saturating_sub(info.private_bytes.unwrap_or(0)),
                ),
                count: 1,
            },
        )
    })
}

fn join(
    groups: &mut Groups,
    key: (u64, u64),
    group: Group,
    limit: usize,
    summary: &mut Sharing,
) {
    reclaim::work();
    if groups.len() >= limit && !groups.contains_key(&key) {
        summary.groups_truncated = true;
        return;
    }
    groups
        .entry(key)
        .and_modify(|old| {
            // A scan is not atomic. Use the smaller observation when a file
            // changes while its clone group is being measured.
            let before = old.duplicates().saturating_add(group.duplicates());
            old.bytes = old.bytes.min(group.bytes);
            old.count += group.count;
            summary.duplicate_bytes = summary
                .duplicate_bytes
                .saturating_sub(before)
                .saturating_add(old.duplicates());
        })
        .or_insert(group);
}

#[cfg(test)]
thread_local! {
    /// Calls of `collect_at`, each of which makes a node's maps.
    static VISITS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// A directory's figures while its children are added: what [`collect`]
/// returns, with the sums its children contribute.
struct Part {
    groups: Groups,
    allocation: reclaim::Accumulator,
    summary: Sharing,
}

impl Part {
    fn new(limit: usize) -> Self {
        Self {
            groups: Groups::default(),
            allocation: reclaim::Accumulator::new(limit),
            summary: Sharing::default(),
        }
    }

    /// Add one child: a file folded straight into these maps, or a
    /// directory collected and then merged.
    fn add(&mut self, child: &mut NodeMut<'_>, limit: usize, depth: usize) {
        if !child.is_dir() {
            fold_leaf(
                child,
                limit,
                &mut self.groups,
                &mut self.allocation,
                &mut self.summary,
            );
            return;
        }
        let (groups, allocation) = collect_at(child, limit, depth + 1);
        let summary = child.sharing();
        self.merge(
            Self {
                groups,
                allocation,
                summary,
            },
            limit,
        );
    }

    /// Merge the figures of children that follow these.
    fn merge(&mut self, mut other: Self, limit: usize) {
        self.allocation.merge(other.allocation);
        let (mine, theirs) = (&mut self.summary, other.summary);
        mine.duplicate_bytes =
            mine.duplicate_bytes.saturating_add(theirs.duplicate_bytes);
        mine.groups_truncated |= theirs.groups_truncated;
        mine.files += theirs.files;
        mine.bytes = mine.bytes.saturating_add(theirs.bytes);
        mine.private_bytes =
            mine.private_bytes.saturating_add(theirs.private_bytes);
        mine.unknown_files += theirs.unknown_files;
        if self.groups.len() < other.groups.len() {
            std::mem::swap(&mut self.groups, &mut other.groups);
        }
        for (key, group) in other.groups {
            join(&mut self.groups, key, group, limit, &mut self.summary);
        }
    }
}

fn collect_at(
    node: &mut NodeMut<'_>,
    limit: usize,
    depth: usize,
) -> (Groups, reclaim::Accumulator) {
    #[cfg(test)]
    VISITS.set(VISITS.get() + 1);
    let mut part = Part::new(limit);
    if !node.is_dir() {
        fold_leaf(
            node,
            limit,
            &mut part.groups,
            &mut part.allocation,
            &mut part.summary,
        );
        return (part.groups, part.allocation);
    }
    if let Some((key, group)) = own(node, &mut part.summary) {
        part.groups.insert(key, group);
    }
    if depth < crate::tree::PARALLEL_LEVELS {
        // Subdirectories near the top are independent work, as in the
        // totals pass. Each worker folds a run of children into one set of
        // maps and runs are merged as they finish, in child order, so only
        // a few sets are held at once and a truncated group set keeps the
        // groups the one-thread pass kept.
        let children = node
            .par_children_mut()
            .fold(
                || Part::new(limit),
                |mut part, mut child| {
                    part.add(&mut child, limit, depth);
                    part
                },
            )
            .reduce(
                || Part::new(limit),
                |mut left, right| {
                    left.merge(right, limit);
                    left
                },
            );
        part.merge(children, limit);
    } else {
        for mut child in node.children_mut() {
            part.add(&mut child, limit, depth);
        }
    }
    let Part {
        groups,
        mut allocation,
        mut summary,
    } = part;
    if node.allocation.as_deref().is_some_and(|info| info.followed) {
        allocation = reclaim::Accumulator::new(limit);
    }
    summary.estimate = Some(allocation.estimate());
    // `set_sharing` rewrites a directory's figures in place, so a second
    // pass allocates nothing for a directory that already shares.
    let kept = summary.files > 0
        || node.allocation.is_some()
        || allocation.needs_selection()
        || summary.reclaimable(node.bytes) != node.bytes
        || summary.possible(node.bytes) != node.bytes;
    node.set_sharing(kept.then_some(summary));
    (groups, allocation)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree::{Draft, Tree};
    use crate::tree::{Metric, NodeKind, Seen, aggregate, aggregate_deduped};

    fn clone(name: &str, device: u64, id: u64, private: Option<u64>) -> Draft {
        let mut node = Draft::entry(name, NodeKind::File, 100);
        node.allocation = Some(Box::new(Allocation {
            bytes: node.bytes,
            identity: None,
            links: 1,
            followed: false,
            clone: Some(CloneInfo {
                device,
                id,
                references: 2,
                private_bytes: private,
                data_bytes: 80,
            }),
        }));
        node
    }

    #[test]
    fn selecting_a_complete_clone_group_reclaims_its_data_once() {
        let mut root = Draft::directory("root");
        root.children =
            vec![clone("a", 1, 1, Some(0)), clone("b", 1, 1, Some(0))];
        let mut root = Tree::from(root);
        aggregate(&mut root, Metric::Bytes);
        assert_eq!(root.sharing().reclaimable(root.bytes), 80);
    }

    #[test]
    fn group_limit_marks_the_summary_partial_without_losing_private_bytes() {
        let mut root = Draft::directory("root");
        root.children =
            vec![clone("a", 1, 1, Some(20)), clone("b", 1, 2, Some(20))];
        let mut root = Tree::from(root);
        aggregate(&mut root, Metric::Bytes);
        collect(&mut root.root_mut(), 1);
        assert!(root.sharing().groups_truncated);
        assert_eq!(root.sharing().private_bytes, 40);
    }

    #[test]
    fn groups_are_volume_scoped_and_only_data_forks_are_duplicated() {
        let mut root = Draft::directory("root");
        root.children = vec![
            clone("a", 1, 42, Some(20)),
            clone("b", 1, 42, Some(20)),
            clone("c", 2, 42, Some(20)),
        ];
        let mut root = Tree::from(root);
        aggregate(&mut root, Metric::Bytes);
        assert_eq!(root.bytes, 300);
        assert_eq!(root.sharing().duplicate_bytes, 80);
        assert_eq!(root.sharing().reclaimable(root.bytes), 140);
        aggregate(&mut root, Metric::Files);
        assert_eq!(root.sharing().duplicate_bytes, 80);
    }

    #[test]
    fn modified_clones_repeat_only_what_they_still_share() {
        let mut root = Draft::directory("root");
        root.children =
            vec![clone("a", 1, 1, Some(60)), clone("b", 1, 1, Some(60))];
        let mut root = Tree::from(root);
        aggregate(&mut root, Metric::Bytes);
        // Each still shares 40 of its 100 bytes: one copy of that repeats.
        assert_eq!(root.sharing().duplicate_bytes, 40);
        assert!(
            root.sharing().duplicate_bytes <= root.sharing().shared_bytes()
        );
    }

    #[test]
    fn unknown_and_oversized_private_values_stay_conservative() {
        let mut root = Draft::directory("root");
        root.children = vec![
            clone("a", 1, 1, None),
            clone("b", 1, 2, Some(200)),
            Draft::entry("ordinary", NodeKind::File, 30),
        ];
        let mut root = Tree::from(root);
        aggregate(&mut root, Metric::Bytes);
        assert_eq!(root.sharing().unknown_files, 1);
        assert_eq!(root.sharing().reclaimable(root.bytes), 130);
    }

    #[test]
    fn deduplicated_hardlinks_do_not_inflate_sharing() {
        let mut root = Draft::directory("root");
        root.children =
            vec![clone("a", 1, 1, Some(0)), clone("b", 1, 1, Some(0))];
        for node in &mut root.children {
            node.inode =
                Some((1, std::num::NonZeroU64::new(123).expect("inode")));
        }
        let mut root = Tree::from(root);
        aggregate_deduped(&mut root, Metric::Bytes, &Seen::new());
        assert_eq!(root.sharing().files, 1);
        assert_eq!(root.sharing().bytes, 100);
        assert_eq!(root.sharing().duplicate_bytes, 0);
    }

    /// `copies` worktrees nested six levels deep, below the parallel ones,
    /// each holding a clone of the same `files` files, as a directory of
    /// agent worktrees does.
    fn worktrees(copies: u64, files: u64) -> Tree {
        let mut root = Draft::directory("worktrees");
        for copy in 0..copies {
            let mut tree = Draft::directory(format!("w{copy}"));
            for file in 0..files {
                let mut node = clone(&format!("f{file}"), 1, file + 1, Some(0));
                if let Some(info) = node
                    .allocation
                    .as_mut()
                    .and_then(|allocation| allocation.clone.as_mut())
                {
                    info.references = u32::try_from(copies).expect("copies");
                }
                tree.children.push(node);
            }
            for level in 0..6 {
                let mut dir = Draft::directory(format!("w{copy}-{level}"));
                dir.children.push(tree);
                tree = dir;
            }
            root.children.push(tree);
        }
        let mut root = Tree::from(root);
        aggregate(&mut root, Metric::Bytes);
        root
    }

    /// On one thread, so the thread-local counters see all of the pass.
    fn refresh_work(tree: &mut Tree) -> (u64, u64) {
        rayon::ThreadPoolBuilder::new()
            .num_threads(1)
            .build()
            .expect("pool")
            .install(|| {
                VISITS.set(0);
                reclaim::WORK.set(0);
                refresh(&mut tree.root_mut());
                (VISITS.get(), reclaim::WORK.get())
            })
    }

    /// The pass once took 11 s on 20 million nodes, most of it making,
    /// merging and freeing two maps for every file. Hold it to maps per
    /// directory and to work that grows with the files, not faster.
    #[test]
    fn refresh_work_grows_with_files_and_makes_no_map_per_file() {
        let mut counts = Vec::new();
        for (copies, files) in [(8, 64), (64, 512)] {
            let mut tree = worktrees(copies, files);
            let duplicates = tree.sharing().duplicate_bytes;
            assert_eq!(duplicates, 80 * (copies - 1) * files);
            let (visits, work) = refresh_work(&mut tree);
            assert_eq!(tree.sharing().duplicate_bytes, duplicates);
            assert_eq!(
                visits,
                u64::from(tree.dirs),
                "maps are made per directory, never per file"
            );
            // Each file is added to its directory's two maps, and its
            // group moves into the root's two maps once per other copy:
            // fewer than four steps a file however many there are.
            let budget = 4 * copies * files;
            assert!(work <= budget, "{copies}x{files}: {work} > {budget}");
            counts.push((work, copies * files));
        }
        eprintln!("refresh map steps (work, files): {counts:?}");
    }

    #[test]
    fn deep_and_parallel_levels_merge_like_one_directory() {
        // Members below the parallel levels, beside them and at the top:
        // every path through `collect_at` must reach the same totals.
        let mut flat = Draft::directory("root");
        let mut nested = Draft::directory("root");
        for (depth, private) in [(0, Some(0)), (3, Some(0)), (7, Some(20))] {
            flat.children
                .push(clone(&format!("f{depth}"), 1, 9, private));
            let mut leaf = clone("file", 1, 9, private);
            for level in 0..depth {
                let mut dir = Draft::directory(format!("d{depth}-{level}"));
                dir.children.push(leaf);
                leaf = dir;
            }
            nested.children.push(leaf);
        }
        flat.children.push(Draft::entry("plain", NodeKind::File, 7));
        nested
            .children
            .push(Draft::entry("plain", NodeKind::File, 7));
        let mut flat = Tree::from(flat);
        let mut nested = Tree::from(nested);
        aggregate(&mut flat, Metric::Bytes);
        aggregate(&mut nested, Metric::Bytes);
        assert_eq!(nested.sharing(), flat.sharing());
        assert_eq!(nested.sharing().duplicate_bytes, 2 * 80);

        // A second pass keeps every figure and grows no arena.
        let before = nested.sharing();
        let arena = nested.arena_bytes();
        aggregate(&mut nested, Metric::Files);
        assert_eq!(nested.sharing(), before);
        assert_eq!(nested.arena_bytes(), arena);

        // A directory that no longer shares gives its figures up.
        let mut draft = nested.to_draft();
        let deep = draft
            .children
            .iter_mut()
            .find(|child| child.kind.is_dir())
            .expect("a nested branch");
        let emptied = deep.name.clone();
        deep.children.clear();
        let mut nested = Tree::from(draft);
        aggregate(&mut nested, Metric::Bytes);
        let emptied = nested
            .children
            .iter()
            .find(|child| *child.name == *emptied)
            .expect("the emptied branch");
        assert!(emptied.sharing.is_none());
        assert_eq!(nested.sharing().files, 2);
    }

    #[test]
    fn clone_groups_merge_across_directories_and_refresh_after_removal() {
        let mut root = Draft::directory("root");
        for name in ["a", "b"] {
            let mut dir = Draft::directory(name);
            dir.children.push(clone("file", 1, 1, Some(0)));
            root.children.push(dir);
        }
        let mut root = Tree::from(root);
        aggregate(&mut root, Metric::Bytes);
        assert_eq!(root.sharing().duplicate_bytes, 80);
        assert_eq!(root.children[0].sharing().duplicate_bytes, 0);
        // Removing a directory is a rescan into a new tree.
        let mut draft = root.to_draft();
        draft.children.pop();
        let mut root = Tree::from(draft);
        aggregate(&mut root, Metric::Bytes);
        assert_eq!(root.sharing().duplicate_bytes, 0);
        assert_eq!(root.sharing().files, 1);
    }
}
