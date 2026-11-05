"""Adapters preserve baseline UI; fixture cleanup runs on success and failure."""

import importlib.util
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

SCRIPTS = Path(__file__).resolve().parent
SOURCE = SCRIPTS.parent.parent
SPEC = importlib.util.spec_from_file_location(
    "install_baseline", SCRIPTS / "install-baseline.py")
ADAPTER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(ADAPTER)


class BaselineTests(unittest.TestCase):
    def baseline(self, root):
        app = root / "crates/disktree-app"
        (app / "src").mkdir(parents=True)
        (app / "Cargo.toml").write_text('''[dependencies]
gpui-kit = { version = "=0.7.3" }
[target.'cfg(target_os = "macos")'.dependencies]
''')
        (app / "src/main.rs").write_text('''#![windows_subsystem = "windows"]
mod state;
fn main() -> Result<()> {
    // First: Setup
    original_startup()
}
''')
        (app / "src/state.rs").write_text('''impl Disktree {
    #[cfg(test)]
    pub fn new() {}
    #[cfg(test)]
    pub fn with_tree() {}
}
''')
        (app / "src/views.rs").write_text("baseline UI must survive\n")
        (root / "Cargo.lock").write_text('''version = 4
[[package]]
name = "disktree-app"
version = "0.1.0"
dependencies = [
 "gpui-kit",
]
[[package]]
name = "image"
version = "0.25.10"
''')
        return app

    def test_transplant_is_idempotent_and_preserves_ui(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            app = self.baseline(root)
            ADAPTER.install(SOURCE, root)
            first = {p: p.read_bytes() for p in root.rglob("*") if p.is_file()}
            ADAPTER.install(SOURCE, root)
            self.assertEqual(first, {p: p.read_bytes() for p in first})
            self.assertEqual((app / "src/views.rs").read_text(),
                             "baseline UI must survive\n")
            self.assertIn("original_startup()", (app / "src/main.rs").read_text())
            self.assertEqual((root / "Cargo.lock").read_text().count(' "image",'), 1)

    def test_changed_anchors_or_gpui_fail_before_writing(self):
        for filename, old, new in (
                ("Cargo.toml", "=0.7.3", "=0.1.0"),
                ("src/state.rs", "pub fn new(", "pub fn renamed(")):
            with self.subTest(filename=filename), tempfile.TemporaryDirectory() as d:
                root = Path(d)
                app = self.baseline(root)
                path = app / filename
                path.write_text(path.read_text().replace(old, new))
                before = {p: p.read_bytes() for p in root.rglob("*") if p.is_file()}
                with self.assertRaises(ValueError):
                    ADAPTER.install(SOURCE, root)
                self.assertEqual(before, {p: p.read_bytes() for p in before})
                self.assertFalse((app / "src/screenshots.rs").exists())


class FixtureTests(unittest.TestCase):
    def test_detaches_after_success_command_failure_and_fill_failure(self):
        if Path("/Volumes/Fixtures").exists():
            self.skipTest("a real Fixtures mount belongs to somebody else")
        for command, fill_status, expected in (("true", 0, 0),
                                                ("false", 0, 1),
                                                ("true", 1, 1)):
            with self.subTest(command=command, fill_status=fill_status), \
                    tempfile.TemporaryDirectory() as d:
                root = Path(d)
                fake = root / "bin"
                fake.mkdir()
                mocks = {
                    "uname": "echo Darwin",
                    "hdiutil": 'echo "$*" >> "$SHOT_TEST_LOG"',
                    "mkdir": ":",
                    "dd": f"exit {fill_status}",
                    "find": ":",
                }
                for name, body in mocks.items():
                    path = fake / name
                    path.write_text(f"#!/bin/bash\n{body}\n")
                    path.chmod(0o755)
                log = root / "calls"
                env = dict(os.environ, PATH=f"{fake}:{os.environ['PATH']}",
                           TMPDIR=str(root), SHOT_TEST_LOG=str(log))
                result = subprocess.run(
                    ["bash", str(SCRIPTS / "fixtures.sh"), command],
                    env=env, capture_output=True, check=False)
                self.assertEqual(result.returncode, expected, result.stderr)
                self.assertTrue(log.read_text().endswith("detach /Volumes/Fixtures\n"))
                self.assertEqual(list(root.glob("pwrdisk-fixtures.*")), [])


class PairTests(unittest.TestCase):
    def test_pair_builds_each_revision_even_with_stale_cargo_cache(self):
        # Model the observed Cargo behavior: path crates of the same version
        # keep the first checkout's code until explicitly invalidated. The
        # binary oracle reports its compiled source, independently of cwd.
        for invalidate in (True, False):
            with self.subTest(invalidate=invalidate), tempfile.TemporaryDirectory() as d:
                root = Path(d)
                repo = root / "repo"
                scripts = repo / "scripts/screenshots"
                scripts.mkdir(parents=True)
                app = repo / "crates/disktree-app"
                app.mkdir(parents=True)
                shutil.copyfile(SOURCE / "crates/disktree-app/Cargo.toml",
                                app / "Cargo.toml")
                shutil.copytree(SOURCE / "crates/disktree-app/src", app / "src")
                template = root / "baseline"
                BaselineTests().baseline(template)
                for filename in ("pair.sh", "render.sh", "install-baseline.py"):
                    text = (SCRIPTS / filename).read_text()
                    if not invalidate:
                        text = "\n".join(line for line in text.splitlines()
                                         if "cargo clean -p" not in line) + "\n"
                    path = scripts / filename
                    path.write_text(text)
                    path.chmod(0o755)
                fake = root / "bin"
                fake.mkdir()
                (fake / "uname").write_text("#!/bin/bash\necho Darwin\n")
                (fake / "git").write_text('''#!/bin/bash
if [[ $1 == worktree && $2 == add ]]; then
    cp -R "$SHOT_TEST_BASELINE" "$4"
elif [[ $1 == worktree && $2 == remove ]]; then
    rm -rf -- "$4"
fi
''')
                (fake / "cargo").write_text('''#!/usr/bin/env python3
import os
from pathlib import Path
import sys
target = Path(os.environ["CARGO_TARGET_DIR"])
target.mkdir(parents=True, exist_ok=True)
cached = target / "cached-source"
if sys.argv[1] == "clean":
    cached.unlink(missing_ok=True)
else:
    if not cached.exists():
        cached.write_text(str(Path.cwd()))
    compiled = cached.read_text()
    binary = target / "debug/pwrdisk"
    binary.parent.mkdir(exist_ok=True)
    binary.write_text("#!/usr/bin/env python3\\nimport os, sys\\n" +
        "from pathlib import Path\\n" +
        "expected = os.environ.get('PWRDISK_SCREENSHOT_CHECKOUT', os.getcwd())\\n" +
        "sys.exit(0 if str(Path(expected).resolve()) == " +
        repr(compiled) + " else 91)\\n")
    binary.chmod(0o755)
''')
                for path in fake.iterdir():
                    path.chmod(0o755)
                env = dict(os.environ, PATH=f"{fake}:{os.environ['PATH']}",
                           SHOT_TEST_BASELINE=str(template),
                           PWRDISK_FIXTURE_ROOT=str(root / "fixtures"),
                           CARGO_TARGET_DIR=str(repo / "target"))
                scenario = root / "scenario.json"
                scenario.write_text("{}")
                result = subprocess.run(
                    ["bash", str(scripts / "pair.sh"), str(scenario),
                     str(root / "out")], env=env, capture_output=True, check=False)
                if invalidate:
                    self.assertEqual(result.returncode, 0, result.stderr)
                else:
                    # Reproduces the pre-fix command: it must fail this oracle.
                    self.assertEqual(result.returncode, 91, result.stderr)
                self.assertEqual(list((repo / "target").glob("screenshot-pair.*")), [])


if __name__ == "__main__":
    unittest.main()
