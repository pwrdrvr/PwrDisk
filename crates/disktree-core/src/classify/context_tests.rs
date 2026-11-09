use super::*;
use crate::scan::{ScanOptions, scan};
use proptest::prelude::*;

fn put(root: &Path, relative: &str) {
    let path = root.join(relative);
    std::fs::create_dir_all(path.parent().expect("parent"))
        .expect("contrived directories");
    std::fs::write(path, "invented data").expect("contents");
}

fn named<'a>(tree: &'a Tree, relative: &str) -> Node<'a> {
    relative.split('/').fold(tree.root(), |node, name| {
        node.child_named(name).expect("contrived child")
    })
}

fn same_subtree(broad: Node<'_>, narrow: Node<'_>) {
    assert_eq!(narrow.category, broad.category, "{}", broad.name());
    assert_eq!(narrow.reclaim, broad.reclaim, "{}", broad.name());
    assert_eq!(narrow.children().len(), broad.children().len());
    for child in broad.children() {
        same_subtree(
            child,
            narrow.child_named(child.name()).expect("same child"),
        );
    }
}

/// Narrowing a descriptive location must retain its context and must not
/// turn Git, VM or macOS store contents into cleanup candidates.
fn context_across_roots(
    container: &str,
    expected: &str,
    names: &[String],
    protected: Option<&str>,
) {
    let fixture = tempfile::tempdir().expect("fixture");
    let relative = format!("private/{container}/{}", names.join("/"));
    put(fixture.path(), &format!("{relative}/payload"));
    if let Some(owner) = protected {
        put(
            fixture.path(),
            &format!("{relative}/{owner}/cache/tmp/payload"),
        );
    }
    let broad = scan(fixture.path(), ScanOptions::default()).expect("scan");
    assert_eq!(
        named(&broad, &format!("private/{container}"))
            .category
            .name(),
        expected
    );
    let mut roots = vec![format!("private/{container}")];
    for name in names {
        roots.push(format!("{}/{name}", roots.last().expect("parent")));
    }
    if let Some(owner) = protected {
        for part in [owner, "cache", "tmp"] {
            roots.push(format!("{}/{part}", roots.last().expect("parent")));
        }
    }
    for relative in roots {
        let path = fixture.path().join(&relative);
        let narrow = scan(&path, ScanOptions::default()).expect("narrow");
        same_subtree(named(&broad, &relative), narrow.root());
        let canonical = path.canonicalize().expect("real fixture path");
        if canonical != path {
            let resolved = scan(&canonical, ScanOptions::default())
                .expect("canonical narrow scan");
            same_subtree(narrow.root(), resolved.root());
        }
        // The same completed subtree must agree for SSH's POSIX rules too.
        let mut remote = Tree::copy_of(narrow.root());
        let remote_path =
            PathStyle::Posix.join(Path::new("/fixture"), &relative);
        classify_at(&mut remote, &remote_path, PathStyle::Posix);
        same_subtree(narrow.root(), remote.root());
    }
}

#[test]
fn a_narrow_tmp_scan_keeps_temporary_context_and_git_ownership() {
    context_across_roots(
        "tmp",
        "temporary",
        &["claude-501".into()],
        Some(".git"),
    );
}

#[test]
fn trash_keeps_its_label_when_git_clears_the_cleanup_hint() {
    context_across_roots(".Trash", "trash", &["kept".into()], Some(".git"));
}

#[test]
fn diagnostic_logs_are_described_without_a_cache_claim() {
    let fixture = tempfile::tempdir().expect("fixture");
    for name in ["DiagOutputDir", "RDClientAutoTrace"] {
        let relative = format!(".cache/{name}/session");
        put(fixture.path(), &format!("{relative}/payload"));
        let broad = scan(fixture.path(), ScanOptions::default()).expect("scan");
        let logs = named(&broad, &format!(".cache/{name}"));
        for node in [logs, named(&broad, &relative)] {
            assert_eq!(node.category.name(), "diagnostic logs");
            assert_eq!(node.reclaim, None);
        }
        let narrow =
            scan(&fixture.path().join(&relative), ScanOptions::default())
                .expect("narrow");
        same_subtree(named(&broad, &relative), narrow.root());
        assert!(
            crate::cleanup::inspection_reason(&fixture.path().join(&relative))
                .is_none()
        );
    }
}

#[test]
fn temporary_names_override_an_inherited_regenerable_hint() {
    for name in ["tmp", ".tmp", "temp", "Temp", "CrashDumps"] {
        let fixture = tempfile::tempdir().expect("fixture");
        put(fixture.path(), &format!(".cache/{name}/session/payload"));
        put(fixture.path(), ".cache/ordinary/payload");
        let tree = scan(fixture.path(), ScanOptions::default()).expect("scan");
        let node = named(&tree, &format!(".cache/{name}/session"));
        assert_eq!(node.category, Category::Temporary);
        assert_eq!(node.reclaim, Some(Reclaim::Temporary));
        assert_eq!(
            named(&tree, ".cache/ordinary").reclaim,
            Some(Reclaim::Regenerable)
        );
    }
}

/// The broad scan is an independent traversal oracle for every narrower
/// root, including category-filtered searches used by MCP and the link.
fn ownership_across_roots(
    owner: &str,
    descendants: &[String],
    expected: Category,
    reclaim: Option<Reclaim>,
) {
    let fixture = tempfile::tempdir().expect("fixture");
    let relative = format!("{owner}/{}", descendants.join("/"));
    put(fixture.path(), &format!("{relative}/payload"));
    let broad = scan(fixture.path(), ScanOptions::default()).expect("scan");
    let leaf = named(&broad, &relative);
    assert_eq!((leaf.category, leaf.reclaim), (expected, reclaim));
    let mut relative = owner.to_owned();
    for next in std::iter::once(None).chain(descendants.iter().map(Some)) {
        if let Some(next) = next {
            relative.push('/');
            relative.push_str(next);
        }
        let path = fixture.path().join(&relative);
        let narrow = scan(&path, ScanOptions::default()).expect("narrow");
        let subtree = named(&broad, &relative);
        same_subtree(subtree, narrow.root());
        let filter = crate::query::Filter {
            kind: crate::query::Kind::File,
            category: Some(expected),
            ..crate::query::Filter::default()
        };
        let broad_hits = crate::query::find(&path, subtree, &filter, 100);
        let narrow_hits =
            crate::query::find(&path, narrow.root(), &filter, 100);
        let paths = |hits: crate::query::Found<'_>| {
            hits.matches
                .into_iter()
                .map(|(path, _)| path)
                .collect::<Vec<_>>()
        };
        assert_eq!(paths(narrow_hits), paths(broad_hits));
        let canonical = path.canonicalize().expect("real path");
        let resolved = scan(&canonical, ScanOptions::default()).expect("real");
        same_subtree(subtree, resolved.root());
        let mut remote = Tree::copy_of(subtree);
        let remote_path =
            PathStyle::Posix.join(Path::new("/fixture"), &relative);
        classify_at(&mut remote, &remote_path, PathStyle::Posix);
        same_subtree(subtree, remote.root());
    }
}

#[test]
fn temporary_inside_pip_keeps_its_hint_when_the_scan_narrows() {
    ownership_across_roots(
        ".cache/pip",
        &["tmp".into(), "session".into()],
        Category::Temporary,
        Some(Reclaim::Temporary),
    );
}

#[test]
fn git_and_vm_ownership_enclose_macos_store_lookalikes() {
    for (owner, expected) in [
        ("repo/.git", Category::Git),
        ("guest.utm", Category::VirtualMachine),
    ] {
        for store in [".fseventsd", ".Spotlight-V100"] {
            ownership_across_roots(
                owner,
                &[store.into(), "session".into()],
                expected,
                None,
            );
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 32,
        failure_persistence: None,
        ..ProptestConfig::default()
    })]

    #[test]
    fn descriptive_context_and_protected_ownership_survive_narrow_scans(
        (container, expected) in prop::sample::select(vec![
            ("tmp", "temporary"), (".tmp", "temporary"),
            ("Temp", "temporary"), ("CrashDumps", "temporary"),
            (".Trash", "trash"), ("trash", "trash"),
            ("$Recycle.Bin", "trash"),
        ]),
        names in prop::collection::vec(prop_oneof![
            "session-[a-z]{1,6}", Just("src".to_owned()),
            Just("Documents".to_owned()), Just(".cache".to_owned()),
            Just(".claude".to_owned()),
        ], 1..4),
        protected in prop::option::of(prop::sample::select(vec![
            ".git", "guest.utm", ".fseventsd", ".Spotlight-V100",
        ])),
    ) {
        context_across_roots(container, expected, &names, protected);
    }

    #[test]
    fn nearer_hints_inside_artifacts_agree_across_scan_roots(
        owner in prop::sample::select(vec![
            ".cache/pip", ".cache/uv", ".gradle/caches",
            ".cache/sccache", ".cache/ms-playwright",
            ".pnpm-store", ".cache/pnpm/store",
        ]),
        mut names in prop::collection::vec(prop_oneof![
            "session-[a-z]{1,6}", Just("Documents".to_owned()),
            Just(".cache".to_owned()), Just("DiagOutputDir".to_owned()),
            Just(".next".to_owned()),
        ], 0..8),
        temporary in prop::sample::select(vec!["tmp", ".tmp", "Temp", "CrashDumps"]),
    ) {
        names.extend([temporary.into(), "session".into()]);
        ownership_across_roots(
            owner, &names, Category::Temporary, Some(Reclaim::Temporary),
        );
    }

    #[test]
    fn the_first_protected_owner_wins_at_every_scan_root(
        (owner, expected) in prop::sample::select(vec![
            ("repo/.git", Category::Git),
            ("guest.utm", Category::VirtualMachine),
            (".fseventsd", Category::FilesystemEvents),
            (".Spotlight-V100", Category::Spotlight),
        ]),
        inner in prop::sample::select(vec![
            ".git", "guest.utm", ".fseventsd", ".Spotlight-V100",
        ]),
        mut names in prop::collection::vec("session-[a-z]{1,6}", 0..8),
    ) {
        names.extend([inner.into(), "cache".into(), "tmp".into(), "session".into()]);
        ownership_across_roots(owner, &names, expected, None);
    }
}
