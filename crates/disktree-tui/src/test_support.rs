//! Fixtures whose classification comes from their contents, not the host.

pub fn fixture_dir() -> tempfile::TempDir {
    // OS temp directories seed temporary classification; macOS also guards
    // their resolved /private ancestry. Canonical workspace roots keep these
    // fixtures neutral and match CLI path spelling on Windows.
    let workspace = crate::app::canonicalize_path(
        &std::env::current_dir().expect("workspace directory"),
    )
    .expect("canonical workspace directory");
    tempfile::Builder::new()
        .prefix("disktree-tui-")
        .tempdir_in(workspace)
        .expect("workspace fixture")
}

#[test]
fn fixture_classification_matches_a_tree_without_host_ancestry() {
    use disktree_core::classify::{Category, Reclaim, classify};
    use disktree_core::scan::{ScanOptions, scan};
    use disktree_core::tree::Node;

    fn classifications(
        root: Node<'_>,
    ) -> Vec<(String, Category, Option<Reclaim>)> {
        let mut pending = vec![root];
        let mut result = Vec::new();
        while let Some(node) = pending.pop() {
            result.push((node.name().to_string(), node.category, node.reclaim));
            pending.extend(node.children());
        }
        result
    }

    let fixture = fixture_dir();
    for name in ["plain", "Documents", "Projects", ".cache", "tmp", "logs"] {
        let directory = fixture.path().join(name);
        std::fs::create_dir(&directory).expect("directory");
        std::fs::write(directory.join("payload"), b"invented data")
            .expect("payload");
    }
    let actual = scan(fixture.path(), ScanOptions::default()).expect("scan");
    let mut oracle = actual.clone();
    classify(&mut oracle);
    assert_eq!(
        classifications(actual.root()),
        classifications(oracle.root()),
        "fixture ancestry changed the classification of its contents"
    );
}
