// Model cold compilation separately from test execution using ci.yml's
// actual commands and deadlines. No Rust builds or wall-clock sleeps.
const assert = require('node:assert/strict');
const {readFileSync} = require('node:fs');
const {join} = require('node:path');
const {test} = require('node:test');

const workflow = readFileSync(
  join(__dirname, '../../.github/workflows/ci.yml'), 'utf8');
function budget(jobName, runner) {
  const job = workflow.split(`\n  ${jobName}:\n`)[1]
    ?.split(/\n  [a-z][\w-]*:\n/)[0];
  assert.ok(job, `missing ${jobName} job`);
  const matrix = runner && job.match(new RegExp(
    `runner: ${runner}, job_timeout: (\\d+), test_timeout: (\\d+)`));
  if (runner) assert.ok(matrix, `missing ${runner} budgets`);
  const jobLimit = Number(matrix?.[1] ||
    job.match(/\n    timeout-minutes: (\d+)/)?.[1]);
  const steps = job.split(/^      - /m).slice(1);
  const testIndex = steps.findIndex(step => /run: cargo xtask test/.test(step));
  assert.ok(testIndex >= 0, 'missing test gate');
  const testLimit = Number(matrix?.[2] ||
    steps[testIndex].match(/timeout-minutes: (\d+)/)?.[1]);
  if (runner) assert.match(steps[testIndex],
    /timeout-minutes: \$\{\{ matrix\.test_timeout \}\}/);
  const compile = steps.slice(0, testIndex).find(step => /--no-run/.test(step));
  if (compile && runner) {
    assert.match(compile, /if: matrix\.runner == 'macos-15-intel'/);
  }
  const compileLimit = compile
    ? Number(compile.match(/timeout-minutes: (\d+)/)?.[1]) : jobLimit;
  for (const limit of [jobLimit, testLimit, compileLimit]) {
    assert.ok(Number.isFinite(limit) && limit > 0, 'finite phase deadline');
  }
  // testMinutes includes its later screenshot-feature build and scripts.
  const warm = compile && [
    /cargo build -p xtask(?:\n|$)/,
    /cargo test --workspace --no-run(?:\n|$)/,
  ].every(command => command.test(compile));
  function completion(buildMinutes, testMinutes) {
    const buildStep = warm ? buildMinutes : 0;
    const testStep = testMinutes + (warm ? 0 : buildMinutes);
    if (buildStep > compileLimit) return 'compile deadline';
    if (testStep > testLimit) return 'test deadline';
    // Reserve a minute for setup; the job still bounds both phases.
    if (1 + buildMinutes + testMinutes > jobLimit) return 'job deadline';
    return 'complete';
  }
  return {compileLimit, testLimit, jobLimit, completion};
}

const intel = budget('other-architectures', 'macos-15-intel');
const arm = budget('check-macos');

test('a cold Intel build leaves the execution budget intact', () => {
  // Run 42317321199 spent 10m44s compiling before executing tests. The
  // oracle passed, but the shared 20-minute step expired during cleanup.
  assert.equal(intel.completion(11, 10), 'complete');
});

test('Apple silicon compilation leaves room for every test suite', () => {
  // Run 42325538195 compiled for 3m06s, then spent 6m23s in core alone.
  // Round up compilation and allow the remaining suites another minute.
  assert.equal(arm.completion(4, 8), 'complete');
});

for (const [platform, config] of [['Intel', intel], ['Apple silicon', arm]]) {
  const {compileLimit, testLimit, jobLimit, completion} = config;
  test(`${platform}: compilation cannot consume the last execution minute`, () => {
    assert.equal(completion(1, testLimit), 'complete');
  });

  test(`${platform}: work fitting both phases and the job can finish`, () => {
    for (let build = 1; build <= compileLimit; build++) {
      for (let execution = 1; execution <= testLimit; execution++) {
        if (1 + build + execution > jobLimit) continue;
        assert.equal(completion(build, execution), 'complete',
          `compile ${build}m, execute ${execution}m`);
      }
    }
  });

  test(`${platform}: separate phases still enforce deadlines`, () => {
    assert.equal(completion(compileLimit + 1, 0), 'compile deadline');
    assert.equal(completion(0, testLimit + 1), 'test deadline');
    // Even individually valid phases cannot exceed the enclosing job.
    if (1 + compileLimit + testLimit > jobLimit) {
      assert.equal(completion(compileLimit, testLimit), 'job deadline');
    }
  });
}
