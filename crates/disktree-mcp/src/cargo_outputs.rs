//! Cargo is one metadata-backed detector behind the generic output query.
use serde::Serialize;
use serde_json::Value;
use std::fs::{self, File};
use std::io::Read as _;
use std::path::{Path, PathBuf};
const METADATA_LIMIT: u64 = 256 * 1_024;
const TAG: &[u8] = b"Signature: 8a477f597d28d172789f06886806bc55";

#[derive(Debug, Default, Serialize)]
pub struct Evidence {
    pub rustc_info: bool,
    pub cache_tag: bool,
    pub fingerprint_json: Vec<PathBuf>,
    pub dependency_directories: Vec<PathBuf>,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::outputs::git_tracking;
    use std::process::Command;
    use std::time::{Duration, Instant};

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
}
