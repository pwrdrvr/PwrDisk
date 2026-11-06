#!/bin/bash
# Compare Review over a relocated npm cache and an unrelated folder.
set -euo pipefail
[[ $# == 1 ]] || { echo "usage: $0 OUTPUT_DIR" >&2; exit 1; }
scripts=$(cd "$(dirname "$0")" && pwd)
if [[ -z ${PWRDISK_FIXTURE_ROOT:-} ]]; then
    exec "$scripts/fixtures.sh" "$0" "$@"
fi
[[ $PWRDISK_FIXTURE_ROOT == /Volumes/Fixtures/Demo ]] || {
    echo 'Relocated cache pictures require the contrived Fixtures image.' >&2
    exit 1
}
cache="$PWRDISK_FIXTURE_ROOT/Projects/relay/vendor/_cacache/index-v5"
mkdir -p "$cache"
dd if=/dev/zero of="$cache/archive" bs=1024 count=128 2>/dev/null
find "$PWRDISK_FIXTURE_ROOT/Projects/relay" -exec touch -t 202401011200 {} +
exec "$scripts/pair.sh" "$scripts/relocated-cache.json" "$1"
