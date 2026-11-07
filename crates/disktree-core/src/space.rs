//! Free space on the volume a path lives on.
//!
//! This is what makes "amount of crap I found" a real number rather than an
//! estimate: the app measures the volume before and after a removal, and shows
//! the projection while marking.

use std::io;
use std::path::{Path, PathBuf};

/// A volume's capacity in bytes.
#[derive(
    Clone,
    Copy,
    Debug,
    Default,
    PartialEq,
    Eq,
    serde::Serialize,
    serde::Deserialize,
)]
pub struct SpaceInfo {
    /// Total size of the volume.
    pub total: u64,
    /// Free blocks, including the reserve only root may write to.
    pub free: u64,
    /// Free blocks this user may actually write; what `df -h` reports.
    pub available: u64,
}

impl SpaceInfo {
    /// Space in use, computed from `free` rather than `available` so the figure
    /// does not jump when a reserve is opened to root.
    pub const fn used(&self) -> u64 {
        self.total.saturating_sub(self.free)
    }

    /// Share of the volume in use, `0.0..=1.0`.
    pub fn used_fraction(&self) -> f32 {
        if self.total == 0 {
            0.0
        } else {
            (self.used() as f64 / self.total as f64) as f32
        }
    }

    /// Free space after `bytes` are removed, saturating at the volume size and
    /// never counting the same byte twice.
    #[must_use]
    pub fn after_removing(&self, bytes: u64) -> Self {
        let available = self.available.saturating_add(bytes).min(self.total);
        let free = self.free.saturating_add(bytes).min(self.total);
        Self {
            total: self.total,
            free,
            available,
        }
    }
}

/// A volume's used bytes can estimate a whole-volume walk, but a folder's
/// unknown size cannot use its containing disk as a completion denominator.
pub(crate) fn scan_volume_used(
    root: &Path,
    disk_root: Option<&Path>,
    space: Option<SpaceInfo>,
    platform: &str,
) -> Option<u64> {
    let whole_volume = disk_root == Some(root)
        // The Data mount is exposed as `/` by `volume_root_for` on macOS.
        || (platform == "macos"
            && disk_root == Some(Path::new("/"))
            && root == Path::new("/System/Volumes/Data"));
    space
        .filter(|_| whole_volume)
        .map(|space| space.used())
        .filter(|&used| used > 0)
}

/// Read the space on the volume containing `path`.
#[cfg(windows)]
pub fn space_info(path: &Path) -> io::Result<SpaceInfo> {
    crate::windows::space_info(path)
}

/// Read the space on the volume containing `path`.
#[cfg(not(windows))]
pub fn space_info(path: &Path) -> io::Result<SpaceInfo> {
    let stat = rustix::fs::statvfs(path)?;
    // `f_frsize` is the fragment size the block counts are expressed in;
    // `f_bsize` is only a hint for I/O. Some filesystems report zero for
    // `f_frsize`, so fall back rather than claiming a zero-sized volume.
    let block = if stat.f_frsize == 0 {
        stat.f_bsize
    } else {
        stat.f_frsize
    };
    Ok(SpaceInfo {
        total: stat.f_blocks.saturating_mul(block),
        free: stat.f_bfree.saturating_mul(block),
        available: stat.f_bavail.saturating_mul(block),
    })
}

/// The volume a path is on, as Windows names it: `C:\`, a share, or the
/// folder a volume is mounted on.
#[cfg(windows)]
pub fn device_for(path: &Path) -> Option<String> {
    crate::windows::volume_root(path).map(|root| root.display().to_string())
}

/// The device a path's filesystem is mounted from, such as
/// `/dev/nvme0n1p2`: the mount with the longest prefix of `path` in
/// `/proc/self/mountinfo`. `None` where that table cannot be read. Non-ASCII
/// bytes and percent signs are percent-escaped so this string is a lossless
/// volume key, including for sources that are not UTF-8.
#[cfg(not(any(target_os = "macos", windows)))]
pub fn device_for(path: &Path) -> Option<String> {
    let mounts = read_mounts(Path::new("/proc/self/mountinfo")).ok()?;
    let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    covering(&mounts, &path).map(|mount| source_key(&mount.source))
}

/// [`device_for`] over a given mount table, for testing.
pub fn device_in(table: &str, path: &Path) -> Option<String> {
    covering(&parse_mounts(table), path).map(|mount| source_key(&mount.source))
}

/// One line of the mount table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mount {
    /// Raw source identity. Decode lossily only when displaying it.
    pub source: Vec<u8>,
    pub point: PathBuf,
    pub fstype: String,
    pub options: String,
}

/// Parse `/proc/self/mounts`.
pub fn parse_mounts(table: &str) -> Vec<Mount> {
    parse_mounts_bytes(table.as_bytes())
}

fn fields(line: &[u8]) -> impl Iterator<Item = &[u8]> {
    line.split(u8::is_ascii_whitespace)
        .filter(|field| !field.is_empty())
}

fn text_field(field: &[u8]) -> String {
    String::from_utf8_lossy(&unescape_octal(field)).into_owned()
}

fn source_key(source: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut key = String::new();
    for &byte in source {
        if byte.is_ascii() && byte != b'%' {
            key.push(char::from(byte));
        } else {
            // Escape the escape marker too: a literal "%80" must not
            // collide with the raw byte 0x80.
            write!(key, "%{byte:02X}").expect("write to string");
        }
    }
    key
}

fn path_field(field: &[u8]) -> PathBuf {
    let bytes = unescape_octal(field);
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt as _;
        std::ffi::OsString::from_vec(bytes).into()
    }
    #[cfg(not(unix))]
    PathBuf::from(String::from_utf8_lossy(&bytes).into_owned())
}

fn parse_mounts_bytes(table: &[u8]) -> Vec<Mount> {
    table
        .split(|&byte| byte == b'\n')
        .filter_map(|line| {
            let mut fields = fields(line);
            // Decode paths as bytes: the kernel need not write UTF-8.
            let source = unescape_octal(fields.next()?);
            let point = path_field(fields.next()?);
            let fstype = text_field(fields.next()?);
            let options = text_field(fields.next().unwrap_or_default());
            Some(Mount {
                source,
                point,
                fstype,
                options,
            })
        })
        .collect()
}

/// Mount points below `root` that are not part of `root`'s volume, which a
/// scan of that volume must not enter.
///
/// "The volume" is the mount *source*, not the device number: btrfs gives
/// every subvolume its own `st_dev`, so `/home` on Omarchy looks like a
/// different filesystem from `/` though it is the same disk. Everything
/// else is left out: pseudo filesystems (`/proc`, `/sys`), tmpfs, other
/// disks, network shares, and automount points — entering one of those
/// would mount a NAS just to measure it. Snapshot subvolumes are left out
/// too, because every file in them shares its blocks with the live one and
/// counting them would count the disk twice.
pub fn foreign_mounts(mounts: &[Mount], root: &Path) -> Vec<PathBuf> {
    let Some(own) = mounts
        .iter()
        .filter(|mount| root.starts_with(&mount.point))
        .max_by_key(|mount| mount.point.as_os_str().len())
    else {
        return Vec::new();
    };
    mounts
        .iter()
        .filter(|mount| mount.point != root && mount.point.starts_with(root))
        .filter(|mount| {
            mount.source != own.source
                || mount.fstype != own.fstype
                || is_snapshot(mount)
        })
        .map(|mount| mount.point.clone())
        .collect()
}

fn is_snapshot(mount: &Mount) -> bool {
    let named = mount
        .point
        .file_name()
        .is_some_and(|name| name == ".snapshots");
    named
        || mount.options.split(',').any(|option| {
            option.starts_with("subvol=") && option.contains("snapshots")
        })
}

/// Visible mounts from `/proc/self/mountinfo`, where the platform keeps it.
///
/// `None` on macOS and Windows, which have no such table and tell a path's
/// volume from the path itself, and wherever `/proc` cannot be read. Read
/// once by whoever asks per frame: [`attribution`] takes it, because the
/// review screen plans on every frame.
#[cfg(not(any(target_os = "macos", windows)))]
pub fn mount_table() -> Option<Vec<Mount>> {
    read_mounts(Path::new("/proc/self/mountinfo")).ok()
}

/// Read separately from parsing so raw kernel path bytes are testable.
#[cfg(not(windows))]
fn read_mounts(path: &Path) -> io::Result<Vec<Mount>> {
    std::fs::read(path).map(|table| {
        let views = parse_mountinfo_bytes(&table);
        mounts_from_views(&views)
    })
}

fn mounts_from_views(views: &[MountView]) -> Vec<Mount> {
    let visible = visible_mount_ids(views);
    views
        .iter()
        .filter(|view| visible.contains(&view.id))
        .map(|view| Mount {
            source: view.source.clone(),
            point: view.point.clone(),
            fstype: view.fstype.clone(),
            options: view.options.clone(),
        })
        .collect()
}

/// [`mount_table`] where there is no `/proc/self/mounts` to read.
#[cfg(any(target_os = "macos", windows))]
pub const fn mount_table() -> Option<Vec<Mount>> {
    None
}

/// Which volume's free space a removal gives back, as far as the meter can
/// tell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Attribution {
    /// The scanned volume: what is removed here is what the meter, which
    /// measures this volume, sees come back.
    Scanned,
    /// Another volume. The space comes back there, so projecting it here
    /// would lend one disk another's bytes.
    Other,
    /// Nothing says which: no saving is claimed.
    Unknown,
}

/// Which volume's free space removing `path` gives back, judged against the
/// volume `root` is on.
///
/// The scan's own rule decides ([`foreign_mounts`]): a volume is the mount
/// *source* and type, not a device number, so `/home` on its own Btrfs
/// subvolume is part of the `/` a user scans, while another disk, a tmpfs, a
/// network share and a snapshot subvolume are not. Marks are not bounded by
/// that rule — `-X` crosses filesystems, and a disk mounted under the root
/// can be marked — so the meter, which measures one volume, has to ask.
///
/// `mounts` is a read of [`mount_table`], kept by the caller because this is
/// asked for every frame; `None` falls back to what the platform itself
/// calls the disk a path is on, the test the scan falls back to when the
/// table cannot be read.
pub fn attribution(
    mounts: Option<&[Mount]>,
    root: &Path,
    path: &Path,
) -> Attribution {
    match mounts {
        Some(mounts) => attribution_in(mounts, root, path),
        None => attributed_by_platform(root, path),
    }
}

/// [`attribution`] over a given mount table.
fn attribution_in(mounts: &[Mount], root: &Path, path: &Path) -> Attribution {
    let (Some(own), Some(other)) =
        (covering(mounts, root), covering(mounts, path))
    else {
        // A path no mount point covers, as one reached through a spelling
        // the table does not carry: there is nothing to attribute it to.
        return Attribution::Unknown;
    };
    if same_volume(own, other) {
        Attribution::Scanned
    } else {
        Attribution::Other
    }
}

/// The mount a path is shown by: the longest of the points above it.
fn covering<'a>(mounts: &'a [Mount], path: &Path) -> Option<&'a Mount> {
    mounts
        .iter()
        .filter(|mount| path.starts_with(&mount.point))
        .max_by_key(|mount| mount.point.as_os_str().len())
}

/// Whether two mounts are one volume, as the scan counts one: one mount, or
/// two mounts of the same source — a Btrfs subvolume beside its filesystem,
/// a bind mount of the same disk, the same disk mounted twice.
///
/// A snapshot subvolume is not its filesystem even though it shares the
/// source: its files share their blocks with the live ones, so removing them
/// gives back almost nothing, and the scan keeps out of it for the same
/// reason. The mount the scan itself asked for is kept at all events, since
/// removing something there empties the very space the meter reads. Only
/// the other mount is tested, as [`foreign_mounts`] does: a system booted
/// from a snapshot (openSUSE's Snapper default) has one at `/`, and its
/// `/var` and `/home` are still the scan's own.
fn same_volume(own: &Mount, other: &Mount) -> bool {
    if own.point == other.point {
        return true;
    }
    own.source == other.source
        && own.fstype == other.fstype
        && !is_snapshot(other)
}

/// [`attribution`] with no table to go by: what the platform itself says
/// about the path, which is what the scan compares in the same straits.
///
/// The device number is all Unix has without `/proc/self/mounts`: an equal
/// number is certainly the same filesystem, and a different one may be
/// another disk or another Btrfs subvolume of this one, which only the table
/// could tell apart, so nothing is claimed for it.
#[cfg(all(unix, not(target_os = "macos")))]
fn attributed_by_platform(root: &Path, path: &Path) -> Attribution {
    use std::os::unix::fs::MetadataExt as _;
    let (Ok(own), Ok(other)) =
        (std::fs::metadata(root), std::fs::metadata(path))
    else {
        return Attribution::Unknown;
    };
    if own.dev() == other.dev() {
        Attribution::Scanned
    } else {
        Attribution::Unknown
    }
}

/// [`attribution`] with no table to go by on macOS: the disk a path is
/// mounted on, with the Data volume's mount folded into `/` by
/// [`volume_root_for`], so `/Users` is the `/` the user thinks of.
#[cfg(target_os = "macos")]
fn attributed_by_platform(root: &Path, path: &Path) -> Attribution {
    compared(root, path, volume_root_for)
}

/// [`attribution`] with no table to go by on Windows: the volume a path is
/// mounted on, folded the way Windows folds spellings, so `C:\` and `c:\`.
#[cfg(windows)]
fn attributed_by_platform(root: &Path, path: &Path) -> Attribution {
    compared(root, path, |path| {
        crate::windows::volume_root(path)
            .map(|root| crate::windows::guard_key(&root))
    })
}

/// Two platform-named volume roots: one root is one volume.
#[cfg(any(target_os = "macos", windows))]
fn compared(
    root: &Path,
    path: &Path,
    volume: impl Fn(&Path) -> Option<PathBuf>,
) -> Attribution {
    let (Some(own), Some(other)) = (volume(root), volume(path)) else {
        return Attribution::Unknown;
    };
    if own == other {
        Attribution::Scanned
    } else {
        Attribution::Other
    }
}

/// There is no way to tell one volume from another here.
#[cfg(not(any(unix, windows)))]
const fn attributed_by_platform(_root: &Path, _path: &Path) -> Attribution {
    Attribution::Unknown
}

/// The top of the disk `path` lives on.
///
/// The shortest mount point above it with the same source. On Omarchy the home directory is the `@home`
/// subvolume at `/home`, and the disk is `/`; with a separate home disk it
/// would be `/home`.
pub fn volume_root(mounts: &[Mount], path: &Path) -> Option<PathBuf> {
    let own = mounts
        .iter()
        .filter(|mount| path.starts_with(&mount.point))
        .max_by_key(|mount| mount.point.as_os_str().len())?;
    mounts
        .iter()
        .filter(|mount| {
            mount.source == own.source
                && mount.fstype == own.fstype
                && path.starts_with(&mount.point)
        })
        .min_by_key(|mount| mount.point.as_os_str().len())
        .map(|mount| mount.point.clone())
}

/// [`volume_root`] for this machine.
#[cfg(not(any(target_os = "macos", windows)))]
pub fn volume_root_for(path: &Path) -> Option<PathBuf> {
    let mounts = read_mounts(Path::new("/proc/self/mountinfo")).ok()?;
    let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    volume_root(&mounts, &path)
}

/// The top of the volume `path` lives on: its drive, such as `C:\`.
#[cfg(windows)]
pub fn volume_root_for(path: &Path) -> Option<PathBuf> {
    crate::windows::volume_root(path)
}

/// Every nested mount is a scan boundary, including bind mounts and
/// same-device subvolumes. Selecting a mount explicitly makes it a root.
pub fn mount_boundaries(mounts: &[Mount], root: &Path) -> Vec<PathBuf> {
    mounts
        .iter()
        .filter(|mount| mount.point != root && mount.point.starts_with(root))
        .map(|mount| mount.point.clone())
        .collect()
}

/// Boundaries from this machine's own mount namespace.
pub fn mount_boundaries_for(root: &Path) -> Option<Vec<PathBuf>> {
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        Some(mount_boundaries(&mount_table()?, root))
    }
    #[cfg(target_os = "macos")]
    {
        let output = std::process::Command::new("mount").output().ok()?;
        if !output.status.success() {
            return None;
        }
        // Data's firmlinks are another namespace for the same entries;
        // canonicalize keeps that spelling. A mount made through /Users
        // must also bound a scan through /System/Volumes/Data/Users.
        let alias = root
            .strip_prefix(MACOS_DATA_VOLUME)
            .ok()
            .map(|below| Path::new("/").join(below));
        Some(
            parse_macos_mounts(&String::from_utf8_lossy(&output.stdout))
                .into_iter()
                .filter(|point| point != root)
                .filter_map(|point| {
                    point
                        .strip_prefix(root)
                        .ok()
                        .or_else(|| {
                            alias.as_ref().and_then(|alias| {
                                point.strip_prefix(alias).ok()
                            })
                        })
                        .filter(|below| !below.as_os_str().is_empty())
                        .map(|below| root.join(below))
                })
                .collect(),
        )
    }
    #[cfg(windows)]
    {
        Some(
            crate::windows::mount_points()
                .into_iter()
                .filter(|point| point != root && point.starts_with(root))
                .collect(),
        )
    }
}

/// [`foreign_mounts`] for this machine; `None` when the mount table cannot
/// be read, so the caller can fall back to comparing devices.
#[cfg(not(windows))]
pub fn foreign_mounts_for(root: &Path) -> Option<Vec<PathBuf>> {
    let mounts = read_mounts(Path::new("/proc/self/mountinfo")).ok()?;
    Some(foreign_mounts(&mounts, root))
}

/// One line of `/proc/self/mountinfo`.
///
/// What [`Mount`] says, plus which directory of the filesystem the mount
/// shows. `/proc/self/mounts` leaves that out, and it is what tells a bind
/// mount from the filesystem itself.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MountView {
    id: u64,
    parent: u64,
    /// Kernel filesystem identity; source names such as `tmpfs` repeat.
    pub device: String,
    pub source: Vec<u8>,
    pub fstype: String,
    pub point: PathBuf,
    /// The directory inside the filesystem that appears at `point`: `/` for
    /// a whole filesystem, `/@home` for a Btrfs subvolume, the bound
    /// directory for a bind mount.
    pub fs_root: PathBuf,
    pub options: String,
}

/// Parse `/proc/self/mountinfo`. Lines it cannot read are skipped.
pub fn parse_mountinfo(table: &str) -> Vec<MountView> {
    parse_mountinfo_bytes(table.as_bytes())
}

/// Parse every layer without requiring UTF-8 names.
///
/// Keep hidden layers too: removal guards need the complete table to refuse
/// ambiguous stacked media.
/// Discovery and accounting resolve visibility separately, by parent ID.
pub fn parse_mountinfo_bytes(table: &[u8]) -> Vec<MountView> {
    table
        .split(|&byte| byte == b'\n')
        .filter_map(|line| {
            // Optional fields run up to a lone `-`; after it come the type
            // and the source.
            let separator = line.windows(3).position(|part| part == b" - ")?;
            let mut left = fields(&line[..separator]);
            let id = std::str::from_utf8(left.next()?).ok()?.parse().ok()?;
            let parent =
                std::str::from_utf8(left.next()?).ok()?.parse().ok()?;
            let device = text_field(left.next()?);
            let fs_root = path_field(left.next()?);
            let point = path_field(left.next()?);
            let mut options = text_field(left.next()?);
            let mut right = fields(&line[separator + 3..]);
            let fstype = text_field(right.next()?);
            let source = unescape_octal(right.next()?);
            // /proc/mounts combined per-mount and superblock options.
            // Keep that policy input, including Btrfs's subvol= option.
            options.push(',');
            options.push_str(&text_field(right.next()?));
            Some(MountView {
                id,
                parent,
                device,
                source,
                fstype,
                point,
                fs_root,
                options,
            })
        })
        .collect()
}

fn visible_mount_ids(records: &[MountView]) -> std::collections::HashSet<u64> {
    use std::collections::{HashMap, HashSet};
    let ids: HashSet<_> = records.iter().map(|record| record.id).collect();
    let mut children: HashMap<_, Vec<_>> = HashMap::new();
    let mut pending = Vec::new();
    for record in records {
        if record.parent == record.id || !ids.contains(&record.parent) {
            // chroot can leave the root's parent outside the table.
            pending.push(record);
        } else {
            children.entry(record.parent).or_default().push(record);
        }
    }
    for group in children.values_mut() {
        group.sort_by_key(|record| {
            (record.point.components().count(), record.id)
        });
    }
    let mut visible = HashMap::new();
    let mut visited = HashSet::new();
    while let Some(mut record) = pending.pop() {
        if !visited.insert(record.id) {
            continue;
        }
        // A nearer mount can hide a sibling attached to the old parent
        // below it, even though that sibling has no same-point overmount.
        let parent = record.point.parent().and_then(|point| {
            point.ancestors().find_map(|point| visible.get(point))
        });
        if parent.is_some_and(|&id| id != record.parent) {
            continue;
        }
        // At a stacked point, only the top layer's children are reachable.
        // proc_pid_mountinfo(5): the upper mount names the lower as parent.
        while let Some(upper) = children.get(&record.id).and_then(|group| {
            group.iter().find(|upper| upper.point == record.point)
        }) {
            if !visited.insert(upper.id) {
                break;
            }
            record = upper;
        }
        visible.insert(record.point.as_path(), record.id);
        if let Some(group) = children.get(&record.id) {
            // Visit nearer siblings first, so their hidden descendants
            // are rejected by the ancestor check above.
            pending.extend(group.iter().rev().copied());
        }
    }
    visible.into_values().collect()
}

/// The kernel writes space, tab, newline and backslash in mount fields as
/// three octal digits after a backslash. Decoded in one pass, so a name that
/// really contains `\040` is not decoded twice.
fn unescape_octal(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        let digits = bytes.get(index + 1..index + 4);
        let value = digits
            .filter(|_| bytes[index] == b'\\')
            .and_then(|digits| std::str::from_utf8(digits).ok())
            .and_then(|digits| u8::from_str_radix(digits, 8).ok());
        if let Some(value) = value {
            out.push(value);
            index += 4;
        } else {
            out.push(bytes[index]);
            index += 1;
        }
    }
    out
}

/// Paths under `root` that show a directory the scan already reaches
/// another way, so walking them would count the same files twice.
///
/// A bind mount, a second mount of the same disk, or the top of a Btrfs
/// filesystem mounted beside its subvolumes all show one directory at two
/// paths. Device numbers cannot tell: a bind mount has its original's. The
/// mount table can, since each line names the filesystem and the directory
/// of it that is shown. When one directory is visible twice, the view
/// showing the widest part of the filesystem is kept, and the narrower one
/// is left out. The scanned root itself is never left out; if it is the
/// narrower view, its copy inside the wider one is.
pub fn repeated_mounts(mounts: &[MountView], root: &Path) -> Vec<PathBuf> {
    repeated_mounts_excluding(mounts, root, &[])
}

fn same_filesystem(left: &MountView, right: &MountView) -> bool {
    left.fstype == right.fstype
        && (left.device == right.device
            // Btrfs assigns device numbers per subvolume. Its block device
            // identifies the filesystem shared by those different roots.
            || (left.fstype == "btrfs"
                && left.source.starts_with(b"/dev/")
                && left.source == right.source))
}

fn covering_mount<'a>(
    mounts: &'a [MountView],
    path: &Path,
) -> Option<&'a MountView> {
    mounts
        .iter()
        .filter(|mount| path.starts_with(&mount.point))
        .max_by_key(|mount| mount.point.components().count())
}

fn repeated_mounts_excluding(
    mounts: &[MountView],
    root: &Path,
    excluded: &[PathBuf],
) -> Vec<PathBuf> {
    let visible = visible_mount_ids(mounts);
    let mounts: Vec<_> = mounts
        .iter()
        .filter(|mount| visible.contains(&mount.id))
        .cloned()
        .collect();
    let mounts = mounts.as_slice();
    let own = covering_mount(mounts, root);
    let mut visible: Vec<&MountView> = mounts
        .iter()
        .filter(|mount| {
            (mount.point.starts_with(root) || Some(*mount) == own)
                && !excluded.iter().any(|skip| mount.point.starts_with(skip))
        })
        .collect();
    visible.sort_by_key(|mount| {
        (
            mount.fs_root.components().count(),
            mount.point.components().count(),
            &mount.point,
        )
    });
    let mut kept: Vec<&MountView> = Vec::new();
    let mut repeated: Vec<PathBuf> = Vec::new();
    for mount in visible {
        let elsewhere = kept.iter().find_map(|wider| {
            if !same_filesystem(wider, mount) {
                return None;
            }
            let inside = mount.fs_root.strip_prefix(&wider.fs_root).ok()?;
            let path = wider.point.join(inside);
            if !path.starts_with(root)
                || path == mount.point
                || excluded
                    .iter()
                    .chain(&repeated)
                    .any(|skip| path.starts_with(skip))
            {
                return None;
            }
            // Another mount can hide the supposed original directory.
            let covering = covering_mount(mounts, &path)?;
            let relative = path.strip_prefix(&covering.point).ok()?;
            (same_filesystem(covering, mount)
                && covering.fs_root.join(relative) == mount.fs_root)
                .then_some(path)
        });
        let skip = elsewhere.map(|copy| {
            if mount.point != root && mount.point.starts_with(root) {
                mount.point.clone()
            } else {
                copy
            }
        });
        if let Some(skip) = skip
            && !root.starts_with(&skip)
            // A duplicate view can contain independently mounted data.
            // Keep it rather than hiding that data with its parent.
            && !mounts.iter().any(|child| child.point != skip && child.point.starts_with(&skip))
        {
            if skip != mount.point {
                kept.push(mount);
            }
            repeated.push(skip);
        } else {
            kept.push(mount);
        }
    }
    repeated
}

/// [`repeated_mounts`] for this machine, in `root`'s own spelling. Empty
/// where there is no `/proc/self/mountinfo`.
pub fn repeated_mounts_for(
    root: &Path,
    canonical: &Path,
    one_filesystem: bool,
) -> Vec<PathBuf> {
    let Ok(table) = std::fs::read("/proc/self/mountinfo") else {
        return Vec::new();
    };
    let excluded = if one_filesystem {
        foreign_mounts_for(canonical).unwrap_or_default()
    } else {
        Vec::new()
    };
    repeated_mounts_excluding(
        &parse_mountinfo_bytes(&table),
        canonical,
        &excluded,
    )
    .iter()
    .filter_map(|path| path.strip_prefix(canonical).ok())
    .map(|below| root.join(below))
    .collect()
}

/// The mount points in what macOS's `mount` prints.
///
/// One per line: `/dev/disk3s5 on /System/Volumes/Data (apfs, local, …)`.
/// The point is
/// between the first ` on ` and the last ` (`, so a name with spaces or
/// brackets in it survives.
pub fn parse_macos_mounts(output: &str) -> Vec<PathBuf> {
    output
        .lines()
        .filter_map(|line| {
            let (_, rest) = line.split_once(" on ")?;
            let (point, _) = rest.rsplit_once(" (")?;
            Some(PathBuf::from(point))
        })
        .collect()
}

/// Where the Data volume is mounted on macOS.
///
/// Since Catalina `/` is a read-only system volume, and everything the user
/// can write — `/Users`,
/// `/Applications`, `/Library`, `/private`, … — lives on the Data volume,
/// joined into `/` by firmlinks. The two report the same device, and
/// `/Users` and `/System/Volumes/Data/Users` are the same inode.
pub const MACOS_DATA_VOLUME: &str = "/System/Volumes/Data";

/// Directories under `root` a scan must never enter, whatever the volume
/// rules say, given where `root` really is (`canonical`).
///
/// On macOS: `/Network`, an automount point that would reach for a server.
/// The Data volume is restricted separately to its private service stores,
/// so firmlinked directories are not visited again. The
/// paths are returned in `root`'s own spelling, because that is how the walk
/// names what it finds.
pub fn never_scanned(root: &Path, canonical: &Path) -> Vec<PathBuf> {
    if !cfg!(target_os = "macos") {
        return Vec::new();
    }
    std::iter::once("/Network")
        .filter_map(|skip| Path::new(skip).strip_prefix(canonical).ok())
        .filter(|below| !below.as_os_str().is_empty())
        .map(|below| root.join(below))
        .collect()
}

/// A Data mount below the root must expose only private service stores.
/// Scanning Data directly still walks the entire volume once.
pub fn data_services_only(root: &Path, canonical: &Path) -> Option<PathBuf> {
    if !cfg!(target_os = "macos") {
        return None;
    }
    Path::new(MACOS_DATA_VOLUME)
        .strip_prefix(canonical)
        .ok()
        .filter(|below| !below.as_os_str().is_empty())
        .map(|below| root.join(below))
}

/// The top of the disk `path` lives on: its mount point, from `statfs`.
///
/// The Data volume answers `/System/Volumes/Data`, but the disk a user
/// thinks of is `/`, which shows the same files under the names Finder uses
/// and adds the system volume beside them.
#[cfg(target_os = "macos")]
pub fn volume_root_for(path: &Path) -> Option<PathBuf> {
    let stat = rustix::fs::statfs(path).ok()?;
    let mount = PathBuf::from(c_chars(&stat.f_mntonname));
    if mount.as_os_str().is_empty() {
        return None;
    }
    Some(if mount == Path::new(MACOS_DATA_VOLUME) {
        PathBuf::from("/")
    } else {
        mount
    })
}

/// The device a path's filesystem is mounted from, such as
/// `/dev/disk3s5`, from `statfs`.
#[cfg(target_os = "macos")]
pub fn device_for(path: &Path) -> Option<String> {
    // `/` is a snapshot of the system volume (`disk3s1s1`); name the disk
    // the user's files are on instead, which is what the meter measures.
    let path = if path == Path::new("/") {
        Path::new(MACOS_DATA_VOLUME)
    } else {
        path
    };
    let stat = rustix::fs::statfs(path).ok()?;
    let device = c_chars(&stat.f_mntfromname);
    (!device.is_empty()).then_some(device)
}

/// A NUL-terminated C string field as text.
#[cfg(target_os = "macos")]
fn c_chars(field: &[std::ffi::c_char]) -> String {
    let bytes: Vec<u8> = field
        .iter()
        .take_while(|&&byte| byte != 0)
        .map(|&byte| byte as u8)
        .collect();
    String::from_utf8_lossy(&bytes).into_owned()
}

/// Nothing to leave out by path on Windows.
///
/// There, the only way into another volume below `root` is a folder that
/// volume is mounted on, which is a reparse point the walk treats as a link
/// and does not enter unless links are followed; other drives are separate
/// trees altogether.
#[cfg(windows)]
#[allow(
    clippy::unnecessary_wraps,
    reason = "the Option is the Unix answer when the mount table is missing"
)]
pub const fn foreign_mounts_for(_root: &Path) -> Option<Vec<PathBuf>> {
    Some(Vec::new())
}

/// A mounted volume the picker can switch the scan to: where it is mounted
/// and how much room it has. Listed by [`volumes`].
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Volume {
    /// Where the volume is mounted: `C:\` on Windows, `/` or `/home` on
    /// Linux, `/` on macOS.
    pub point: PathBuf,
    /// What is mounted there, when the table names it: a device such as
    /// `/dev/nvme0n1p2`, or a name like `tmpfs`.
    pub device: Option<String>,
    /// Free space on the volume now; `None` when it cannot be read.
    pub space: Option<SpaceInfo>,
}

/// Every volume worth offering as a scan root, fullest first.
///
/// Pseudo filesystems (`/proc`, `/sys`, tmpfs, …), snapshot subvolumes and
/// automount points are left out: switching the scan to one of those would
/// measure the wrong thing, the same reason [`foreign_mounts`] keeps a scan
/// from entering them. Same-device mounts remain individually selectable:
/// a scan no longer crosses into another mount implicitly.
#[cfg(not(any(target_os = "macos", windows)))]
pub fn volumes() -> Vec<Volume> {
    let Ok(mounts) = read_mounts(Path::new("/proc/self/mountinfo")) else {
        return Vec::new();
    };
    volumes_in(&mounts)
}

/// Read root candidates without opening them, measuring free space, or
/// touching network mounts. Discovery is not permission to start scanning.
pub fn volume_roots() -> io::Result<Vec<Volume>> {
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        let mounts = mount_table()
            .ok_or_else(|| io::Error::other("cannot read mount table"))?;
        Ok(volume_roots_in(&mounts))
    }
    #[cfg(target_os = "macos")]
    {
        let output = std::process::Command::new("mount").output()?;
        if !output.status.success() {
            return Err(io::Error::other("cannot list mounted volumes"));
        }
        Ok(parse_macos_mounts(&String::from_utf8_lossy(&output.stdout))
            .into_iter()
            .filter(|point| point.as_os_str() != MACOS_DATA_VOLUME)
            .map(|point| Volume {
                point,
                device: None,
                space: None,
            })
            .collect())
    }
    #[cfg(windows)]
    {
        Ok(crate::windows::mount_points()
            .into_iter()
            .map(|point| Volume {
                point,
                device: None,
                space: None,
            })
            .collect())
    }
}

/// Root candidates from a visible mount table; never calls statvfs. Live
/// readers resolve mountinfo's parent graph before passing mounts here.
pub fn volume_roots_in(mounts: &[Mount]) -> Vec<Volume> {
    let mut seen: Vec<&Mount> = Vec::new();
    let mut points = std::collections::HashSet::new();
    for mount in mounts.iter().rev() {
        // Record even filtered mounts: a service filesystem may hide a disk.
        if !points.insert(&mount.point) || !is_volume_candidate(mount) {
            continue;
        }
        // Nested mounts are not traversed. Even mounts sharing a device
        // must remain individually selectable; only overmounts collapse.
        seen.push(mount);
    }
    let mut volumes: Vec<Volume> = seen
        .iter()
        .map(|mount| Volume {
            point: mount.point.clone(),
            device: Some(String::from_utf8_lossy(&mount.source).into_owned()),
            space: None,
        })
        .collect();
    volumes.sort_by(|left, right| left.point.cmp(&right.point));
    volumes
}

/// [`volumes`] over a given mount table, for testing.
pub fn volumes_in(mounts: &[Mount]) -> Vec<Volume> {
    let mut volumes = volume_roots_in(mounts);
    for volume in &mut volumes {
        volume.space = space_info(&volume.point).ok();
    }
    sort_by_free_space(&mut volumes);
    volumes
}

/// Fullest first; an unreadable volume sorts last.
fn sort_by_free_space(volumes: &mut [Volume]) {
    volumes.sort_by_key(|volume| {
        (
            volume.space.is_none(),
            volume.space.map_or(0, |space| space.available),
            volume.point.clone(),
        )
    });
}

/// Whether the mount is a real disk worth scanning: a device-backed
/// filesystem that is not a snapshot, an automount point, or one of the
/// pseudo filesystems the scan itself refuses to enter.
fn is_volume_candidate(mount: &Mount) -> bool {
    if is_snapshot(mount) {
        return false;
    }
    if mount
        .options
        .split(',')
        .any(|option| option == "automounted" || option.starts_with("autofs"))
    {
        return false;
    }
    // Sub-mounts under `/proc`, `/sys` and `/dev` (such as
    // `/proc/sys/fs/binfmt_misc` or `/sys/fs/bpf`) are kernel and device
    // interfaces, not storage volumes to offer for a scan.
    if mount.point.starts_with("/proc")
        || mount.point.starts_with("/sys")
        || mount.point.starts_with("/dev")
    {
        return false;
    }
    !matches!(
        mount.fstype.as_str(),
        "autofs"
            | "binfmt_misc"
            | "bpf"
            | "cgroup"
            | "cgroup2"
            | "configfs"
            | "debugfs"
            | "devpts"
            | "devtmpfs"
            | "efivarfs"
            | "fuse.portal"
            | "fuse.lxcfs"
            | "fusectl"
            | "hugetlbfs"
            | "mqueue"
            | "nsfs"
            | "overlay"
            | "proc"
            | "pstore"
            | "ramfs"
            | "rpc_pipefs"
            | "securityfs"
            | "sysfs"
            | "tmpfs"
            | "tracefs"
    )
}

/// Every volume worth offering as a scan root, fullest first.
///
/// The Data volume's second mount is the same disk under another name, so it
/// is left out; the firmlinks joined into `/` mean `/` already shows it.
#[cfg(target_os = "macos")]
pub fn volumes() -> Vec<Volume> {
    use std::process::Command;

    let output = Command::new("mount")
        .output()
        .ok()
        .and_then(|output| String::from_utf8(output.stdout).ok());
    let Some(output) = output else {
        return Vec::new();
    };
    let mut points = parse_macos_mounts(&output);
    points.retain(|point| {
        point.as_os_str() != MACOS_DATA_VOLUME && !point.as_os_str().is_empty()
    });
    let mut volumes: Vec<Volume> = points
        .iter()
        .map(|point| Volume {
            point: point.clone(),
            device: device_for(point),
            space: space_info(point).ok(),
        })
        .collect();
    sort_by_free_space(&mut volumes);
    volumes
}

/// Every place a volume is mounted that can be scanned.
///
/// Drive roots such as `D:\` and folders a volume is mounted on. Unready
/// drives (an empty card reader reports a path but no space) are left out,
/// since there is nothing to measure there.
#[cfg(windows)]
pub fn volumes() -> Vec<Volume> {
    let mut volumes: Vec<Volume> = crate::windows::mount_points()
        .into_iter()
        .filter_map(|point| {
            space_info(&point).ok().map(|space| Volume {
                point,
                device: None,
                space: Some(space),
            })
        })
        .collect();
    sort_by_free_space(&mut volumes);
    volumes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scan_estimates_require_a_whole_volume_and_known_used_space() {
        let space = Some(SpaceInfo {
            total: 1000,
            free: 100,
            available: 80,
        });
        let root = Path::new("/");
        let estimate = |path, disk, space, platform| {
            scan_volume_used(path, disk, space, platform)
        };
        assert_eq!(estimate(root, Some(root), space, "linux"), Some(900));
        assert_eq!(estimate(root, None, space, "linux"), None);
        assert_eq!(estimate(root, Some(root), None, "linux"), None);
        assert_eq!(
            estimate(root, Some(root), Some(SpaceInfo::default()), "linux"),
            None
        );
        assert_eq!(
            estimate(Path::new("/home"), Some(root), space, "linux"),
            None
        );
        let data = Path::new("/System/Volumes/Data");
        assert_eq!(estimate(data, Some(root), space, "macos"), Some(900));
        assert_eq!(estimate(data, Some(root), space, "linux"), None);
        assert_eq!(estimate(data, None, space, "macos"), None);
    }

    const OMARCHY: &str = "\
sys /sys sysfs rw 0 0
run /run tmpfs rw 0 0
/dev/mapper/root / btrfs rw,subvolid=256,subvol=/@ 0 0
/dev/mapper/root /home btrfs rw,subvolid=257,subvol=/@home 0 0
/dev/mapper/root /var/log btrfs rw,subvolid=258,subvol=/@log 0 0
/dev/mapper/root /.snapshots btrfs rw,subvolid=260,subvol=/@snapshots 0 0
/dev/nvme0n1p1 /boot vfat rw 0 0
systemd-1 /mnt/nas-home autofs rw,direct 0 0
tmpfs /tmp tmpfs rw 0 0
portal /run/user/1000/doc fuse.portal rw 0 0
";

    const MOUNTINFO: &str = "\
22 1 0:21 /@ / rw,relatime - btrfs /dev/mapper/root rw,subvol=/@
23 22 0:21 /@home /home rw,relatime - btrfs /dev/mapper/root rw,subvol=/@home
24 22 0:21 /@log /var/log rw,relatime - btrfs /dev/mapper/root rw,subvol=/@log
25 22 0:21 /@snapshots /.snapshots rw - btrfs /dev/mapper/root rw
26 22 0:30 / /data rw,relatime shared:5 - btrfs /dev/mapper/data rw
27 22 0:30 / /mnt/data rw,relatime shared:5 - btrfs /dev/mapper/data rw
28 22 0:31 / /tmp rw - tmpfs tmpfs rw
29 22 259:3 / /boot rw - vfat /dev/nvme0n1p1 rw
";

    fn repeated(table: &str, root: &str) -> Vec<PathBuf> {
        let mut paths =
            repeated_mounts(&parse_mountinfo(table), Path::new(root));
        paths.sort();
        paths
    }

    fn check_source_identity(left: &[u8], right: &[u8]) {
        use std::fmt::Write as _;
        let escape = |bytes: &[u8]| {
            bytes.iter().fold(String::new(), |mut out, byte| {
                write!(out, "\\{byte:03o}").expect("source field");
                out
            })
        };
        let (left_text, right_text) = (escape(left), escape(right));
        let table = format!(
            "{left_text} / nfs rw 0 0\n\
             {right_text} /other nfs rw 0 0\n"
        );
        let mounts = parse_mounts(&table);
        let same = left == right;
        assert_eq!(mounts[0].source == mounts[1].source, same);
        assert_eq!(
            device_in(&table, Path::new("/"))
                == device_in(&table, Path::new("/other")),
            same,
            "volume keys preserve the source identity"
        );
        assert_eq!(
            attribution_in(&mounts, Path::new("/"), Path::new("/other")),
            if same {
                Attribution::Scanned
            } else {
                Attribution::Other
            }
        );
        let views = parse_mountinfo(&format!(
            "1 0 0:1 / / rw - nfs {left_text} rw\n\
             2 1 0:2 / /other rw - nfs {right_text} rw\n"
        ));
        assert_eq!(views[0].source == views[1].source, same);
    }

    #[test]
    fn distinct_non_utf8_sources_have_distinct_volume_identities() {
        check_source_identity(&[128], &[129]);
        check_source_identity(b"server:/disk-\x80", b"server:/disk-\x81");
        check_source_identity(b"server:/disk-\x80", b"server:/disk-%80");
        check_source_identity(
            b"server:/disk-\x80",
            "server:/disk-�".as_bytes(),
        );
    }

    fn check_moved_stack(name: &str, order: [u32; 4], service: bool) {
        let point = format!("/mnt/{name}");
        // An older USB mount (2) was moved onto mount 3. The kernel
        // preserves list order; parent IDs describe the actual stack.
        let hidden = if service {
            format!("3 1 0:9 / {point} rw - tmpfs tmpfs rw")
        } else {
            format!("3 1 8:1 /data {point} rw - ext4 /dev/root rw")
        };
        let mut records = [
            "1 0 8:1 / / rw - ext4 /dev/root rw".to_owned(),
            format!("2 3 8:2 / {point} rw - ext4 /dev/usb rw"),
            hidden,
            format!("4 3 8:3 / {point}/hidden rw - ext4 /dev/hidden rw"),
        ]
        .into_iter()
        .zip(order)
        .collect::<Vec<_>>();
        records.sort_by_key(|(_, rank)| *rank);
        let table = records
            .into_iter()
            .map(|(line, _)| line)
            .collect::<Vec<_>>()
            .join("\n");
        let views = parse_mountinfo(&table);
        assert_eq!(views.len(), 4, "guards retain every layer");
        let mounts = mounts_from_views(&views);
        let visible = covering(&mounts, Path::new(&point)).expect("USB");
        assert_eq!(visible.source, b"/dev/usb");
        let roots = volume_roots_in(&mounts);
        assert_eq!(roots.len(), 2, "only the root and visible USB disk");
        assert!(roots.iter().any(|root| root.point == Path::new(&point)
            && root.device.as_deref() == Some("/dev/usb")));
        assert!(repeated_mounts(&views, Path::new("/")).is_empty());
    }

    #[test]
    fn moving_an_older_disk_over_a_newer_mount_keeps_the_disk_visible() {
        check_moved_stack("a", [0; 4], false);
        check_moved_stack("backup", [0, 1, 2, 3], true);
        check_moved_stack("backup", [3, 2, 1, 0], false);
    }

    #[test]
    fn overmounts_hide_children_attached_to_the_lower_layer() {
        let views = parse_mountinfo(
            "1 0 8:1 / / rw - ext4 /dev/root rw\n\
             2 1 8:2 / /mnt/hidden rw - ext4 /dev/hidden rw\n\
             3 1 8:3 / /mnt rw - ext4 /dev/upper rw\n\
             4 3 8:4 / /mnt/visible rw - ext4 /dev/child rw\n",
        );
        assert_eq!(
            mounts_from_views(&views)
                .iter()
                .map(|view| view.point.clone())
                .collect::<Vec<_>>(),
            [Path::new("/"), Path::new("/mnt"), Path::new("/mnt/visible")]
        );
    }

    fn check_snapshot_options(name: &str) {
        let point = format!("/mnt/{name}");
        let views = parse_mountinfo(&format!(
            "1 0 8:1 / / rw - btrfs /dev/root rw,subvol=/@\n\
             2 1 8:1 /@snapshots/a {point} ro - btrfs /dev/root ro,subvol=/@snapshots/a\n"
        ));
        let mounts: Vec<_> = views
            .into_iter()
            .map(|view| Mount {
                source: view.source,
                point: view.point,
                fstype: view.fstype,
                options: view.options,
            })
            .collect();
        let legacy = parse_mounts(&format!(
            "/dev/root / btrfs rw,subvol=/@ 0 0\n\
             /dev/root {point} btrfs ro,subvol=/@snapshots/a 0 0\n"
        ));
        assert_eq!(
            foreign_mounts(&mounts, Path::new("/")),
            foreign_mounts(&legacy, Path::new("/"))
        );
        assert_eq!(volume_roots_in(&mounts), volume_roots_in(&legacy));
    }

    #[test]
    fn mountinfo_keeps_snapshot_options_from_the_superblock() {
        check_snapshot_options("a");
    }

    /// Run by scripts/test-linux-mounts.sh, entirely in its private namespace.
    #[cfg(target_os = "linux")]
    #[test]
    #[ignore = "requires the isolated mount fixture"]
    fn moved_layers_and_source_bytes_match_the_kernel() {
        let point = Path::new("/run/disktree-layers/newer");
        let peer = Path::new("/run/disktree-layers/peer");
        let mounts = mount_table().expect("live mountinfo");
        let own = covering(&mounts, point).expect("moved layer");
        assert_eq!(own.source, b"disktree-layer-\x80");
        assert!(
            !mounts
                .iter()
                .any(|mount| mount.point == point.join("hidden"))
        );
        assert_eq!(attribution(Some(&mounts), point, peer), Attribution::Other);
        assert_ne!(device_for(point), device_for(peer));
    }

    proptest::proptest! {
        #![proptest_config(proptest::test_runner::Config {
            failure_persistence: None,
            ..proptest::test_runner::Config::default()
        })]
        #[test]
        fn source_identity_agrees_with_raw_bytes(
            left in proptest::collection::vec(1_u8..=255, 1..16),
            right in proptest::collection::vec(1_u8..=255, 1..16),
        ) {
            check_source_identity(&left, &right);
            check_source_identity(&left, &left);
        }

        #[test]
        fn mount_visibility_follows_parent_ids_in_every_record_order(
            name in "[a-z]{1,12}", order in proptest::array::uniform4(
                proptest::num::u32::ANY), service in proptest::bool::ANY,
        ) {
            check_moved_stack(&name, order, service);
        }

        #[test]
        fn switching_tables_preserves_the_snapshot_exclusion_rule(
            name in "[a-z]{1,12}",
        ) {
            check_snapshot_options(&name);
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_raw_non_utf8_mount_does_not_break_discovery() {
        use std::os::unix::ffi::OsStrExt as _;
        let table = tempfile::NamedTempFile::new().expect("table");
        std::fs::write(
            table.path(),
            b"1 0 8:1 / / rw - ext4 /dev/root rw\n\
              2 1 8:2 / /mnt/\xff rw - ext4 /dev/usb rw\n",
        )
        .expect("write table");
        let mounts = read_mounts(table.path()).expect("read every mount");
        assert_eq!(volume_roots_in(&mounts).len(), 2);
        assert_eq!(mounts[1].point.as_os_str().as_bytes(), b"/mnt/\xff");
    }

    #[cfg(unix)]
    fn check_mount_name_bytes(name: &[u8]) {
        use std::fmt::Write as _;
        use std::os::unix::ffi::OsStrExt as _;
        let escaped = name.iter().fold(String::new(), |mut out, byte| {
            write!(out, "\\{byte:03o}").expect("write to string");
            out
        });
        let mounts = parse_mounts(&format!("/dev/usb /mnt/{escaped} ext4 rw"));
        let views = parse_mountinfo(&format!(
            "1 0 8:1 / /mnt/{escaped} rw - ext4 /dev/usb rw"
        ));
        let mut expected = b"/mnt/".to_vec();
        expected.extend_from_slice(name);
        assert_eq!(mounts[0].point.as_os_str().as_bytes(), expected);
        assert_eq!(views[0].point.as_os_str().as_bytes(), expected);
        let mut raw = b"1 0 8:1 / /mnt/".to_vec();
        raw.extend_from_slice(name);
        raw.extend_from_slice(b" rw - ext4 /dev/usb rw");
        assert_eq!(
            parse_mountinfo_bytes(&raw)[0].point.as_os_str().as_bytes(),
            expected
        );
    }

    #[cfg(unix)]
    proptest::proptest! {
        #![proptest_config(proptest::test_runner::Config {
            failure_persistence: None,
            ..proptest::test_runner::Config::default()
        })]
        #[test]
        fn mount_paths_preserve_every_non_utf8_byte(
            name in proptest::collection::vec(128_u8..=255, 1..16)
        ) {
            check_mount_name_bytes(&name);
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_mount_path_with_ff_is_not_replaced() {
        check_mount_name_bytes(&[128]);
        check_mount_name_bytes(&[255]);
    }

    fn overmounted_bind_table(point: &str) -> String {
        format!(
            "1 0 8:1 / / rw - ext4 /dev/root rw\n\
            2 1 8:1 /data {point} rw - ext4 /dev/root rw\n\
            3 2 8:2 / {point} rw - ext4 /dev/usb rw\n"
        )
    }

    proptest::proptest! {
        #![proptest_config(proptest::test_runner::Config {
            failure_persistence: None,
            ..proptest::test_runner::Config::default()
        })]
        #[test]
        fn hidden_bind_mounts_never_hide_the_visible_disk(name in "[a-z]{1,12}") {
            let point = format!("/mnt/{name}");
            let mounts = parse_mountinfo(&overmounted_bind_table(&point));
            proptest::prop_assert!(repeated_mounts(&mounts, Path::new("/"))
                .is_empty());
        }

        #[test]
        fn discovery_and_lookup_agree_on_the_visible_mount(name in "[a-z]{1,12}") {
            let point = format!("/mnt/{name}");
            let text = format!("/dev/root {point} ext4 rw 0 0\n\
                /dev/usb {point} ext4 rw 0 0\n");
            let mounts = parse_mounts(&text);
            let volumes = volume_roots_in(&mounts);
            proptest::prop_assert_eq!(volumes.len(), 1);
            proptest::prop_assert_eq!(&volumes[0].device,
                &device_in(&text, Path::new(&point)));
        }

        #[test]
        fn discovery_never_offers_kernel_service_filesystems(
            parent in proptest::sample::select(vec!["/run", "/var/lib/nfs"]),
            kind in proptest::sample::select(vec!["rpc_pipefs", "fuse.lxcfs"]),
        ) {
            let mounts = parse_mounts(&format!(
                "service {parent}/service {kind} rw 0 0\n"));
            proptest::prop_assert!(volume_roots_in(&mounts).is_empty());
        }
    }

    #[test]
    fn a_usb_disk_over_a_bind_mount_is_not_a_repeat() {
        for point in ["/mnt/a", "/mnt/backup"] {
            assert!(repeated(&overmounted_bind_table(point), "/").is_empty());
        }
    }

    #[test]
    fn discovery_keeps_the_usb_disk_over_a_hidden_mount() {
        for point in ["/mnt/a", "/mnt/backup"] {
            let mounts = parse_mounts(&format!(
                "/dev/root {point} ext4 rw 0 0\n\
                 /dev/usb {point} ext4 rw 0 0\n"
            ));
            assert_eq!(
                volume_roots_in(&mounts)[0].device.as_deref(),
                Some("/dev/usb")
            );
        }
    }

    #[test]
    fn rpc_pipefs_and_lxcfs_are_not_scan_roots() {
        for (point, kind) in [
            ("/run/service", "rpc_pipefs"),
            ("/run/rpc_pipefs", "rpc_pipefs"),
            ("/var/lib/nfs/rpc_pipefs", "rpc_pipefs"),
            ("/var/lib/lxcfs", "fuse.lxcfs"),
        ] {
            let mounts =
                parse_mounts(&format!("service {point} {kind} rw 0 0"));
            assert!(volume_roots_in(&mounts).is_empty(), "{point}");
        }
    }

    #[test]
    fn subvolumes_are_not_repeats_but_a_second_mount_of_a_disk_is() {
        assert_eq!(repeated(MOUNTINFO, "/"), [PathBuf::from("/mnt/data")]);
        assert!(repeated(MOUNTINFO, "/home/tobi").is_empty());
        assert!(repeated(MOUNTINFO, "/data").is_empty());
        assert!(
            repeated(MOUNTINFO, "/mnt/data").is_empty(),
            "the view that was asked for is scanned"
        );
    }

    #[test]
    fn a_bind_mount_inside_the_scan_is_left_out() {
        // The parent is the filesystem at the destination (/srv), not
        // the source (/home) of the bind mount.
        let table = format!(
            "{MOUNTINFO}30 22 0:21 /@home/tobi/src /srv/src rw - btrfs /dev/mapper/root rw\n"
        );
        assert_eq!(
            repeated(&table, "/"),
            [PathBuf::from("/mnt/data"), PathBuf::from("/srv/src")]
        );
        // From home, the original is outside the scan: nothing repeats.
        assert!(repeated(&table, "/home/tobi").is_empty());
        assert!(repeated(&table, "/srv").is_empty());
    }

    #[test]
    fn the_top_of_a_btrfs_disk_beside_its_subvolumes_is_counted_once() {
        let table = format!(
            "{MOUNTINFO}31 22 0:21 / /mnt/top rw - btrfs /dev/mapper/root rw\n"
        );
        // The root cannot be left out, so its copy under /mnt/top is; the
        // other subvolumes are seen under /mnt/top instead of twice.
        assert_eq!(
            repeated(&table, "/"),
            [
                PathBuf::from("/.snapshots"),
                PathBuf::from("/home"),
                PathBuf::from("/mnt/data"),
                PathBuf::from("/mnt/top/@"),
                PathBuf::from("/var/log"),
            ]
        );
    }

    #[test]
    fn unrelated_tmpfs_mounts_are_not_duplicates() {
        let table = "1 0 8:1 / / rw - ext4 /dev/root rw\n\
                     2 1 0:20 / /one rw - tmpfs tmpfs rw\n\
                     3 1 0:21 / /two rw - tmpfs tmpfs rw\n";
        assert!(repeated(table, "/").is_empty());
        let alias =
            format!("{table}4 1 0:20 / /three rw - tmpfs different-name rw\n");
        assert_eq!(repeated(&alias, "/"), [PathBuf::from("/three")]);
    }

    #[test]
    fn hidden_original_and_nested_mounts_are_preserved() {
        let hidden = "1 0 8:1 / / rw - ext4 /dev/root rw\n\
                      2 1 0:20 / /data rw - tmpfs tmpfs rw\n\
                      3 2 8:1 /data/source /data/bind rw - ext4 /dev/root rw\n";
        assert!(repeated(hidden, "/").is_empty());
        let nested = "1 0 8:1 / / rw - ext4 /dev/root rw\n\
                      2 1 8:1 /original /view rw - ext4 /dev/root rw\n\
                      3 2 0:20 / /view/unique rw - tmpfs tmpfs rw\n";
        assert!(repeated(nested, "/").is_empty());
    }

    #[test]
    fn excluded_views_cannot_replace_visible_files() {
        let mounts = parse_mountinfo(
            "1 0 8:1 / / rw - ext4 /dev/root rw\n\
                                     2 1 8:1 /original /view rw - ext4 /dev/root rw\n",
        );
        assert!(
            repeated_mounts_excluding(
                &mounts,
                Path::new("/"),
                &[PathBuf::from("/original")]
            )
            .is_empty()
        );
    }

    #[test]
    fn btrfs_subvolume_devices_can_differ() {
        let table = "1 0 0:20 /@ / rw - btrfs /dev/root rw\n\
                     2 1 0:21 /@home /home rw - btrfs /dev/root rw\n\
                     3 1 0:22 / /top rw - btrfs /dev/root rw\n";
        assert_eq!(
            repeated(table, "/"),
            [PathBuf::from("/home"), PathBuf::from("/top/@")]
        );
    }

    #[test]
    fn mountinfo_escapes_are_decoded_once() {
        let mounts = parse_mountinfo(
            "not a mount line\n\
             40 22 8:1 /a\\040b /media/My\\040Disk\\134040 rw - ext4 /dev/sda1 rw\n",
        );
        assert_eq!(mounts.len(), 1);
        assert_eq!(mounts[0].fs_root, Path::new("/a b"));
        assert_eq!(mounts[0].point, Path::new("/media/My Disk\\040"));
        assert_eq!(mounts[0].source, b"/dev/sda1");
        assert_eq!(mounts[0].fstype, "ext4");
    }

    #[test]
    fn mounts_escapes_are_decoded_once() {
        let table = "/dev/sda1 /media/My\\040Disk\\134040 ext4 rw 0 0\n\
                     //nas/My\\040Share /mnt/tab\\011name cifs rw 0 0\n";
        let mounts = parse_mounts(table);
        assert_eq!(mounts.len(), 2);
        assert_eq!(mounts[0].point, Path::new("/media/My Disk\\040"));
        assert_eq!(mounts[1].point, Path::new("/mnt/tab\tname"));
        assert_eq!(mounts[1].source, b"//nas/My Share");
        assert_eq!(
            device_in(table, Path::new("/media/My Disk\\040/a")).as_deref(),
            Some("/dev/sda1")
        );
        assert_eq!(
            device_in(table, Path::new("/mnt/tab\tname/a")).as_deref(),
            Some("//nas/My Share")
        );
    }

    #[test]
    fn mount_sources_decode_tabs_newlines_and_literal_escape_text_once() {
        let source = "//nas/My Share\ttab\nline\\040";
        let table = "//nas/My\\040Share\\011tab\\012line\\134040 /mnt/share cifs rw 0 0\n";
        let mounts = parse_mounts(table);
        assert_eq!(mounts[0].source, source.as_bytes());
        assert_eq!(
            device_in(table, Path::new("/mnt/share/child")).as_deref(),
            Some(source)
        );
        assert_eq!(device_in(table, Path::new("/mnt/share-other")), None);
        let info = "1 0 0:1 / /mnt/share rw - cifs //nas/My\\040Share\\011tab\\012line\\134040 rw\n";
        assert_eq!(parse_mountinfo(info)[0].source, mounts[0].source);
    }

    #[test]
    fn volumes_with_least_space_come_first_and_unknowns_last() {
        let volume = |name: &str, available: Option<u64>| Volume {
            point: PathBuf::from(name),
            device: None,
            space: available.map(|available| SpaceInfo {
                total: 100,
                free: available,
                available,
            }),
        };
        let mut volumes = vec![
            volume("unknown", None),
            volume("roomy", Some(90)),
            volume("full", Some(0)),
            volume("nearly-full", Some(5)),
        ];
        sort_by_free_space(&mut volumes);
        let names: Vec<_> = volumes.iter().map(|v| v.point.clone()).collect();
        assert_eq!(
            names,
            ["full", "nearly-full", "roomy", "unknown"].map(PathBuf::from)
        );
    }

    #[test]
    fn discovery_candidates_do_not_probe_unavailable_mounts() {
        let table = parse_mounts(
            "/dev/root /not-present-disktree-root ext4 rw 0 0\n\
             /dev/root /not-present-disktree-bind ext4 rw 0 0\n\
             server:/share /unreachable-disktree-network nfs rw 0 0\n\
             systemd-1 /untriggered-disktree-autofs autofs rw 0 0\n",
        );
        let volumes = volume_roots_in(&table);
        assert_eq!(volumes.len(), 3);
        assert!(volumes.iter().all(|volume| volume.space.is_none()));
        assert!(
            !volumes.iter().any(|volume| volume.point
                == Path::new("/untriggered-disktree-autofs"))
        );
    }

    #[test]
    fn strict_scan_boundaries_include_same_device_and_bind_mounts() {
        let mounts = parse_mounts(
            "/dev/root / btrfs rw 0 0\n\
             /dev/root /home btrfs rw,subvol=@home 0 0\n\
             /dev/root /bind btrfs rw 0 0\n\
             /dev/other /external ext4 rw 0 0\n",
        );
        let boundaries = mount_boundaries(&mounts, Path::new("/"));
        assert_eq!(
            boundaries,
            ["/home", "/bind", "/external"].map(PathBuf::from)
        );
        assert!(mount_boundaries(&mounts, Path::new("/home")).is_empty());
    }

    #[test]
    fn a_volume_includes_its_subvolumes_and_nothing_else() {
        let mounts = parse_mounts(OMARCHY);
        let mut foreign = foreign_mounts(&mounts, Path::new("/"));
        foreign.sort();
        let expected: Vec<PathBuf> = [
            "/.snapshots",
            "/boot",
            "/mnt/nas-home",
            "/run",
            "/run/user/1000/doc",
            "/sys",
            "/tmp",
        ]
        .iter()
        .map(PathBuf::from)
        .collect();
        assert_eq!(foreign, expected, "/home and /var/log are the same disk");
    }

    #[test]
    fn volume_candidates_are_real_disks_not_pseudo_filesystems() {
        let mounts = parse_mounts(OMARCHY);
        let points: Vec<PathBuf> = volumes_in(&mounts)
            .iter()
            .map(|volume| volume.point.clone())
            .collect();
        // Same-device subvolumes stay selectable because a scan of the
        // parent now stops at their mount boundaries.
        assert!(points.contains(&PathBuf::from("/")), "{points:?}");
        assert!(points.contains(&PathBuf::from("/boot")), "{points:?}");
        assert!(points.contains(&PathBuf::from("/home")), "{points:?}");
        assert!(points.contains(&PathBuf::from("/var/log")), "{points:?}");
    }

    #[test]
    fn kernel_pseudo_filesystems_are_excluded_from_volume_candidates() {
        let table = "\
/dev/sda1 / ext4 rw 0 0
/dev/sdb1 /home ext4 rw 0 0
proc /proc proc rw 0 0
binfmt_misc /proc/sys/fs/binfmt_misc binfmt_misc rw 0 0
sysfs /sys sysfs rw 0 0
bpf /sys/fs/bpf bpf rw 0 0
efivarfs /sys/firmware/efi/efivars efivarfs rw 0 0
devtmpfs /dev devtmpfs rw 0 0
devpts /dev/pts devpts rw 0 0
none /mnt/ramdisk ramfs rw 0 0
";
        let volumes = volume_roots_in(&parse_mounts(table));
        assert!(volumes.iter().all(|volume| volume.space.is_none()));
        let points: Vec<_> = volumes
            .iter()
            .map(|volume| volume.point.as_path())
            .collect();
        assert_eq!(
            points,
            [Path::new("/"), Path::new("/home")],
            "only real storage disks remain as volume candidates"
        );
    }

    #[test]
    fn discovery_filters_types_and_whole_kernel_path_components() {
        let mounts = parse_mounts(
            "unknown /proc/unknown unknown rw 0 0\n\
             unknown /sys/unknown unknown rw 0 0\n\
             unknown /dev/unknown unknown rw 0 0\n\
             none /mnt/binfmt binfmt_misc rw 0 0\n\
             none /mnt/bpf bpf rw 0 0\n\
             none /mnt/efi efivarfs rw 0 0\n\
             none /mnt/ram ramfs rw 0 0\n\
             /dev/a /process ext4 rw 0 0\n\
             /dev/b /system btrfs rw 0 0\n\
             nas:/disk /device nfs rw 0 0\n",
        );
        let roots = volume_roots_in(&mounts);
        let points: Vec<_> = roots.iter().map(|v| v.point.as_path()).collect();
        assert_eq!(
            points,
            [
                Path::new("/device"),
                Path::new("/process"),
                Path::new("/system")
            ]
        );
        assert!(roots.iter().all(|v| v.space.is_none()));
    }

    #[test]
    fn separate_home_disk_is_its_own_volume() {
        let separate = parse_mounts(
            "/dev/sda1 / ext4 rw 0 0\n/dev/sdb1 /home ext4 rw 0 0\n",
        );
        let points: Vec<PathBuf> = volumes_in(&separate)
            .iter()
            .map(|volume| volume.point.clone())
            .collect();
        assert!(points.contains(&PathBuf::from("/")), "{points:?}");
        assert!(points.contains(&PathBuf::from("/home")), "{points:?}");
    }

    #[test]
    fn the_whole_disk_is_the_top_of_the_home_volume() {
        let mounts = parse_mounts(OMARCHY);
        let root = volume_root(&mounts, Path::new("/home/tobi"));
        assert_eq!(root, Some(PathBuf::from("/")), "@home is on the root disk");
        let separate = parse_mounts(
            "/dev/sda1 / ext4 rw 0 0\n/dev/sdb1 /home ext4 rw 0 0\n",
        );
        let root = volume_root(&separate, Path::new("/home/tobi"));
        assert_eq!(root, Some(PathBuf::from("/home")), "a separate home disk");
    }

    #[test]
    fn a_home_scan_has_no_foreign_mounts_here() {
        let mounts = parse_mounts(OMARCHY);
        assert!(foreign_mounts(&mounts, Path::new("/home/tobi")).is_empty());
    }

    #[test]
    fn a_volume_keeps_its_subvolumes_and_loses_everything_else() {
        let mounts = parse_mounts(OMARCHY);
        let there = |path: &str| {
            attribution(Some(&mounts), Path::new("/"), Path::new(path))
        };
        assert_eq!(there("/home/tobi/cache"), Attribution::Scanned);
        assert_eq!(there("/var/log/journal"), Attribution::Scanned);
        assert_eq!(there("/boot/vmlinuz"), Attribution::Other);
        assert_eq!(there("/mnt/nas-home/x"), Attribution::Other);
        assert_eq!(there("/tmp/x"), Attribution::Other);
        assert_eq!(
            there("/.snapshots/12/x"),
            Attribution::Other,
            "snapshot blocks are shared with the live files"
        );
        // Asking for the snapshot itself measures the snapshot, so its own
        // files are the space that report goes up by.
        assert_eq!(
            attribution(
                Some(&mounts),
                Path::new("/.snapshots"),
                Path::new("/.snapshots/12/x")
            ),
            Attribution::Scanned
        );
        // The same disk under a home of its own is one volume throughout,
        // and the root disk beside it is not.
        let separate = parse_mounts(
            "/dev/sda1 / ext4 rw 0 0\n/dev/sdb1 /home ext4 rw 0 0\n",
        );
        assert_eq!(
            attribution(
                Some(&separate),
                Path::new("/home/tobi"),
                Path::new("/home/tobi/x")
            ),
            Attribution::Scanned
        );
        assert_eq!(
            attribution(
                Some(&separate),
                Path::new("/home/tobi"),
                Path::new("/var/tmp/x")
            ),
            Attribution::Other
        );
    }

    /// openSUSE boots from a Snapper snapshot, so `/` itself is one; the
    /// scan still walks `/var` and `/home`, and so does the projection.
    #[test]
    fn a_root_booted_from_a_snapshot_keeps_its_subvolumes() {
        let mounts = parse_mounts(
            "/dev/vda2 / btrfs rw,subvol=/@/.snapshots/1/snapshot 0 0\n\
             /dev/vda2 /var btrfs rw,subvol=/@/var 0 0\n\
             /dev/vda2 /home btrfs rw,subvol=/@/home 0 0\n",
        );
        assert!(foreign_mounts(&mounts, Path::new("/")).is_empty());
        for path in ["/var/cache/x", "/home/me/x", "/usr/x"] {
            assert_eq!(
                attribution(Some(&mounts), Path::new("/"), Path::new(path)),
                Attribution::Scanned,
                "{path}"
            );
        }
    }

    #[test]
    fn a_table_that_does_not_place_a_path_claims_nothing() {
        let partial = parse_mounts("/dev/sdb1 /data ext4 rw 0 0\n");
        assert_eq!(
            attribution(
                Some(&partial),
                Path::new("/srv/scan"),
                Path::new("/srv/scan/x")
            ),
            Attribution::Unknown,
            "the root is not in the table"
        );
        assert_eq!(
            attribution(
                Some(&partial),
                Path::new("/data/scan"),
                Path::new("/srv/scan/x")
            ),
            Attribution::Unknown,
            "the marked path is not in it either"
        );
        assert_eq!(
            attribution(Some(&[]), Path::new("/"), Path::new("/x")),
            Attribution::Unknown,
            "an empty table places nothing"
        );
    }

    #[test]
    fn without_a_table_the_same_place_is_one_volume() {
        let temp = tempfile::TempDir::new().expect("tempdir");
        let marked = temp.path().join("marked");
        std::fs::write(&marked, b"x").expect("write");
        assert_eq!(
            attribution(None, temp.path(), &marked),
            Attribution::Scanned,
            "one place cannot be two volumes"
        );
    }

    #[test]
    #[cfg(not(any(target_os = "macos", windows)))]
    fn the_machine_table_places_a_path_on_this_volume() {
        let Some(mounts) = mount_table() else {
            return;
        };
        let temp = std::env::temp_dir();
        assert_eq!(
            attribution(Some(&mounts), &temp, &temp),
            Attribution::Scanned,
            "{:?}",
            mounts.iter().map(|mount| &mount.point).collect::<Vec<_>>()
        );
    }

    #[test]
    fn the_device_is_the_longest_matching_mount() {
        let table = "\
/dev/nvme0n1p2 / btrfs rw 0 0
tmpfs /tmp tmpfs rw 0 0
/dev/sda1 /home/tobi/big\\040disk ext4 rw 0 0
";
        let device = |path: &str| device_in(table, Path::new(path));
        assert_eq!(device("/home/tobi").as_deref(), Some("/dev/nvme0n1p2"));
        assert_eq!(device("/tmp/x").as_deref(), Some("tmpfs"));
        assert_eq!(
            device("/home/tobi/big disk/a").as_deref(),
            Some("/dev/sda1"),
            "escaped spaces in mount points"
        );
    }

    #[test]
    fn macos_mount_points_keep_spaces_and_brackets() {
        let output = "\
/dev/disk3s1s1 on / (apfs, sealed, local, read-only, journaled)
devfs on /dev (devfs, local, nobrowse)
/dev/disk3s5 on /System/Volumes/Data (apfs, local, journaled, nobrowse)
map auto_home on /System/Volumes/Data/home (autofs, automounted, nobrowse)
/dev/disk5s1 on /Volumes/My Disk (2) (apfs, local, nodev, nosuid)
//tobi@nas/share on /Volumes/share on nas (smbfs, nodev, nosuid)
";
        let points: Vec<PathBuf> = [
            "/",
            "/dev",
            MACOS_DATA_VOLUME,
            "/System/Volumes/Data/home",
            "/Volumes/My Disk (2)",
            "/Volumes/share on nas",
        ]
        .iter()
        .map(PathBuf::from)
        .collect();
        assert_eq!(parse_macos_mounts(output), points);
    }

    #[test]
    fn macos_restricts_data_aliases_but_direct_data_scans_are_unrestricted() {
        let skipped = never_scanned(Path::new("/"), Path::new("/"));
        let below_system =
            never_scanned(Path::new("/System/"), Path::new("/System"));
        let inside =
            never_scanned(Path::new("/Users/tobi"), Path::new("/Users/tobi"));
        let itself = never_scanned(
            Path::new(MACOS_DATA_VOLUME),
            Path::new(MACOS_DATA_VOLUME),
        );
        if cfg!(target_os = "macos") {
            assert_eq!(skipped, vec![PathBuf::from("/Network")]);
            assert!(below_system.is_empty());
            assert_eq!(
                data_services_only(
                    Path::new("/spelling"),
                    Path::new("/System")
                ),
                Some(PathBuf::from("/spelling/Volumes/Data"))
            );
        } else {
            assert!(skipped.is_empty() && below_system.is_empty());
        }
        assert!(inside.is_empty());
        assert!(itself.is_empty(), "asking for it by name scans it");
        assert_eq!(
            data_services_only(
                Path::new(MACOS_DATA_VOLUME),
                Path::new(MACOS_DATA_VOLUME)
            ),
            None
        );
        assert_eq!(
            data_services_only(
                Path::new("/Users/tobi"),
                Path::new("/Users/tobi")
            ),
            None
        );
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn the_home_disk_on_macos_is_the_root_and_has_a_device() {
        let home = std::env::var_os("HOME").map(PathBuf::from);
        let home = home.expect("HOME is set");
        assert_eq!(volume_root_for(&home), Some(PathBuf::from("/")));
        let device = device_for(Path::new("/")).expect("a device");
        assert!(device.starts_with("/dev/disk"), "{device}");
    }

    #[test]
    fn a_real_volume_reports_plausible_numbers() {
        let temp = std::env::temp_dir();
        let space = space_info(&temp).expect("temp dir has a volume");
        assert!(space.total > 0, "{space:?}");
        assert!(space.free <= space.total, "{space:?}");
        assert!(space.available <= space.free, "{space:?}");
        assert!((0.0..=1.0).contains(&space.used_fraction()), "{space:?}");
    }

    #[test]
    fn a_missing_path_reports_the_io_error() {
        let error = space_info(Path::new("/definitely/not/here"))
            .expect_err("no volume");
        assert_eq!(error.kind(), io::ErrorKind::NotFound);
    }

    #[test]
    fn projecting_removal_cannot_exceed_the_volume() {
        let space = SpaceInfo {
            total: 1000,
            free: 100,
            available: 100,
        };
        let after = space.after_removing(50);
        assert_eq!(after.available, 150);
        assert_eq!(after.free, 150);
        assert_eq!(after.total, 1000);

        let capped = space.after_removing(10_000);
        assert_eq!(capped.available, 1000);
        assert_eq!(capped.used(), 0);
        assert!((capped.used_fraction() - 0.0).abs() < f32::EPSILON);
    }

    #[test]
    fn used_fraction_handles_an_empty_volume_report() {
        let space = SpaceInfo::default();
        assert!((space.used_fraction() - 0.0).abs() < f32::EPSILON);
    }
}
