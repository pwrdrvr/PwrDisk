//! Saved reports, one folder of them: named so they sort by capture time,
//! listed by their headers alone.
//!
//! A report holds real paths, so the folder is made private to its owner
//! where the system has modes. A report is written beside its final name
//! and renamed into place, so a listing never meets half of one.

use std::fs::{self, File};
use std::io::{self, BufReader, BufWriter, Write as _};
use std::path::{Path, PathBuf};

use super::{Header, Report};

/// A report in the folder, by its header.
#[derive(Clone, Debug)]
pub struct Saved {
    pub path: PathBuf,
    pub header: Header,
}

/// A file in the folder that is not a report this version reads.
#[derive(Clone, Debug)]
pub struct Unreadable {
    pub path: PathBuf,
    pub reason: String,
}

const EXTENSION: &str = "jsonl";

/// `20261111T203738Z-users-me.jsonl`: the capture time first, in UTC, so
/// names sort by it, then the root, so a folder of them reads.
pub fn file_name(header: &Header) -> String {
    let time = chrono::DateTime::from_timestamp(header.captured_at, 0)
        .map_or_else(
            || header.captured_at.to_string(),
            |time| time.format("%Y%m%dT%H%M%SZ").to_string(),
        );
    format!("{time}-{}.{EXTENSION}", slug(&header.root))
}

/// The root as a few safe characters: `/` is `disk`, `C:\` `c`.
fn slug(root: &str) -> String {
    let mut slug = String::new();
    for character in root.chars() {
        if character.is_ascii_alphanumeric() {
            slug.push(character.to_ascii_lowercase());
        } else if !slug.ends_with('-') && !slug.is_empty() {
            slug.push('-');
        }
    }
    let slug = slug.trim_end_matches('-');
    // Enough to tell roots apart at a glance; the header has the rest.
    let slug: String = slug.chars().take(40).collect();
    if slug.is_empty() {
        "disk".to_owned()
    } else {
        slug
    }
}

/// Writes `report` into `dir`, made if missing, under [`file_name`]; a
/// second report of the same second gets a number. Returns where.
pub fn save(dir: &Path, report: &Report) -> io::Result<PathBuf> {
    make_private_dir(dir)?;
    let name = file_name(&report.header);
    let stem = name.trim_end_matches(&format!(".{EXTENSION}")).to_owned();
    let mut temp = tempfile::Builder::new()
        .prefix(".writing-")
        .suffix(".tmp")
        .tempfile_in(dir)?;
    {
        let mut out = BufWriter::new(temp.as_file_mut());
        report.write(&mut out)?;
        out.flush()?;
    }
    temp.as_file().sync_all()?;
    for attempt in 1..1000_u32 {
        let path = if attempt == 1 {
            dir.join(&name)
        } else {
            dir.join(format!("{stem}-{attempt}.{EXTENSION}"))
        };
        match temp.persist_noclobber(&path) {
            Ok(_) => return Ok(path),
            Err(error)
                if error.error.kind() == io::ErrorKind::AlreadyExists =>
            {
                temp = error.file;
            }
            Err(error) => return Err(error.error),
        }
    }
    Err(io::Error::other("too many reports captured in one second"))
}

fn make_private_dir(dir: &Path) -> io::Result<()> {
    fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

/// Every report in `dir`, newest capture first, and every `.jsonl` file
/// there that is not one, with why. A missing folder holds none.
pub fn list(dir: &Path) -> (Vec<Saved>, Vec<Unreadable>) {
    let mut saved = Vec::new();
    let mut unreadable = Vec::new();
    let Ok(entries) = fs::read_dir(dir) else {
        return (saved, unreadable);
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path
            .extension()
            .is_none_or(|extension| extension != EXTENSION)
        {
            continue;
        }
        match read_header(&path) {
            Ok(header) => saved.push(Saved { path, header }),
            Err(error) => unreadable.push(Unreadable {
                path,
                reason: error.to_string(),
            }),
        }
    }
    saved.sort_by(|left, right| {
        right
            .header
            .captured_at
            .cmp(&left.header.captured_at)
            .then_with(|| left.path.cmp(&right.path))
    });
    unreadable.sort_by(|left, right| left.path.cmp(&right.path));
    (saved, unreadable)
}

/// The header of the report at `path`.
pub fn read_header(path: &Path) -> io::Result<Header> {
    Report::read_header(BufReader::new(File::open(path)?))
}

/// The whole report at `path`.
pub fn read(path: &Path) -> io::Result<Report> {
    Report::read(BufReader::new(File::open(path)?))
}

/// Bytes the folder's reports take, for its footer.
pub fn bytes(saved: &[Saved]) -> u64 {
    saved
        .iter()
        .filter_map(|report| fs::metadata(&report.path).ok())
        .map(|meta| meta.len())
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(captured_at: i64, root: &str) -> Header {
        let mut report = crate::usage_report::tests::sample(root);
        report.header.captured_at = captured_at;
        report.header
    }

    #[test]
    fn names_sort_by_capture_time_and_say_the_root() {
        assert_eq!(
            file_name(&header(1_794_429_458, "/")),
            "20261111T203738Z-disk.jsonl"
        );
        assert_eq!(
            file_name(&header(0, "/Users/Me/My Code")),
            "19700101T000000Z-users-me-my-code.jsonl"
        );
        assert_eq!(slug("C:\\"), "c");
    }

    #[test]
    fn a_saved_report_lists_newest_first_and_reads_back() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("reports");
        let mut older = crate::usage_report::tests::sample("/r");
        older.header.captured_at = 100;
        let mut newer = older.clone();
        newer.header.captured_at = 200;
        let first = save(&folder, &older).unwrap();
        let second = save(&folder, &newer).unwrap();
        // The same second twice is two files, not one overwritten.
        let again = save(&folder, &newer).unwrap();
        assert_ne!(second, again);
        fs::write(folder.join("notes.jsonl"), "not a report\n").unwrap();
        fs::write(folder.join("notes.txt"), "ignored").unwrap();

        let (saved, unreadable) = list(&folder);
        let order: Vec<i64> = saved
            .iter()
            .map(|report| report.header.captured_at)
            .collect();
        assert_eq!(order, [200, 200, 100]);
        assert_eq!(saved[2].path, first);
        assert_eq!(unreadable.len(), 1);
        assert_eq!(read(&first).unwrap(), older);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = fs::metadata(&folder).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o700);
        }
        assert!(
            !fs::read_dir(&folder).unwrap().flatten().any(|entry| {
                entry.file_name().to_string_lossy().starts_with(".writing-")
            }),
            "no half-written file is left behind"
        );
    }
}
