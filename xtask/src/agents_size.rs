//! The root `AGENTS.md` stays under the agent instruction budget.
//!
//! Codex reads at most `project_doc_max_bytes` (32 KiB by default) of
//! AGENTS.md content and silently drops the rest; Claude Code loads the
//! whole file into every thread's first turn. The limit leaves headroom
//! under 32 KiB for nested AGENTS.md files on a deeper working directory's
//! path. CI's classifier runs the same check in Node
//! (`scripts/ci/check-agent-instructions-size.cjs`) for docs-only PRs,
//! which skip lint; its test keeps the two limits equal.

use std::path::Path;

pub const ROOT_AGENTS_MD_MAX_BYTES: u64 = 24 * 1024;

/// Checks a size against the limit, saying how far over it is and what to
/// do about it.
pub fn check(bytes: u64, max_bytes: u64) -> Result<(), String> {
    if bytes <= max_bytes {
        return Ok(());
    }
    Err(format!(
        "AGENTS.md is {bytes} bytes; the limit is {max_bytes} bytes ({} \
         over). Codex truncates AGENTS.md content past 32 KiB and Claude \
         Code loads all of it on every first turn. Move detailed sections \
         into linked documents under docs/agents/ and keep a short rule \
         plus a when-to-read trigger in AGENTS.md.",
        bytes - max_bytes
    ))
}

pub fn check_root() -> Result<(), String> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../AGENTS.md");
    let bytes = std::fs::metadata(&path)
        .map_err(|err| format!("cannot read {}: {err}", path.display()))?
        .len();
    check(bytes, ROOT_AGENTS_MD_MAX_BYTES)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exactly_the_sizes_at_or_under_the_limit_pass() {
        let limit = 100;
        for bytes in 0..=2 * limit {
            let result = check(bytes, limit);
            assert_eq!(result.is_ok(), bytes <= limit, "{bytes} bytes");
            if let Err(message) = result {
                let over = format!("({} over)", bytes - limit);
                assert!(message.contains(&over), "{message}");
            }
        }
    }

    #[test]
    fn the_limit_is_24_kib_under_codexs_32() {
        assert_eq!(ROOT_AGENTS_MD_MAX_BYTES, 24_576);
        assert!(check(24_576, ROOT_AGENTS_MD_MAX_BYTES).is_ok());
        assert!(check(24_577, ROOT_AGENTS_MD_MAX_BYTES).is_err());
        assert!(check(32 * 1024 + 1, ROOT_AGENTS_MD_MAX_BYTES).is_err());
    }

    #[test]
    fn this_checkout_is_within_the_limit() {
        check_root().unwrap();
    }
}
