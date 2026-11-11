//! A folder-level usage report, kept to compare with a later one.
//!
//! A scan's tree is tens of millions of nodes; what a later look at "where
//! did the space go" needs is the folders that matter. A report keeps
//! every directory, at any depth, whose subtree reaches a threshold
//! (100 MB by default), and nothing below it: no file, no small folder, no
//! clone group. Subtree totals only grow towards the root, so every kept
//! folder's parent is kept too and a report is one tree with the scanned
//! root at the top.
//!
//! It is JSON Lines: one header, then one line per folder, parents before
//! children. Each line stands alone for `grep` and `jq`, a report can be
//! read without holding a document, and a cut-off file is caught by the
//! record count the header promises.
//!
//! Every figure is read from the finished tree: `bytes` is the scan's own
//! subtree total (see [`crate::tree::aggregate`]), in the size mode the
//! header names. A folder also says what is *its own* among kept folders,
//! `exclusive_bytes`: its subtree less its kept subfolders. Those never
//! overlap, so they add up to the root's total, which is what lets a
//! comparison say where growth happened without counting a byte under
//! every ancestor. See [`diff`] for what two reports can and cannot say.

use std::io::{self, BufRead, Write};

use serde::{Deserialize, Serialize};

use crate::identity::HostIdentity;
use crate::path_style::PathStyle;
use crate::scan::ScanOptions;
use crate::space::SpaceInfo;
use crate::tree::{Node, Tree};

pub mod diff;
#[cfg(test)]
mod tests;

/// What a header's `format` says, so a stray JSONL file is not read as one.
pub const FORMAT: &str = "pwrdisk-usage-report";
/// Bumped when a field changes meaning; added fields keep the version.
pub const VERSION: u32 = 1;
/// 100 MB, decimal, as macOS and disk vendors count.
pub const DEFAULT_THRESHOLD: u64 = 100_000_000;

/// Whether `bytes` is allocation or apparent length.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SizeMode {
    /// Blocks the volume spends: `st_blocks * 512` or what the platform
    /// listing reports. Summed allocation can exceed what a volume holds
    /// when files share blocks.
    Allocated,
    /// Length, as `ls -l` shows it.
    Apparent,
}

/// How the report's paths are spelled.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Spelling {
    Posix,
    Windows,
}

impl Spelling {
    pub const fn of(style: PathStyle) -> Self {
        if matches!(style, PathStyle::Native) && cfg!(windows) {
            Self::Windows
        } else {
            Self::Posix
        }
    }

    pub const fn separator(self) -> char {
        match self {
            Self::Posix => '/',
            Self::Windows => '\\',
        }
    }
}

/// The options that change what a scan counts. Two reports taken with
/// different ones measure different things.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Options {
    pub include_hidden: bool,
    pub follow_links: bool,
    pub one_filesystem: bool,
    pub dedup_hardlinks: bool,
    pub max_depth: Option<usize>,
    pub apfs_clone_metadata: bool,
}

impl From<&ScanOptions> for Options {
    fn from(options: &ScanOptions) -> Self {
        Self {
            include_hidden: options.include_hidden,
            follow_links: options.follow_links,
            one_filesystem: options.one_filesystem,
            dedup_hardlinks: options.dedup_hardlinks,
            max_depth: options.max_depth,
            apfs_clone_metadata: options.apfs_clone_metadata,
        }
    }
}

/// The volume the root is on, as measured around the scan. Free space is
/// sampled before and after, since a long walk is not an instant.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Volume {
    /// The device the root's filesystem is mounted from.
    pub device: Option<String>,
    /// Where that volume is mounted.
    pub mount: Option<String>,
    pub space_before: Option<SpaceInfo>,
    pub space_after: Option<SpaceInfo>,
}

/// What the scan could not see.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Completeness {
    /// Nothing unreadable, nothing cancelled, no depth limit.
    pub complete: bool,
    pub cancelled: bool,
    /// Directories or entries that could not be read. Their contents are
    /// in no total.
    pub unreadable: u64,
    /// The first few of those, with the error, as the scan kept them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unreadable_detail: Vec<String>,
    /// Paths the scan never enters, whatever the options: other mounts
    /// and automount points.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub excluded: Vec<String>,
}

/// What the sharing figures on a folder mean, or that there are none.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SharingNote {
    /// Clone identities and private sizes were read (APFS, local macOS).
    /// Without them a folder carries no sharing figures and its
    /// allocation may still share blocks with another.
    pub collected: bool,
    pub note: String,
}

/// Root totals and how many folders follow.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize,
)]
pub struct Totals {
    pub bytes: u64,
    pub files: u64,
    pub dirs: u64,
    pub records: u64,
}

/// The first line.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Header {
    pub format: String,
    pub version: u32,
    pub generator: String,
    /// Unix seconds when the tree was finished.
    pub captured_at: i64,
    /// Unix seconds when the walk began, if this process saw it begin.
    pub scan_started_at: Option<i64>,
    pub scan_elapsed_ms: Option<u64>,
    pub host: Option<HostIdentity>,
    pub root: String,
    pub path_style: Spelling,
    pub volume: Volume,
    /// Every byte figure below is in these.
    pub units: String,
    /// A folder is kept when its `bytes` is at least this; the root always.
    pub threshold_bytes: u64,
    pub size_mode: SizeMode,
    pub options: Options,
    pub completeness: Completeness,
    pub sharing: SharingNote,
    pub totals: Totals,
}

/// APFS clone figures for a folder, from [`crate::sharing::Sharing`].
/// Present only where the scan found clone or hardlink metadata below it.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize,
)]
pub struct FolderSharing {
    /// Files below with clone metadata.
    pub files: u64,
    /// Their allocation, part of `bytes`.
    pub bytes: u64,
    /// Of that, data-fork allocation the volume reports as private.
    pub private_bytes: u64,
    /// Allocation repeated across clones of one group within this folder.
    pub duplicate_bytes: u64,
    /// Files whose sharing could not be read.
    pub unknown_files: u64,
    /// Removal estimate before snapshots: at least, at most.
    pub reclaim_lower: Option<u64>,
    pub reclaim_upper: Option<u64>,
    /// The estimate is missing metadata or extent relationships.
    pub partial: bool,
    pub groups_truncated: bool,
}

impl From<crate::sharing::Sharing> for FolderSharing {
    fn from(sharing: crate::sharing::Sharing) -> Self {
        Self {
            files: sharing.files,
            bytes: sharing.bytes,
            private_bytes: sharing.private_bytes,
            duplicate_bytes: sharing.duplicate_bytes,
            unknown_files: sharing.unknown_files,
            reclaim_lower: sharing.estimate.map(|estimate| estimate.lower),
            reclaim_upper: sharing.estimate.map(|estimate| estimate.upper),
            partial: sharing.estimate.is_some_and(|estimate| estimate.partial),
            groups_truncated: sharing.groups_truncated,
        }
    }
}

/// One kept folder.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Folder {
    pub path: String,
    /// Levels below the root, which is `0`. A folder's parent is the
    /// nearest earlier line one level up.
    pub depth: u32,
    /// Everything at or below, in the header's size mode.
    pub bytes: u64,
    pub files: u64,
    pub dirs: u64,
    /// Files, links and other entries directly in this folder.
    pub direct_bytes: u64,
    pub direct_files: u64,
    /// `bytes` less every kept subfolder's: direct entries and subfolders
    /// under the threshold. Never counted under another kept folder.
    pub exclusive_bytes: u64,
    /// Something at or below could not be read; `bytes` is a floor.
    #[serde(default, skip_serializing_if = "is_false")]
    pub unreadable: bool,
    /// Something not kept as its own folder may be unreadable here: the
    /// folder itself, a direct entry, or a subfolder under the threshold.
    /// A path missing below this folder may then have been hidden rather
    /// than small. The tree merges a folder's own errors with its
    /// children's, so an entry this folder failed to read is missed here
    /// when a kept subfolder also had errors; `unreadable` still says so,
    /// and the header counts every error.
    #[serde(default, skip_serializing_if = "is_false")]
    pub residual_unreadable: bool,
    /// Newest write time at or below, in Unix seconds; `0` when unknown.
    pub modified: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sharing: Option<FolderSharing>,
}

#[expect(clippy::trivially_copy_pass_by_ref, reason = "serde passes a ref")]
const fn is_false(value: &bool) -> bool {
    !*value
}

/// One line of a report.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Line {
    Header(Box<Header>),
    Dir(Folder),
}

/// A whole report, read back.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Report {
    pub header: Header,
    /// Parents before children, as written.
    pub folders: Vec<Folder>,
}

/// What the tree cannot say about the scan that made it.
#[derive(Clone, Debug)]
pub struct Capture {
    pub root: String,
    pub style: PathStyle,
    pub options: ScanOptions,
    pub captured_at: i64,
    pub scan_started_at: Option<i64>,
    pub scan_elapsed_ms: Option<u64>,
    pub host: Option<HostIdentity>,
    pub volume: Volume,
    pub cancelled: bool,
    pub unreadable: u64,
    pub unreadable_detail: Vec<String>,
    pub excluded: Vec<String>,
}

/// Every folder of `tree` whose subtree reaches `threshold`, and the root.
///
/// Reads a finished tree; it does not measure anything again.
pub fn build(tree: &Tree, capture: &Capture, threshold: u64) -> Report {
    let spelling = Spelling::of(capture.style);
    let mut folders = Vec::new();
    // Depth first, parents before children, so a reader can link each line
    // to its parent from the depths alone.
    let mut stack = vec![(tree.root(), capture.root.clone(), 0_u32)];
    while let Some((node, path, depth)) = stack.pop() {
        let kept =
            |child: &Node<'_>| child.is_dir() && child.bytes >= threshold;
        let mut kept_bytes = 0_u64;
        let mut kept_errors = false;
        let mut residual_errors = false;
        let mut direct_bytes = 0_u64;
        let mut direct_files = 0_u64;
        let mut below = Vec::new();
        for child in node.children() {
            if !child.is_dir() {
                direct_bytes = direct_bytes.saturating_add(child.bytes);
                direct_files += child.files;
                residual_errors |= child.read_error;
            } else if kept(&child) {
                kept_bytes = kept_bytes.saturating_add(child.bytes);
                kept_errors |= child.read_error;
                below.push(child);
            } else {
                residual_errors |= child.read_error;
            }
        }
        // An error no child can account for is this folder's own. One it
        // met beside a kept subfolder's is merged into the same flag by
        // `aggregate` and cannot be seen here; see the field.
        residual_errors |= node.read_error && !kept_errors && !residual_errors;
        folders.push(Folder {
            path: path.clone(),
            depth,
            bytes: node.bytes,
            files: node.files,
            dirs: u64::from(node.dirs),
            direct_bytes,
            direct_files,
            exclusive_bytes: node.bytes.saturating_sub(kept_bytes),
            unreadable: node.read_error,
            residual_unreadable: residual_errors,
            modified: node.modified,
            sharing: node.stored_sharing().copied().map(FolderSharing::from),
        });
        // Reversed, so the largest kept child is written first.
        for child in below.into_iter().rev() {
            let child_path = join(&path, child.name(), spelling);
            stack.push((child, child_path, depth + 1));
        }
    }
    let root = tree.root();
    let unreadable = capture.unreadable > 0 || root.read_error;
    let header = Header {
        format: FORMAT.to_owned(),
        version: VERSION,
        generator: format!("pwrdisk {}", env!("CARGO_PKG_VERSION")),
        captured_at: capture.captured_at,
        scan_started_at: capture.scan_started_at,
        scan_elapsed_ms: capture.scan_elapsed_ms,
        host: capture.host.clone(),
        root: capture.root.clone(),
        path_style: spelling,
        volume: capture.volume.clone(),
        units: "bytes".to_owned(),
        threshold_bytes: threshold,
        size_mode: if capture.options.apparent_size {
            SizeMode::Apparent
        } else {
            SizeMode::Allocated
        },
        options: Options::from(&capture.options),
        completeness: Completeness {
            complete: !unreadable
                && !capture.cancelled
                && capture.options.max_depth.is_none(),
            cancelled: capture.cancelled,
            unreadable: capture.unreadable.max(u64::from(root.read_error)),
            unreadable_detail: capture.unreadable_detail.clone(),
            excluded: capture.excluded.clone(),
        },
        sharing: sharing_note(&capture.options),
        totals: Totals {
            bytes: root.bytes,
            files: root.files,
            dirs: u64::from(root.dirs),
            records: folders.len() as u64,
        },
    };
    Report { header, folders }
}

fn sharing_note(options: &ScanOptions) -> SharingNote {
    let collected = cfg!(target_os = "macos")
        && options.apfs_clone_metadata
        && !options.apparent_size;
    SharingNote {
        collected,
        note: if collected {
            "APFS clone identities and private data-fork sizes, where the \
             volume reports them. Allocation is listed per file, so a \
             folder's bytes can exceed what the volume holds for it. \
             Snapshots retain blocks no listing shows."
        } else {
            "No clone metadata: listed allocation may still share blocks \
             with other files or snapshots."
        }
        .to_owned(),
    }
}

/// `name` beneath `base`, with no doubled separator after a root such as
/// `/` or `C:\`.
fn join(base: &str, name: &str, spelling: Spelling) -> String {
    let separator = spelling.separator();
    let mut path = String::with_capacity(base.len() + name.len() + 1);
    path.push_str(base);
    if !base.ends_with(separator) {
        path.push(separator);
    }
    path.push_str(name);
    path
}

impl Report {
    /// The report as JSON Lines.
    pub fn write(&self, out: &mut impl Write) -> io::Result<()> {
        let header = Line::Header(Box::new(self.header.clone()));
        serde_json::to_writer(&mut *out, &header)?;
        out.write_all(b"\n")?;
        for folder in &self.folders {
            serde_json::to_writer(&mut *out, &Line::Dir(folder.clone()))?;
            out.write_all(b"\n")?;
        }
        out.flush()
    }

    /// Read a report, refusing one that is cut off or out of order: a
    /// missing line would read as a folder that went away.
    pub fn read(input: impl BufRead) -> io::Result<Self> {
        let invalid = |message: String| {
            io::Error::new(io::ErrorKind::InvalidData, message)
        };
        let mut lines = input.lines();
        let first = lines
            .next()
            .ok_or_else(|| invalid("an empty file is not a report".into()))??;
        let Ok(Line::Header(header)) = serde_json::from_str(&first) else {
            return Err(invalid(
                "the first line is not a report header".into(),
            ));
        };
        if header.format != FORMAT {
            return Err(invalid(format!("not a {FORMAT} file")));
        }
        if header.version > VERSION {
            return Err(invalid(format!(
                "report version {} is newer than this PwrDisk reads \
                 ({VERSION})",
                header.version
            )));
        }
        let separator = header.path_style.separator();
        let mut folders: Vec<Folder> = Vec::new();
        // The path of the last folder at each depth: a folder's parent.
        let mut open: Vec<usize> = Vec::new();
        for (number, line) in lines.enumerate() {
            let line = line?;
            if line.is_empty() {
                continue;
            }
            let at = number + 2;
            let Line::Dir(folder) = serde_json::from_str(&line)
                .map_err(|error| invalid(format!("line {at}: {error}")))?
            else {
                return Err(invalid(format!("line {at}: a second header")));
            };
            let depth = folder.depth as usize;
            if depth == 0 {
                if !folders.is_empty() || folder.path != header.root {
                    return Err(invalid(format!(
                        "line {at}: the root must come first, once"
                    )));
                }
            } else {
                let parent = open
                    .get(depth - 1)
                    .map(|&index| &folders[index])
                    .ok_or_else(|| {
                        invalid(format!("line {at}: no parent before it"))
                    })?;
                if !is_child(&folder.path, &parent.path, separator) {
                    return Err(invalid(format!(
                        "line {at}: not inside the folder before it"
                    )));
                }
            }
            open.truncate(depth);
            open.push(folders.len());
            folders.push(folder);
        }
        if folders.len() as u64 != header.totals.records {
            return Err(invalid(format!(
                "{} folders where the header promises {}: the file is \
                 incomplete",
                folders.len(),
                header.totals.records
            )));
        }
        Ok(Self {
            header: *header,
            folders,
        })
    }

    /// Each folder's parent, by index; `None` for the root.
    pub fn parents(&self) -> Vec<Option<usize>> {
        let mut open: Vec<usize> = Vec::new();
        self.folders
            .iter()
            .enumerate()
            .map(|(index, folder)| {
                let depth = folder.depth as usize;
                open.truncate(depth);
                let parent = depth.checked_sub(1).map(|up| open[up]);
                open.push(index);
                parent
            })
            .collect()
    }
}

/// Whether `path` is one level directly beneath `parent`.
pub(crate) fn is_child(path: &str, parent: &str, separator: char) -> bool {
    let Some(rest) = path.strip_prefix(parent) else {
        return false;
    };
    let name = if parent.ends_with(separator) {
        rest
    } else {
        match rest.strip_prefix(separator) {
            Some(name) => name,
            None => return false,
        }
    };
    !name.is_empty() && !name.contains(separator)
}

/// Whether `path` is `base` or below it.
pub(crate) fn is_within(path: &str, base: &str, separator: char) -> bool {
    path == base
        || path.strip_prefix(base).is_some_and(|rest| {
            base.ends_with(separator) || rest.starts_with(separator)
        })
}

/// A size such as `100MB`, `1.5 GiB` or `250000000`. Decimal units are
/// powers of 1000 and binary ones (`KiB`, `MiB`, …) of 1024, so the
/// number written is the number meant.
pub fn parse_size(text: &str) -> Option<u64> {
    let text = text.trim();
    let split = text
        .find(|c: char| !(c.is_ascii_digit() || c == '.'))
        .unwrap_or(text.len());
    let (number, unit) = text.split_at(split);
    let scale: u64 = match unit.trim().to_ascii_lowercase().as_str() {
        "" | "b" => 1,
        "k" | "kb" => 1_000,
        "m" | "mb" => 1_000_000,
        "g" | "gb" => 1_000_000_000,
        "t" | "tb" => 1_000_000_000_000,
        "kib" => 1 << 10,
        "mib" => 1 << 20,
        "gib" => 1 << 30,
        "tib" => 1 << 40,
        _ => return None,
    };
    if number.is_empty() {
        return None;
    }
    if let Ok(whole) = number.parse::<u64>() {
        return whole.checked_mul(scale);
    }
    // A fraction is exact only to what `f64` holds; a threshold need not be
    // finer than a byte in 2^53.
    let value: f64 = number.parse().ok()?;
    let bytes = (value * scale as f64).round();
    (bytes.is_finite() && bytes >= 0.0 && bytes < u64::MAX as f64)
        .then_some(bytes as u64)
}
