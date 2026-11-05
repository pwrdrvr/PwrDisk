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
Projects/beacon/node_modules,.cache,Photos/Summer,Music,Documents}
# Real allocation, not sparse lengths: the scanner weighs st_blocks * 512.
fill() { dd if=/dev/urandom of="$PWRDISK_FIXTURE_ROOT/$1" \
    bs=1048576 count="$2" 2>/dev/null; }
fill Projects/atlas/src/engine.rs 24
fill Projects/beacon/src/app.ts 12
fill Projects/beacon/node_modules/bundle.js 8
fill .cache/preview.bin 4
fill Photos/Summer/coast.raw 32
fill Music/orbit.flac 16
fill Documents/field-notes.pdf 8
# A rebuild between captures must not turn "just now" into "1 minute ago".
find "$PWRDISK_FIXTURE_ROOT" -exec touch -t 202401011200 {} +
"$@"
