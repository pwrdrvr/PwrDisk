#!/usr/bin/env node
// Fails when the repository-root AGENTS.md outgrows the agent instruction
// budget. Codex reads at most `project_doc_max_bytes` (32 KiB by default) of
// AGENTS.md content and silently drops the rest; Claude Code loads the whole
// file into every thread's first turn. The limit leaves headroom under 32 KiB
// for nested AGENTS.md files on the path to a deeper working directory.
//
// CI's classifier runs this on every PR, docs-only ones included, since
// those skip the lint jobs. `cargo xtask lint` holds the same check in Rust
// (xtask/src/agents_size.rs) so the local gate needs no Node; the test
// beside this file keeps the two limits equal.
const {readFileSync} = require('node:fs');
const {join} = require('node:path');

const ROOT_AGENTS_MD_MAX_BYTES = 24 * 1024;

function checkAgentInstructionsSize(bytes, maxBytes = ROOT_AGENTS_MD_MAX_BYTES) {
  if (bytes <= maxBytes) return {ok: true, bytes, maxBytes};
  return {
    ok: false,
    bytes,
    maxBytes,
    message:
      `AGENTS.md is ${bytes} bytes; the limit is ${maxBytes} bytes ` +
      `(${bytes - maxBytes} over). Codex truncates AGENTS.md content past ` +
      '32 KiB and Claude Code loads all of it on every first turn. Move ' +
      'detailed sections into linked documents under docs/agents/ and keep ' +
      'a short rule plus a when-to-read trigger in AGENTS.md.',
  };
}

function main() {
  const path = join(__dirname, '../../AGENTS.md');
  const bytes = readFileSync(path).byteLength;
  const result = checkAgentInstructionsSize(bytes);
  if (!result.ok) {
    console.error(result.message);
    process.exit(1);
  }
  console.log(`AGENTS.md is ${bytes} of ${result.maxBytes} bytes.`);
}

module.exports = {ROOT_AGENTS_MD_MAX_BYTES, checkAgentInstructionsSize};

if (require.main === module) main();
