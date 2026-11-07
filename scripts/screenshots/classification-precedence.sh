#!/bin/bash
# Compare narrow-root labels on owned, invented fixture data only.
set -euo pipefail
[[ $# -ge 1 && $# -le 2 ]] || {
    echo "usage: $0 OUTPUT_DIR [BEFORE_REF=origin/main]" >&2; exit 1
}
scripts=$(cd "$(dirname "$0")" && pwd)
if [[ -z ${PWRDISK_FIXTURE_ROOT:-} ]]; then
    exec "$scripts/fixtures.sh" bash "$0" "$@"
fi
[[ $PWRDISK_FIXTURE_ROOT == /Volumes/Fixtures/Demo ]] || {
    echo 'Precedence pictures require the contrived Fixtures image.' >&2
    exit 1
}
for relative in .cache/pip/tmp repo/.git/.fseventsd; do
    path="$PWRDISK_FIXTURE_ROOT/$relative/session"
    mkdir -p "$path"
    dd if=/dev/zero of="$path/notes" bs=1024 count=128 2>/dev/null
done
gitdir="$PWRDISK_FIXTURE_ROOT/repo/.git"
mkdir -p "$gitdir/objects" "$gitdir/refs"
printf 'ref: refs/heads/main\n' > "$gitdir/HEAD"
for relative in .cache repo; do
    find "$PWRDISK_FIXTURE_ROOT/$relative" -exec touch -t 202401011200 {} +
done
before=${2:-origin/main}
PWRDISK_FIXTURE_ROOT="$PWRDISK_FIXTURE_ROOT/.cache/pip/tmp" \
    "$scripts/pair.sh" "$scripts/narrow-owner.json" "$1/temporary" "$before"
PWRDISK_FIXTURE_ROOT="$PWRDISK_FIXTURE_ROOT/repo/.git/.fseventsd" \
    "$scripts/pair.sh" "$scripts/narrow-protected-owner.json" "$1/git" "$before"
