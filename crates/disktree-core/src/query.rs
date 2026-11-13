//! Questions an agent asks of a scanned tree: what is in a directory, and
//! the largest things matching a description.
//!
//! The window answers these over its link and `pwrdisk mcp` answers them
//! over its own scans, so the logic lives here, once.

use std::path::{Path, PathBuf};

mod outputs;
pub use outputs::find as find_outputs;

use crate::classify::Category;
use crate::tree::{Node, NodeKind};

/// What kind of entry a [`Filter`] matches.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Kind {
    /// Files and directories; a matching directory hides its contents.
    #[default]
    Any,
    File,
    Dir,
}

impl Kind {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Any => "any",
            Self::File => "file",
            Self::Dir => "dir",
        }
    }

    pub fn named(name: &str) -> Option<Self> {
        match name {
            "any" => Some(Self::Any),
            "file" => Some(Self::File),
            "dir" => Some(Self::Dir),
            _ => None,
        }
    }
}

/// Which figure a search or a listing ranks by, largest first. On the
/// link as `"size"` or `"private"`; absent means size.
#[derive(
    Clone,
    Copy,
    Debug,
    Default,
    PartialEq,
    Eq,
    serde::Serialize,
    serde::Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum Order {
    /// Listed allocation: what the entry takes on disk.
    #[default]
    Size,
    /// What the entry holds alone ([`Node::private_bytes`]): on APFS, the
    /// part of a clone no other clone may hold.
    Private,
}

impl Order {
    /// `node`'s figure in this order.
    pub fn value(self, node: Node<'_>) -> u64 {
        match self {
            Self::Size => node.bytes,
            Self::Private => node.private_bytes(),
        }
    }
}

/// What a search keeps. Every field narrows; the default keeps everything.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Filter {
    pub kind: Kind,
    /// Only entries of at least this many bytes.
    pub min_bytes: u64,
    /// Only entries holding at least this many bytes alone.
    pub min_private_bytes: u64,
    pub category: Option<Category>,
    /// Cleanup candidates by classification, independent of removal guards.
    pub reclaimable: bool,
    /// Only names containing this, compared lowercased.
    pub name: Option<String>,
    /// Only entries last written before this Unix time. For a directory
    /// that is the newest write anywhere beneath it.
    pub before: Option<i64>,
}

impl Filter {
    pub fn matches(&self, node: Node<'_>) -> bool {
        let kind_ok = match self.kind {
            Kind::Any => true,
            Kind::File => !node.is_dir(),
            Kind::Dir => node.is_dir(),
        };
        kind_ok
            && node.bytes >= self.min_bytes
            && self.private_enough(node)
            && self
                .category
                .is_none_or(|category| node.category == category)
            && (!self.reclaimable || node.reclaim.is_some())
            && self
                .name
                .as_deref()
                .is_none_or(|name| node.name().to_lowercase().contains(name))
            && self.before.is_none_or(|before| {
                node.modified > 0 && node.modified < before
            })
    }

    /// Whether `node` holds enough alone. Reading a private size costs a
    /// look at sharing figures, so it is skipped when nothing is asked.
    fn private_enough(&self, node: Node<'_>) -> bool {
        self.min_private_bytes == 0
            || node.private_bytes() >= self.min_private_bytes
    }

    /// Collect matches beneath `node`, topmost only: a matching directory
    /// is listed once, without what is inside it. Sizes, private ones too,
    /// shrink going down, so a node below a minimum ends the walk there.
    pub fn collect<'a>(
        &self,
        path: &Path,
        node: Node<'a>,
        found: &mut Vec<(PathBuf, Node<'a>)>,
    ) {
        if node.bytes < self.min_bytes
            || !self.private_enough(node)
            || node.kind == NodeKind::Other
        {
            return;
        }
        if self.matches(node) {
            found.push((path.to_path_buf(), node));
            return;
        }
        for child in node.children() {
            self.collect(&path.join(child.name()), child, found);
        }
    }
}

/// What a search found.
#[derive(Clone, Debug, Default)]
pub struct Found<'a> {
    /// The largest matches, largest first, at most the limit asked for.
    pub matches: Vec<(PathBuf, Node<'a>)>,
    /// How many matched in all.
    pub count: usize,
    /// Bytes of every match, not only the ones listed.
    pub bytes: u64,
    /// What every match holds alone, summed. Matches never nest, and a
    /// private size adds up, so the sum is the union's own.
    pub private_bytes: u64,
}

/// The largest `limit` entries beneath `root` (at `root_path`) that pass
/// `filter`, ranked by `order`. The root itself is never a match.
pub fn find<'a>(
    root_path: &Path,
    root: Node<'a>,
    filter: &Filter,
    order: Order,
    limit: usize,
) -> Found<'a> {
    let mut matches = Vec::new();
    for child in root.children() {
        filter.collect(&root_path.join(child.name()), child, &mut matches);
    }
    let mut ranked: Vec<(u64, (PathBuf, Node<'a>))> = matches
        .into_iter()
        .map(|found| (order.value(found.1), found))
        .collect();
    // Stable: ties keep the walk's order, as they always have.
    ranked.sort_by_key(|(value, _)| std::cmp::Reverse(*value));
    let count = ranked.len();
    let bytes = ranked.iter().map(|(_, (_, node))| node.bytes).sum();
    let private_bytes = ranked
        .iter()
        .map(|(value, (_, node))| match order {
            Order::Private => *value,
            Order::Size => node.private_bytes(),
        })
        .sum();
    ranked.truncate(limit);
    Found {
        matches: ranked.into_iter().map(|(_, found)| found).collect(),
        count,
        bytes,
        private_bytes,
    }
}

#[cfg(test)]
mod private_tests {
    use std::path::{Path, PathBuf};

    use proptest::prelude::*;

    use super::{Filter, Order, find};
    use crate::test_clones;
    use crate::tree::{Node, NodeKind};

    /// The topmost entries holding at least `min` alone, found by visiting
    /// every node: no pruning, so it is the oracle for the walk that
    /// stops early.
    fn every_topmost(
        path: &Path,
        node: Node<'_>,
        min: u64,
        out: &mut Vec<PathBuf>,
    ) {
        if node.kind != NodeKind::Other && node.private_bytes() >= min {
            out.push(path.to_path_buf());
            return;
        }
        for child in node.children() {
            every_topmost(&path.join(child.name()), child, min, out);
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig {
            cases: 256,
            failure_persistence: None,
            ..ProptestConfig::default()
        })]

        /// Ranking by private finds what an exhaustive walk finds, most
        /// private first, and its total is what they hold alone together.
        #[test]
        fn finding_by_private_matches_an_exhaustive_walk(
            spec in test_clones::dir(),
            min in 0_u64..20_000,
        ) {
            let tree = spec.tree();
            let root = Path::new("/r");
            let filter = Filter {
                min_private_bytes: min,
                ..Filter::default()
            };
            let found = find(root, tree.root(), &filter, Order::Private, 10_000);

            let mut oracle = Vec::new();
            for child in tree.root().children() {
                every_topmost(&root.join(child.name()), child, min, &mut oracle);
            }
            let mut got: Vec<PathBuf> =
                found.matches.iter().map(|(path, _)| path.clone()).collect();
            got.sort();
            oracle.sort();
            prop_assert_eq!(&got, &oracle);
            prop_assert_eq!(found.count, oracle.len());

            let values: Vec<u64> = found
                .matches
                .iter()
                .map(|(_, node)| node.private_bytes())
                .collect();
            prop_assert!(values.windows(2).all(|pair| pair[0] >= pair[1]));
            prop_assert_eq!(found.private_bytes, values.iter().sum::<u64>());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::classify::Reclaim;
    use crate::tree::{Draft, Tree};
    use crate::tree::{Metric, aggregate};

    fn tree() -> Tree {
        let mut root = Draft::directory("r");
        let mut cache = Draft::directory(".cache");
        cache.reclaim = Some(Reclaim::Regenerable);
        cache.category = Category::Cache;
        let mut blob = Draft::entry("blob.bin", NodeKind::File, 400);
        blob.modified = 1_000;
        cache.children.push(blob);
        let mut src = Draft::directory("src");
        let mut target = Draft::directory("target");
        target.reclaim = Some(Reclaim::BuildOutput);
        target
            .children
            .push(Draft::entry("app", NodeKind::File, 300));
        src.children.push(target);
        let mut main = Draft::entry("main.rs", NodeKind::File, 20);
        main.modified = 5_000;
        src.children.push(main);
        root.children.push(cache);
        root.children.push(src);
        root.children
            .push(Draft::entry("Movie.iso", NodeKind::File, 250));
        let mut root = Tree::from(root);
        aggregate(&mut root, Metric::Bytes);
        root
    }

    #[test]
    fn reclaimable_matches_are_topmost_and_largest_first() {
        let tree = tree();
        let root = tree.root();
        let root_path = Path::new("/r");
        let found = find(
            root_path,
            root,
            &Filter {
                reclaimable: true,
                ..Filter::default()
            },
            Order::Size,
            10,
        );
        let paths: Vec<PathBuf> =
            found.matches.iter().map(|(path, _)| path.clone()).collect();
        assert_eq!(
            paths,
            [
                root_path.join(".cache"),
                root_path.join("src").join("target")
            ]
        );
        assert_eq!(found.count, 2);
        assert_eq!(found.bytes, 700);
    }

    #[test]
    fn files_over_a_size_and_names_and_ages_filter() {
        let tree = tree();
        let root = tree.root();
        let big = find(
            Path::new("/r"),
            root,
            &Filter {
                kind: Kind::File,
                min_bytes: 260,
                ..Filter::default()
            },
            Order::Size,
            10,
        );
        assert_eq!(big.matches.len(), 2);
        assert_eq!(big.matches[0].1.name(), "blob.bin");
        assert_eq!(big.matches[1].1.name(), "app");

        let named = find(
            Path::new("/r"),
            root,
            &Filter {
                name: Some("iso".into()),
                ..Filter::default()
            },
            Order::Size,
            1,
        );
        assert_eq!(named.count, 1);
        assert_eq!(named.matches[0].1.name(), "Movie.iso");

        // Older than 2_000: blob.bin (1_000), not main.rs (5_000), and not
        // the unknown-age iso.
        let old = find(
            Path::new("/r"),
            root,
            &Filter {
                kind: Kind::File,
                before: Some(2_000),
                ..Filter::default()
            },
            Order::Size,
            10,
        );
        assert_eq!(old.count, 1);
        assert_eq!(old.matches[0].1.name(), "blob.bin");

        // The limit trims the list but not the totals.
        let one =
            find(Path::new("/r"), root, &Filter::default(), Order::Size, 1);
        assert_eq!(one.matches.len(), 1);
        assert_eq!(one.count, 3);
    }
}
