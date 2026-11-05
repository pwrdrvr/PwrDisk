//! The filesystem, rather than clone IDs, decides what deletion frees.

use std::fs::{self, File, OpenOptions};
use std::io::{Read as _, Seek as _, SeekFrom, Write as _};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::{Mutex, MutexGuard};

use proptest::prelude::*;

use super::estimate;
use crate::scan::{ScanOptions, scan};

const BLOCK: usize = 4096;
// DiskImages serializes device setup internally. Avoid competing oracle
// mounts while libtest runs the named regression and property together.
static IMAGES: Mutex<()> = Mutex::new(());

struct Image {
    temp: Option<tempfile::TempDir>,
    mounted: bool,
    device: Option<PathBuf>,
    eject: fn(&Path, bool) -> Result<(), String>,
    _guard: MutexGuard<'static, ()>,
}

fn attachment_device(plist: &[u8]) -> PathBuf {
    let source = tempfile::NamedTempFile::new().expect("attachment plist");
    fs::write(source.path(), plist).expect("attachment records");
    let output = checked(
        Command::new("plutil")
            .args(["-extract", "system-entities", "json", "-o", "-"])
            .arg(source.path()),
    );
    let entries: Vec<serde_json::Value> =
        serde_json::from_slice(&output.stdout)
            .expect("structured attachment records");
    let mut devices = entries.iter().filter_map(|entry| {
        (entry["content-hint"] == "GUID_partition_scheme")
            .then(|| entry["dev-entry"].as_str())
            .flatten()
    });
    let device = devices.next().expect("backing image device");
    assert!(devices.next().is_none(), "one private backing image");
    let suffix = device.strip_prefix("/dev/disk").expect("disk device");
    assert!(!suffix.is_empty() && suffix.bytes().all(|b| b.is_ascii_digit()));
    PathBuf::from(device)
}

fn eject(root: &Path, force: bool) -> Result<(), String> {
    let mut command = Command::new("hdiutil");
    command.args(["detach", "-quiet"]);
    if force {
        command.arg("-force");
    }
    let output = command
        .arg(root)
        .output()
        .map_err(|error| error.to_string())?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "{command:?}: {}\n{}\n{}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        ))
    }
}

fn checked(command: &mut Command) -> Output {
    let output = command.output().expect("start filesystem oracle command");
    assert!(
        output.status.success(),
        "{command:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

impl Image {
    fn new() -> Self {
        let guard = IMAGES
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut image = Self {
            temp: Some(tempfile::tempdir().expect("private image directory")),
            mounted: false,
            device: None,
            eject,
            _guard: guard,
        };
        checked(
            Command::new("hdiutil")
                .args([
                    "create",
                    "-quiet",
                    "-size",
                    "128m",
                    "-type",
                    "SPARSE",
                    "-fs",
                    "APFS",
                    "-volname",
                    "PwrDisk-reclaim-oracle",
                ])
                .arg(image.backing()),
        );
        fs::create_dir(image.root()).expect("private mount point");
        image.attach();
        fs::write(image.root().join(".metadata_never_index"), [])
            .expect("keep Spotlight out of the oracle");
        fs::create_dir(image.root().join("sel")).expect("selection");
        image
    }

    fn backing(&self) -> PathBuf {
        self.temp
            .as_ref()
            .expect("image directory")
            .path()
            .join("oracle.sparseimage")
    }

    fn root(&self) -> PathBuf {
        self.temp
            .as_ref()
            .expect("image directory")
            .path()
            .join("mount")
    }

    fn attach(&mut self) {
        // A reattach can receive another device number. Never retain the
        // previous attachment's identity if decoding this one fails.
        self.device = None;
        let output = checked(
            Command::new("hdiutil")
                .args([
                    "attach",
                    "-plist",
                    "-nobrowse",
                    "-noautoopen",
                    "-mountpoint",
                ])
                .arg(self.root())
                .arg(self.backing()),
        );
        self.mounted = true;
        self.device = Some(attachment_device(&output.stdout));
    }

    fn detach(&mut self) {
        self.try_detach().expect("detach private APFS image");
    }

    fn try_detach(&mut self) -> Result<(), String> {
        let target = self.device.clone().unwrap_or_else(|| self.root());
        (self.eject)(&target, false).or_else(|normal| {
            // A busy eject can leave APFS unmounted. The backing device
            // returned by our attach still exists when the mount path does not.
            (self.eject)(&target, true)
                .map_err(|forced| format!("{normal}\nforced detach: {forced}"))
        })?;
        self.mounted = false;
        self.device = None;
        Ok(())
    }

    fn close(mut self) {
        // A final unmount failure still fails a successful test. Drop is
        // best-effort because it also runs during assertion unwinding.
        self.detach();
    }

    fn free(&mut self) -> u64 {
        // A remount commits APFS's deferred frees and retires old checkpoints.
        // No other process writes files on this private, unbrowsed volume.
        self.detach();
        self.attach();
        let output = checked(Command::new("df").arg("-kP").arg(self.root()));
        let text = String::from_utf8(output.stdout).expect("df output");
        text.lines()
            .nth(1)
            .expect("df row")
            .split_whitespace()
            .nth(3)
            .expect("available KiB")
            .parse::<u64>()
            .expect("available count")
            * 1024
    }
}

fn fail_eject(_: &Path, _: bool) -> Result<(), String> {
    Err("injected detach failure".into())
}

#[test]
fn failed_image_cleanup_does_not_panic_or_remove_its_backing_files() {
    let temp = tempfile::tempdir().expect("cleanup fixture");
    let retained = temp.path().to_owned();
    fs::write(retained.join("oracle.sparseimage"), b"owned image")
        .expect("backing file");
    // A failed unmount means the directory can still hold a live mount.
    // A stand-in ejector exercises that state without leaving a real disk.
    let image = Image {
        temp: Some(temp),
        mounted: true,
        device: None,
        eject: fail_eject,
        _guard: IMAGES
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner),
    };
    let outcome =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            drop(image);
        }));
    assert!(outcome.is_ok(), "cleanup must not introduce a second panic");
    assert!(retained.join("oracle.sparseimage").exists());
    fs::remove_dir_all(retained).expect("remove the unmounted stand-in");
}

fn eject_succeeds(_: &Path, force: bool) -> Result<(), String> {
    if force {
        Err("unnecessary forced retry".into())
    } else {
        Ok(())
    }
}

fn force_succeeds(root: &Path, force: bool) -> Result<(), String> {
    if force {
        Ok(())
    } else {
        fail_eject(root, false)
    }
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 32,
        failure_persistence: None,
        ..ProptestConfig::default()
    })]

    /// Failed unmounts leave the backing files intact; destruction never
    /// introduces a panic, including when it runs during another failure.
    #[test]
    fn image_cleanup_follows_the_unmount_result(
        normal_ok in any::<bool>(),
        forced_ok in any::<bool>(),
        unwinding in any::<bool>(),
    ) {
        let temp = tempfile::tempdir().expect("cleanup model");
        let retained = temp.path().to_owned();
        fs::write(retained.join("oracle.sparseimage"), b"owned image")
            .expect("backing file");
        let eject = if normal_ok { eject_succeeds }
            else if forced_ok { force_succeeds }
            else { fail_eject };
        let image = Image {
            temp: Some(temp),
            mounted: true,
            device: None,
            eject,
            _guard: IMAGES.lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        };
        let outcome = std::panic::catch_unwind(
            std::panic::AssertUnwindSafe(|| {
                let image = image;
                assert!(!unwinding, "original oracle failure");
                drop(image);
            }),
        );
        prop_assert_eq!(outcome.is_err(), unwinding);
        if let Err(payload) = outcome {
            prop_assert_eq!(payload.downcast_ref::<&str>(),
                Some(&"original oracle failure"));
        }
        prop_assert_eq!(retained.join("oracle.sparseimage").exists(),
            !normal_ok && !forced_ok);
        if retained.exists() {
            fs::remove_dir_all(retained)
                .expect("remove the unmounted stand-in");
        }
    }
}

#[test]
fn an_unmounted_volume_still_detaches_its_backing_image() {
    let mut image = Image::new();
    let device = image.device.clone().expect("attached device");
    let retained = image
        .temp
        .as_ref()
        .expect("image directory")
        .path()
        .to_owned();
    // A busy detach can unmount APFS before failing to eject the image.
    // Reproduce that state directly, without relying on background readers.
    checked(
        Command::new("diskutil")
            .args(["unmount", "force"])
            .arg(image.root()),
    );
    let outcome =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            image.detach();
        }));
    drop(image);
    if outcome.is_err() {
        // On pre-fix code the mount point is gone, but the recorded image
        // device still belongs to this test and must be released explicitly.
        eject(&device, true).expect("release the retained private image");
        fs::remove_dir_all(retained).expect("remove retained backing files");
    }
    assert!(outcome.is_ok(), "unmounting must not lose the eject target");
}

impl Drop for Image {
    fn drop(&mut self) {
        if self.mounted
            && let Err(error) = self.try_detach()
        {
            // TempDir would recurse into a still-mounted volume. Retain the
            // backing directory, and never turn an assertion into an abort.
            let retained = self.temp.take().expect("image directory").keep();
            eprintln!(
                "{error}\nprivate APFS image retained at {}",
                retained.display()
            );
        }
    }
}

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
    image: &mut Image,
    uncertain: bool,
) -> Result<(), TestCaseError> {
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
    Ok(())
}

#[test]
fn truncated_external_clone_retains_half_a_selected_clone_group() {
    let mut image = Image::new();
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
    check_deletion(&mut image, true)
        .expect("lower bound survives real deletion");
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
        let mut image = Image::new();
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
        check_deletion(&mut image, uncertain)?;
        image.close();
    }
}
