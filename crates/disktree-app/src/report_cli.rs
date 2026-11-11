//! `pwrdisk report` and `pwrdisk report-diff`: a folder-level usage report
//! written from a scan, and two of them compared, with no window.
//!
//! Both only read. The scan is the window's own, with the same options and
//! the same boundaries; the report is [`disktree_core::usage_report`].

use std::ffi::OsString;
use std::fs::File;
use std::io::{BufReader, BufWriter, Write as _};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result};
use disktree_core::identity::HostIdentity;
use disktree_core::path_style::PathStyle;
use disktree_core::scan::ScanHandle;
use disktree_core::size::{human_bytes, human_count};
use disktree_core::space;
use disktree_core::usage_report::{
    self, Capture, DEFAULT_THRESHOLD, Report, Volume, diff,
};

pub const USAGE: &str = "\
usage: pwrdisk report [SCAN OPTIONS] [--threshold SIZE] [-o FILE | --save]
                      (PATH | -D)
       pwrdisk report-diff BEFORE AFTER [--json] [--limit N]
                           [--allow-incompatible]

report scans PATH (or, with -D, the whole disk the home directory is on)
and writes every folder whose total reaches SIZE, at any depth, as JSON
Lines: a header, then one line per folder. No file is listed. SIZE is in
bytes unless given a unit: 100MB and 1GB are decimal, 512MiB binary
(default 100MB). Without -o the report goes to standard output; --save
writes it to PwrDisk's reports folder, which File > Usage History lists. Scan
options are those of pwrdisk itself. Paths in a report are private: keep
it where you would keep the disk's contents.

report-diff compares two reports of the same volume and says where the
listed total changed. A folder in one report only is under that report's
threshold or absent, never assumed deleted.
";

/// Runs `pwrdisk report …`; `args` follow the word `report`.
pub fn report(
    args: impl Iterator<Item = OsString>,
    saved: crate::power::PowerEfficiency,
) -> Result<()> {
    let mut threshold = DEFAULT_THRESHOLD;
    let mut output: Option<PathBuf> = None;
    let mut save = false;
    let mut scan_args = Vec::new();
    let mut args = args;
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("-h" | "--help") => {
                println!("{USAGE}");
                return Ok(());
            }
            Some("--threshold") => {
                let value = args
                    .next()
                    .and_then(|value| value.into_string().ok())
                    .context("--threshold needs a size, such as 100MB")?;
                threshold =
                    usage_report::parse_size(&value).with_context(|| {
                        format!("{value} is not a size; try 100MB or 1GiB")
                    })?;
            }
            Some("--save") => save = true,
            Some("-o" | "--output") => {
                output = Some(PathBuf::from(
                    args.next().context("-o needs a file")?,
                ));
            }
            _ => scan_args.push(arg),
        }
    }
    anyhow::ensure!(
        !(save && output.is_some()),
        "--save and -o cannot be combined"
    );
    let folder = if save {
        Some(
            crate::usage_history::reports_dir()
                .context("this system has no folder for PwrDisk's data")?,
        )
    } else {
        None
    };
    let parsed = crate::parse_args_with_power(scan_args.into_iter(), saved)?;
    anyhow::ensure!(
        parsed.remote.is_none(),
        "report scans this computer; SSH hosts are not supported yet"
    );
    anyhow::ensure!(!parsed.check, "--check is for pwrdisk ssh");
    anyhow::ensure!(
        !parsed.choose_root,
        "report needs a PATH or -D\n\n{USAGE}"
    );
    let root = parsed.root;
    let options = parsed.options;

    let space_before = space::display_space_info(&root).ok();
    let started_at = chrono::Utc::now().timestamp();
    let clock = Instant::now();
    eprintln!(
        "Scanning {} read-only. A whole volume can take minutes.",
        root.display()
    );
    let handle = ScanHandle::spawn(root.clone(), options.clone());
    let mut said = Instant::now();
    let tree = loop {
        if let Some(outcome) = handle.poll() {
            break outcome
                .with_context(|| format!("scan {}", root.display()))?;
        }
        // The walk runs on its own thread; this one only reports on it.
        std::thread::sleep(Duration::from_millis(100));
        if said.elapsed() >= Duration::from_secs(15) {
            said = Instant::now();
            let progress = handle.progress.snapshot();
            eprintln!(
                "  {:>5}s  {} files, {} folders, {}",
                clock.elapsed().as_secs(),
                human_count(progress.files),
                human_count(progress.dirs),
                human_bytes(progress.bytes),
            );
        }
    };
    let elapsed = clock.elapsed();
    let progress = handle.progress.snapshot();
    let capture = Capture {
        root: root.to_string_lossy().into_owned(),
        style: PathStyle::Native,
        captured_at: chrono::Utc::now().timestamp(),
        scan_started_at: Some(started_at),
        scan_elapsed_ms: u64::try_from(elapsed.as_millis()).ok(),
        host: HostIdentity::current(),
        volume: Volume {
            device: space::device_for(&root),
            mount: space::volume_root_for(&root)
                .map(|mount| mount.to_string_lossy().into_owned()),
            space_before,
            space_after: space::display_space_info(&root).ok(),
        },
        cancelled: progress.cancelled,
        unreadable: progress.errors,
        unreadable_detail: progress.messages,
        excluded: not_entered(&root, options.one_filesystem),
        options,
    };
    let report = usage_report::build(&tree, &capture, threshold);
    drop(tree);

    if let Some(folder) = &folder {
        output = Some(
            usage_report::library::save(folder, &report)
                .with_context(|| format!("save in {}", folder.display()))?,
        );
    }
    match &output {
        _ if folder.is_some() => {}
        Some(path) if path != Path::new("-") => {
            let file = File::create(path)
                .with_context(|| format!("create {}", path.display()))?;
            report.write(&mut BufWriter::new(file))?;
        }
        _ => report.write(&mut BufWriter::new(std::io::stdout().lock()))?,
    }
    let totals = report.header.totals;
    eprintln!(
        "{} folders of {} or more, out of {} folders and {} files ({}) in \
         {:.1}s; {} entries unreadable.",
        human_count(totals.records),
        human_bytes(threshold),
        human_count(totals.dirs),
        human_count(totals.files),
        human_bytes(totals.bytes),
        elapsed.as_secs_f64(),
        human_count(report.header.completeness.unreadable),
    );
    if let Some(path) = output.filter(|path| path != Path::new("-")) {
        let size = std::fs::metadata(&path).map_or(0, |meta| meta.len());
        eprintln!("Wrote {} ({size} bytes).", path.display());
    }
    Ok(())
}

/// Mounts below `root` the walk does not enter, and automount points it
/// never touches. Aliases of folders it reaches by another path (macOS
/// firmlinks, repeated bind mounts) are not listed: their files are
/// counted once, where the walk found them.
fn not_entered(root: &Path, one_filesystem: bool) -> Vec<String> {
    let canonical = root.canonicalize().unwrap_or_else(|_| root.into());
    let data = space::data_volume_below(root, &canonical);
    let mut paths = space::never_scanned(root, &canonical);
    if one_filesystem
        && let Some(mounts) = space::mount_boundaries_for(&canonical)
    {
        // The walk enters the Data volume below `/` on purpose.
        paths.extend(
            mounts
                .into_iter()
                .filter(|path| Some(path) != data.as_ref()),
        );
    }
    paths.sort();
    paths.dedup();
    paths
        .into_iter()
        .map(|path| path.to_string_lossy().into_owned())
        .collect()
}

/// Runs `pwrdisk report-diff …`; `args` follow the word `report-diff`.
pub fn compare(args: impl Iterator<Item = OsString>) -> Result<()> {
    let mut json = false;
    let mut allow = false;
    let mut limit = 25_usize;
    let mut files = Vec::new();
    let mut args = args;
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("-h" | "--help") => {
                println!("{USAGE}");
                return Ok(());
            }
            Some("--json") => json = true,
            Some("--allow-incompatible") => allow = true,
            Some("--limit") => {
                limit = args
                    .next()
                    .and_then(|value| value.into_string().ok())
                    .and_then(|value| value.parse().ok())
                    .context("--limit needs a number")?;
            }
            Some(other) if other.starts_with('-') => {
                anyhow::bail!("unknown option {other}\n\n{USAGE}");
            }
            _ => files.push(PathBuf::from(arg)),
        }
    }
    let [before, after] = files.as_slice() else {
        anyhow::bail!("report-diff needs two reports\n\n{USAGE}");
    };
    let read = |path: &Path| -> Result<Report> {
        let file = File::open(path)
            .with_context(|| format!("open {}", path.display()))?;
        Report::read(BufReader::new(file))
            .with_context(|| format!("read {}", path.display()))
    };
    let found = diff::compare(&read(before)?, &read(after)?)
        .map_err(anyhow::Error::msg)?;
    anyhow::ensure!(
        allow || found.incompatible.is_empty(),
        "the reports measure different things:\n  {}\n\
         pass --allow-incompatible to compare them anyway",
        found.incompatible.join("\n  ")
    );
    let mut out = std::io::stdout().lock();
    if json {
        serde_json::to_writer_pretty(&mut out, &found)?;
        writeln!(out)?;
    } else {
        write!(out, "{}", diff::render(&found, limit))?;
    }
    Ok(())
}
