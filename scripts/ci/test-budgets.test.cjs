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
  const compile = steps.slice(0, testIndex).find(step => {
    const guard = step.match(/if: matrix\.runner == '([^']+)'/);
    return /--no-run/.test(step) && (!guard || guard[1] === runner);
  });
  const fixtureIndex = steps.findIndex(step =>
    /run: cargo test -p disktree-core test_paths::disk_image\b/.test(step));
  const fixtureLimit = fixtureIndex < 0 ? 0 : Number(
    steps[fixtureIndex].match(/timeout-minutes: (\d+)/)?.[1]);
  if (compile && runner) {
    assert.ok(compile.includes(`if: matrix.runner == '${runner}'`));
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
  const fixtureCommand = steps[fixtureIndex]?.split('run: ')[1]
    ?.split('\n')[0].split(' -- ')[0];
  // Workspace feature unification can produce a different test binary.
  // Warm the exact package invocation used by the lifecycle execution step.
  const warmFixture = compile && steps.indexOf(compile) < fixtureIndex &&
    compile.split('\n').some(line =>
      line.trim() === `${fixtureCommand} --no-run`);
  function completion(buildMinutes, testMinutes, fixtureMinutes = 0,
    coreBuildMinutes = 0) {
    // Core compilation is part of the total build workload. A fixture
    // command encountered first has to compile its own test binary.
    const fixtureBuild = fixtureIndex >= 0 && !warmFixture
      ? coreBuildMinutes : 0;
    if (fixtureMinutes + fixtureBuild > fixtureLimit) return 'fixture deadline';
    const buildStep = warm ? buildMinutes - fixtureBuild : 0;
    const testStep = testMinutes + (warm ? 0 : buildMinutes - fixtureBuild);
    if (buildStep > compileLimit) return 'compile deadline';
    if (testStep > testLimit) return 'test deadline';
    // Reserve a minute for setup; the job still bounds both phases.
    if (1 + fixtureMinutes + buildMinutes + testMinutes > jobLimit) {
      return 'job deadline';
    }
    return 'complete';
  }
  return {
    compileLimit, testLimit, jobLimit, completion, fixtureLimit,
    warmFixture,
    warmScreenshots: Boolean(compile?.includes(
      'cargo test -p disktree-app --features screenshots screenshots:: --no-run')),
    warmExecutable: Boolean(compile?.includes(
      'cargo build --locked -p disktree-app')),
    fixtureBeforeGate: fixtureIndex >= 0 && fixtureIndex < testIndex,
    isolatedFixture: fixtureIndex >= 0 && fixtureIndex < steps.indexOf(compile),
  };
}

const intel = budget('other-architectures', 'macos-15-intel');
const arm = budget('check-macos');

test('a cold Intel build leaves the execution budget intact', () => {
  // Run 42317321199 spent 10m44s compiling before executing tests. The
  // oracle passed, but the shared 20-minute step expired during cleanup.
  assert.equal(intel.completion(11, 10), 'complete');
});

test('Intel: cold core compilation cannot consume the fixture deadline', () => {
  // Run 42529711572 compiled core for 4m21s, then passed all 33 lifecycle
  // tests in 348.35s. Those independent phases exceeded the shared 10m step.
  assert.equal(intel.completion(11, 10, 348.35 / 60, 261.2 / 60), 'complete');
});

test('Intel: the enclosing job covers every valid phase budget', () => {
  // Lifecycle, compilation and execution have independent deadlines. A
  // valid phase must not lose its budget to time spent in earlier phases.
  for (let fixture = 0; fixture <= intel.fixtureLimit; fixture++) {
    for (let build = 0; build <= intel.compileLimit; build++) {
      for (let execution = 0; execution <= intel.testLimit; execution++) {
        for (let core = 0; core <= build; core++) {
          assert.equal(intel.completion(build, execution, fixture, core), 'complete',
            `fixture ${fixture}m, compile ${build}m (${core}m core), execute ${execution}m`);
        }
      }
    }
  }
  assert.ok(intel.jobLimit >=
    intel.fixtureLimit + intel.compileLimit + intel.testLimit + 5,
  'leave five minutes for setup and cleanup beyond the phase deadlines');
});

test('Intel: a cold build and long APFS run leave room for feature tests', () => {
  // Run 42415250815 passed all workspace tests, including 10m07s in core,
  // then the 30-minute job ended during the screenshot-feature build.
  // This workload fits each phase's deadline but exceeded that job limit.
  assert.equal(intel.completion(14, 14, 4), 'complete');
});

test('Apple silicon compilation leaves room for every test suite', () => {
  // Run 42325538195 compiled for 3m06s, then spent 6m23s in core alone.
  // Round up compilation and allow the remaining suites another minute.
  assert.equal(arm.completion(4, 8), 'complete');
});

for (const [platform, config] of [['Intel', intel], ['Apple silicon', arm]]) {
  const {compileLimit, testLimit, jobLimit, completion, fixtureLimit} = config;
  test(`${platform}: fixtures are isolated from the full workspace test gate`, () => {
    assert.ok(config.fixtureBeforeGate);
    if (platform === 'Intel') {
      assert.ok(config.warmFixture,
        'compile the lifecycle binary before starting its execution deadline');
    } else {
      assert.ok(config.isolatedFixture);
    }
  });

  test(`${platform}: isolated fixture and cold test gate fit the job`, () => {
    const workload = platform === 'Intel' ? [11, 10, 4] : [4, 8, 3];
    assert.equal(completion(...workload), 'complete');
  });
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
    assert.equal(completion(0, 0, fixtureLimit + 1), 'fixture deadline');
    assert.equal(completion(compileLimit + 1, 0), 'compile deadline');
    assert.equal(completion(0, testLimit + 1), 'test deadline');
    // Even individually valid phases cannot exceed the enclosing job.
    if (1 + compileLimit + testLimit > jobLimit) {
      assert.equal(completion(compileLimit, testLimit), 'job deadline');
    }
  });
}

// Run 42559851081 exhausted these deadlines during compilation, before
// executing tests. Model one minute beyond each observed cutoff plus test
// execution: restoring a warm cache cannot be required for correctness.
for (const [platform, config, build, execution] of [
  ['Linux x64', budget('check'), 11, 2],
  ['Linux ARM', budget('other-architectures', 'ubuntu-24.04-arm'), 9, 2],
  ['macOS ARM', arm, 11, 8],
  ['macOS Intel', intel, 16, 10],
]) {
  test(`${platform}: cold compilation exceeding the old limit still runs tests`, () => {
    assert.equal(config.completion(build, execution), 'complete');
  });
  test(`${platform}: compilation cannot spend the execution deadline`, () => {
    assert.equal(config.completion(1, config.testLimit), 'complete');
  });
}

// Run 42562931627: ARM passed workspace tests but compiled screenshot tests
// inside its 15-minute gate. x64 passed lint/tests but its later executable
// build consumed the enclosing 30-minute job. Model both cold workloads.
for (const [platform, config, build, execution] of [
  ['Windows x64', budget('windows'), 28, 4],
  ['Windows ARM', budget('other-architectures', 'windows-11-arm'), 16, 2],
]) {
  test(`${platform}: cold build and execution have separate budgets`, () => {
    assert.equal(config.completion(build, execution), 'complete');
    assert.ok(config.warmScreenshots,
      'screenshot-feature code generation belongs to compilation');
  });
  test(`${platform}: every valid compile and execution phase fits the job`, () => {
    for (let build = 0; build <= config.compileLimit; build++) {
      for (let execution = 0; execution <= config.testLimit; execution++) {
        assert.equal(config.completion(build, execution), 'complete');
      }
    }
  });
}
test('Windows icon validation uses an executable built in the compile phase', () => {
  assert.ok(budget('windows').warmExecutable);
});
