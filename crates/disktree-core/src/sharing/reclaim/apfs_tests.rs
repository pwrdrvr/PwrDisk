//! The filesystem, rather than clone IDs, decides what deletion frees.

use std::fs::{self, File, OpenOptions};
use std::io::{Read as _, Seek as _, SeekFrom, Write as _};
use std::path::Path;
use std::process::Command;

use proptest::prelude::*;

use super::estimate;
use crate::scan::{ScanOptions, scan};
use crate::test_paths::disk_image::{Image, checked};

const BLOCK: usize = 4096;

fn random_file(path: &Path, bytes: usize) {
    let mut data = vec![0; bytes];
    File::open("/dev/urandom")
        .expect("random data")
        .read_exact(&mut data)
        .expect("incompressible contents");
    fs::write(path, data).expect("write original");
}

fn clone_file(source: &Path, target: &Path) {
    checked(Command::new("cp").arg("-c").arg(source).arg(target));
}

fn check_deletion(
    image: &Image,
    uncertain: bool,
) -> Result<u64, TestCaseError> {
    let before = image.free();
    let tree =
        scan(&image.root(), ScanOptions::default()).expect("scan volume");
    let selected = tree
        .root()
        .children()
        .iter()
        .find(|node| node.name() == "sel")
        .expect("scanned selection");
    // A modified survivor may also be outside the scanned root entirely.
    let narrow = scan(&image.root().join("sel"), ScanOptions::default())
        .expect("scan only the selection");
    let bounds = [estimate([selected]), estimate([narrow.root()])];
    let cached = selected.sharing();
    fs::remove_dir_all(image.root().join("sel"))
        .expect("delete only the private image's selection");
    let after = image.free();
    let freed = after.saturating_sub(before);
    for bound in bounds {
        prop_assert!(
            freed >= bound.lower,
            "deletion freed {freed} bytes, estimate {bound:?}"
        );
        prop_assert!(
            !uncertain || bound.partial,
            "clone IDs cannot rule out modified survivors: {bound:?}"
        );
        prop_assert!(bound.upper >= bound.lower, "{bound:?}");
    }
    prop_assert!(
        freed >= cached.reclaimable(selected.bytes),
        "deletion freed {freed} bytes, cached sharing {cached:?}"
    );
    prop_assert_eq!(cached.estimate, Some(bounds[0]));
    Ok(freed)
}

#[test]
fn truncated_external_clone_retains_half_a_selected_clone_group() {
    let image = Image::new();
    let source = image.root().join("sel/A");
    random_file(&source, 4 * 1024 * 1024);
    clone_file(&source, &image.root().join("sel/B"));
    let survivor = image.root().join("C");
    clone_file(&source, &survivor);
    OpenOptions::new()
        .write(true)
        .open(survivor)
        .expect("survivor")
        .set_len(2 * 1024 * 1024)
        .expect("truncate clone");
    let freed = check_deletion(&image, true)
        .expect("lower bound survives real deletion");
    assert!(
        freed < 4 * 1024 * 1024,
        "the modified survivor must still retain original blocks: {freed}"
    );
    image.close();
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 32,
        failure_persistence: None,
        // Each case mounts a real filesystem; the named regression retains
        // the minimal counterexample without hundreds of extra mounts.
        max_shrink_iters: 16,
        ..ProptestConfig::default()
    })]

    #[test]
    fn apfs_deletion_frees_at_least_the_estimated_lower_bound(
        blocks in 256_usize..1025,
        copies in prop::collection::vec(
            (any::<bool>(), 0_u8..4, 1_u64..128), 1..6,
        ),
    ) {
        let image = Image::new();
        let source = image.root().join("sel/A");
        random_file(&source, blocks * BLOCK);
        clone_file(&source, &image.root().join("sel/B"));
        let paths: Vec<_> = copies.iter().enumerate().map(|(at, (inside, _, _))| {
            let path = image.root().join(if *inside {
                format!("sel/copy-{at}")
            } else {
                format!("copy-{at}")
            });
            clone_file(&source, &path);
            path
        }).collect();
        for (path, (_, operation, offset)) in paths.iter().zip(&copies) {
            let mut file = OpenOptions::new().write(true).open(path)
                .expect("clone to modify");
            match operation {
                0 => {},
                1 => file.set_len(offset * BLOCK as u64).expect("truncate"),
                2 => {
                    file.seek(SeekFrom::End(0)).expect("append position");
                    file.write_all(&vec![0x5a; *offset as usize * BLOCK])
                        .expect("append");
                },
                _ => {
                    file.seek(SeekFrom::Start(offset * BLOCK as u64))
                        .expect("overwrite position");
                    file.write_all(&[0xa5; BLOCK]).expect("overwrite");
                },
            }
            file.sync_all().expect("commit modified clone");
        }
        // Ordinary private files must keep contributing a useful lower bound.
        random_file(&image.root().join("sel/private"), 16 * BLOCK);
        // An untouched external member still conclusively retains the full
        // original data fork. Otherwise selecting its whole ID is uncertain.
        let uncertain = copies.iter().all(|(inside, operation, _)| {
            *inside || *operation != 0
        });
        check_deletion(&image, uncertain)?;
        image.close();
    }
}
