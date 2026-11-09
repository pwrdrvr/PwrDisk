//! Private ranking follows measured allocation, even with uncertain extents.

use std::fs::{self, File};
use std::os::unix::fs::MetadataExt as _;

use proptest::prelude::*;
use serde_json::Value;

use super::*;
use std::process::Command;

fn clone_file(source: &Path, destination: &Path) {
    let output = Command::new("cp")
        .arg("-c")
        .arg(source)
        .arg(destination)
        .output()
        .expect("clone file");
    assert!(
        output.status.success(),
        "cp -c: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn check_private_ranking(
    copies: u32,
    shared_blocks: usize,
    private_blocks: usize,
    modified: u8,
) -> Result<(), TestCaseError> {
    // Keep guarded paths in the checkout, under their canonical spelling.
    let temp = tempfile::tempdir_in(std::env::current_dir().expect("cwd"))
        .expect("fixture");
    let root = temp.path().canonicalize().expect("root");
    let mixed = root.join("mixed");
    let zero = root.join("zero");
    let outside = root.join("outside");
    for path in [&mixed, &zero, &outside] {
        fs::create_dir(path).expect("directory");
    }
    let source = mixed.join("source");
    fs::write(&source, vec![b'a'; shared_blocks * 4096]).expect("shared");
    for index in 1..copies {
        clone_file(&source, &mixed.join(format!("copy-{index}")));
    }
    if modified != 0 {
        let path =
            if modified == 1 { &mixed } else { &outside }.join("modified");
        clone_file(&source, &path);
        File::options()
            .write(true)
            .open(path)
            .expect("modified clone")
            .set_len(4096)
            .expect("truncate");
    }
    let private = mixed.join("private");
    fs::write(&private, vec![b'b'; private_blocks * 4096]).expect("private");
    // A different clone group gives an independently measured zero candidate.
    let retained = outside.join("retained");
    fs::write(&retained, vec![b'c'; 8192]).expect("retained");
    clone_file(&retained, &zero.join("clone"));
    let expected = fs::metadata(private).expect("stat").blocks() * 512;
    prop_assert!(expected > 0);

    let scan = run_scan(root.clone(), ScanOptions::default()).expect("scan");
    let mut session = Session::detached(root.join("unused.sock"), None);
    session.store_scan(scan);
    let ranked = session
        .rank(&RankParams {
            paths: vec![zero.clone(), root.join("missing"), mixed.clone()],
            by: RankBy::default(),
        })
        .expect("ranking");
    let (_, json) = ranked.split_once('\n').expect("JSON after introduction");
    let report: Value = serde_json::Deserializer::from_str(json)
        .into_iter()
        .next()
        .expect("report")
        .expect("valid report");
    let candidates = report["candidates"].as_array().expect("candidates");
    prop_assert_eq!(&candidates[0]["paths"], &serde_json::json!([mixed]));
    prop_assert_eq!(candidates[0]["value"].as_u64(), Some(expected));
    prop_assert_eq!(
        candidates[0]["private_allocation"].as_u64(),
        Some(expected)
    );
    prop_assert_eq!(candidates[0]["metadata_complete"].as_bool(), Some(true));
    prop_assert_eq!(candidates[0]["reclaim"]["partial"].as_bool(), Some(true));
    prop_assert_eq!(candidates[0]["reclaim"]["lower"].as_u64(), Some(expected));
    prop_assert_eq!(&candidates[1]["paths"], &serde_json::json!([zero]));
    prop_assert_eq!(candidates[1]["value"].as_u64(), Some(0));
    prop_assert!(candidates[2]["value"].is_null());
    Ok(())
}

#[test]
fn complete_clone_group_with_private_files_keeps_its_rank() {
    check_private_ranking(2, 2, 1, 0).expect("private bytes stay rankable");
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 32,
        failure_persistence: None,
        ..ProptestConfig::default()
    })]

    #[test]
    fn measured_private_allocation_stays_rankable_with_uncertain_extents(
        copies in 2_u32..6,
        shared_blocks in 2_usize..17,
        private_blocks in 1_usize..17,
        modified in 0_u8..3,
    ) {
        check_private_ranking(copies, shared_blocks, private_blocks, modified)?;
    }
}
