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
// hdiutil leaves this content hint untranslated for synthesized containers.
const APFS_CONTAINER: &str = "EF57347C-0000-11AA-AA11-00306543ECAC";
// DiskImages serializes device setup internally. Avoid competing oracle
// mounts while libtest runs the named regression and property together.
static IMAGES: Mutex<()> = Mutex::new(());

struct Image {
    temp: Option<tempfile::TempDir>,
    mounted: bool,
    device: Option<PathBuf>,
    volume: Option<PathBuf>,
    eject: fn(&Path, bool) -> Result<(), String>,
    attachment: fn(&Path) -> Result<Option<PathBuf>, String>,
    _guard: MutexGuard<'static, ()>,
}

fn plist_field(plist: &[u8], field: &str) -> Result<serde_json::Value, String> {
    let source = tempfile::NamedTempFile::new()
        .map_err(|error| format!("attachment plist: {error}"))?;
    fs::write(source.path(), plist)
        .map_err(|error| format!("attachment records: {error}"))?;
    let output = run(Command::new("plutil")
        .args(["-extract", field, "json", "-o", "-"])
        .arg(source.path()))?;
    serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("structured attachment records: {error}"))
}

fn physical_device(entries: &[serde_json::Value]) -> Result<PathBuf, String> {
    let mut devices = entries.iter().filter_map(|entry| {
        (entry["content-hint"] == "GUID_partition_scheme")
            .then(|| entry["dev-entry"].as_str())
            .flatten()
    });
    let device = devices.next().ok_or("missing backing image device")?;
    if devices.next().is_some() {
        return Err("multiple backing image devices".into());
    }
    disk_device(device)
}

fn disk_device(device: &str) -> Result<PathBuf, String> {
    let suffix = device.strip_prefix("/dev/disk").unwrap_or("");
    if suffix.is_empty() || !suffix.bytes().all(|b| b.is_ascii_digit()) {
        return Err(format!("invalid backing image device: {device}"));
    }
    Ok(PathBuf::from(device))
}

fn cleanup_device(entries: &[serde_json::Value]) -> Result<PathBuf, String> {
    let physical = physical_device(entries)?;
    let mut containers = entries.iter().filter_map(|entry| {
        (entry["content-hint"] == APFS_CONTAINER)
            .then(|| entry["dev-entry"].as_str())
            .flatten()
    });
    let Some(container) = containers.next() else {
        // A partially completed detach may have retired the APFS container
        // already. The remaining owned physical attachment still needs to go.
        return Ok(physical);
    };
    if containers.next().is_some() {
        return Err("multiple owned APFS containers".into());
    }
    // Unmounting volumes does not retire their synthesized container. Ask
    // that container to detach before ejecting the store it holds open.
    disk_device(container)
}

fn attachment(backing: &Path) -> Result<Option<PathBuf>, String> {
    let output = run(Command::new("hdiutil").args(["info", "-plist"]))?;
    let records = plist_field(&output.stdout, "images")?;
    let images = records.as_array().ok_or("missing image records")?;
    let backing = backing
        .canonicalize()
        .map_err(|error| format!("owned backing image: {error}"))?;
    let mut owned = None;
    for image in images {
        let path = image["image-path"]
            .as_str()
            .ok_or("missing image path in attachment records")?;
        if Path::new(path)
            .canonicalize()
            .is_ok_and(|path| path == backing)
            && owned.replace(image).is_some()
        {
            return Err("multiple attachments of our backing image".into());
        }
    }
    let Some(image) = owned else {
        return Ok(None);
    };
    let entries = image["system-entities"]
        .as_array()
        .ok_or("missing owned attachment devices")?;
    cleanup_device(entries).map(Some)
}

fn attachment_devices(plist: &[u8], root: &Path) -> (PathBuf, PathBuf) {
    let records = plist_field(plist, "system-entities")
        .expect("structured attachment records");
    let entries = records.as_array().expect("attachment devices");
    let device = physical_device(entries).expect("backing image device");
    let mut mounted = entries
        .iter()
        .filter(|entry| entry["mount-point"].is_string());
    let volume = mounted.next().expect("one mounted APFS volume");
    assert!(
        mounted.next().is_none(),
        "only our private volume is mounted"
    );
    let mount = Path::new(volume["mount-point"].as_str().expect("mount point"));
    assert_eq!(
        mount.canonicalize().expect("mounted root"),
        root.canonicalize().expect("private mount point")
    );
    assert_eq!(volume["volume-kind"], "apfs");
    let volume = volume["dev-entry"].as_str().expect("volume device");
    let suffix = volume.strip_prefix("/dev/disk").expect("APFS device");
    let parts: [&str; 2] =
        suffix.split_once('s').expect("APFS volume slice").into();
    assert!(parts.iter().all(|part| {
        !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit())
    }));
    (device, PathBuf::from(volume))
}

fn eject(root: &Path, force: bool) -> Result<(), String> {
    if force {
        // Release the chosen whole disk's volumes before native detach.
        let unmount = run(Command::new("diskutil")
            .args(["unmountDisk", "force"])
            .arg(root));
        // DiskImages owns the attachment and its helper process. Let its
        // native detach finish cleanup after Disk Arbitration unmounts APFS;
        // debug output preserves the reason if the device remains busy.
        return run(Command::new("hdiutil")
            .args(["detach", "-force", "-debug"])
            .arg(root))
        .map(|_| ())
        .map_err(|detach| match unmount {
            Ok(_) => detach,
            Err(unmount) => format!("{unmount}\n{detach}"),
        });
    }
    run(Command::new("diskutil").arg("eject").arg(root)).map(|_| ())
}

fn run(command: &mut Command) -> Result<Output, String> {
    let output = command
        .output()
        .map_err(|error| format!("{command:?}: {error}"))?;
    if output.status.success() {
        Ok(output)
    } else {
        Err(format!(
            "{command:?}: {}\n{}\n{}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ))
    }
}

fn checked(command: &mut Command) -> Output {
    run(command).expect("filesystem oracle command")
}

fn describe(command: &mut Command) -> String {
    run(command).map_or_else(std::convert::identity, |output| {
        format!(
            "{command:?}:\n{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
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
            volume: None,
            eject,
            attachment,
            _guard: guard,
        };
        run(Command::new("hdiutil")
            .args([
                "create",
                "-verbose",
                "-size",
                "128m",
                "-type",
                "SPARSE",
                "-fs",
                "APFS",
                "-volname",
                "PwrDisk-reclaim-oracle",
            ])
            .arg(image.backing()))
        .unwrap_or_else(|error| {
            let space = describe(
                Command::new("df")
                    .arg("-kP")
                    .arg(image.temp.as_ref().expect("image directory").path()),
            );
            panic!(
                "create private APFS image: {error}\nbacking space:\n{space}"
            );
        });
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
        self.volume = None;
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
        let (device, volume) = attachment_devices(&output.stdout, &self.root());
        self.device = Some(device);
        self.volume = Some(volume);
    }

    fn detach(&mut self) {
        self.try_detach().expect("detach private APFS image");
    }

    fn try_detach(&mut self) -> Result<(), String> {
        // Device numbers can disappear or be reused. Resolve ownership from
        // the backing path before each operation, including after an error:
        // a command's exit status alone does not say whether it detached.
        let mut failure = String::new();
        let mut target = (self.attachment)(&self.backing())?;
        for force in [false, true] {
            let Some(device) = target.take() else {
                break;
            };
            if let Err(error) = (self.eject)(&device, force) {
                failure.push('\n');
                failure.push_str(&error);
            }
            target = (self.attachment)(&self.backing())
                .map_err(|error| format!("{failure}\n{error}"))?;
        }
        if let Some(device) = target {
            // Query only the remaining owned device and our private paths.
            // In-memory cleanup models never invoke native diagnostics.
            let diagnostics = if self.device.is_some() {
                format!(
                    "\n{}\n{}\n{}",
                    describe(
                        Command::new("diskutil")
                            .args(["info", "-plist"])
                            .arg(&device)
                    ),
                    describe(
                        Command::new("lsof")
                            .args(["-nP", "--"])
                            .arg(self.backing())
                            .arg(&device)
                    ),
                    describe(
                        Command::new("lsof")
                            .args(["-nP", "+D"])
                            .arg(self.root())
                    )
                )
            } else {
                String::new()
            };
            return Err(format!(
                "{failure}\nowned image still attached: {}{diagnostics}",
                device.display(),
            ));
        }
        self.mounted = false;
        self.device = None;
        self.volume = None;
        Ok(())
    }

    fn close(mut self) {
        // A final unmount failure still fails a successful test. Drop is
        // best-effort because it also runs during assertion unwinding.
        self.detach();
    }

    fn free(&self) -> u64 {
        // A remount commits APFS's deferred frees and retires old checkpoints.
        // Keep the image attached: ejecting its physical store for every
        // reading needlessly tears down the whole APFS container.
        let volume = self.volume.as_ref().expect("attached APFS volume");
        checked(
            Command::new("diskutil")
                .args(["unmount", "force"])
                .arg(volume),
        );
        fs::create_dir_all(self.root()).expect("restore private mount point");
        checked(
            Command::new("diskutil")
                .args(["mount", "nobrowse", "-mountPoint"])
                .arg(self.root())
                .arg(volume),
        );
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

fn unknown_attachment(_: &Path) -> Result<Option<PathBuf>, String> {
    Err("injected attachment query failure".into())
}

fn model_attachment(backing: &Path) -> Result<Option<PathBuf>, String> {
    // Each byte models one observation: gone (0), live container (1), or
    // residual physical store (2). Command status is generated separately.
    let states = fs::read(backing).map_err(|error| error.to_string())?;
    let (state, remaining) = states
        .split_first()
        .ok_or("exhausted attachment-state model")?;
    fs::write(backing, remaining).map_err(|error| error.to_string())?;
    Ok(match state {
        0 => None,
        1 => Some(PathBuf::from("/dev/disk999998")),
        2 => Some(PathBuf::from("/dev/disk999999")),
        _ => return Err("invalid attachment-state model".into()),
    })
}

fn check_retained_backing(eject: fn(&Path, bool) -> Result<(), String>) {
    let temp = tempfile::tempdir().expect("cleanup fixture");
    let retained = temp.path().to_owned();
    fs::write(retained.join("oracle.sparseimage"), b"owned image")
        .expect("backing file");
    // A listed attachment can still hold a live mount, regardless of command
    // status. Stand-ins exercise that state without leaving a real disk.
    let image = Image {
        temp: Some(temp),
        mounted: true,
        device: None,
        volume: None,
        eject,
        attachment: |_| Ok(Some(PathBuf::from("/dev/disk999999"))),
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

#[test]
fn failed_image_cleanup_does_not_panic_or_remove_its_backing_files() {
    check_retained_backing(fail_eject);
}

#[test]
fn successful_eject_status_does_not_remove_an_attached_image() {
    check_retained_backing(force_succeeds);
}

fn eject_succeeds(root: &Path, force: bool) -> Result<(), String> {
    if force {
        fail_eject(root, force)
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

    /// Only confirmed detachment permits backing-file removal. Command
    /// status cannot replace that observation or mask another failure.
    #[test]
    fn image_cleanup_follows_the_unmount_result(
        normal_ok in any::<bool>(),
        forced_ok in any::<bool>(),
        normal_detached in any::<bool>(),
        forced_detached in any::<bool>(),
        attachment_known in any::<bool>(),
        unwinding in any::<bool>(),
    ) {
        let temp = tempfile::tempdir().expect("cleanup model");
        let retained = temp.path().to_owned();
        fs::write(retained.join("oracle.sparseimage"), [
            1, if normal_detached { 0 } else { 2 },
            if forced_detached { 0 } else { 2 },
        ])
            .expect("backing file");
        let eject: fn(&Path, bool) -> Result<(), String> =
            match (normal_ok, forced_ok) {
                (true, true) => |_, _| Ok(()),
                (true, false) => eject_succeeds,
                (false, true) => force_succeeds,
                (false, false) => fail_eject,
            };
        let image = Image {
            temp: Some(temp),
            mounted: true,
            device: None,
            volume: None,
            eject,
            attachment: if attachment_known {
                model_attachment
            } else { unknown_attachment },
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
            !attachment_known || (!normal_detached && !forced_detached));
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

fn check_checkpoint_without_eject(
    checkpoints: usize,
) -> Result<(), TestCaseError> {
    let mut image = Image::new();
    let device = image.device.clone();
    let native_eject = image.eject;
    // Measuring free space must not depend on being able to remove the
    // backing device. Model unavailable ejection without stranding a disk.
    image.eject = fail_eject;
    let outcome =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            for _ in 0..checkpoints {
                assert!(image.free() > 0, "the private volume has free space");
                assert_eq!(image.device, device, "keep the same attachment");
            }
        }));
    image.eject = native_eject;
    image.detach();
    drop(image);
    prop_assert!(outcome.is_ok(), "checkpointing must not eject the image");
    Ok(())
}

#[test]
fn free_space_checkpoint_does_not_need_to_eject_the_image() {
    check_checkpoint_without_eject(1).expect("remount only the APFS volume");
}

#[test]
fn an_open_reader_does_not_prevent_private_image_cleanup() {
    let mut image = Image::new();
    let retained = image.backing();
    // Keep a descriptor alive through cleanup so its fallback must unmount
    // the APFS volume before ejecting the physical image device.
    let reader = File::open(image.root().join(".metadata_never_index"))
        .expect("open reader on our private volume");
    image.detach();
    drop(image);
    drop(reader);
    assert!(
        !retained.exists(),
        "successful cleanup removes its backing file"
    );
}

fn eject_then_report_failure(root: &Path, force: bool) -> Result<(), String> {
    eject(root, force)?;
    Err("injected error after successful device removal".into())
}

fn check_already_ejected(
    checkpoints: usize,
    reported_error: bool,
) -> Result<(), TestCaseError> {
    let mut image = Image::new();
    for _ in 0..checkpoints {
        image.free();
    }
    let backing = image.backing();
    // Exercise completed detachment independently of command exit status,
    // both before cleanup and during a cleanup command that reports failure.
    if reported_error {
        image.eject = eject_then_report_failure;
    } else {
        eject(image.device.as_ref().expect("attached device"), true)
            .expect("externally eject our private image");
    }
    let outcome = image.try_detach();
    drop(image);
    if backing.exists() {
        fs::remove_dir_all(backing.parent().expect("private directory"))
            .expect("remove already detached backing files");
    }
    prop_assert!(outcome.is_ok(), "already detached image: {outcome:?}");
    Ok(())
}

#[test]
fn an_already_ejected_image_is_successful_cleanup() {
    check_already_ejected(0, false).expect("detached backing image is gone");
}

#[test]
fn a_failed_command_that_detached_the_image_is_successful_cleanup() {
    check_already_ejected(0, true).expect("detached backing image is gone");
}

fn busy_physical_store(target: &Path, force: bool) -> Result<(), String> {
    let output = run(Command::new("hdiutil").args(["info", "-plist"]))?;
    let records = plist_field(&output.stdout, "images")?;
    let images = records.as_array().ok_or("attached image records")?;
    let parent = images.iter().any(|image| {
        image["system-entities"].as_array().is_some_and(|entries| {
            entries.iter().any(|entry| {
                entry["dev-entry"].as_str().map(Path::new) == Some(target)
                    && entry["content-hint"] == "GUID_partition_scheme"
            })
        })
    });
    if parent {
        // Model the CI failure: unmounted APFS still holds its store open.
        // The container itself remains detachable with the native tools.
        Err("modeled EBUSY: physical store has a live APFS container".into())
    } else {
        eject(target, force)
    }
}

fn check_busy_parent_cleanup(checkpoints: usize) -> Result<(), TestCaseError> {
    let mut image = Image::new();
    for _ in 0..checkpoints {
        image.free();
    }
    image.eject = busy_physical_store;
    let outcome = image.try_detach();
    let remaining = attachment(&image.backing()).expect("attachment state");
    image.eject = eject;
    // Always release the real image, including on the pre-fix fixture.
    image.mounted = remaining.is_some();
    if image.mounted {
        image.detach();
    }
    drop(image);
    prop_assert!(outcome.is_ok(), "busy physical store: {outcome:?}");
    prop_assert!(remaining.is_none(), "release the entire backing image");
    Ok(())
}

#[test]
fn container_cleanup_releases_a_busy_physical_store() {
    check_busy_parent_cleanup(4).expect("release APFS before its store");
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 4,
        failure_persistence: None,
        ..ProptestConfig::default()
    })]

    #[test]
    fn free_space_checkpoints_preserve_the_backing_attachment(
        checkpoints in 1_usize..5,
    ) {
        check_checkpoint_without_eject(checkpoints)?;
    }

    #[test]
    fn image_cleanup_accepts_an_already_detached_backing_image(
        checkpoints in 0_usize..3,
        reported_error in any::<bool>(),
    ) {
        check_already_ejected(checkpoints, reported_error)?;
    }

    #[test]
    fn image_cleanup_releases_a_busy_parent_after_checkpoints(
        checkpoints in 0_usize..5,
    ) {
        check_busy_parent_cleanup(checkpoints)?;
    }
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
