#!/bin/bash
# Worth a look under Private: invented agent worktrees that are APFS
# clones of a pack kept elsewhere, 66 MiB allocated and nothing alone,
# beside a cache that holds 66 MiB alone. Before and after of the explore
# screen, Allocated and then Private.
set -euo pipefail
[[ $# == 1 ]] || {
    echo "usage: $0 OUTPUT_DIR" >&2
    exit 1
}
scripts=$(cd "$(dirname "$0")" && pwd)
if [[ -z ${PWRDISK_FIXTURE_ROOT:-} ]]; then
    exec "$scripts/fixtures.sh" bash "$0" "$@"
fi
[[ $PWRDISK_FIXTURE_ROOT == /Volumes/Fixtures/Demo ]] || {
    echo 'Private-size pictures require the contrived Fixtures image.' >&2
    exit 1
}
root=$PWRDISK_FIXTURE_ROOT
# Real allocated bytes: a sparse file has no blocks to clone. 33 MiB twice
# over clears Worth a look's 64 MiB floor by allocation alone, and the
# image has room for the one physical copy.
dd if=/dev/urandom of="$root/Projects/atlas/vendor.pack" bs=1048576 \
    count=33 2>/dev/null
for name in feature-a feature-b; do
    mkdir -p "$root/.codex/worktrees/$name"
    # `cp -c` asks APFS for a clone: the copy shares every block.
    cp -c "$root/Projects/atlas/vendor.pack" \
        "$root/.codex/worktrees/$name/vendor.pack"
done
# 62 more than the fixture's 4: the cache holds 66 MiB alone.
dd if=/dev/urandom of="$root/.cache/models.bin" bs=1048576 count=62 \
    2>/dev/null
find "$root" -exec touch -t 202401011200 {} +
"$scripts/pair.sh" "$scripts/private-insights.json" "$1"
