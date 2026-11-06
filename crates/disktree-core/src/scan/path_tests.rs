use super::*;
use crate::test_paths::{DiskImage, spellings};

fn snapshot(node: crate::tree::Node<'_>) -> Vec<(PathBuf, u64, u64)> {
    fn visit(
        node: crate::tree::Node<'_>,
        path: &Path,
        out: &mut Vec<(PathBuf, u64, u64)>,
    ) {
        out.push((path.to_path_buf(), node.bytes, node.files));
        for child in node.children() {
            visit(child, &path.join(child.name()), out);
        }
    }
    let mut out = Vec::new();
    visit(node, Path::new(""), &mut out);
    out.sort();
    out
}

#[test]
fn a_nested_disk_image_is_a_boundary_under_every_root_spelling() {
    let temp = tempfile::tempdir_in("/tmp").expect("tree");
    let root = temp.path().canonicalize().expect("root");
    let _image = DiskImage::mount_at(&root.join("a/mnt"));
    fs::write(root.join("file"), vec![1_u8; 1024 * 1024]).expect("file");
    fs::write(root.join("a/mnt/foreign"), vec![2_u8; 2 * 1024 * 1024])
        .expect("mounted file");
    let aliases = tempfile::tempdir().expect("aliases");
    let options = ScanOptions {
        apparent_size: true,
        apfs_clone_metadata: false,
        ..ScanOptions::default()
    };
    let canonical = scan(&root, options.clone()).expect("canonical scan");
    // This independent total proves both scans didn't merely cross the
    // mount together. Compare the whole relative tree as well as bytes.
    assert_eq!(canonical.root().bytes, 1024 * 1024);
    for spelling in spellings(&root, &aliases.path().join("root")) {
        let tree = scan(&spelling, options.clone()).expect("alias scan");
        assert_eq!(
            snapshot(canonical.root()),
            snapshot(tree.root()),
            "{spelling:?}"
        );
        let crossing = scan(
            &spelling,
            ScanOptions {
                one_filesystem: false,
                ..options.clone()
            },
        )
        .expect("explicit cross-filesystem scan");
        assert!(crossing.root().bytes >= 3 * 1024 * 1024);
    }
}
