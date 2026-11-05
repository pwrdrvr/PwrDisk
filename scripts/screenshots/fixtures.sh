#!/bin/bash
# Run a command with an invented, allocated tree on a hidden APFS image.
set -euo pipefail
[[ $(uname -s) == Darwin ]] || {
    echo 'Offscreen screenshots require macOS.' >&2; exit 1;
}
[[ $# -gt 0 ]] || { echo "usage: $0 COMMAND [ARG...]" >&2; exit 1; }
mountpoint=/Volumes/Fixtures
[[ ! -e $mountpoint ]] || {
    echo "$mountpoint already exists; leaving it alone." >&2; exit 1;
}
scratch=$(mktemp -d "${TMPDIR:-/tmp}/pwrdisk-fixtures.XXXXXX")
mounted=0
cleanup() {
    status=$?
    trap - EXIT
    if [[ $mounted == 1 ]]; then
        if ! hdiutil detach "$mountpoint"; then
            echo "Detach failed: $mountpoint; image kept at $scratch" >&2
            exit 1
        fi
    fi
    rm -rf -- "$scratch"
    exit "$status"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
hdiutil create -ov -size 256m -fs APFS -volname Fixtures \
    "$scratch/fixtures.dmg"
hdiutil attach -nobrowse -mountpoint "$mountpoint" "$scratch/fixtures.dmg"
mounted=1
export PWRDISK_FIXTURE_ROOT="$mountpoint/Demo"
mkdir -p "$PWRDISK_FIXTURE_ROOT"/{Projects/atlas/src,Projects/beacon/src,\
Projects/beacon/node_modules,.cache,Photos/Summer,Music,Documents,\
Projects/atlas/target/debug/deps,Projects/atlas/target/debug/incremental}
# Real allocation, not sparse lengths: the scanner weighs st_blocks * 512.
fill() { dd if=/dev/urandom of="$PWRDISK_FIXTURE_ROOT/$1" \
    bs=1048576 count="$2" 2>/dev/null; }
fill Projects/atlas/src/engine.rs 24
fill Projects/beacon/src/app.ts 12
# A Cargo build, with the cache tag Cargo writes into every target.
printf '%s\n' 'Signature: 8a477f597d28d172789f06886806bc55' \
    '# This file is a cache directory tag created by cargo.' |
    dd of="$PWRDISK_FIXTURE_ROOT/Projects/atlas/target/CACHEDIR.TAG" \
    2>/dev/null
fill Projects/atlas/target/debug/deps/libatlas.rlib 6
fill Projects/atlas/target/debug/incremental/atlas.bin 4
fill Projects/beacon/node_modules/bundle.js 8
fill .cache/preview.bin 4
fill Photos/Summer/coast.raw 32
fill Music/orbit.flac 16
fill Documents/field-notes.pdf 8
# A rebuild between captures must not turn "just now" into "1 minute ago".
find "$PWRDISK_FIXTURE_ROOT" -exec touch -t 202401011200 {} +
"$@"
