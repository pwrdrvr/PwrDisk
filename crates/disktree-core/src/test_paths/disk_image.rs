//! Owned, serialized disk images for macOS filesystem oracles.

mod command;

use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use proptest::prelude::*;

// hdiutil leaves this content hint untranslated for synthesized containers.
const APFS_CONTAINER: &str = "EF57347C-0000-11AA-AA11-00306543ECAC";
// Boundary and allocation oracles must share one image lifecycle guard,
// including setup and teardown, while libtest runs them concurrently.
static IMAGES: Mutex<()> = Mutex::new(());
// These private-image operations normally take seconds. A wedged native
// request must fail the oracle instead of holding every other image test.
static COMMANDS: command::Commands =
    command::Commands::new(Duration::from_secs(30));
// Retirement can finish after deleteContainer returns. Allow three bounded
// graceful/forced pairs for the remaining attachment to release its store.
const RELEASE_ATTEMPTS: usize = 6;

fn settle() {
    // Let asynchronous device retirement advance before observing ownership
    // again. Each command remains bounded by COMMANDS's separate deadline.
    std::thread::sleep(Duration::from_millis(250));
}

pub struct Image {
    temp: Option<tempfile::TempDir>,
    mount: Option<PathBuf>,
    mount_temp: Option<tempfile::TempDir>,
    mounted: bool,
    device: Option<PathBuf>,
    volume: Option<PathBuf>,
    eject: fn(&Path, bool) -> Result<(), String>,
    attachment: fn(&Path) -> Result<Option<PathBuf>, String>,
    retire: fn(&Path) -> Result<(), String>,
    settle: fn(),
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

fn container_device(
    entries: &[serde_json::Value],
) -> Result<Option<PathBuf>, String> {
    let mut containers = entries.iter().filter_map(|entry| {
        (entry["content-hint"] == APFS_CONTAINER)
            .then(|| entry["dev-entry"].as_str())
            .flatten()
    });
    let Some(container) = containers.next() else {
        return Ok(None);
    };
    if containers.next().is_some() {
        return Err("multiple owned APFS containers".into());
    }
    disk_device(container).map(Some)
}

fn cleanup_device(entries: &[serde_json::Value]) -> Result<PathBuf, String> {
    let physical = physical_device(entries)?;
    // Unmounted containers can still hold their physical stores open.
    // A partially completed detach can leave only the physical attachment.
    Ok(container_device(entries)?.unwrap_or(physical))
}

fn owned_entities(
    backing: &Path,
) -> Result<Option<Vec<serde_json::Value>>, String> {
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
    physical_device(entries)?;
    Ok(Some(entries.clone()))
}

fn attachment(backing: &Path) -> Result<Option<PathBuf>, String> {
    owned_entities(backing)?
        .map(|entries| cleanup_device(&entries))
        .transpose()
}

fn retire_container(backing: &Path) -> Result<(), String> {
    let Some(entries) = owned_entities(backing)? else {
        return Ok(());
    };
    let Some(container) = container_device(&entries)? else {
        return Ok(());
    };
    // Ejection can leave an unmounted APFS container holding its store open.
    // This filesystem is disposable after measurement. Delete only the
    // container freshly resolved from our backing file, then detach the
    // remaining physical image using another ownership observation.
    run(Command::new("diskutil")
        .args(["apfs", "deleteContainer"])
        .arg(container))
    .map(|_| ())
}

fn only_child(entries: &[serde_json::Value], parent: &Path) -> PathBuf {
    let prefix = format!("{}s", parent.display());
    let mut children = entries.iter().filter_map(|entry| {
        let device = entry["dev-entry"].as_str()?;
        let suffix = device.strip_prefix(&prefix)?;
        (!suffix.is_empty() && suffix.bytes().all(|b| b.is_ascii_digit()))
            .then(|| PathBuf::from(device))
    });
    let child = children.next().expect("one owned child device");
    assert!(
        children.next().is_none(),
        "only one owned partition or volume"
    );
    child
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
    COMMANDS.run(command)
}

pub fn checked(command: &mut Command) -> Output {
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
    pub(crate) fn new() -> Self {
        let mut image = Self::blank(None);
        image.create_container();
        image.add_volume();
        image.mount_volume();
        fs::create_dir(image.root().join("sel")).expect("selection");
        image
    }

    pub(crate) fn mount_at(mount: &Path) -> Self {
        let mut image = Self::blank(Some(mount.to_path_buf()));
        image.create_container();
        image.add_volume();
        image.mount_volume();
        image
    }

    fn blank(mount: Option<PathBuf>) -> Self {
        let guard = IMAGES
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        COMMANDS.check().expect("native APFS oracle unavailable");
        let mut image = Self {
            temp: Some(tempfile::tempdir().expect("private image directory")),
            mount,
            // External scratch can have ownership disabled. Disk Arbitration
            // then requires administrator approval for each custom mount.
            // Only the empty placeholder lives here; all payload remains in
            // the image backing file on the configured scratch volume.
            mount_temp: Some(
                tempfile::tempdir_in("/tmp").expect("private mount directory"),
            ),
            // Formatting and attaching can leave a partial attachment even
            // on failure. Only an authoritative query permits file removal.
            mounted: true,
            device: None,
            volume: None,
            eject,
            attachment,
            retire: retire_container,
            settle,
            _guard: guard,
        };
        // With an explicit layout and no -fs, hdiutil allocates only the
        // backing file. Formatting inside create mounts and ejects its own
        // temporary image; EBUSY can delete the file while leaving it live.
        // Keep every attachment and formatting stage under this guard.
        run(Command::new("hdiutil")
            .args([
                "create",
                "-verbose",
                "-size",
                "128m",
                "-type",
                "SPARSE",
                "-layout",
                "GPTSPUD",
                "-partitionType",
                "Apple_APFS",
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
        fs::create_dir_all(image.root()).expect("private mount point");
        let output = checked(
            Command::new("hdiutil")
                .args(["attach", "-plist", "-nomount"])
                .arg(image.backing()),
        );
        let records = plist_field(&output.stdout, "system-entities")
            .expect("raw attachment records");
        image.device = Some(
            physical_device(
                records.as_array().expect("raw attachment devices"),
            )
            .expect("owned physical image"),
        );
        image
    }

    fn backing(&self) -> PathBuf {
        self.temp
            .as_ref()
            .expect("image directory")
            .path()
            .join("oracle.sparseimage")
    }

    pub(crate) fn root(&self) -> PathBuf {
        self.mount.clone().unwrap_or_else(|| {
            self.mount_temp
                .as_ref()
                .expect("mount directory")
                .path()
                .join("mount")
        })
    }

    fn create_container(&self) {
        let entries = owned_entities(&self.backing())
            .expect("owned attachment")
            .expect("attached backing image");
        let physical = physical_device(&entries).expect("owned physical disk");
        // This owned partition has never been mounted. Format it directly:
        // diskutil's extra unmount/arbitration phase stalled on Intel CI.
        // -w waits for the container to be published before the next stage.
        checked(
            Command::new("/sbin/newfs_apfs")
                .args(["-C", "-w"])
                .arg(only_child(&entries, &physical)),
        );
    }

    fn add_volume(&self) {
        let entries = owned_entities(&self.backing())
            .expect("owned attachment")
            .expect("attached backing image");
        let container = container_device(&entries)
            .expect("owned container")
            .expect("created APFS container");
        // The -nomount attachment keeps automatic mounting disabled while
        // newfs publishes the volume. Only mount_volume chooses its path.
        checked(
            Command::new("/sbin/newfs_apfs")
                .args(["-A", "-w", "-v", "PwrDisk-oracle"])
                .arg(container),
        );
    }

    fn mount_volume(&mut self) {
        let entries = owned_entities(&self.backing())
            .expect("owned attachment")
            .expect("attached backing image");
        let container = container_device(&entries)
            .expect("owned container")
            .expect("created APFS container");
        let volume = only_child(&entries, &container);
        checked(
            Command::new("diskutil")
                .args(["mount", "nobrowse", "-mountPoint"])
                .arg(self.root())
                .arg(&volume),
        );
        let mounted = owned_entities(&self.backing())
            .expect("mounted attachment")
            .expect("owned backing image");
        let record = mounted
            .iter()
            .find(|entry| {
                entry["dev-entry"].as_str().map(Path::new) == Some(&volume)
            })
            .expect("mounted volume record");
        let root = Path::new(
            record["mount-point"].as_str().expect("mounted APFS volume"),
        );
        assert_eq!(
            root.canonicalize().expect("mounted path"),
            self.root().canonicalize().expect("private mount point")
        );
        self.volume = Some(volume);
        fs::write(self.root().join(".metadata_never_index"), [])
            .expect("keep Spotlight out of the oracle");
    }

    fn detach(&mut self) {
        self.try_detach().expect("detach private APFS image");
    }

    fn detach_externally(&self) -> Result<(), String> {
        // External-removal tests need a detached backing image with stale
        // bookkeeping. Establish that through the same owned lifecycle as
        // cleanup: a forced eject of the cached physical store can be busy
        // until its synthesized APFS container has retired.
        self.release_backing()
    }

    fn try_detach(&mut self) -> Result<(), String> {
        self.release_backing()?;
        self.mounted = false;
        self.device = None;
        self.volume = None;
        Ok(())
    }

    fn release_backing(&self) -> Result<(), String> {
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
        if target.is_some() {
            if let Err(error) = (self.retire)(&self.backing()) {
                failure.push('\n');
                failure.push_str(&error);
            }
            (self.settle)();
            target = (self.attachment)(&self.backing())
                .map_err(|error| format!("{failure}\n{error}"))?;
            for attempt in 0..RELEASE_ATTEMPTS {
                let Some(device) = target.take() else {
                    break;
                };
                // Restart with graceful ejection after retirement; forced
                // detach can still see EBUSY while the store is releasing.
                if let Err(error) = (self.eject)(&device, attempt % 2 != 0) {
                    failure.push('\n');
                    failure.push_str(&error);
                }
                (self.settle)();
                target = (self.attachment)(&self.backing())
                    .map_err(|error| format!("{failure}\n{error}"))?;
            }
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
        Ok(())
    }

    pub(crate) fn close(mut self) {
        // A final unmount failure still fails a successful test. Drop is
        // best-effort because it also runs during assertion unwinding.
        self.detach();
    }

    pub(crate) fn free(&self) -> u64 {
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

#[test]
fn default_mount_uses_system_scratch_without_moving_the_backing() {
    use std::os::unix::fs::MetadataExt as _;

    let mut image = Image::blank(None);
    let mount_device = fs::metadata(image.root()).expect("mount point").dev();
    let backing_device = fs::metadata(image.backing()).expect("backing").dev();
    image.detach();
    drop(image);
    assert_eq!(
        mount_device,
        fs::metadata("/tmp").expect("system scratch").dev(),
        "mount placeholders must not inherit an external TMPDIR's policy"
    );
    assert_eq!(
        backing_device,
        fs::metadata(std::env::temp_dir())
            .expect("configured scratch")
            .dev(),
        "image payload must stay on the configured scratch volume"
    );
}

fn check_interrupted_setup(stages: u8) -> Result<(), TestCaseError> {
    use std::os::unix::fs::MetadataExt as _;

    let mut image = Image::blank(None);
    // The setup model must preserve the separation at every stage, including
    // an unformatted attachment and a mounted volume whose device differs.
    let mount_parent = image.root().parent().expect("mount parent").to_owned();
    prop_assert_eq!(
        fs::metadata(mount_parent).expect("mount directory").dev(),
        fs::metadata("/tmp").expect("system scratch").dev(),
    );
    prop_assert_eq!(
        fs::metadata(image.backing()).expect("backing").dev(),
        fs::metadata(std::env::temp_dir())
            .expect("configured scratch")
            .dev(),
    );
    let entries = owned_entities(&image.backing())
        .expect("owned raw attachment")
        .expect("attached backing file");
    let premature = container_device(&entries).expect("container records");
    if premature.is_some() {
        image.close();
        return Err(TestCaseError::fail(
            "allocation formatted APFS before fixture-owned setup began",
        ));
    }
    if stages >= 1 {
        image.create_container();
    }
    if stages >= 2 {
        image.add_volume();
    }
    if stages >= 3 {
        image.mount_volume();
    }
    let entries = owned_entities(&image.backing())
        .expect("owned setup attachment")
        .expect("attached backing file");
    let container = container_device(&entries).expect("container records");
    let mounted = entries.iter().any(|entry| entry["mount-point"].is_string());
    let backing_exists = image.backing().exists();
    let backing = image.backing();
    image.detach();
    drop(image);
    prop_assert!(backing_exists, "setup retains its owned backing file");
    prop_assert!(!backing.exists(), "cleanup releases its backing file");
    prop_assert_eq!(container.is_some(), stages >= 1);
    prop_assert_eq!(mounted, stages >= 3);
    Ok(())
}

#[test]
fn an_unformatted_attachment_can_be_released_without_formatting() {
    check_interrupted_setup(0).expect("allocate without hidden formatting");
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 4,
        max_shrink_iters: 4,
        failure_persistence: None,
        ..ProptestConfig::default()
    })]

    #[test]
    fn interrupted_setup_keeps_every_attachment_owned(stages in 0_u8..4) {
        check_interrupted_setup(stages)?;
    }
}

fn unknown_attachment(_: &Path) -> Result<Option<PathBuf>, String> {
    Err("injected attachment query failure".into())
}

fn fail_retirement(_: &Path) -> Result<(), String> {
    Err("injected container retirement failure".into())
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
        mount: None,
        mount_temp: None,
        mounted: true,
        device: None,
        volume: None,
        eject,
        attachment: |_| Ok(Some(PathBuf::from("/dev/disk999999"))),
        retire: |_| Ok(()),
        settle: || {},
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
        retirement_ok in any::<bool>(),
        retirement_detached in any::<bool>(),
        final_detached in any::<bool>(),
        attachment_known in any::<bool>(),
        unwinding in any::<bool>(),
    ) {
        let temp = tempfile::tempdir().expect("cleanup model");
        let retained = temp.path().to_owned();
        let mount_temp = tempfile::tempdir_in("/tmp").expect("mount model");
        let mount_retained = mount_temp.path().to_owned();
        fs::write(mount_retained.join("must-survive"), b"mounted stand-in")
            .expect("mount sentinel");
        fs::write(retained.join("oracle.sparseimage"), [
            1, if normal_detached { 0 } else { 2 },
            if forced_detached { 0 } else { 2 },
            if retirement_detached { 0 } else { 2 },
            if final_detached { 0 } else { 2 },
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
            mount: None,
            mount_temp: Some(mount_temp),
            mounted: true,
            device: None,
            volume: None,
            eject,
            attachment: if attachment_known {
                model_attachment
            } else { unknown_attachment },
            retire: if retirement_ok {
                |_| Ok(())
            } else { fail_retirement },
            settle: || {},
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
            !attachment_known || !(normal_detached || forced_detached
                || retirement_detached || final_detached));
        prop_assert_eq!(mount_retained.join("must-survive").exists(),
            retained.exists(), "uncertain mounts must not be traversed");
        if retained.exists() {
            fs::remove_dir_all(retained)
                .expect("remove the unmounted stand-in");
        }
        if mount_retained.exists() {
            fs::remove_dir_all(mount_retained)
                .expect("remove the unmounted mount stand-in");
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

fn check_external_detachment(
    busy_observations: usize,
) -> Result<(), TestCaseError> {
    let temp = tempfile::tempdir().expect("external detach model");
    let backing = temp.path().join("oracle.sparseimage");
    let mut states = vec![1, 1, 1, 2];
    states.extend(std::iter::repeat_n(2, busy_observations));
    states.extend([0, 0, 0]);
    fs::write(&backing, states).expect("attachment observations");
    let cached_device = Some(PathBuf::from("/dev/disk999997"));
    let mut image = Image {
        temp: Some(temp),
        mount: None,
        mount_temp: None,
        mounted: true,
        device: cached_device.clone(),
        volume: None,
        eject: fail_eject,
        attachment: model_attachment,
        retire: |_| Ok(()),
        settle: || {},
        _guard: IMAGES
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner),
    };
    let external = image.detach_externally();
    let cached_state = (image.mounted, image.device.clone());
    let detached = external.is_ok().then(|| model_attachment(&backing));
    // Clean the stand-in before asserting so a pre-fix failure is safe too.
    let cleanup = image.try_detach();
    drop(image);
    prop_assert!(external.is_ok(), "external setup: {external:?}");
    prop_assert_eq!(detached, Some(Ok(None)));
    prop_assert_eq!(cached_state, (true, cached_device));
    prop_assert!(cleanup.is_ok(), "already detached cleanup: {cleanup:?}");
    prop_assert!(
        !backing.exists(),
        "confirmed detachment releases the fixture"
    );
    Ok(())
}

#[test]
fn external_detachment_handles_a_busy_physical_device() {
    check_external_detachment(1)
        .expect("external setup releases the owned image");
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 32, failure_persistence: None, ..ProptestConfig::default()
    })]

    // Setup must establish observed detachment without changing the cached
    // Image state, even when APFS keeps its physical store busy until retirement.
    #[test]
    fn external_detachment_preserves_stale_bookkeeping(busy_observations in 0_usize..6) {
        check_external_detachment(busy_observations)?;
    }
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
        image
            .detach_externally()
            .expect("externally eject our private image");
    }
    let outcome = image.try_detach();
    drop(image);
    // Drop retains a backing file when ownership cannot prove detachment.
    // Never recursively remove that retained directory to rescue a test.
    prop_assert!(outcome.is_ok(), "already detached image: {outcome:?}");
    prop_assert!(
        !backing.exists(),
        "confirmed detachment removes the backing file"
    );
    Ok(())
}

#[test]
fn an_already_ejected_image_is_successful_cleanup() {
    check_already_ejected(0, false).expect("detached backing image is gone");
}

#[test]
fn an_already_ejected_image_after_one_checkpoint_is_successful_cleanup() {
    // The Intel CI property's minimal failure was fixture setup after one
    // checkpoint, before it could test the already-detached observation.
    check_already_ejected(1, false).expect("checkpointed image is detached");
}

#[test]
fn an_already_ejected_image_after_two_checkpoints_is_successful_cleanup() {
    check_already_ejected(2, false).expect("Intel CI's detached backing image");
}

#[test]
fn a_failed_command_that_detached_the_image_is_successful_cleanup() {
    check_already_ejected(0, true).expect("detached backing image is gone");
}

fn busy_physical_store(target: &Path, force: bool) -> Result<(), String> {
    let output = run(Command::new("hdiutil").args(["info", "-plist"]))?;
    let records = plist_field(&output.stdout, "images")?;
    let images = records.as_array().ok_or("attached image records")?;
    let held = images.iter().any(|image| {
        image["system-entities"].as_array().is_some_and(|entries| {
            entries.iter().any(|entry| {
                entry["dev-entry"].as_str().map(Path::new) == Some(target)
            }) && entries
                .iter()
                .any(|entry| entry["content-hint"] == APFS_CONTAINER)
        })
    });
    if held {
        // CI refused both container and physical-store ejection after all
        // volumes unmounted. Keep that hold until APFS retires its container.
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

fn check_delayed_release(
    busy_observations: usize,
) -> Result<(), TestCaseError> {
    let temp = tempfile::tempdir().expect("delayed retirement model");
    let backing = temp.path().join("oracle.sparseimage");
    // Ejection is busy until retirement starts. The physical attachment
    // remains visible for a generated number of observations afterward.
    let mut states = vec![1, 1, 1, 2];
    states.extend(std::iter::repeat_n(2, busy_observations));
    states.push(0);
    fs::write(&backing, states).expect("attachment observations");
    let mut image = Image {
        temp: Some(temp),
        mount: None,
        mount_temp: None,
        mounted: true,
        device: None,
        volume: None,
        eject: fail_eject,
        attachment: model_attachment,
        retire: |_| Ok(()),
        settle: || {},
        _guard: IMAGES
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner),
    };
    let outcome = image.try_detach();
    let observed_gone = fs::read(&backing)
        .expect("remaining model observations")
        .is_empty();
    drop(image);
    let retained = backing.exists();
    if retained {
        fs::remove_dir_all(backing.parent().expect("model directory"))
            .expect("remove the unmounted stand-in");
    }
    prop_assert!(outcome.is_ok(), "pending retirement: {outcome:?}");
    prop_assert!(observed_gone, "cleanup must observe the completed release");
    prop_assert!(!retained, "completed cleanup releases its backing file");
    Ok(())
}

#[test]
fn retirement_can_finish_after_the_first_eject_attempt() {
    check_delayed_release(1).expect("wait for pending container retirement");
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 32,
        failure_persistence: None,
        ..ProptestConfig::default()
    })]

    #[test]
    fn cleanup_waits_for_pending_container_retirement(
        busy_observations in 0_usize..6,
    ) {
        check_delayed_release(busy_observations)?;
    }
}

#[test]
fn container_cleanup_releases_a_busy_physical_store() {
    check_busy_parent_cleanup(4).expect("release APFS before its store");
}

#[test]
fn container_cleanup_releases_a_busy_store_after_one_checkpoint() {
    check_busy_parent_cleanup(1).expect("release the CI regression's store");
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
            if let Some(mount) = self.mount_temp.take() {
                eprintln!(
                    "mount directory retained at {}",
                    mount.keep().display()
                );
            }
            eprintln!(
                "{error}\nprivate APFS image retained at {}",
                retained.display()
            );
        }
    }
}
