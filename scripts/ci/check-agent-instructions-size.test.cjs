const assert = require('node:assert/strict');
const {readFileSync} = require('node:fs');
const {join} = require('node:path');
const {test} = require('node:test');

const {
  ROOT_AGENTS_MD_MAX_BYTES,
  checkAgentInstructionsSize,
} = require('./check-agent-instructions-size.cjs');

const CODEX_PROJECT_DOC_DEFAULT_MAX_BYTES = 32 * 1024;

test("the limit stays under Codex's default project-doc budget", () => {
  assert.ok(ROOT_AGENTS_MD_MAX_BYTES < CODEX_PROJECT_DOC_DEFAULT_MAX_BYTES);
});

test('exactly the sizes at or under the limit pass', () => {
  const limit = 100;
  for (let bytes = 0; bytes <= 2 * limit; bytes += 1) {
    const result = checkAgentInstructionsSize(bytes, limit);
    assert.equal(result.ok, bytes <= limit, `${bytes} bytes`);
    if (!result.ok) assert.match(result.message, new RegExp(`\\(${bytes - limit} over\\)`));
  }
});

test('the default limit is 24 KiB, both sides of it', () => {
  assert.equal(ROOT_AGENTS_MD_MAX_BYTES, 24576);
  assert.ok(checkAgentInstructionsSize(24576).ok);
  assert.ok(!checkAgentInstructionsSize(24577).ok);
  assert.ok(!checkAgentInstructionsSize(CODEX_PROJECT_DOC_DEFAULT_MAX_BYTES + 1).ok);
});

// `cargo xtask lint` checks the same file without Node; a limit changed in
// one place only would let the local gate and CI disagree.
test("xtask's copy of the limit is the same", () => {
  const rust = readFileSync(join(__dirname, '../../xtask/src/agents_size.rs'), 'utf8');
  // A product of integer literals, such as `24 * 1024` or `24_576`.
  const declared = rust.match(/pub const ROOT_AGENTS_MD_MAX_BYTES: u64 =\s*([\d_]+(?:\s*\*\s*[\d_]+)*);/);
  assert.ok(declared, 'xtask/src/agents_size.rs: missing ROOT_AGENTS_MD_MAX_BYTES');
  const value = declared[1].split('*')
    .reduce((product, factor) => product * Number(factor.replace(/[_\s]/g, '')), 1);
  assert.equal(value, ROOT_AGENTS_MD_MAX_BYTES);
});
