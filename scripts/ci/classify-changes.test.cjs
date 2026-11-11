// Fixtures for ci.yml's "Classify Changes" job, ported from PwrRoute.
// Run with `node --test scripts/ci/classify-changes.test.cjs`; CI runs it in
// that job, and it needs no npm packages or GitHub credentials.
//
// Expensive CI skips only when every changed path, and a rename's old path,
// is one of these. Anything else, unknown paths included, runs full CI.
//
// - `docs/**` and Markdown anywhere (`.md`, any case): nothing builds or
//   tests them. `README.md` is copied into release archives by release.yml,
//   which is not this workflow. The exception is `CHANGELOG.md`: xtask's
//   release check reads it, and so does a test of that check. So is
//   `skills/**`: `pwrdisk mcp` compiles those skills in and tests them.
// - `.agents/skills/**`: agent resources, not app or CI inputs.
// - `.github/ISSUE_TEMPLATE/**`, `.github/PULL_REQUEST_TEMPLATE/**` and
//   exactly `.github/dependabot.yml`.
// - exactly the root `.gitignore`: git reads it, and no build or test does.
//   Cargo's packaging honours it, but CI never packages a crate. A nested
//   `.gitignore` still runs full CI.
//
// If a build, test or `include_str!` starts reading one of these, take it
// out of the allowlist first. The last test below catches the
// `include_str!` case.

const assert = require('node:assert/strict');
const { execFileSync } = require('node:child_process');
const { readFileSync } = require('node:fs');
const { dirname, join, posix, relative } = require('node:path');
const { test } = require('node:test');

const root = join(__dirname, '../..');

// Execute the actual github-script body, so fixtures cannot drift from CI.
const workflow = readFileSync(join(root, '.github/workflows/ci.yml'), 'utf8');
const script = workflow.match(/          script: \|\n((?:            .*\n|\n)+)/)?.[1];
assert.ok(script, 'the classifier must remain covered by these tests');
const classify = new (async function () {}).constructor('github', 'context', 'core', script);

const sha = (c) => c.repeat(40);

async function run({ paths, files = paths?.map(filename => ({ filename, status: 'modified' })),
  count = files?.length, event = 'pull_request', apiError = false, summaryError = false,
  labels = [], labelError = false, before = sha('a'), after = sha('b'),
  compare = { status: 'ahead', files }, compareError = false }) {
  const outputs = {};
  const calls = [];
  const summary = {
    addHeading() { return this; },
    addTable() { return this; },
    addRaw() { return this; },
    async write() { if (summaryError) throw new Error('summary unavailable'); },
  };
  await classify({
    rest: {
      pulls: {
        listFiles: 'pulls.listFiles',
        async get(args) {
          calls.push({ method: 'pulls.get', args });
          if (labelError) throw new Error('labels unavailable');
          return { data: { labels: labels.map(name => ({ name })) } };
        },
      },
      repos: {
        async compareCommitsWithBasehead(args) {
          calls.push({ method: 'repos.compareCommitsWithBasehead', args });
          if (compareError) throw new Error('compare unavailable');
          return { data: compare };
        },
      },
    },
    async paginate(method, args) {
      calls.push({ method, args });
      if (apiError) throw new Error('API unavailable');
      return files;
    },
  }, {
    eventName: event,
    repo: { owner: 'pwrdrvr', repo: 'PwrDisk' },
    issue: { number: 11 },
    payload: event === 'push' ? { before, after } : { pull_request: { changed_files: count } },
  }, {
    setOutput(name, value) { outputs[name] = value; },
    notice() {},
    summary,
  });
  return { outputs, calls };
}

// PR #11, docs(agents): agent guidance and the PR screenshot procedure.
const agentGuidance = ['AGENTS.md', 'CLAUDE.md', 'docs/pr-screenshots.md'];

for (const [name, paths] of [
  ['PR #11: agent guidance', agentGuidance],
  ...agentGuidance.map(path => [`PR #11: ${path}`, [path]]),
  ['skill files, scripts and images', [
    '.agents/skills/example/SKILL.md',
    '.agents/skills/example/agents/openai.yaml',
    '.agents/skills/example/scripts/check.sh',
    '.agents/skills/example/assets/icon.png',
  ]],
  ['documentation, logs and patches under docs/', [
    'README.md', '.github/workflows/README.md', 'crates/disktree-core/NOTES.MD',
    'docs/performance/mark-resolution/green.log',
    'docs/performance/mark-resolution/additional-audit.patch',
  ]],
  ['Dependabot configuration', ['.github/dependabot.yml']],
  ['the root .gitignore', ['.gitignore']],
  ['docs and the root .gitignore together', ['.gitignore', 'docs/guide.md', '.agents/skills/example/SKILL.md']],
  ['issue and PR templates', ['.github/ISSUE_TEMPLATE/bug_report.yml', '.github/PULL_REQUEST_TEMPLATE/custom.yml']],
]) {
  test(`${name} skips expensive CI`, async () => {
    assert.equal((await run({ paths })).outputs.code_impacting, 'false');
  });
}

for (const path of [
  'crates/disktree-core/src/scan.rs', 'crates/disktree-app/Cargo.toml',
  'Cargo.toml', 'Cargo.lock', 'rust-toolchain.toml', '.cargo/config.toml',
  'clippy.toml', 'rustfmt.toml', 'Makefile', 'xtask/src/main.rs',
  'assets/disktree.svg', 'assets/screenshot.png', 'packaging/macos/Info.plist.in',
  'packaging/pwrdisk.desktop.in', 'scripts/release/install-vpk.sh',
  '.github/workflows/ci.yml', '.github/workflows/release.yml',
  '.github/actions/setup-rust/action.yml', 'scripts/ci/classify-changes.test.cjs', 'licenses/velopack.txt',
  'LICENSE', 'CHANGELOG.md', 'skills/pwrdisk-free-space/SKILL.md',
  'skills/pwrdisk-free-space/references/macos.md', 'crates/disktree-app/.gitignore', '.agents/config.toml', 'unknown.yaml',
]) {
  test(`${path} alone or mixed with docs runs full CI`, async () => {
    for (const paths of [[path], [...agentGuidance, path]]) {
      assert.equal((await run({ paths })).outputs.code_impacting, 'true');
    }
  });
}

for (const [previous_filename, filename, full] of [
  ['docs/old.md', 'docs/new.md', false],
  ['.agents/skills/old/SKILL.md', '.agents/skills/new/SKILL.md', false],
  ['scripts/test-ssh.ts', 'docs/test-ssh.ts', true],
  ['docs/notes.md', 'CHANGELOG.md', true],
  ['README.md', 'Cargo.toml', true],
]) {
  test(`rename ${previous_filename} to ${filename}`, async () => {
    const result = await run({ files: [{ status: 'renamed', filename, previous_filename }] });
    assert.equal(result.outputs.code_impacting, String(full));
  });
}

for (const [name, input] of [
  ['empty list', { files: [] }],
  ['incomplete pagination', { paths: agentGuidance, count: 101 }],
  ['API 3,000-file cap', { paths: Array.from({ length: 3000 }, (_, i) => `docs/${i}.md`), count: 3001 }],
  ['missing count', { paths: agentGuidance, count: null }],
  ['noninteger count', { paths: agentGuidance, count: 3.5 }],
  ['wrong count type', { paths: agentGuidance, count: '3' }],
  ['duplicate pages', { paths: [agentGuidance[0], agentGuidance[0]] }],
  ['malformed file', { files: [{}] }],
  ['null file', { files: [null] }],
  ['empty filename', { files: [{ filename: '' }] }],
  ['invalid filename', { files: [{ filename: 42 }] }],
  ['rename without old path', { files: [{ filename: 'docs/a.md', status: 'renamed' }] }],
  ['invalid old path', { files: [{ filename: 'docs/a.md', previous_filename: 42 }] }],
  ['empty old path', { files: [{ filename: 'docs/a.md', previous_filename: '' }] }],
  ['API failure', { paths: agentGuidance, apiError: true }],
]) {
  test(`${name} runs full CI`, async () => {
    const { outputs } = await run(input);
    assert.equal(outputs.code_impacting, 'true');
    assert.equal(outputs.licenses_impacting, 'true');
    assert.equal(outputs.packaging_impacting, 'true');
  });
}

test('uses the paginating API for more than one page', async () => {
  const paths = Array.from({ length: 205 }, (_, i) => `docs/${i}.md`);
  const result = await run({ paths });
  assert.equal(result.outputs.code_impacting, 'false');
  assert.deepEqual(result.calls.filter(c => c.method !== 'pulls.get'), [{ method: 'pulls.listFiles', args: {
    owner: 'pwrdrvr', repo: 'PwrDisk', pull_number: 11, per_page: 100,
  } }]);
});

test('pushes always lint, test and check notices, without querying PR files', async () => {
  for (const paths of [agentGuidance, ['crates/disktree-core/src/scan.rs']]) {
    const result = await run({ paths, event: 'push' });
    assert.equal(result.outputs.code_impacting, 'true');
    assert.equal(result.outputs.licenses_impacting, 'true');
    assert.ok(result.calls.every(c => !c.method.startsWith('pulls.')));
  }
});

// Bundles, installers and the ARM legs: what they are built from, the
// dependencies and toolchain, and CI's own setup.
const packagingPaths = [
  'packaging/macos/Info.plist.in', 'packaging/macos/AppIcon.icon/icon.json',
  'packaging/pwrdisk.desktop.in', 'assets/disktree.svg',
  'scripts/release/install-vpk.sh', 'xtask/src/bundle.rs', 'xtask/Cargo.toml',
  'crates/disktree-app/build.rs', 'Cargo.lock', 'Cargo.toml',
  'crates/disktree-core/Cargo.toml', 'rust-toolchain.toml', '.cargo/config.toml',
  '.github/workflows/ci.yml', '.github/actions/setup-rust/action.yml',
];
const ordinaryCode = [
  'crates/disktree-core/src/scan.rs', 'crates/disktree-app/src/views.rs',
  'skills/pwrdisk-free-space/SKILL.md', 'CHANGELOG.md', 'Makefile',
  'docs/packaging.md', 'packaging-notes.txt', 'other/xtask/main.rs',
];

test('packaging inputs select the packaging checks on PRs, including renames', async () => {
  for (const path of packagingPaths) {
    for (const paths of [[path], [...ordinaryCode, path]]) {
      const { outputs } = await run({ paths });
      assert.equal(outputs.packaging_impacting, 'true', path);
      assert.equal(outputs.code_impacting, 'true', path);
    }
    for (const [previous_filename, filename] of [[path, 'src/moved.rs'], ['src/moved.rs', path]]) {
      assert.equal((await run({ files: [{ filename, previous_filename, status: 'renamed' }] }))
        .outputs.packaging_impacting, 'true', `rename: ${previous_filename} to ${filename}`);
    }
  }
});

test('ordinary code and docs skip the packaging checks on PRs', async () => {
  for (const paths of [ordinaryCode, agentGuidance]) {
    assert.equal((await run({ paths })).outputs.packaging_impacting, 'false');
  }
});

test('the full-ci label runs everything, even for docs, read live', async () => {
  const result = await run({ paths: agentGuidance, labels: ['documentation', 'full-ci'] });
  for (const field of ['code_impacting', 'licenses_impacting', 'packaging_impacting']) {
    assert.equal(result.outputs[field], 'true', field);
  }
  assert.deepEqual(result.calls[0], { method: 'pulls.get', args: {
    owner: 'pwrdrvr', repo: 'PwrDisk', pull_number: 11 } });
  const other = await run({ paths: ordinaryCode, labels: ['build-preview'] });
  assert.equal(other.outputs.packaging_impacting, 'false');
});

test('a failed label lookup runs full CI', async () => {
  const { outputs } = await run({ paths: agentGuidance, labelError: true });
  assert.equal(outputs.code_impacting, 'true');
  assert.equal(outputs.packaging_impacting, 'true');
});

test('main pushes run packaging checks only when the pushed range has a packaging input', async () => {
  for (const path of packagingPaths) {
    const result = await run({ event: 'push', paths: [...ordinaryCode, path] });
    assert.equal(result.outputs.packaging_impacting, 'true', path);
  }
  const result = await run({ event: 'push', paths: ordinaryCode });
  assert.equal(result.outputs.packaging_impacting, 'false');
  assert.deepEqual(result.calls, [{ method: 'repos.compareCommitsWithBasehead', args: {
    owner: 'pwrdrvr', repo: 'PwrDisk', basehead: `${sha('a')}...${sha('b')}` } }]);
});

for (const [name, input] of [
  ['new branch (zero before)', { before: sha('0') }],
  ['missing before', { before: null }],
  ['malformed after', { after: 'main' }],
  ['compare failure', { compareError: true }],
  ['force push', { compare: { status: 'diverged', files: [] } }],
  ['identical range', { compare: { status: 'identical', files: [] } }],
  ['empty file list', { compare: { status: 'ahead', files: [] } }],
  ['300-file compare cap', { compare: { status: 'ahead',
    files: Array.from({ length: 300 }, (_, i) => ({ filename: `src/${i}.rs` })) } }],
  ['malformed file', { compare: { status: 'ahead', files: [{}] } }],
  ['missing files', { compare: { status: 'ahead' } }],
]) {
  test(`main push with ${name} runs the packaging checks`, async () => {
    const { outputs } = await run({ event: 'push', paths: ordinaryCode, ...input });
    assert.equal(outputs.packaging_impacting, 'true');
    assert.equal(outputs.code_impacting, 'true');
  });
}

test('license inputs and gate changes select the notice check, including renames', async () => {
  for (const path of [
    'Cargo.lock', 'Cargo.toml', 'crates/disktree-app/Cargo.toml',
    'xtask/Cargo.toml', 'licenses/about.toml', 'licenses/about.hbs',
    'licenses/bundled.txt', 'licenses/velopack.txt', 'licenses/nested/NOTES.md',
    'THIRD_PARTY_LICENSES', '.github/workflows/ci.yml',
    '.github/actions/setup-rust/action.yml',
    '.github/actions/setup-rust/prune.sh', 'xtask/src/licenses.rs',
  ]) {
    for (const status of ['modified', 'added', 'removed']) {
      assert.equal((await run({ files: [{ filename: path, status }] }))
        .outputs.licenses_impacting, 'true', `${status}: ${path}`);
    }
    for (const [previous_filename, filename] of [[path, 'docs/old.md'], ['docs/old.md', path]]) {
      assert.equal((await run({ files: [{ filename, previous_filename, status: 'renamed' }] }))
        .outputs.licenses_impacting, 'true', `rename: ${previous_filename} to ${filename}`);
    }
  }
});

test('unrelated changes skip the notice check even when they run full CI', async () => {
  for (const paths of [agentGuidance, [
    'crates/disktree-core/src/scan.rs', 'assets/disktree.svg', 'CHANGELOG.md',
    'docs/Cargo.toml.md', 'other/Cargo.lock', 'licenses-other/about.toml',
  ]]) {
    assert.equal((await run({ paths })).outputs.licenses_impacting, 'false');
  }
});

for (const field of ['code_impacting', 'licenses_impacting', 'packaging_impacting']) {
  test(`${field} job output fails open after script failures or missing output`, async () => {
    assert.match(workflow, /continue-on-error: true/);
    const expression = workflow.match(new RegExp(`${field}: \\$\\{\\{ (.+) \\}\\}`))[1];
    const output = (outcome, value) => new Function('steps', `return (${expression});`)({
      classify: { outcome, outputs: { [field]: value } },
    });
    assert.equal(output('success', 'false'), false);
    for (const [outcome, value] of [
      ['success', 'true'], ['success', undefined], ['failure', 'false'],
      ['failure', undefined], ['skipped', undefined], ['cancelled', 'false'],
    ]) assert.equal(output(outcome, value), true);
    await assert.rejects(run({ paths: agentGuidance, summaryError: true }), /summary unavailable/);
  });
}

test('every other job runs on classifier failure and skips only on explicit irrelevant success', () => {
  const jobs = workflow.split('\njobs:\n')[1];
  const ids = [...jobs.matchAll(/^  ([a-z][\w-]*):\n/gm)].map(m => m[1]);
  assert.deepEqual(ids, ['changes', 'licenses', 'check', 'check-macos', 'bundle-macos-26', 'windows',
    'other-architectures'],
    'a new job needs a decision here: gate it on the classifier or not');
  for (const id of ids.slice(1)) {
    const job = workflow.match(new RegExp(`^  ${id}:\\n([\\s\\S]*?)(?=^  [a-z].*:|$(?![\\s\\S]))`, 'm'))?.[1];
    assert.ok(job, id);
    assert.match(job, /^    needs: changes\n/m, `${id} must wait for the classifier`);
    const expression = job.match(/    if: >-\n([\s\S]*?)(?=^    \w)/m)?.[1];
    assert.ok(expression, `${id} must have an explicit fail-safe job condition`);
    const evaluate = new Function('needs', 'cancelled', `return (${expression});`);
    for (const [result, output, expected] of [
      ['success', 'false', false], ['success', 'true', true],
      ['success', '', true], ['failure', 'false', true], ['failure', '', true],
      ['skipped', '', true],
    ]) {
      const field = id === 'licenses' ? 'licenses_impacting'
        : ['bundle-macos-26', 'other-architectures'].includes(id) ? 'packaging_impacting'
        : 'code_impacting';
      const outputs = { code_impacting: 'false', licenses_impacting: 'false',
        packaging_impacting: 'false', [field]: output };
      const needs = { changes: { result, outputs } };
      assert.equal(evaluate(needs, () => false), expected, `${id}: ${result}/${output}`);
      assert.equal(evaluate(needs, () => true), false, `${id}: cancelled`);
    }
  }
});

test('the Apple silicon job bundles only when packaging inputs changed', () => {
  const job = workflow.split('\n  check-macos:\n')[1].split(/\n  [a-z][\w-]*:\n/)[0];
  const step = job.split(/^      - /m).find(s => /run: cargo xtask bundle/.test(s));
  const expression = step.match(/if: >-\n([\s\S]*?)\n        run:/)?.[1];
  assert.ok(expression, 'the Bundle step must be gated');
  const evaluate = new Function('needs', `return (${expression});`);
  for (const [result, output, expected] of [
    ['success', 'false', false], ['success', 'true', true],
    ['failure', 'false', true], ['skipped', '', true],
  ]) {
    assert.equal(evaluate({ changes: { result, outputs: { packaging_impacting: output } } }),
      expected, `${result}/${output}`);
  }
});

test('the workflow trigger has no path filters', () => {
  // A path filter makes jobs vanish instead of skip, which leaves any
  // required check pending forever.
  assert.doesNotMatch(workflow, /^\s+paths(?:-ignore)?:/m);
});

test('nothing compiled in with include_str! or include_bytes! is on the allowlist', async () => {
  const sources = execFileSync('git', ['ls-files', '*.rs'], { cwd: root, encoding: 'utf8' })
    .split('\n').filter(Boolean);
  const included = [];
  for (const source of sources) {
    const text = readFileSync(join(root, source), 'utf8');
    for (const [call, literal] of text.matchAll(/include_(?:str|bytes)!\(\s*("[^"]+"\s*\))?/g)) {
      // Only a plain literal can be checked here. A concat!/env! or raw
      // string path would slip past, so it fails until rewritten as one.
      assert.ok(literal, `${source}: \`${call}\` needs a plain string literal path`);
      // Paths are relative to the including file, as rustc resolves them.
      const path = relative(root, join(root, dirname(source), literal.slice(1, literal.lastIndexOf('"')))).split('\\').join(posix.sep);
      included.push(path);
    }
  }
  assert.ok(included.includes('packaging/macos/Info.plist.in'), 'the scan found xtask\'s Info.plist');
  for (const path of included) {
    assert.equal((await run({ paths: [path] })).outputs.code_impacting, 'true', path);
  }
});
