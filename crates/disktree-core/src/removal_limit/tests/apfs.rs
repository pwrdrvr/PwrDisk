//! Real clone identities distinguish APFS sharing from hardlink aliases.

use std::os::unix::fs::MetadataExt as _;
use std::{fs, path::Path, process::Command};

use super::{Measurement, Target, oracle_bytes};
use crate::scan::{ScanOptions, scan};
use proptest::prelude::*;

fn clone_file(from: &Path, to: &Path) {
    assert!(
        Command::new("cp")
            .arg("-c")
            .arg(from)
            .arg(to)
            .status()
            .expect("cp -c")
            .success()
    );
    assert_ne!(
        fs::metadata(from).expect("source").ino(),
        fs::metadata(to).expect("clone").ino(),
        "separate files"
    );
}

fn compare(clones: bool, hardlinks: bool, blocks: usize) {
    let temp = tempfile::tempdir().expect("APFS fixture");
    let root = temp.path();
    fs::create_dir(root.join("a")).expect("directory");
    fs::create_dir(root.join("b")).expect("directory");
    let source = root.join("a/original");
    fs::write(&source, vec![b'x'; blocks * 4096]).expect("source");
    if clones {
        clone_file(&source, &root.join("b/clone"));
    }
    if hardlinks {
        fs::hard_link(&source, root.join("b/alias")).expect("hardlink");
    }
    let paths = vec![root.join("a"), root.join("b")];
    let unique = oracle_bytes(&paths);
    let source_bytes =
        fs::metadata(&source).expect("source stat").blocks() * 512;
    assert_eq!(
        unique,
        source_bytes * if clones { 2 } else { 1 },
        "clones have separate allocation records; hardlinks share one"
    );
    for clone_metadata in [false, true] {
        for dedup in [false, true] {
            let options = ScanOptions {
                apfs_clone_metadata: clone_metadata,
                dedup_hardlinks: dedup,
                ..ScanOptions::default()
            };
            let tree = scan(root, options.clone()).expect("scan");
            if clones && clone_metadata {
                let a = tree
                    .root()
                    .descend(Path::new("a/original"))
                    .expect("original")
                    .1
                    .allocation()
                    .expect("allocation")
                    .clone
                    .expect("original clone metadata");
                let b = tree
                    .root()
                    .descend(Path::new("b/clone"))
                    .expect("clone")
                    .1
                    .allocation()
                    .expect("allocation")
                    .clone
                    .expect("clone metadata");
                assert_ne!(a.device, 0);
                assert_ne!(a.id, 0);
                assert!(a.references >= 2);
                assert_eq!(
                    (a.device, a.id),
                    (b.device, b.id),
                    "fixture must be a real APFS clone group"
                );
            }
            // Split the selection into one directory and individual files,
            // so the recheck must use the same bulk entry measurement too.
            let mut targets = vec![Target {
                path: root.join("a"),
                bytes: 0,
                is_dir: true,
                hidden: false,
            }];
            for entry in fs::read_dir(root.join("b")).expect("entries") {
                targets.push(Target {
                    path: entry.expect("entry").path(),
                    bytes: 0,
                    is_dir: false,
                    hidden: false,
                });
            }
            let recheck = Measurement::new(&options, &targets);
            let fresh: u64 = targets
                .iter()
                .map(|target| recheck.target(target).expect("recheck").bytes)
                .sum();
            let expected = if dedup {
                unique
            } else {
                source_bytes * (1 + u64::from(clones) + u64::from(hardlinks))
            };
            assert_eq!(tree.root().bytes, expected);
            assert_eq!(fresh, expected);
        }
    }
}

#[test]
fn clones_and_hardlinks_have_distinct_capacity_accounting() {
    compare(true, false, 4);
    compare(false, true, 4);
    compare(true, true, 4);
}

#[test]
fn individual_file_rechecks_preserve_filesystem_aliases() {
    let temp = tempfile::tempdir().expect("APFS fixture");
    let original = temp.path().join("café");
    fs::write(&original, vec![b'x'; 8192]).expect("file");
    std::os::unix::fs::symlink("café", temp.path().join("link"))
        .expect("symlink");
    for (name, actual) in
        [("CAFÉ", "café"), ("cafe\u{301}", "café"), ("LINK", "link")]
    {
        let path = temp.path().join(name);
        // Case-sensitive APFS may not expose every alternate spelling.
        if fs::symlink_metadata(&path).is_err() {
            continue;
        }
        let meta = fs::symlink_metadata(temp.path().join(actual))
            .expect("identity oracle");
        let alias = fs::symlink_metadata(&path).expect("alias");
        assert_eq!((meta.dev(), meta.ino()), (alias.dev(), alias.ino()));
        let target = Target {
            path,
            bytes: 0,
            is_dir: false,
            hidden: false,
        };
        let recheck = Measurement::new(
            &ScanOptions::default(),
            std::slice::from_ref(&target),
        );
        let measured = recheck.target(&target).expect("alias recheck");
        assert_eq!(measured.bytes, meta.blocks() * 512);
    }
}

#[test]
fn sibling_aliases_share_one_identity_lookup_pass() {
    let temp = tempfile::tempdir().expect("APFS fixture");
    let mut targets = Vec::new();
    for index in 0..12 {
        let original = temp.path().join(format!("café{index}"));
        fs::write(&original, vec![b'x'; 8192]).expect("file");
        let alias = temp.path().join(format!("CAFÉ{index}"));
        // Case-sensitive APFS does not expose alternate case spellings.
        if fs::symlink_metadata(&alias).is_ok() {
            targets.push(Target {
                path: alias,
                bytes: 0,
                is_dir: false,
                hidden: false,
            });
        }
    }
    let paths: Vec<_> = targets.iter().map(|t| t.path.clone()).collect();
    let expected = oracle_bytes(&paths);
    let recheck = Measurement::new(&ScanOptions::default(), &targets);
    crate::scan::leaf_work::reset();
    let actual: u64 = targets
        .iter()
        .map(|target| recheck.target(target).expect("alias recheck").bytes)
        .sum();
    let (opens, entries) = crate::scan::leaf_work::snapshot();
    assert_eq!(actual, expected);
    assert!(opens <= 2, "{opens} listings for one parent");
    assert!(entries <= 24, "{entries} entries for 12 siblings");
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 16, failure_persistence: None, ..ProptestConfig::default()
    })]
    #[test]
    fn real_apfs_scan_and_recheck_agree(
        clones in any::<bool>(), hardlinks in any::<bool>(),
        blocks in 1..17_usize
    ) {
        compare(clones, hardlinks, blocks);
    }
}
