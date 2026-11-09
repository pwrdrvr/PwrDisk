//! Output boundaries share the scan classifier's rules. Generic locations
//! remain visible beside typed descendants; a match never stops discovery.

use std::path::{Path, PathBuf};

use crate::artifact::{self, Artifact};
use crate::classify::Category;
use crate::path_style::PathStyle;
use crate::tree::Node;

use super::{Filter, Found};

const fn protected(category: Category) -> bool {
    matches!(
        category,
        Category::Git
            | Category::VirtualMachine
            | Category::FilesystemEvents
            | Category::Spotlight
    )
}

const fn location(category: Category) -> bool {
    matches!(
        category,
        Category::Temporary
            | Category::AgentScratch
            | Category::Cache
            | Category::DiagnosticLogs
            | Category::Trash
    )
}

const fn output(kind: Artifact) -> bool {
    !kind.is_vm() && !matches!(kind, Artifact::Worktree | Artifact::GitRepo)
}

#[derive(Clone, Copy, Default)]
struct Location {
    parent: Option<Category>,
    active: bool,
    covered: bool,
}

struct Walker<'filter, 'tree> {
    filter: &'filter Filter,
    found: Vec<(PathBuf, Node<'tree>)>,
}

impl<'tree> Walker<'_, 'tree> {
    fn visit<'parts>(
        &mut self,
        node: Node<'tree>,
        path: &mut PathBuf,
        parts: &mut Vec<&'parts str>,
        state: Location,
        mut excluded: bool,
    ) where
        'tree: 'parts,
    {
        if !node.is_dir()
            || node.bytes < self.filter.min_bytes
            || protected(node.category)
        {
            return;
        }
        let detection = if state.parent.is_none() {
            artifact::output_detection(path, node, true)
        } else {
            artifact::output_boundary(parts, true, &mut excluded)
        };
        if detection
            .is_some_and(|d| d.kind.is_vm() || d.kind == Artifact::GitRepo)
        {
            return;
        }
        let descriptive =
            detection.is_none_or(|d| d.kind == Artifact::DiagLogs);
        let fresh = descriptive
            && location(node.category)
            && state.parent != Some(node.category);
        let active =
            descriptive && location(node.category) && (fresh || state.active);
        let mut covered = !fresh && active && state.covered;
        let boundary =
            detection.is_some_and(|d| output(d.kind)) || (active && !covered);
        let matched = boundary && self.filter.matches(node);
        if matched {
            self.found.push((path.clone(), node));
        }
        // A fresh or unreadable location cannot cover its eligible older,
        // named or readable children. Known artifact boundaries remain visible.
        covered |= active && matched && !node.read_error;
        let next = Location {
            parent: Some(node.category),
            active,
            covered,
        };
        for child in node.children().iter().filter(|n| n.is_dir()) {
            path.push(child.name());
            parts.push(child.name());
            self.visit(child, path, parts, next, excluded);
            parts.pop();
            path.pop();
        }
    }
}

/// Recognized artifact boundaries plus descriptive temporary/cache/log/trash
/// locations.
///
/// Includes a matching scan root and nested types, unlike `find`.
/// `bytes` measures their union; overlapping row sizes must not be summed.
pub fn find<'a>(
    root_path: &Path,
    root: Node<'a>,
    filter: &Filter,
    limit: usize,
) -> Found<'a> {
    let names = PathStyle::Native.names(root_path);
    let mut parts: Vec<&str> = names.iter().map(AsRef::as_ref).collect();
    let excluded = artifact::excluded_tool_contents(&parts);
    let mut walker = Walker {
        filter,
        found: Vec::new(),
    };
    walker.visit(
        root,
        &mut root_path.to_path_buf(),
        &mut parts,
        Location::default(),
        excluded,
    );
    let mut matches = walker.found;
    matches.sort_by(|a, b| a.0.cmp(&b.0));
    let mut ancestors: Vec<&Path> = Vec::new();
    let mut bytes = 0_u64;
    for (path, node) in &matches {
        if !ancestors.iter().any(|parent| path.starts_with(parent)) {
            bytes = bytes.saturating_add(node.bytes);
            ancestors.push(path);
        }
    }
    let count = matches.len();
    matches.sort_by_key(|(_, node)| std::cmp::Reverse(node.bytes));
    matches.truncate(limit);
    Found {
        matches,
        count,
        bytes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::classify::classify_at;
    use crate::tree::{Draft, Metric, NodeKind, Tree, aggregate};
    use std::collections::BTreeSet;

    #[test]
    fn unreadable_locations_do_not_cover_readable_scratch_children() {
        let path = Path::new("/invented/tmp");
        let mut root = Draft::directory("tmp");
        root.read_error = true;
        let mut child = Draft::directory("kept");
        child
            .children
            .push(Draft::entry("blob", NodeKind::File, 100));
        root.children.push(child);
        let mut tree = Tree::from(root);
        aggregate(&mut tree, Metric::Bytes);
        classify_at(&mut tree, path, PathStyle::Native);
        let found = super::find(path, tree.root(), &Filter::default(), 200);
        assert!(
            found
                .matches
                .iter()
                .any(|(p, n)| p == &path.join("kept") && !n.read_error)
        );
    }

    #[test]
    fn filtered_temporary_locations_follow_the_directory_query_oracle() {
        for count in 1..6 {
            for name in [None, Some("stale".to_string())] {
                let path = Path::new("/invented/tmp");
                let mut root = Draft::directory("tmp");
                for i in 0..count {
                    let mut dir = Draft::directory(format!("stale-{i}"));
                    let mut file =
                        Draft::entry("blob", NodeKind::File, 100 + i);
                    file.modified = 1_000 + i.cast_signed();
                    dir.children.push(file);
                    root.children.push(dir);
                }
                let mut recent = Draft::entry("recent", NodeKind::File, 10);
                recent.modified = 900_000;
                root.children.push(recent);
                let mut tree = Tree::from(root);
                aggregate(&mut tree, Metric::Bytes);
                classify_at(&mut tree, path, PathStyle::Native);
                let filter = Filter {
                    kind: super::super::Kind::Dir,
                    before: Some(500_000),
                    min_bytes: 50,
                    name,
                    ..Filter::default()
                };
                let expected =
                    super::super::find(path, tree.root(), &filter, 200);
                let got = super::find(path, tree.root(), &filter, 200);
                let paths = |f: &Found<'_>| {
                    f.matches
                        .iter()
                        .map(|(p, _)| p.clone())
                        .collect::<BTreeSet<_>>()
                };
                assert_eq!(
                    paths(&got),
                    paths(&expected),
                    "a fresh generic location must not hide older scratch subtrees"
                );
                assert_eq!(got.bytes, expected.bytes);
            }
        }
    }

    #[test]
    fn nested_output_discovery_agrees_with_a_declared_tree_oracle() {
        for depth in 0..4 {
            for count in 1..5 {
                let path = Path::new("/invented")
                    .join(format!("scope-{depth}-{count}"));
                let mut cache = Draft::directory(".cache");
                let mut expected = BTreeSet::from([path.join(".cache")]);
                let mut total = 0;
                for i in 0..count {
                    let mut scala = Draft::directory("scala-3.3");
                    let bytes = 10 + i;
                    scala.children.push(Draft::entry(
                        "blob",
                        NodeKind::File,
                        bytes,
                    ));
                    let mut target = Draft::directory("target");
                    target.children.push(scala);
                    let name = format!("project-{i}");
                    let mut project = Draft::directory(name.as_str());
                    project.children.push(target);
                    for d in 0..depth {
                        let wrapper = format!("nested-{d}");
                        let mut next = Draft::directory(wrapper);
                        next.children.push(project);
                        project = next;
                    }
                    // Wrapper construction reverses the chain.
                    let suffix: PathBuf = (0..depth)
                        .rev()
                        .map(|d| format!("nested-{d}"))
                        .collect();
                    expected.insert(
                        path.join(".cache")
                            .join(suffix)
                            .join(name)
                            .join("target/scala-3.3"),
                    );
                    cache.children.push(project);
                    total += bytes;
                }
                let mut root = Draft::directory("scope");
                root.children.push(cache);
                for name in
                    [".git", "guest.utm", ".fseventsd", ".Spotlight-V100"]
                {
                    let mut protected = Draft::directory(name);
                    let mut modules = Draft::directory("node_modules");
                    modules.children.push(Draft::entry(
                        "unique",
                        NodeKind::File,
                        100,
                    ));
                    protected.children.push(modules);
                    root.children.push(protected);
                }
                let mut tree = Tree::from(root);
                aggregate(&mut tree, Metric::Bytes);
                classify_at(&mut tree, &path, PathStyle::Native);
                let found = super::find(
                    &path,
                    tree.root(),
                    &Filter {
                        reclaimable: true,
                        ..Filter::default()
                    },
                    200,
                );
                let got: BTreeSet<_> =
                    found.matches.iter().map(|(p, _)| p.clone()).collect();
                assert_eq!(
                    got, expected,
                    "all typed boundaries must remain visible"
                );
                assert_eq!(
                    found.bytes, total,
                    "the parent and children count once"
                );
            }
        }
    }
}
