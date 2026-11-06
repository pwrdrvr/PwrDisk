const {readFileSync} = require('node:fs');
const {join} = require('node:path');
const {test} = require('node:test');
const assert = require('node:assert/strict');

const root = join(__dirname, '../..');
const job = (file, id) => {
  const workflow = readFileSync(join(root, '.github/workflows', file), 'utf8');
  const body = workflow.match(new RegExp(`^  ${id}:\\n([\\s\\S]*?)(?=^  [a-z][\\w-]*:|$(?![\\s\\S]))`, 'm'))?.[1];
  assert.ok(body, `${file}: missing ${id}`);
  return body;
};

const runnerTemp = (body, windowsOnly = false) => {
  const steps = body.split('\n    steps:\n')[1];
  assert.ok(steps);
  const step = steps.match(/^      - name: Use runner temporary storage\n([\s\S]*?)(?=\n      - |$(?![\s\S]))/m)?.[1];
  assert.ok(step);
  assert.doesNotMatch(steps.slice(0, steps.indexOf('      - name: Use runner temporary storage')), /^      - /m);
  assert.match(step, /shell: pwsh/);
  if (windowsOnly) assert.match(step, /if: runner\.os == 'Windows'/);
  for (const name of ['TEMP', 'TMP']) {
    assert.match(step, new RegExp(`^          ${name}: \\$\\{\\{ runner\\.temp \\}\\}$`, 'm'));
    assert.ok(step.includes(`"${name}=$env:${name}" >> $env:GITHUB_ENV`));
  }
};

test('Windows build, test and packaging jobs use runner.temp', () => {
  for (const [file, ids] of [
    ['ci.yml', ['windows']],
    ['release.yml', ['windows-prepare', 'windows-sign']],
    ['preview-build.yml', ['windows-prepare', 'windows-pack']],
  ]) {
    for (const id of ids) runnerTemp(job(file, id));
  }
});

test('the Windows architecture job uses runner.temp', () => {
  const body = job('ci.yml', 'other-architectures');
  assert.match(body, /runner: windows-11-arm/);
  assert.match(body, /run: cargo xtask test/);
  runnerTemp(body, true);
});
