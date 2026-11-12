#!/bin/bash
# Private sizes: three invented worktrees that are APFS clones of one
# another, two with a little data of their own, beside the usual fixture.
# Before and after of the explore screen, then the Sizes menu and private
# weights, which only the after build has.
set -euo pipefail
[[ $# == 1 ]] || { echo "usage: $0 OUTPUT_DIR" >&2; exit 1; }
scripts=$(cd "$(dirname "$0")" && pwd)
if [[ -z ${PWRDISK_FIXTURE_ROOT:-} ]]; then
    exec "$scripts/fixtures.sh" bash "$0" "$@"
fi
[[ $PWRDISK_FIXTURE_ROOT == /Volumes/Fixtures/Demo ]] || {
    echo 'Private-size pictures require the contrived Fixtures image.' >&2
    exit 1
}
trees="$PWRDISK_FIXTURE_ROOT/Worktrees"
mkdir -p "$trees/main"
# Real allocated bytes: a sparse file has no blocks to clone.
dd if=/dev/urandom of="$trees/main/vendor.pack" bs=1048576 count=40 \
    2>/dev/null
for name in feature-a feature-b; do
    mkdir -p "$trees/$name"
    # `cp -c` asks APFS for a clone: the copy shares every block.
    cp -c "$trees/main/vendor.pack" "$trees/$name/vendor.pack"
done
# What each feature worktree holds alone.
dd if=/dev/urandom of="$trees/feature-a/build.out" bs=1048576 count=12 \
    2>/dev/null
dd if=/dev/urandom of="$trees/feature-b/build.out" bs=1048576 count=3 \
    2>/dev/null
find "$trees" -exec touch -t 202401011200 {} +
"$scripts/pair.sh" "$scripts/private-sizes.json" "$1"
"$scripts/render.sh" "$scripts/private-sizes-menu.json" "$1/after"
