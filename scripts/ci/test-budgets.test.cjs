// Model cold compilation separately from test execution using ci.yml's
// actual commands and deadlines. No Rust builds or wall-clock sleeps.
const assert = require('node:assert/strict');
const {readFileSync} = require('node:fs');
const {join} = require('node:path');
const {test} = require('node:test');

const workflow = readFileSync(
  join(__dirname, '../../.github/workflows/ci.yml'), 'utf8');
const job = workflow.split('\n  other-architectures:\n')[1]
  ?.split(/\n  [a-z][\w-]*:\n/)[0];
assert.ok(job, 'missing architecture test job');
const matrix = job.match(
  /runner: macos-15-intel, job_timeout: (\d+), test_timeout: (\d+)/);
assert.ok(matrix, 'missing Intel deadline budgets');
const jobLimit = Number(matrix[1]);
const testLimit = Number(matrix[2]);
const steps = job.split(/^      - /m).slice(1);
const testIndex = steps.findIndex(step => /run: cargo xtask test/.test(step));
assert.ok(testIndex >= 0, 'missing test gate');
assert.match(steps[testIndex],
  /timeout-minutes: \$\{\{ matrix\.test_timeout \}\}/);
const compile = steps.slice(0, testIndex).find(step => /--no-run/.test(step));
if (compile) {
  assert.match(compile, /if: matrix\.runner == 'macos-15-intel'/);
  assert.match(compile, /timeout-minutes: \d+/);
}
const compileLimit = compile
  ? Number(compile.match(/timeout-minutes: (\d+)/)[1]) : jobLimit;
// Prepare the cold workspace graph and xtask before starting the gate.
// testMinutes includes its later screenshot-feature build and script checks.
const warm = compile && [
  /cargo build -p xtask(?:\n|$)/,
  /cargo test --workspace --no-run(?:\n|$)/,
].every(command => command.test(compile));

function completion(buildMinutes, testMinutes) {
  const buildStep = warm ? buildMinutes : 0;
  const testStep = testMinutes + (warm ? 0 : buildMinutes);
  if (buildStep > compileLimit) return 'compile deadline';
  if (testStep > testLimit) return 'test deadline';
  // Reserve a minute for setup; the job deadline still bounds both phases.
  if (1 + buildMinutes + testMinutes > jobLimit) return 'job deadline';
  return 'complete';
}

test('a cold Intel build leaves the execution budget intact', () => {
  // Run 42317321199 spent 10m44s compiling before executing tests. The
  // oracle passed, but the shared 20-minute step expired during cleanup.
  assert.equal(completion(11, 10), 'complete');
});

test('one compilation minute cannot consume the last execution minute', () => {
  assert.equal(completion(1, testLimit), 'complete');
});

test('work fitting each phase and the whole job can finish', () => {
  for (let build = 1; build <= compileLimit; build++) {
    for (let execution = 1; execution <= testLimit; execution++) {
      if (1 + build + execution > jobLimit) continue;
      assert.equal(completion(build, execution), 'complete',
        `compile ${build}m, execute ${execution}m`);
    }
  }
});

test('separate phases still enforce their deadlines', () => {
  assert.equal(completion(compileLimit + 1, 0), 'compile deadline');
  assert.equal(completion(0, testLimit + 1), 'test deadline');
  assert.equal(completion(compileLimit, testLimit), 'job deadline');
});
