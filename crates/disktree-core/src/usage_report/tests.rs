//! A report against a slow oracle: a plain recursive model of the same
//! tree, summed by hand, never through the tree's own passes.

use std::collections::BTreeMap;
use std::io::Cursor;

use proptest::prelude::*;

use super::diff::{Bound, compare};
use super::*;
use crate::scan::ScanOptions;
use crate::test_rng::Rng;
use crate::tree::{Draft, Metric, NodeKind, aggregate};

/// A directory or file, sized by hand.
#[derive(Clone, Debug)]
struct Model {
    name: String,
    dir: bool,
    bytes: u64,
    /// This entry itself could not be read.
    error: bool,
    children: Vec<Self>,
}

impl Model {
    fn file(name: String, bytes: u64) -> Self {
        Self {
            name,
            dir: false,
            bytes,
            error: false,
            children: Vec::new(),
        }
    }

    fn dir(name: String) -> Self {
        Self {
            name,
            dir: true,
            bytes: 0,
            error: false,
            children: Vec::new(),
        }
    }

    fn total(&self) -> u64 {
        if self.dir {
            self.children.iter().map(Self::total).sum()
        } else {
            self.bytes
        }
    }

    fn any_error(&self) -> bool {
        self.error || self.children.iter().any(Self::any_error)
    }

    fn draft(&self) -> Draft {
        let mut draft = if self.dir {
            Draft::directory(self.name.as_str())
        } else {
            Draft::entry(self.name.as_str(), NodeKind::File, self.bytes)
        };
        draft.read_error = self.error;
        draft.children = self.children.iter().map(Self::draft).collect();
        draft
    }

    /// Every directory by path, with its oracle total, error and node.
    fn dirs<'m>(&'m self, path: &str, out: &mut BTreeMap<String, &'m Self>) {
        out.insert(path.to_owned(), self);
        for child in self.children.iter().filter(|child| child.dir) {
            child.dirs(&join(path, &child.name, Spelling::Posix), out);
        }
    }

    fn at(&self, path: &str) -> Option<&Self> {
        let rest = path.strip_prefix(ROOT)?;
        let mut node = self;
        for name in rest.split('/').filter(|name| !name.is_empty()) {
            node = node.children.iter().find(|child| child.name == name)?;
        }
        Some(node)
    }
}

const ROOT: &str = "/r";

/// Sizes cluster around a few values so thresholds land on exact totals.
fn random_dir(rng: &mut Rng, name: String, depth: u32) -> Model {
    let mut dir = Model::dir(name);
    dir.error = rng.percent(5);
    let count = rng.below(if depth >= 4 { 3 } else { 6 });
    for index in 0..count {
        if depth < 5 && rng.percent(40) {
            dir.children
                .push(random_dir(rng, format!("d{index}"), depth + 1));
        } else {
            let bytes = [0, 1, 99, 100, 101, 1_000][rng.below(6) as usize]
                * (1 + rng.below(3));
            let mut file = Model::file(format!("f{index}"), bytes);
            file.error = rng.percent(3);
            dir.children.push(file);
        }
    }
    dir
}

/// Grow, shrink, remove and add, as a day's work on a disk does.
fn mutate(rng: &mut Rng, dir: &mut Model, depth: u32) {
    dir.children.retain(|_| !rng.percent(10));
    for child in &mut dir.children {
        if child.dir {
            mutate(rng, child, depth + 1);
        } else if rng.percent(30) {
            child.bytes = rng.below(1_200);
        }
        child.error = rng.percent(4);
    }
    if rng.percent(20) {
        let name = format!("n{}", rng.below(1_000_000));
        dir.children.push(random_dir(rng, name, depth + 1));
    }
}

fn tree_of(model: &Model) -> Tree {
    let mut tree = Tree::from(model.draft());
    aggregate(&mut tree, Metric::Bytes);
    tree
}

fn capture(options: ScanOptions) -> Capture {
    Capture {
        root: ROOT.to_owned(),
        style: PathStyle::Posix,
        options,
        captured_at: 1_000,
        scan_started_at: Some(990),
        scan_elapsed_ms: Some(10_000),
        host: None,
        volume: Volume::default(),
        cancelled: false,
        unreadable: 0,
        unreadable_detail: Vec::new(),
        excluded: Vec::new(),
    }
}

fn report(model: &Model, threshold: u64) -> Report {
    build(&tree_of(model), &capture(ScanOptions::default()), threshold)
}

/// A threshold at, just under or just over some directory's total.
fn threshold_for(rng: &mut Rng, model: &Model) -> u64 {
    let mut dirs = BTreeMap::new();
    model.dirs(ROOT, &mut dirs);
    let totals: Vec<u64> = dirs.values().map(|dir| dir.total()).collect();
    let pick = totals[rng.below(totals.len() as u64) as usize];
    match rng.below(3) {
        0 => pick.saturating_sub(1),
        1 => pick,
        _ => pick + 1,
    }
}

fn round_trip(report: &Report) -> Report {
    let mut bytes = Vec::new();
    report.write(&mut bytes).expect("written");
    Report::read(Cursor::new(bytes)).expect("read back")
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 512,
        ..ProptestConfig::default()
    })]

    /// Kept folders are exactly the root and the directories whose oracle
    /// total reaches the threshold, and their own parts partition the
    /// root's total.
    #[test]
    fn a_report_keeps_exactly_the_folders_at_the_threshold(seed: u64) {
        let mut rng = Rng::new(seed);
        let model = random_dir(&mut rng, "r".into(), 0);
        let threshold = threshold_for(&mut rng, &model);
        let report = report(&model, threshold);

        let mut dirs = BTreeMap::new();
        model.dirs(ROOT, &mut dirs);
        let expected: Vec<&String> = dirs
            .iter()
            .filter(|(path, dir)| {
                path.as_str() == ROOT || dir.total() >= threshold
            })
            .map(|(path, _)| path)
            .collect();
        let mut kept: Vec<&String> =
            report.folders.iter().map(|folder| &folder.path).collect();
        kept.sort();
        prop_assert_eq!(kept, expected);

        let mut own = 0_u64;
        for folder in &report.folders {
            let dir = dirs[&folder.path];
            prop_assert_eq!(folder.bytes, dir.total());
            let direct: u64 = dir
                .children
                .iter()
                .filter(|child| !child.dir)
                .map(|child| child.bytes)
                .sum();
            prop_assert_eq!(folder.direct_bytes, direct);
            let kept_below: u64 = dir
                .children
                .iter()
                .filter(|child| child.dir && child.total() >= threshold)
                .map(Model::total)
                .sum();
            prop_assert_eq!(folder.exclusive_bytes, dir.total() - kept_below);
            prop_assert_eq!(folder.unreadable, dir.any_error());
            // Errors outside kept subfolders are flagged, and so is the
            // folder's own when no kept subfolder had one. Beside a kept
            // subfolder's, the tree has merged it: see the field.
            let is_kept =
                |child: &Model| child.dir && child.total() >= threshold;
            let hidden = dir
                .children
                .iter()
                .any(|child| !is_kept(child) && child.any_error())
                || (dir.error
                    && !dir
                        .children
                        .iter()
                        .any(|child| is_kept(child) && child.any_error()));
            if hidden {
                prop_assert!(
                    folder.residual_unreadable,
                    "{} hides an unreadable entry", folder.path
                );
            }
            own += folder.exclusive_bytes;
        }
        prop_assert_eq!(own, model.total());
        prop_assert_eq!(report.header.totals.bytes, model.total());
        prop_assert_eq!(&round_trip(&report), &report);
    }

    /// Own changes add up to the total change, and what one report alone
    /// keeps is never said to be smaller in the other than it was.
    #[test]
    fn a_comparison_places_every_byte_and_bounds_what_it_cannot_see(
        seed: u64,
    ) {
        let mut rng = Rng::new(seed);
        let before = random_dir(&mut rng, "r".into(), 0);
        let mut after = before.clone();
        mutate(&mut rng, &mut after, 0);
        let (t_before, t_after) =
            (threshold_for(&mut rng, &before), threshold_for(&mut rng, &after));
        let diff = compare(
            &round_trip(&report(&before, t_before)),
            &round_trip(&report(&after, t_after)),
        )
        .expect("same root");

        prop_assert!(diff.incompatible.is_empty());
        let total: i128 =
            diff.changed.iter().map(super::diff::Changed::own_delta).sum();
        prop_assert_eq!(
            total,
            i128::from(after.total()) - i128::from(before.total())
        );
        for entry in &diff.changed {
            prop_assert!(entry.own_before <= entry.before);
            prop_assert!(entry.own_after <= entry.after);
        }
        for (list, other, threshold) in [
            (&diff.appeared, &before, t_before),
            (&diff.disappeared, &after, t_after),
        ] {
            for entry in list {
                prop_assert!(entry.bytes >= diff.threshold_bytes);
                let there = other.at(&entry.path);
                match &entry.other {
                    Bound::Under(under) => {
                        prop_assert_eq!(*under, threshold);
                        // Under the threshold, or not there at all.
                        prop_assert!(
                            there.is_none_or(|it| it.total() < threshold),
                            "{}", entry.path
                        );
                        prop_assert!(
                            !there.is_some_and(Model::any_error),
                            "{} was unreadable there", entry.path
                        );
                    }
                    Bound::Unknown(_) => {}
                }
            }
        }
    }

    #[test]
    fn a_size_written_in_bytes_reads_back(bytes: u64) {
        prop_assert_eq!(parse_size(&bytes.to_string()), Some(bytes));
    }
}

#[test]
fn sizes_read_in_the_units_written() {
    assert_eq!(parse_size("100MB"), Some(100_000_000));
    assert_eq!(parse_size("100 mb"), Some(100_000_000));
    assert_eq!(parse_size("1.5GiB"), Some(1_610_612_736));
    assert_eq!(parse_size("2KiB"), Some(2_048));
    assert_eq!(parse_size("0.5 GB"), Some(500_000_000));
    for wrong in ["", "MB", "12 parsecs", "-1", "1.2.3"] {
        assert_eq!(parse_size(wrong), None, "{wrong}");
    }
}

/// `/r` with `a` (300 bytes in `a/x`), `b` (150) and a 50-byte file.
fn fixture() -> Model {
    let mut root = Model::dir("r".into());
    let mut a = Model::dir("a".into());
    let mut x = Model::dir("x".into());
    x.children.push(Model::file("big".into(), 300));
    a.children.push(x);
    let mut b = Model::dir("b".into());
    b.children.push(Model::file("mid".into(), 150));
    root.children.push(a);
    root.children.push(b);
    root.children.push(Model::file("loose".into(), 50));
    root
}

#[test]
fn growth_is_placed_in_the_deepest_folder_both_reports_keep() {
    let before = fixture();
    let mut after = fixture();
    // `a/x` grows by 200; a new folder `c` of 120 appears; `b` shrinks
    // under the threshold.
    after.children[0].children[0].children[0].bytes = 500;
    let mut c = Model::dir("c".into());
    c.children.push(Model::file("new".into(), 120));
    after.children.push(c);
    after.children[1].children[0].bytes = 40;

    let diff = compare(&report(&before, 100), &report(&after, 100)).unwrap();
    let own = |path: &str| {
        diff.changed
            .iter()
            .find(|entry| entry.path == path)
            .map(super::diff::Changed::own_delta)
    };
    assert_eq!(own("/r/a/x"), Some(200));
    assert_eq!(own("/r/a"), Some(0), "a's growth is all in x");
    // The root holds `c` and `b` now: neither is kept in both.
    assert_eq!(own("/r"), Some(120 - 150 + 40));
    assert_eq!(diff.appeared.len(), 1);
    assert_eq!(diff.appeared[0].path, "/r/c");
    assert_eq!(diff.appeared[0].other, Bound::Under(100));
    assert_eq!(diff.disappeared[0].path, "/r/b");
    assert_eq!(diff.disappeared[0].other, Bound::Under(100));
    let text = super::diff::render(&diff, 10);
    assert!(text.contains("not known to be deleted"), "{text}");
}

#[test]
fn a_folder_missing_beside_an_unreadable_one_has_unknown_size() {
    let before = fixture();
    let mut after = fixture();
    after.children[1].error = true;
    after.children[1].children.clear();
    let diff = compare(&report(&before, 100), &report(&after, 100)).unwrap();
    assert_eq!(diff.disappeared[0].path, "/r/b");
    assert!(
        matches!(diff.disappeared[0].other, Bound::Unknown(_)),
        "{:?}",
        diff.disappeared[0]
    );
}

#[test]
fn reports_measured_differently_are_flagged_and_roots_narrowed() {
    let model = fixture();
    let before = report(&model, 100);
    let options = ScanOptions {
        apparent_size: true,
        ..ScanOptions::default()
    };
    let mut narrow = capture(options);
    narrow.root = "/r/a".to_owned();
    let a = model.children[0].clone();
    let after = build(&tree_of(&a), &narrow, 200);
    let diff = compare(&before, &after).unwrap();
    assert_eq!(diff.scope, "/r/a");
    assert!(diff.incompatible.iter().any(|it| it.contains("size mode")));
    assert!(diff.warnings.iter().any(|it| it.contains("thresholds")));
    assert!(diff.warnings.iter().any(|it| it.contains("roots")));
    assert_eq!(diff.threshold_bytes, 200);

    let mut elsewhere = narrow;
    elsewhere.root = "/q".to_owned();
    assert!(compare(&before, &build(&tree_of(&a), &elsewhere, 1)).is_err());
}

#[test]
fn a_report_cut_short_or_reordered_is_refused() {
    let mut bytes = Vec::new();
    report(&fixture(), 100).write(&mut bytes).unwrap();
    let text = String::from_utf8(bytes).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert!(lines.len() >= 4, "{text}");

    let cut = lines[..lines.len() - 1].join("\n");
    assert!(Report::read(Cursor::new(cut)).is_err());

    let mut swapped = lines.clone();
    swapped.swap(1, 2);
    assert!(Report::read(Cursor::new(swapped.join("\n"))).is_err());

    let other = text.replace(FORMAT, "something-else");
    assert!(Report::read(Cursor::new(other)).is_err());
}

#[test]
fn no_file_name_reaches_a_report() {
    let mut bytes = Vec::new();
    report(&fixture(), 1).write(&mut bytes).unwrap();
    let text = String::from_utf8(bytes).unwrap();
    for name in ["big", "mid", "loose"] {
        assert!(!text.contains(name), "{name} leaked: {text}");
    }
}

/// The figures are the scan's own: a real tree, scanned, matches what
/// the filesystem reports for its files.
#[cfg(unix)]
#[test]
fn a_report_of_a_real_scan_uses_its_allocation() {
    use std::os::unix::fs::MetadataExt as _;

    let temp = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(temp.path()).unwrap();
    std::fs::create_dir_all(root.join("big/inner")).unwrap();
    std::fs::create_dir(root.join("small")).unwrap();
    std::fs::write(root.join("big/inner/data"), vec![7; 1 << 20]).unwrap();
    std::fs::write(root.join("small/data"), b"tiny").unwrap();
    let tree = crate::scan::scan(&root, ScanOptions::default()).unwrap();
    let mut capture = capture(ScanOptions::default());
    capture.root = root.to_string_lossy().into_owned();
    capture.style = PathStyle::Native;
    let report = build(&tree, &capture, 512 * 1024);

    let blocks = |path: &str| {
        std::fs::symlink_metadata(root.join(path)).unwrap().blocks() * 512
    };
    let paths: Vec<_> = report
        .folders
        .iter()
        .map(|folder| folder.path.clone())
        .collect();
    let big = root.join("big").to_string_lossy().into_owned();
    let inner = root.join("big/inner").to_string_lossy().into_owned();
    assert_eq!(paths[1..], [big, inner.clone()]);
    let folder = report.folders.iter().find(|f| f.path == inner).unwrap();
    assert_eq!(folder.bytes, blocks("big/inner/data"));
    assert_eq!(folder.direct_bytes, folder.bytes);
    assert_eq!(report.header.size_mode, SizeMode::Allocated);
}

/// A small report of [`fixture`] rooted at `root`, for the library's tests.
pub fn sample(root: &str) -> Report {
    let mut capture = capture(ScanOptions::default());
    capture.root = root.to_owned();
    build(&tree_of(&fixture()), &capture, 100)
}
