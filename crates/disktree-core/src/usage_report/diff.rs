//! Two reports compared, saying only what both can support.
//!
//! A folder in both reports has a before and an after. Its change in
//! `bytes` overlaps every ancestor's, so the change is also split without
//! overlap: each folder kept in *both* reports gets its subtree less its
//! subfolders kept in both. Those parts add up to the change of the whole
//! scope, whatever either threshold left out, so growth is placed at the
//! deepest folder both reports can see.
//!
//! A folder in one report only is not new, and not deleted. Absent from a
//! report means "under that report's threshold or not there", and only
//! when nothing near it was unreadable, the scan was not cut short and it
//! was not below a depth limit; otherwise its size there is unknown.
//!
//! These are listed allocation figures. A change in them is not a change
//! in what the volume holds: clones share blocks, snapshots keep deleted
//! ones, and other volumes in an APFS container draw from the same free
//! space. Nor is it write history: a folder that grew by 1 GB may have
//! had 10 GB written and 9 GB removed.

use std::fmt::Write as _;

use rustc_hash::FxHashMap;
use serde::Serialize;

use super::{Folder, FolderSharing, Header, Report, is_within};
use crate::size::human_bytes;

/// What a comparison found.
#[derive(Clone, Debug, Serialize)]
pub struct Diff {
    pub before: Side,
    pub after: Side,
    /// The root both reports cover: the deeper of the two roots.
    pub scope: String,
    /// The larger threshold: a folder under it in one report may simply
    /// not have been kept by the other.
    pub threshold_bytes: u64,
    /// Differences that make the figures measure different things.
    pub incompatible: Vec<String>,
    pub warnings: Vec<String>,
    /// Folders in both, by size of their own change, largest first.
    pub changed: Vec<Changed>,
    /// Folders kept only in the after report, at or over `threshold_bytes`.
    pub appeared: Vec<OneSided>,
    /// Folders kept only in the before report, at or over
    /// `threshold_bytes`.
    pub disappeared: Vec<OneSided>,
}

/// One report's figures for the scope.
#[derive(Clone, Debug, Serialize)]
pub struct Side {
    pub root: String,
    pub captured_at: i64,
    pub threshold_bytes: u64,
    /// The scope's subtree total.
    pub bytes: u64,
    /// Free space this user could write, sampled after the scan.
    pub available: Option<u64>,
}

/// A folder in both reports.
#[derive(Clone, Debug, Serialize)]
pub struct Changed {
    pub path: String,
    /// Levels below the scope.
    pub depth: u32,
    pub before: u64,
    pub after: u64,
    /// The subtree less subfolders kept in both reports.
    pub own_before: u64,
    pub own_after: u64,
    /// Something below was unreadable in either report.
    pub uncertain: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sharing_before: Option<FolderSharing>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sharing_after: Option<FolderSharing>,
}

impl Changed {
    pub fn delta(&self) -> i128 {
        i128::from(self.after) - i128::from(self.before)
    }

    /// The part of the change no folder below, kept in both, holds.
    pub fn own_delta(&self) -> i128 {
        i128::from(self.own_after) - i128::from(self.own_before)
    }
}

/// A folder kept in one report only.
#[derive(Clone, Debug, Serialize)]
pub struct OneSided {
    pub path: String,
    pub depth: u32,
    /// Its size in the report that kept it.
    pub bytes: u64,
    /// What the other report says of it.
    pub other: Bound,
    /// Its parent is in both reports: the top of what one report alone
    /// shows. Folders below it are inside its `bytes`.
    pub topmost: bool,
    /// Something below was unreadable in the report that kept it.
    pub uncertain: bool,
}

/// What a report that did not keep a folder says about its size.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "detail")]
pub enum Bound {
    /// Under this many bytes, or not there at all.
    Under(u64),
    /// Could have been any size: why.
    Unknown(String),
}

/// Compare two reports of the same volume, `before` first. Fails only
/// when the roots do not overlap; every other difference is reported in
/// [`Diff::incompatible`] or [`Diff::warnings`].
pub fn compare(before: &Report, after: &Report) -> Result<Diff, String> {
    let separator = before.header.path_style.separator();
    let (a, b) = (&before.header, &after.header);
    let scope = if is_within(&b.root, &a.root, separator) {
        b.root.clone()
    } else if is_within(&a.root, &b.root, separator) {
        a.root.clone()
    } else {
        return Err(format!(
            "the reports cover different trees: {} and {}",
            a.root, b.root
        ));
    };
    let index = |report: &Report| -> FxHashMap<String, usize> {
        report
            .folders
            .iter()
            .enumerate()
            .filter(|(_, folder)| is_within(&folder.path, &scope, separator))
            .map(|(at, folder)| (folder.path.clone(), at))
            .collect()
    };
    let (in_before, in_after) = (index(before), index(after));
    let (Some(&scope_before), Some(&scope_after)) =
        (in_before.get(&scope), in_after.get(&scope))
    else {
        return Err(format!(
            "{scope} is under the threshold in the report with the wider \
             root, so the two have nothing in common to compare"
        ));
    };
    let threshold = a.threshold_bytes.max(b.threshold_bytes);
    let mut diff = Diff {
        before: side(before, scope_before),
        after: side(after, scope_after),
        scope: scope.clone(),
        threshold_bytes: threshold,
        incompatible: incompatible(a, b),
        warnings: warnings(a, b, &scope),
        changed: Vec::new(),
        appeared: Vec::new(),
        disappeared: Vec::new(),
    };

    // Each folder in both starts with its subtree and gives up every
    // subfolder in both. Both reports keep every ancestor of what they
    // keep, so a folder in both has its parent in both, under one path.
    let scope_depth = before.folders[scope_before].depth;
    let parents_before = before.parents();
    let parents_after = after.parents();
    let mut changed: Vec<Changed> = Vec::new();
    let mut at_path: FxHashMap<&str, usize> = FxHashMap::default();
    for folder in &before.folders {
        let Some(&other) = in_after.get(&folder.path) else {
            continue;
        };
        if !in_before.contains_key(&folder.path) {
            continue;
        }
        let later = &after.folders[other];
        at_path.insert(&folder.path, changed.len());
        changed.push(Changed {
            path: folder.path.clone(),
            depth: folder.depth - scope_depth,
            before: folder.bytes,
            after: later.bytes,
            own_before: folder.bytes,
            own_after: later.bytes,
            uncertain: folder.unreadable || later.unreadable,
            sharing_before: folder.sharing,
            sharing_after: later.sharing,
        });
    }
    for (at, folder) in before.folders.iter().enumerate() {
        if folder.path == scope || !at_path.contains_key(folder.path.as_str()) {
            continue;
        }
        let parent = parents_before[at].expect("only the root has none");
        let up = at_path[before.folders[parent].path.as_str()];
        let later = after.folders[in_after[&folder.path]].bytes;
        let entry = &mut changed[up];
        entry.own_before = entry.own_before.saturating_sub(folder.bytes);
        entry.own_after = entry.own_after.saturating_sub(later);
    }
    changed.sort_by(|left, right| {
        right
            .own_delta()
            .unsigned_abs()
            .cmp(&left.own_delta().unsigned_abs())
            .then_with(|| left.path.cmp(&right.path))
    });
    diff.changed = changed;

    diff.disappeared = one_sided(
        before,
        &parents_before,
        &in_before,
        after,
        &in_after,
        threshold,
        scope_depth,
    );
    diff.appeared = one_sided(
        after,
        &parents_after,
        &in_after,
        before,
        &in_before,
        threshold,
        after.folders[scope_after].depth,
    );
    Ok(diff)
}

fn side(report: &Report, scope: usize) -> Side {
    Side {
        root: report.header.root.clone(),
        captured_at: report.header.captured_at,
        threshold_bytes: report.header.threshold_bytes,
        bytes: report.folders[scope].bytes,
        available: report
            .header
            .volume
            .space_after
            .map(|space| space.available),
    }
}

/// Folders `kept` has and `other` lacks, at or over `threshold`.
fn one_sided(
    kept: &Report,
    parents: &[Option<usize>],
    in_kept: &FxHashMap<String, usize>,
    other: &Report,
    in_other: &FxHashMap<String, usize>,
    threshold: u64,
    scope_depth: u32,
) -> Vec<OneSided> {
    let mut found = Vec::new();
    for (at, folder) in kept.folders.iter().enumerate() {
        if !in_kept.contains_key(&folder.path)
            || in_other.contains_key(&folder.path)
            || folder.bytes < threshold
        {
            continue;
        }
        // The nearest ancestor both reports keep: the scope at worst.
        let mut up = parents[at].expect("the scope is in both");
        let topmost = in_other.contains_key(&kept.folders[up].path);
        while !in_other.contains_key(&kept.folders[up].path) {
            up = parents[up].expect("the scope is in both");
        }
        let ancestor = &other.folders[in_other[&kept.folders[up].path]];
        let depth_there =
            ancestor.depth + (folder.depth - kept.folders[up].depth);
        found.push(OneSided {
            path: folder.path.clone(),
            depth: folder.depth - scope_depth,
            bytes: folder.bytes,
            other: bound(&other.header, ancestor, depth_there),
            topmost,
            uncertain: folder.unreadable,
        });
    }
    found.sort_by(|left, right| {
        right
            .bytes
            .cmp(&left.bytes)
            .then_with(|| left.path.cmp(&right.path))
    });
    found
}

/// What `header`'s report says of a folder it did not keep, `depth` below
/// its root, whose nearest kept ancestor is `ancestor`.
fn bound(header: &Header, ancestor: &Folder, depth: u32) -> Bound {
    if header.completeness.cancelled {
        return Bound::Unknown("that scan was cancelled".to_owned());
    }
    if let Some(limit) = header.options.max_depth
        && depth as usize > limit
    {
        return Bound::Unknown(format!(
            "deeper than that scan's depth limit of {limit}"
        ));
    }
    if ancestor.residual_unreadable {
        return Bound::Unknown(format!(
            "that scan could not read everything in {}",
            ancestor.path
        ));
    }
    Bound::Under(header.threshold_bytes)
}

fn incompatible(a: &Header, b: &Header) -> Vec<String> {
    let mut found = Vec::new();
    let mut differs = |what: &str, left: String, right: String| {
        if left != right {
            found.push(format!("{what}: {left} before, {right} after"));
        }
    };
    differs(
        "size mode",
        format!("{:?}", a.size_mode),
        format!("{:?}", b.size_mode),
    );
    differs(
        "path style",
        format!("{:?}", a.path_style),
        format!("{:?}", b.path_style),
    );
    let (x, y) = (&a.options, &b.options);
    differs(
        "hidden entries included",
        x.include_hidden.to_string(),
        y.include_hidden.to_string(),
    );
    differs(
        "symbolic links followed",
        x.follow_links.to_string(),
        y.follow_links.to_string(),
    );
    differs(
        "one filesystem",
        x.one_filesystem.to_string(),
        y.one_filesystem.to_string(),
    );
    differs(
        "hardlinks counted once",
        x.dedup_hardlinks.to_string(),
        y.dedup_hardlinks.to_string(),
    );
    differs(
        "depth limit",
        format!("{:?}", x.max_depth),
        format!("{:?}", y.max_depth),
    );
    found
}

fn warnings(a: &Header, b: &Header, scope: &str) -> Vec<String> {
    let mut found = Vec::new();
    if let (Some(x), Some(y)) = (&a.host, &b.host)
        && x != y
    {
        found.push(format!(
            "different machines: {} before, {} after",
            x.hostname, y.hostname
        ));
    }
    if let (Some(x), Some(y)) = (&a.volume.device, &b.volume.device)
        && x != y
    {
        found.push(format!(
            "different devices: {x} before, {y} after; a device name can \
             change between boots"
        ));
    }
    if a.root != b.root {
        found.push(format!(
            "different roots ({} and {}): only {scope} is compared",
            a.root, b.root
        ));
    }
    if a.threshold_bytes != b.threshold_bytes {
        found.push(format!(
            "different thresholds ({} and {}): folders under {} are not \
             listed as appearing or going, since one report would not \
             have kept them",
            a.threshold_bytes,
            b.threshold_bytes,
            a.threshold_bytes.max(b.threshold_bytes)
        ));
    }
    if a.sharing.collected != b.sharing.collected {
        found.push(
            "clone metadata was read for one report only: sharing figures \
             do not compare"
                .to_owned(),
        );
    }
    if b.captured_at < a.captured_at {
        found.push("the after report was captured first".to_owned());
    }
    for (name, header) in [("before", a), ("after", b)] {
        let complete = &header.completeness;
        if complete.cancelled {
            found.push(format!("{name}: the scan was cancelled"));
        }
        if complete.unreadable > 0 {
            found.push(format!(
                "{name}: {} entries could not be read; what is in them is \
                 in no total",
                complete.unreadable
            ));
        }
    }
    found
}

/// The comparison for a terminal: totals, warnings, then the `limit`
/// largest changes of each kind.
pub fn render(diff: &Diff, limit: usize) -> String {
    let mut text = String::new();
    let when = |seconds: i64| {
        chrono::DateTime::from_timestamp(seconds, 0).map_or_else(
            || seconds.to_string(),
            |time| time.format("%Y-%m-%d %H:%M UTC").to_string(),
        )
    };
    for (name, side) in [("before", &diff.before), ("after ", &diff.after)] {
        let _ = writeln!(
            text,
            "{name}  {}  captured {}, folders from {}",
            side.root,
            when(side.captured_at),
            human_bytes(side.threshold_bytes)
        );
    }
    let _ = writeln!(text, "scope   {}", diff.scope);
    for problem in &diff.incompatible {
        let _ = writeln!(text, "INCOMPATIBLE: {problem}");
    }
    let _ = writeln!(
        text,
        "\nListed total: {} -> {} ({})",
        human_bytes(diff.before.bytes),
        human_bytes(diff.after.bytes),
        signed(i128::from(diff.after.bytes) - i128::from(diff.before.bytes)),
    );
    if let (Some(x), Some(y)) = (diff.before.available, diff.after.available) {
        let _ = writeln!(
            text,
            "Free space after each scan: {} -> {} ({})",
            human_bytes(x),
            human_bytes(y),
            signed(i128::from(y) - i128::from(x)),
        );
    }
    let _ = writeln!(
        text,
        "Listed allocation is not what the volume holds: clones share \
         blocks, snapshots keep deleted ones, and other volumes can share \
         its free space. A change is the net of writes and removals."
    );
    if !diff.warnings.is_empty() {
        let _ = writeln!(text, "\nWarnings:");
        for warning in &diff.warnings {
            let _ = writeln!(text, "  - {warning}");
        }
    }
    let _ = writeln!(
        text,
        "\nWhere it changed: each folder's own part, without subfolders \
         listed in both; these add up to the total."
    );
    for entry in diff
        .changed
        .iter()
        .filter(|entry| entry.own_delta() != 0)
        .take(limit)
    {
        let _ = writeln!(
            text,
            "  {:>10}  {}  (folder {} -> {}){}",
            signed(entry.own_delta()),
            entry.path,
            human_bytes(entry.before),
            human_bytes(entry.after),
            if entry.uncertain {
                ", partly unreadable"
            } else {
                ""
            },
        );
    }
    for (title, list) in [
        (
            "Kept only after: new, or grown past the threshold",
            &diff.appeared,
        ),
        (
            "Kept only before: shrunk below the threshold or gone; not known \
             to be deleted",
            &diff.disappeared,
        ),
    ] {
        let tops: Vec<_> = list.iter().filter(|entry| entry.topmost).collect();
        if tops.is_empty() {
            continue;
        }
        let _ = writeln!(text, "\n{title}:");
        for entry in tops.iter().take(limit) {
            let other = match &entry.other {
                Bound::Under(bytes) => {
                    format!("under {} or absent", human_bytes(*bytes))
                }
                Bound::Unknown(why) => format!("unknown: {why}"),
            };
            let _ = writeln!(
                text,
                "  {:>10}  {}  (other report: {other})",
                human_bytes(entry.bytes),
                entry.path
            );
        }
    }
    text
}

fn signed(delta: i128) -> String {
    let size =
        human_bytes(u64::try_from(delta.unsigned_abs()).unwrap_or(u64::MAX));
    match delta.signum() {
        1 => format!("+{size}"),
        -1 => format!("-{size}"),
        _ => size,
    }
}
