#!/bin/bash
# One image, one scenario, two revisions. The baseline gets only the harness.
set -euo pipefail
[[ $(uname -s) == Darwin ]] || {
    echo 'Offscreen screenshots require macOS.' >&2; exit 1;
}
[[ $# -ge 2 && $# -le 3 ]] || {
    echo "usage: $0 SCENARIO.json OUTPUT_DIR [BEFORE_REF=origin/main]" >&2
    exit 1
}
scripts=$(cd "$(dirname "$0")" && pwd)
if [[ -z ${PWRDISK_FIXTURE_ROOT:-} ]]; then
    exec "$scripts/fixtures.sh" "$0" "$@"
fi
repo=$(cd "$scripts/../.." && pwd)
scenario=$(cd "$(dirname "$1")" && pwd)/$(basename "$1")
mkdir -p "$2"
out=$(cd "$2" && pwd)
before=${3:-origin/main}
cd "$repo"
export CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-$repo/target}
mkdir -p "$repo/target"
scratch=$(mktemp -d "$repo/target/screenshot-pair.XXXXXX")
worktree="$scratch/before"
cleanup() {
    status=$?
    trap - EXIT
    if [[ -d $worktree ]]; then
        # Only this script's disposable baseline, never an existing worktree.
        git worktree remove --force "$worktree" || exit 1
    fi
    rmdir "$scratch"
    exit "$status"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
git fetch origin main
git worktree add --detach "$worktree" "$before"
python3 "$scripts/install-baseline.py" "$repo" "$worktree"
mkdir -p "$out/before" "$out/after"
(
    cd "$worktree"
    cargo build --locked -p disktree-app --features screenshots
    "$CARGO_TARGET_DIR/debug/pwrdisk" offscreen-screenshots \
        "$scenario" "$PWRDISK_FIXTURE_ROOT" "$out/before"
)
# Build after last, so the shared target's binary belongs to this workspace.
"$scripts/render.sh" "$scenario" "$out/after"
