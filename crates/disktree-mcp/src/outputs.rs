//! Generic output discovery, evidence, filtering and ordering.
use crate::cargo_outputs::Evidence as CargoEvidence;
use crate::session::CategoryName;
use disktree_core::artifact::{Artifact, Detection, Disposition};
use disktree_link::Entry;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::cmp::Reverse;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
pub const MARKER_LIMIT: usize = 4_096;
pub const ROOT_LIMIT: usize = 512;

/// Ordering applies before pagination. Unknown ages always sort last.
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Order {
    #[default]
    Oldest,
    Largest,
}

macro_rules! named_types {
    ($kind:ident { $($variant:ident => $name:literal),+ $(,)? }) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord,
            Deserialize, Serialize, JsonSchema)]
        pub enum $kind { $(#[serde(rename = $name)] $variant),+ }
        impl $kind {
            pub const ALL: &'static [Self] = &[$(Self::$variant),+];
        }
    };
}
named_types! { OutputType {
    Rust => "rust", Cargo => "cargo", CargoRegistry => "cargo-registry",
    Jvm => "jvm", Scala => "scala", Gradle => "gradle", Maven => "maven",
    Coursier => "coursier", Javascript => "javascript",
    NodeModules => "node-modules", Frontend => "frontend-output",
    PackageCache => "package-cache", Python => "python", Go => "go",
    Dotnet => "dotnet", CompilerCache => "compiler-cache", Cmake => "cmake",
    Terraform => "terraform", Temporary => "temporary", Trash => "trash",
    AgentScratch => "agent-scratch", CacheLocation => "cache-location",
    DiagnosticLogs => "diagnostic-logs",
} }

named_types! { Family {
    Rust => "rust", Jvm => "jvm", Javascript => "javascript", Python => "python",
    Go => "go", Dotnet => "dotnet", Native => "native", Infrastructure => "infrastructure",
    Packages => "packages", Location => "location", Logs => "logs",
}}

impl OutputType {
    pub const fn family(self) -> Family {
        match self {
            Self::Rust | Self::Cargo | Self::CargoRegistry => Family::Rust,
            Self::Jvm
            | Self::Scala
            | Self::Gradle
            | Self::Maven
            | Self::Coursier => Family::Jvm,
            Self::Javascript | Self::NodeModules | Self::Frontend => {
                Family::Javascript
            }
            Self::Python => Family::Python,
            Self::Go => Family::Go,
            Self::Dotnet => Family::Dotnet,
            Self::CompilerCache | Self::Cmake => Family::Native,
            Self::Terraform => Family::Infrastructure,
            Self::PackageCache => Family::Packages,
            Self::DiagnosticLogs => Family::Logs,
            Self::Temporary
            | Self::Trash
            | Self::AgentScratch
            | Self::CacheLocation => Family::Location,
        }
    }

    pub fn accepts(self, kind: Self) -> bool {
        self == kind
            || match self {
                Self::Jvm | Self::Rust | Self::Javascript => {
                    self.family() == kind.family()
                }
                _ => false,
            }
    }

    /// Subtype refinement is restricted to an already recognized JVM
    /// boundary; a scala/gradle/maven name alone cannot invent identity.
    pub fn of(entry: &Entry, path: &Path) -> Option<Self> {
        if let Some(d) = entry.artifact {
            return Some(match d.kind {
                Artifact::CargoRegistry => Self::CargoRegistry,
                Artifact::Jvm => jvm_subtype(path),
                Artifact::NodeModules => Self::NodeModules,
                Artifact::PackageCache => Self::PackageCache,
                Artifact::JsBuild => Self::Frontend,
                Artifact::Python => Self::Python,
                Artifact::GoModule => Self::Go,
                Artifact::Dotnet => Self::Dotnet,
                Artifact::CompilerCache => Self::CompilerCache,
                Artifact::CmakeBuild => Self::Cmake,
                Artifact::Terraform => Self::Terraform,
                Artifact::DiagLogs => Self::DiagnosticLogs,
                Artifact::RustTarget
                | Artifact::Worktree
                | Artifact::GitRepo
                | Artifact::VirtualMachine
                | Artifact::VmDisk
                | Artifact::VmSnapshot
                | Artifact::VmSuspend => return None,
            });
        }
        Some(match entry.category.as_str() {
            "temporary" => Self::Temporary,
            "trash" => Self::Trash,
            "agent scratch" => Self::AgentScratch,
            "cache" => Self::CacheLocation,
            "diagnostic logs" => Self::DiagnosticLogs,
            _ => return None,
        })
    }
}

fn jvm_subtype(path: &Path) -> OutputType {
    let names: Vec<_> = path
        .components()
        .rev()
        .map(|p| p.as_os_str().to_string_lossy().to_ascii_lowercase())
        .collect();
    for (i, name) in names.iter().enumerate() {
        let parent = names.get(i + 1).map(String::as_str);
        let numeric =
            |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
        let scala = name.strip_prefix("scala-").is_some_and(|version| {
            let parts: Vec<_> = version.split('.').collect();
            (parts[0] == "3" || (parts[0] == "2" && parts.len() > 1))
                && parts.iter().all(|s| numeric(s))
        });
        if parent == Some("target") && scala {
            return OutputType::Scala;
        }
        if (parent == Some("target")
            && matches!(name.as_str(), "maven-status" | "maven-archiver"))
            || (parent == Some(".m2") && name == "repository")
        {
            return OutputType::Maven;
        }
        if parent == Some(".gradle")
            && (name == "caches"
                || ((2..=3).contains(&name.split('.').count())
                    && name.split('.').all(numeric)))
            || (name == "dists"
                && parent == Some("wrapper")
                && names.get(i + 2).is_some_and(|n| n == ".gradle"))
        {
            return OutputType::Gradle;
        }
        if name == "coursier"
            && (parent == Some(".cache")
                || (parent == Some("caches")
                    && names.get(i + 2).is_some_and(|n| n == "library")))
        {
            return OutputType::Coursier;
        }
    }
    OutputType::Jvm
}

#[derive(
    Clone,
    Copy,
    Debug,
    Default,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Deserialize,
    Serialize,
    JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum Strength {
    #[default]
    Name,
    Layout,
    Metadata,
}

#[derive(Debug, Serialize)]
pub struct Identification {
    pub strength: Strength,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub classification: Option<Detection>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cargo: Option<CargoEvidence>,
}

/// Search recognized build output, caches, dependencies and descriptive
/// temporary/scratch/log/trash locations. Identity is not disposability.
#[derive(Clone, Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Params {
    /// Search below this scanned directory; default all scanned roots.
    pub path: Option<PathBuf>,
    /// OR of subtypes or families. Omit/empty for all. jvm includes scala,
    /// gradle, maven and coursier; rust includes cargo and cargo-registry.
    #[serde(default)]
    pub types: Vec<OutputType>,
    /// Optional ecosystem/family filter, combined with types by AND.
    pub family: Option<Family>,
    /// Weakest evidence to include. Cargo output always requires metadata.
    /// Name/layout evidence for other types is not metadata verification.
    #[serde(default)]
    pub minimum_evidence: Strength,
    /// Narrow by the existing display category, independently of output type.
    pub category: Option<CategoryName>,
    /// Case-insensitive substring of the candidate root's name.
    pub name: Option<String>,
    /// Minimum listed allocation (or apparent bytes in an apparent scan).
    pub min_bytes: Option<u64>,
    /// Newest file write beneath the root must be older than this many days.
    /// Unknown ages are excluded. This is not access time or activity detection.
    pub older_than_days: Option<u32>,
    /// Oldest first (default), breaking ties by largest; or largest first.
    #[serde(default)]
    pub order: Order,
    /// Number of candidate roots to return; default 25, maximum 200.
    pub limit: Option<usize>,
    /// Skip this many matches after ordering. Use `next_offset` to continue.
    /// Rescan or live metadata changes can change subsequent pages.
    #[serde(default)]
    pub offset: usize,
}

pub struct Candidate {
    pub path: PathBuf,
    pub entry: Entry,
    /// Measurements, filtering and context all belong to this snapshot.
    pub context: ScanContext,
    pub output_type: OutputType,
    pub cargo_profiles: Option<Vec<PathBuf>>,
}

impl Candidate {
    pub const fn strength(&self) -> Strength {
        if self.cargo_profiles.is_some() {
            Strength::Metadata
        } else {
            match self.entry.artifact {
                Some(Detection {
                    evidence: disktree_core::artifact::Evidence::Layout,
                    ..
                }) => Strength::Layout,
                _ => Strength::Name,
            }
        }
    }
}

/// Scanned measurements and live identity evidence have separate times.
#[derive(Debug, Serialize)]
pub struct OutputRow {
    pub path: PathBuf,
    pub output_type: OutputType,
    pub family: Family,
    pub assessment: Disposition,
    pub covered_by: Option<PathBuf>,
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
    pub evidence: Identification,
}

#[derive(Debug, Serialize)]
pub struct Excluded {
    pub path: PathBuf,
    pub listed_bytes: Option<u64>,
    pub reason: String,
}

#[derive(Clone, Debug, Serialize)]
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

impl From<&crate::session::Scan> for ScanContext {
    fn from(scan: &crate::session::Scan) -> Self {
        Self {
            root: scan.root.clone(),
            scanned_at: scan.at,
            apparent_size: scan.options.apparent_size,
            unreadable: scan.unreadable,
            include_hidden: scan.options.include_hidden,
            max_depth: scan.options.max_depth,
            depth_limit_known: true,
            one_filesystem: Some(scan.options.one_filesystem),
        }
    }
}

impl TryFrom<&disktree_link::State> for ScanContext {
    type Error = String;

    fn try_from(state: &disktree_link::State) -> Result<Self, Self::Error> {
        let tree = state.tree.as_ref().ok_or("No completed window scan")?;
        Ok(Self {
            root: state.root.clone(),
            scanned_at: tree.scanned_at,
            apparent_size: tree.apparent_size,
            unreadable: tree.unreadable,
            include_hidden: tree.include_hidden,
            max_depth: None,
            depth_limit_known: false,
            one_filesystem: None,
        })
    }
}

#[derive(Debug, Serialize)]
pub struct TypeSummary {
    pub output_type: OutputType,
    pub roots: usize,
    pub listed_bytes: u64,
}

#[derive(Debug, Serialize)]
pub struct Report {
    pub scans: Vec<ScanContext>,
    pub order: Order,
    pub discovered_roots: usize,
    pub inspected_roots: usize,
    pub discovery_truncated: bool,
    pub inspection_truncated: bool,
    pub candidate_roots: usize,
    pub metadata_verified_roots: usize,
    pub by_type: Vec<TypeSummary>,
    pub metadata_capable_types: Vec<OutputType>,
    pub matching_roots: usize,
    pub matching_listed_bytes: u64,
    pub next_offset: Option<usize>,
    pub outputs: Vec<OutputRow>,
    pub excluded_count: usize,
    pub excluded: Vec<Excluded>,
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
    evidence: Identification,
    output_type: OutputType,
    git_tracking: &str,
) -> OutputRow {
    OutputRow {
        path,
        output_type,
        family: output_type.family(),
        assessment: entry
            .artifact
            .map_or(Disposition::Detected, |d| d.disposition),
        covered_by: None,
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

pub fn passes(
    params: &Params,
    kind: OutputType,
    path: &Path,
    bytes: u64,
    modified: Option<i64>,
    strength: Strength,
    at: i64,
) -> bool {
    (params.types.is_empty()
        || params.types.iter().any(|filter| filter.accepts(kind)))
        && params.family.is_none_or(|family| family == kind.family())
        && strength >= params.minimum_evidence
        && bytes >= params.min_bytes.unwrap_or(0)
        && params.name.as_ref().is_none_or(|name| {
            path.file_name().is_some_and(|s| {
                s.to_string_lossy()
                    .to_lowercase()
                    .contains(&name.to_lowercase())
            })
        })
        && params.older_than_days.is_none_or(|days| {
            modified.is_some_and(|m| {
                m < at.saturating_sub(i64::from(days) * 86_400)
            })
        })
}

pub fn finish(mut report: Report, params: &Params, at: i64) -> Report {
    let mut roots = report.outputs;
    roots.retain(|o| {
        passes(
            params,
            o.output_type,
            &o.path,
            o.listed_bytes,
            o.last_modified,
            o.evidence.strength,
            at,
        )
    });
    // Prefer specific metadata at an identical path. Across different types
    // retain nested rows; otherwise a generic cache location hides its tools.
    roots.sort_by(|a, b| {
        a.path
            .cmp(&b.path)
            .then(b.evidence.strength.cmp(&a.evidence.strength))
    });
    roots.dedup_by(|a, b| a.path == b.path);
    let mut kept: Vec<OutputRow> = Vec::new();
    for mut row in roots {
        if kept.iter().any(|parent| {
            parent.output_type == row.output_type
                && row.path.starts_with(&parent.path)
        }) {
            continue;
        }
        row.covered_by = kept
            .iter()
            .rev()
            .find(|parent| row.path.starts_with(&parent.path))
            .map(|o| o.path.clone());
        kept.push(row);
    }
    report.candidate_roots = kept.len();
    report.metadata_verified_roots = kept
        .iter()
        .filter(|o| o.evidence.strength == Strength::Metadata)
        .count();
    report.matching_listed_bytes = kept
        .iter()
        .filter(|o| o.covered_by.is_none())
        .fold(0_u64, |total, o| total.saturating_add(o.listed_bytes));
    report.by_type = OutputType::ALL
        .iter()
        .filter_map(|kind| {
            let rows: Vec<_> =
                kept.iter().filter(|o| o.output_type == *kind).collect();
            (!rows.is_empty()).then(|| TypeSummary {
                output_type: *kind,
                roots: rows.len(),
                listed_bytes: rows.iter().fold(0_u64, |total, o| {
                    total.saturating_add(o.listed_bytes)
                }),
            })
        })
        .collect();
    let mut roots = kept;
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
    #[test]
    fn the_entire_type_catalog_agrees_with_schema_and_filter_model() {
        let schema = serde_json::to_value(schemars::schema_for!(Params))
            .expect("schema");
        let advertised = schema["$defs"]["OutputType"]["enum"]
            .as_array()
            .expect("types");
        assert_eq!(advertised.len(), OutputType::ALL.len());
        let families = schema["$defs"]["Family"]["enum"]
            .as_array()
            .expect("families");
        assert_eq!(families.len(), Family::ALL.len());
        for family in Family::ALL {
            let value = serde_json::to_value(family).expect("family");
            assert!(families.contains(&value));
            let params: Params =
                serde_json::from_value(serde_json::json!({"family":value}))
                    .expect("advertised family deserializes");
            assert_eq!(params.family, Some(*family));
        }
        let edges = [
            (OutputType::Rust, OutputType::Cargo),
            (OutputType::Rust, OutputType::CargoRegistry),
            (OutputType::Jvm, OutputType::Scala),
            (OutputType::Jvm, OutputType::Gradle),
            (OutputType::Jvm, OutputType::Maven),
            (OutputType::Jvm, OutputType::Coursier),
            (OutputType::Javascript, OutputType::NodeModules),
            (OutputType::Javascript, OutputType::Frontend),
        ];
        for &selector in OutputType::ALL {
            let value = serde_json::to_value(selector).expect("type name");
            assert!(advertised.contains(&value));
            let params: Params = serde_json::from_value(
                serde_json::json!({"types":[value],"limit":200}),
            )
            .expect("every advertised type deserializes");
            let source = OutputType::ALL
                .iter()
                .enumerate()
                .map(|(i, &kind)| {
                    let mut row = row(
                        &format!("/invented/{i:03}"),
                        10 + i as u64,
                        Some(10),
                    );
                    row.output_type = kind;
                    row.family = kind.family();
                    row
                })
                .collect();
            let got = finish(report(source), &params, 1_000_000);
            let expected: Vec<_> = OutputType::ALL
                .iter()
                .copied()
                .filter(|&kind| {
                    kind == selector || edges.contains(&(selector, kind))
                })
                .collect();
            assert_eq!(got.matching_roots, expected.len(), "{selector:?}");
            assert!(
                got.outputs
                    .iter()
                    .all(|row| expected.contains(&row.output_type))
            );
        }
        assert!(
            serde_json::from_value::<Params>(
                serde_json::json!({"types":["unknown-output"]})
            )
            .is_err()
        );
    }

    #[test]
    fn heterogeneous_overlaps_preserve_types_and_measure_the_union() {
        let at = 1_000_000;
        let source = || {
            let mut parent = row("/r/cache", 30, Some(at));
            parent.output_type = OutputType::CacheLocation;
            parent.family = Family::Location;
            parent.evidence.strength = Strength::Name;
            let mut scala = row("/r/cache/scala", 7, Some(1));
            scala.output_type = OutputType::Scala;
            scala.family = Family::Jvm;
            scala.evidence.strength = Strength::Layout;
            vec![parent, row("/r/cache/rust", 11, Some(1)), scala]
        };
        let all = finish(report(source()), &Params::default(), at);
        assert_eq!(all.matching_roots, 3);
        assert_eq!(all.matching_listed_bytes, 30);
        assert_eq!(
            all.outputs
                .iter()
                .filter(|r| r.covered_by.is_some())
                .count(),
            2
        );
        let old = finish(
            report(source()),
            &Params {
                older_than_days: Some(1),
                ..Params::default()
            },
            at,
        );
        assert_eq!(old.matching_roots, 2);
        assert_eq!(old.matching_listed_bytes, 18);
        assert!(old.outputs.iter().all(|r| r.covered_by.is_none()));
        let jvm = finish(
            report(source()),
            &Params {
                types: vec![OutputType::Jvm],
                ..Params::default()
            },
            at,
        );
        assert_eq!(jvm.matching_listed_bytes, 7);
        assert_eq!(jvm.outputs[0].output_type, OutputType::Scala);
        let metadata = finish(
            report(source()),
            &Params {
                minimum_evidence: Strength::Metadata,
                ..Params::default()
            },
            at,
        );
        // Only the Cargo row's metadata proof qualifies, regardless of size.
        assert_eq!(metadata.matching_roots, 1);
        assert_eq!(metadata.outputs[0].output_type, OutputType::Cargo);
    }

    fn row(path: &str, bytes: u64, modified: Option<i64>) -> OutputRow {
        OutputRow {
            path: path.into(),
            output_type: OutputType::Cargo,
            family: Family::Rust,
            assessment: Disposition::Suggested,
            covered_by: None,
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
            evidence: Identification {
                strength: Strength::Metadata,
                classification: None,
                cargo: Some(CargoEvidence::default()),
            },
        }
    }

    fn report(outputs: Vec<OutputRow>) -> Report {
        Report {
            scans: Vec::new(),
            order: Order::Oldest,
            discovered_roots: outputs.len(),
            inspected_roots: outputs.len(),
            discovery_truncated: false,
            inspection_truncated: false,
            candidate_roots: 0,
            metadata_verified_roots: 0,
            by_type: Vec::new(),
            metadata_capable_types: vec![OutputType::Cargo],
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
        assert_eq!(got.candidate_roots, 1);
        assert_eq!(got.matching_listed_bytes, 7);
    }
}
