#!/bin/bash
# Real fixture bytes, logical system paths; never scan the host's root.
set -euo pipefail
[[ $# == 1 ]] || { echo "usage: $0 OUTPUT_DIR" >&2; exit 1; }
scripts=$(cd "$(dirname "$0")" && pwd)
if [[ -z ${PWRDISK_FIXTURE_ROOT:-} ]]; then
    exec "$scripts/fixtures.sh" bash "$0" "$@"
fi
[[ $PWRDISK_FIXTURE_ROOT == /Volumes/Fixtures/Demo ]] || {
    echo 'Protected-folder pictures require the contrived Fixtures image.' >&2
    exit 1
}
mkdir -p "$PWRDISK_FIXTURE_ROOT/Applications/"{PwrDraw.app,PwrNotes.app}
dd if=/dev/zero of="$PWRDISK_FIXTURE_ROOT/Applications/PwrDraw.app/data" \
    bs=1048576 count=8 2>/dev/null
dd if=/dev/zero of="$PWRDISK_FIXTURE_ROOT/Applications/PwrNotes.app/data" \
    bs=1048576 count=4 2>/dev/null
find "$PWRDISK_FIXTURE_ROOT/Applications" -exec touch -t 202401011200 {} +
exec "$scripts/pair.sh" "$scripts/protected-folders.json" "$1"
