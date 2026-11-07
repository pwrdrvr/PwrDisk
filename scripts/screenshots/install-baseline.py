#!/usr/bin/env python3
"""Transplant only the screenshot adapter into a disposable baseline.

Fail on changed anchors or GPUI versions; never copy application UI code.
All edits are prepared before any baseline file is written.
"""

import json
import re
import shutil
import subprocess
import sys
from pathlib import Path


def once(text, old, new):
    if text.count(old) != 1:
        raise ValueError(f"baseline adapter anchor changed: {old!r}")
    return text.replace(old, new, 1)


def read_manifest(path):
    # Cargo is already required to build either revision. Its JSON avoids a
    # second TOML parser and works with macOS's bundled Python 3.9.
    result = subprocess.run([
        "cargo", "read-manifest", "--manifest-path", str(path),
    ], check=True, capture_output=True, text=True)
    return json.loads(result.stdout)


def gpui_version(manifest):
    versions = [dep["req"] for dep in manifest["dependencies"]
                if dep["name"] == "gpui-kit" and dep["kind"] is None
                and dep["target"] is None]
    if len(versions) != 1:
        raise ValueError("expected one normal gpui-kit dependency")
    return versions[0]


def install(source, target):
    app = Path("crates/disktree-app")
    manifest = (target / app / "Cargo.toml").read_text()
    source_manifest = read_manifest(source / app / "Cargo.toml")
    parsed = read_manifest(target / app / "Cargo.toml")
    if gpui_version(parsed) != gpui_version(source_manifest):
        raise ValueError("GPUI versions differ; adapt the baseline manually")
    if "screenshots" not in parsed.get("features", {}):
        if "[features]" in manifest:
            manifest = once(manifest, "[features]", "[features]\n" +
                            'screenshots = ["gpui-kit/test-support", "dep:image"]')
        else:
            manifest += ('\n[features]\n'
                         'screenshots = ["gpui-kit/test-support", "dep:image"]\n')
        manifest = once(
            manifest, "[target.'cfg(target_os = \"macos\")'.dependencies]",
            "[target.'cfg(target_os = \"macos\")'.dependencies]\n" +
            'image = { version = "0.25", default-features = false, '
            'features = ["png"], optional = true }')
    main = (target / app / "src/main.rs").read_text()
    if "mod screenshots;" not in main:
        source_main = (source / app / "src/main.rs").read_text()
        start = source_main.index("    // Must precede every normal startup")
        end = source_main.index("    // First: Setup", start)
        main = once(main, "mod state;", 'mod state;\n'
                    '#[cfg(any(test, feature = "screenshots"))]\nmod screenshots;')
        main = once(main, "fn main() -> Result<()> {\n",
                    "fn main() -> Result<()> {\n" + source_main[start:end])
        main = once(main, '#![windows_subsystem = "windows"]',
                    '#![windows_subsystem = "windows"]\n\n'
                    '#[cfg(all(feature = "screenshots", not(debug_assertions)))]\n'
                    'compile_error!("screenshots require a debug build");')
    state = (target / app / "src/state.rs").read_text()
    # The removal fixture reports success in memory, without deleting files.
    # Expose the baseline's own accounting unchanged to the copied renderer.
    state = state.replace("    fn note_removed(",
                          "    pub(crate) fn note_removed(", 1)
    for name in ("new", "with_tree"):
        old = f"    #[cfg(test)]\n    pub fn {name}("
        new = ('    #[cfg(any(test, all(feature = "screenshots", '
               'target_os = "macos")))]\n' + f"    pub fn {name}(")
        if new not in state:
            state = once(state, old, new)
    lock = (target / "Cargo.lock").read_text()
    # These packages already exist transitively; change only this crate's
    # dependency edges, preserving the baseline's locked package versions.
    if len(re.findall(r'\nname = "image"\n', lock)) != 1:
        raise ValueError("baseline must have exactly one locked image version")
    pattern = r'(name = "disktree-app"\n.*?dependencies = \[\n)(.*?)(\n\])'
    match = re.search(pattern, lock, re.DOTALL)
    if match is None:
        raise ValueError("baseline lockfile app entry missing")
    deps = sorted(set(match[2].splitlines()) | {' "image",'})
    lock = lock[:match.start(2)] + "\n".join(deps) + lock[match.end(2):]
    for path, content in ((app / "Cargo.toml", manifest),
                          (app / "src/main.rs", main),
                          (app / "src/state.rs", state),
                          (Path("Cargo.lock"), lock)):
        (target / path).write_text(content)
    shutil.copyfile(source / app / "src/screenshots.rs",
                    target / app / "src/screenshots.rs")
    shutil.copytree(source / app / "src/screenshots",
                    target / app / "src/screenshots", dirs_exist_ok=True)


if __name__ == "__main__":
    install(Path(sys.argv[1]), Path(sys.argv[2]))
