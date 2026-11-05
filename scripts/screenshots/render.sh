#!/bin/bash
# Build and run only the explicit offscreen entry point, never cargo run.
set -euo pipefail
[[ $(uname -s) == Darwin ]] || {
    echo 'Offscreen screenshots require macOS.' >&2; exit 1;
}
[[ $# == 2 && -n ${PWRDISK_FIXTURE_ROOT:-} ]] || {
    echo "usage: fixtures.sh $0 SCENARIO.json OUTPUT_DIR" >&2; exit 1;
}
repo=$(cd "$(dirname "$0")/../.." && pwd)
scenario=$(cd "$(dirname "$1")" && pwd)/$(basename "$1")
mkdir -p "$2"
out=$(cd "$2" && pwd)
cd "$repo"
export CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-$repo/target}
cargo build --locked -p disktree-app --features screenshots
"$CARGO_TARGET_DIR/debug/pwrdisk" offscreen-screenshots \
    "$scenario" "$PWRDISK_FIXTURE_ROOT" "$out"
