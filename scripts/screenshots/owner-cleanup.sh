#!/bin/bash
# Compare inspection of pnpm's separate metadata/dlx cache, never a real one.
set -euo pipefail
[[ $# == 1 ]] || { echo "usage: $0 OUTPUT_DIR" >&2; exit 1; }
scripts=$(cd "$(dirname "$0")" && pwd)
if [[ -z ${PWRDISK_FIXTURE_ROOT:-} ]]; then
    exec "$scripts/fixtures.sh" "$0" "$@"
fi
[[ $PWRDISK_FIXTURE_ROOT == /Volumes/Fixtures/Demo ]] || {
    echo 'Owner cleanup pictures require the contrived Fixtures image.' >&2
    exit 1
}
mkdir -p "$PWRDISK_FIXTURE_ROOT/.cache/pnpm/dlx/demo"
dd if=/dev/zero of="$PWRDISK_FIXTURE_ROOT/.cache/pnpm/dlx/demo/archive" \
    bs=1024 count=128 2>/dev/null
find "$PWRDISK_FIXTURE_ROOT/.cache/pnpm" -exec touch -t 202401011200 {} +
export XDG_CACHE_HOME="$PWRDISK_FIXTURE_ROOT/.cache"
exec "$scripts/pair.sh" "$scripts/owner-cleanup.json" "$1"
