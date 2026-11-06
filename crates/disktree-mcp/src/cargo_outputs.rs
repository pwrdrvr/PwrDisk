//! Bounded, read-only Cargo identity checks. Names discover candidates;
//! independent metadata contents establish identity, never disposability.

use std::cmp::Reverse;
use std::fs::{self, File};
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use disktree_link::Entry;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const MARKER_LIMIT: usize = 4_096;
pub const ROOT_LIMIT: usize = 512;
const METADATA_LIMIT: u64 = 256 * 1_024;
const TAG: &[u8] = b"Signature: 8a477f597d28d172789f06886806bc55";

/// Ordering applies before pagination. Unknown ages always sort last.
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Order {
    #[default]
    Oldest,
    Largest,
}

/// Cargo roots are discovered inside completed scans, including custom
/// output and build directories. No Cargo commands are executed.
#[derive(Clone, Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Params {
    /// Search below this scanned directory; default all scanned roots.
    pub path: Option<PathBuf>,
    /// Minimum listed allocation (or apparent bytes in an apparent scan).
    pub min_bytes: Option<u64>,
    /// Newest file write beneath the root must be older than this many days.
    /// Unknown ages are excluded. This is not access time or activity detection.
    pub older_than_days: Option<u32>,
    /// Oldest first (default), breaking ties by largest; or largest first.
    #[serde(default)]
    pub order: Order,
    /// Number of confirmed roots to return; default 25, maximum 200.
    pub limit: Option<usize>,
    /// Skip this many matches after ordering. Use `next_offset` to continue.
    /// Rescan or live metadata changes can change subsequent pages.
    #[serde(default)]
    pub offset: usize,
}

#[derive(Debug, Default, Serialize)]
pub struct Evidence {
    pub rustc_info: bool,
    pub cache_tag: bool,
    pub fingerprint_json: Vec<PathBuf>,
    pub dependency_directories: Vec<PathBuf>,
}

/// Scanned measurements and live identity evidence have separate times.
#[derive(Debug, Serialize)]
pub struct Output {
    pub path: PathBuf,
    pub listed_bytes: u64,
    pub last_modified: Option<i64>,
    pub age_days: Option<u64>,
    pub scanned_at: i64,
    pub apparent_size: bool,
    pub scan_unreadable: u64,
    pub scan_hidden_included: bool,
    pub scan_max_depth: Option<usize>,
    pub scan_depth_limit_known: bool,
    pub scan_one_filesystem: Option<bool>,
    pub scan_root: PathBuf,
    pub evidence_checked_at: i64,
    pub git_tracking: String,
    pub evidence: Evidence,
}

#[derive(Debug, Serialize)]
pub struct Excluded {
    pub path: PathBuf,
    pub listed_bytes: Option<u64>,
    pub reason: String,
}

#[derive(Debug, Serialize)]
pub struct ScanContext {
    pub root: PathBuf,
    pub scanned_at: i64,
    pub apparent_size: bool,
    pub unreadable: u64,
    pub include_hidden: bool,
    pub max_depth: Option<usize>,
    pub depth_limit_known: bool,
    pub one_filesystem: Option<bool>,
}

#[derive(Debug, Serialize)]
pub struct Report {
    pub scans: Vec<ScanContext>,
    pub order: Order,
    pub discovered_roots: usize,
    pub inspected_roots: usize,
    pub discovery_truncated: bool,
    pub inspection_truncated: bool,
    pub confirmed_roots: usize,
    pub matching_roots: usize,
    pub matching_listed_bytes: u64,
    pub next_offset: Option<usize>,
    pub outputs: Vec<Output>,
    pub excluded_count: usize,
    pub excluded: Vec<Excluded>,
}

/// Returns at most the bound, rejects links, devices, directories and
/// oversized metadata. Ancestors are checked separately before probing.
fn read_metadata(path: &Path) -> Option<Vec<u8>> {
    let metadata = fs::symlink_metadata(path).ok()?;
    if !metadata.is_file() || metadata.len() > METADATA_LIMIT {
        return None;
    }
    let file = File::open(path).ok()?;
    if !file.metadata().ok()?.is_file() {
        return None;
    }
    let mut bytes = Vec::new();
    file.take(METADATA_LIMIT + 1).read_to_end(&mut bytes).ok()?;
    (bytes.len() as u64 <= METADATA_LIMIT).then_some(bytes)
}

fn json(path: &Path) -> Option<Value> {
    serde_json::from_slice(&read_metadata(path)?).ok()
}

fn directory(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|m| m.is_dir())
}

fn real_ancestors(path: &Path) -> bool {
    path.ancestors().all(directory)
}

fn crate_hash(name: &str) -> bool {
    name.rsplit_once('-').is_some_and(|(name, hash)| {
        !name.is_empty()
            && name
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"_-".contains(&c))
            && hash.len() == 16
            && hash.bytes().all(|c| c.is_ascii_hexdigit())
    })
}

fn fingerprint(folder: &Path) -> Option<PathBuf> {
    // Cargo's metadata is an implementation detail. Unknown formats fail
    // closed; bounded sampling can miss a valid directory, never promote it.
    for entry in fs::read_dir(folder).ok()?.take(64).flatten() {
        if !entry.file_type().ok()?.is_dir()
            || !crate_hash(&entry.file_name().to_string_lossy())
        {
            continue;
        }
        for file in fs::read_dir(entry.path()).ok()?.take(16).flatten() {
            if file.path().extension().is_none_or(|ext| ext != "json") {
                continue;
            }
            let Some(value) = json(&file.path()) else {
                continue;
            };
            if ["rustc", "target", "profile", "path"]
                .iter()
                .all(|key| value[*key].is_u64())
                && value["features"].is_string()
                && value["deps"].is_array()
            {
                return Some(file.path());
            }
        }
    }
    None
}

/// Independent root metadata plus Cargo fingerprint contents and a matching
/// dependency directory; a `target` name or generic cache tag alone fails.
pub fn evidence(root: &Path, profiles: &[PathBuf]) -> Option<Evidence> {
    if !real_ancestors(root) {
        return None;
    }
    let mut evidence = Evidence {
        rustc_info: json(&root.join(".rustc_info.json")).is_some_and(|v| {
            v["rustc_fingerprint"].is_u64()
                && v["outputs"].is_object()
                && v["successes"].is_object()
        }),
        cache_tag: read_metadata(&root.join("CACHEDIR.TAG"))
            .is_some_and(|bytes| bytes.starts_with(TAG)),
        ..Evidence::default()
    };
    if !evidence.rustc_info && !evidence.cache_tag {
        return None;
    }
    for profile in profiles.iter().take(16) {
        let relative = profile.strip_prefix(root).ok()?;
        let depth = relative.components().count();
        // A generic tag cannot establish which enclosing directory owns a
        // two-level layout: it may be a workspace wrapping target/profile.
        // The root's own compiler metadata anchors cross-target profiles.
        if !(1..=2).contains(&depth)
            || (depth == 2 && !evidence.rustc_info)
            || !real_ancestors(profile)
            || !directory(&profile.join("deps"))
            || !directory(&profile.join(".fingerprint"))
        {
            continue;
        }
        if let Some(file) = fingerprint(&profile.join(".fingerprint")) {
            evidence
                .fingerprint_json
                .push(file.strip_prefix(root).ok()?.into());
            evidence.dependency_directories.push(relative.join("deps"));
        }
    }
    (!evidence.fingerprint_json.is_empty()).then_some(evidence)
}

/// `ls-files --error-unmatch` distinguishes a tracked pathspec from an
/// empty match without collecting an unbounded index listing. Disable the
/// optional fsmonitor hook and all optional Git writes; never run Cargo.
pub fn git_tracking(root: &Path, deadline: Instant) -> Option<&'static str> {
    let mut in_worktree = false;
    for parent in root.ancestors() {
        match fs::symlink_metadata(parent.join(".git")) {
            Ok(meta) if meta.is_dir() || meta.is_file() => {
                in_worktree = true;
                break;
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
            Ok(_) | Err(_) => return None,
        }
    }
    if !in_worktree {
        return Some("outside_git_worktree");
    }
    let mut child = Command::new("git")
        .args(["--no-optional-locks", "-c", "core.fsmonitor=false", "-C"])
        .arg(root)
        .args(["ls-files", "--error-unmatch", "--", "."])
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .env_remove("GIT_COMMON_DIR")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let deadline = deadline.min(Instant::now() + Duration::from_secs(2));
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                return match status.code() {
                    Some(0) => Some("contains_tracked_files"),
                    Some(1) => Some("no_tracked_files"),
                    _ => None,
                };
            }
            Err(_) => break,
            Ok(None) if Instant::now() >= deadline => break,
            Ok(None) => std::thread::sleep(Duration::from_millis(10)),
        }
    }
    let _ = child.kill();
    let _ = child.wait();
    None
}

pub fn output(
    path: PathBuf,
    entry: &Entry,
    at: i64,
    evidence: Evidence,
    git_tracking: &str,
) -> Output {
    Output {
        path,
        listed_bytes: entry.bytes,
        last_modified: (entry.modified > 0).then_some(entry.modified),
        age_days: (entry.modified > 0).then(|| {
            (at.saturating_sub(entry.modified).max(0) / 86_400).cast_unsigned()
        }),
        scanned_at: 0,
        apparent_size: false,
        scan_unreadable: 0,
        scan_hidden_included: false,
        scan_max_depth: None,
        scan_depth_limit_known: false,
        scan_one_filesystem: None,
        scan_root: PathBuf::new(),
        evidence_checked_at: at,
        git_tracking: git_tracking.into(),
        evidence,
    }
}

pub fn finish(mut report: Report, params: &Params, at: i64) -> Report {
    // Collapse only positively identified parents. An unverified name must
    // not hide a nested, genuine output root or contribute its source bytes.
    report.outputs.sort_by(|a, b| a.path.cmp(&b.path));
    let mut roots: Vec<Output> = Vec::new();
    for output in report.outputs {
        if !roots
            .iter()
            .any(|parent| output.path.starts_with(&parent.path))
        {
            roots.push(output);
        }
    }
    report.confirmed_roots = roots.len();
    roots.retain(|output| {
        output.listed_bytes >= params.min_bytes.unwrap_or(0)
            && params.older_than_days.is_none_or(|days| {
                output.last_modified.is_some_and(|modified| {
                    modified < at.saturating_sub(i64::from(days) * 86_400)
                })
            })
    });
    match params.order {
        Order::Oldest => roots.sort_by_key(|o| {
            (
                o.last_modified.unwrap_or(i64::MAX),
                Reverse(o.listed_bytes),
                o.path.clone(),
            )
        }),
        Order::Largest => roots.sort_by_key(|o| {
            (
                Reverse(o.listed_bytes),
                o.last_modified.unwrap_or(i64::MAX),
                o.path.clone(),
            )
        }),
    }
    report.matching_roots = roots.len();
    report.matching_listed_bytes = roots
        .iter()
        .fold(0_u64, |total, o| total.saturating_add(o.listed_bytes));
    let limit = params.limit.unwrap_or(25).clamp(1, 200);
    let end = params.offset.saturating_add(limit).min(roots.len());
    report.next_offset = (end < roots.len()).then_some(end);
    report.outputs =
        roots.into_iter().skip(params.offset).take(limit).collect();
    report.excluded_count = report.excluded.len();
    report.excluded.sort_by_key(|e| Reverse(e.listed_bytes));
    report.excluded.truncate(25);
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(root: &Path, bits: u8) -> PathBuf {
        let profile = root.join("audit-profile");
        let crate_dir = profile.join(".fingerprint/demo-0123456789abcdef");
        fs::create_dir_all(&crate_dir).expect("fingerprint");
        if bits & 1 != 0 {
            fs::write(
                root.join(".rustc_info.json"),
                r#"{"rustc_fingerprint":7,"outputs":{},"successes":{}}"#,
            )
            .expect("compiler metadata");
        }
        if bits & 2 != 0 {
            fs::write(root.join("CACHEDIR.TAG"), TAG).expect("cache tag");
        }
        fs::write(crate_dir.join("lib-demo.json"), if bits & 4 != 0 {
            r#"{"rustc":1,"features":"[]","target":2,"profile":3,"path":4,"deps":[]}"#
        } else {
            r#"{"notes":"This is user data, not a Cargo fingerprint"}"#
        }).expect("metadata or decoy");
        if bits & 8 != 0 {
            fs::create_dir(profile.join("deps")).expect("deps");
        }
        profile
    }

    #[test]
    fn identity_requires_independent_content_evidence_for_every_name() {
        let temp = tempfile::tempdir().expect("fixture");
        let temp = temp.path().canonicalize().expect("canonical fixture");
        // Exhaust the independent signals across conventional and custom
        // roots. A manifest, name, directory skeleton or tag is insufficient.
        for name in ["target", "shared-output", "target/source-lookalike"] {
            for bits in 0..16 {
                let root = temp.join(name).join(bits.to_string());
                let profile = fixture(&root, bits);
                let expected = bits & 3 != 0 && bits & 4 != 0 && bits & 8 != 0;
                assert_eq!(
                    evidence(&root, &[profile]).is_some(),
                    expected,
                    "name={name}, independent signals={bits:04b}"
                );
            }
        }
    }

    #[test]
    fn generic_wrapper_tags_do_not_promote_enclosing_source_bytes() {
        let temp = tempfile::tempdir().expect("fixture");
        let temp = temp.path().canonicalize().expect("root");
        // A generic cache tag belongs to many tools. For every output name,
        // its ancestor is still a wrapper, even with genuine Cargo beneath.
        for name in ["target", "custom-output", "build-dir", "cross-output"] {
            let wrapper = temp.join(name);
            let output = wrapper.join(name);
            let profile = fixture(&output, 15);
            fs::write(wrapper.join("CACHEDIR.TAG"), TAG).expect("generic tag");
            fs::write(wrapper.join("keep.rs"), "// unique source")
                .expect("source");
            assert!(
                evidence(&output, std::slice::from_ref(&profile)).is_some()
            );
            assert!(
                evidence(&wrapper, &[profile]).is_none(),
                "a tag on {wrapper:?} must not absorb Cargo at {output:?}"
            );
        }
    }

    #[test]
    fn cross_target_profiles_are_anchored_by_root_compiler_metadata() {
        let temp = tempfile::tempdir().expect("fixture");
        let root = temp.path().canonicalize().expect("root");
        for triple in
            ["aarch64-apple-darwin", "x86_64-pc-windows-msvc", "custom"]
        {
            let output = root.join(triple);
            let profile = fixture(&output.join(triple), 15);
            fs::write(
                output.join(".rustc_info.json"),
                r#"{"rustc_fingerprint":7,"outputs":{},"successes":{}}"#,
            )
            .expect("root compiler metadata");
            assert!(evidence(&output, &[profile]).is_some());
        }
    }

    #[test]
    fn cargo_itself_is_the_identity_oracle_for_custom_outputs_and_profiles() {
        let temp = tempfile::tempdir().expect("fixture");
        let root = temp.path().canonicalize().expect("root");
        fs::create_dir(root.join("src")).expect("src");
        fs::write(root.join("src/lib.rs"), "pub const VALUE: u8 = 1;")
            .expect("source");
        fs::write(root.join("Cargo.toml"),
            "[package]\nname='cargo-identity-oracle'\nversion='0.0.0'\nedition='2024'\n[workspace]\n[profile.audit-profile]\ninherits='dev'\n").expect("manifest");
        let output = root.join("invented-build-output");
        let result = Command::new(env!("CARGO"))
            .args([
                "check",
                "--offline",
                "--profile",
                "audit-profile",
                "--manifest-path",
            ])
            .arg(root.join("Cargo.toml"))
            .arg("--target-dir")
            .arg(&output)
            .env_remove("CARGO_BUILD_BUILD_DIR")
            .output()
            .expect("Cargo check on dependency-free invented crate");
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let proof = evidence(&output, &[output.join("audit-profile")])
            .expect("metadata produced by Cargo itself");
        assert!(!proof.fingerprint_json.is_empty());
        assert!(proof.rustc_info || proof.cache_tag);
        assert!(
            evidence(&root, &[output.join("audit-profile")]).is_none(),
            "a project containing output is not itself an output root"
        );
    }

    #[cfg(unix)]
    #[test]
    fn links_cannot_substitute_for_identity_evidence() {
        use std::os::unix::fs::symlink;
        let temp = tempfile::tempdir().expect("fixture");
        let root = temp.path().canonicalize().expect("root");
        let profile = fixture(&root.join("real"), 15);
        symlink(root.join("real"), root.join("alias")).expect("root alias");
        assert!(
            evidence(&root.join("alias"), &[root.join("alias/audit-profile")])
                .is_none()
        );
        fs::rename(
            root.join("real/.rustc_info.json"),
            root.join("metadata.json"),
        )
        .expect("move");
        fs::remove_file(root.join("real/CACHEDIR.TAG")).expect("remove tag");
        symlink(
            root.join("metadata.json"),
            root.join("real/.rustc_info.json"),
        )
        .expect("metadata alias");
        assert!(evidence(&root.join("real"), &[profile]).is_none());
    }

    #[test]
    fn tracked_source_is_never_promoted_to_an_output_candidate() {
        let temp = tempfile::tempdir().expect("fixture");
        let root = temp.path().canonicalize().expect("root");
        assert!(
            Command::new("git")
                .arg("init")
                .arg("--quiet")
                .arg(&root)
                .status()
                .expect("init")
                .success()
        );
        let output = root.join("target");
        fixture(&output, 15);
        let check =
            || git_tracking(&output, Instant::now() + Duration::from_secs(5));
        assert_eq!(check(), Some("no_tracked_files"));
        fs::write(output.join("keep.rs"), "// source").expect("source");
        assert!(
            Command::new("git")
                .arg("-C")
                .arg(&root)
                .args(["add", "target/keep.rs"])
                .status()
                .expect("add")
                .success()
        );
        assert_eq!(check(), Some("contains_tracked_files"));
    }

    fn row(path: &str, bytes: u64, modified: Option<i64>) -> Output {
        Output {
            path: path.into(),
            listed_bytes: bytes,
            last_modified: modified,
            age_days: None,
            scanned_at: 1_000_000,
            apparent_size: false,
            scan_unreadable: 0,
            scan_hidden_included: true,
            scan_max_depth: None,
            scan_depth_limit_known: false,
            scan_one_filesystem: None,
            scan_root: PathBuf::new(),
            evidence_checked_at: 1_000_000,
            git_tracking: "no_tracked_files".into(),
            evidence: Evidence::default(),
        }
    }

    fn report(outputs: Vec<Output>) -> Report {
        Report {
            scans: Vec::new(),
            order: Order::Oldest,
            discovered_roots: outputs.len(),
            inspected_roots: outputs.len(),
            discovery_truncated: false,
            inspection_truncated: false,
            confirmed_roots: 0,
            matching_roots: 0,
            matching_listed_bytes: 0,
            next_offset: None,
            outputs,
            excluded_count: 0,
            excluded: Vec::new(),
        }
    }

    #[test]
    fn age_size_order_and_pagination_agree_with_a_selection_model() {
        // Generated permutations, ties, unknown/future ages, nested roots,
        // age thresholds and page boundaries. The oracle repeatedly selects
        // the best remaining row instead of sharing the implementation sort.
        let at = 1_000_000;
        for seed in 0..32_u64 {
            for order in [Order::Oldest, Order::Largest] {
                for days in [None, Some(0), Some(3)] {
                    for offset in [0, 1, 7, 50] {
                        let params = Params {
                            min_bytes: Some(2),
                            order,
                            older_than_days: days,
                            offset,
                            limit: Some(3),
                            ..Params::default()
                        };
                        let source = || {
                            (0..20_u64)
                                .map(|i| {
                                    let j = (i * 7 + seed) % 20;
                                    row(
                                        &format!("/fixture/{j:02}"),
                                        (j * 13 + seed) % 100,
                                        (j % 5 != 0).then_some(
                                            at - j.cast_signed() * 40_000,
                                        ),
                                    )
                                })
                                .collect::<Vec<_>>()
                        };
                        let mut remaining = source();
                        remaining.retain(|o| {
                            o.listed_bytes >= 2
                                && days.is_none_or(|d| {
                                    o.last_modified.is_some_and(|m| {
                                        m < at - i64::from(d) * 86_400
                                    })
                                })
                        });
                        let count = remaining.len();
                        let total: u64 =
                            remaining.iter().map(|o| o.listed_bytes).sum();
                        let mut expected = Vec::new();
                        while !remaining.is_empty() {
                            let (index, _) = remaining
                                .iter()
                                .enumerate()
                                .min_by(|(_, a), (_, b)| {
                                    let age = || {
                                        a.last_modified.unwrap_or(i64::MAX).cmp(
                                            &b.last_modified
                                                .unwrap_or(i64::MAX),
                                        )
                                    };
                                    let size =
                                        || b.listed_bytes.cmp(&a.listed_bytes);
                                    match order {
                                        Order::Oldest => age().then_with(size),
                                        Order::Largest => size().then_with(age),
                                    }
                                    .then_with(|| a.path.cmp(&b.path))
                                })
                                .expect("remaining");
                            expected.push(remaining.remove(index).path);
                        }
                        let got = finish(report(source()), &params, at);
                        assert_eq!(got.matching_roots, count);
                        assert_eq!(got.matching_listed_bytes, total);
                        assert_eq!(
                            got.outputs
                                .iter()
                                .map(|o| &o.path)
                                .collect::<Vec<_>>(),
                            expected
                                .iter()
                                .skip(offset)
                                .take(3)
                                .collect::<Vec<_>>()
                        );
                    }
                }
            }
        }
        let got = finish(
            report(vec![
                row("/r/out", 7, Some(1)),
                row("/r/out/nested", 3, Some(1)),
            ]),
            &Params::default(),
            at,
        );
        assert_eq!(got.confirmed_roots, 1);
        assert_eq!(got.matching_listed_bytes, 7);
    }
}
