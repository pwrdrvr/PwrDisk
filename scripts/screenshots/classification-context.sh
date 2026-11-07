#!/bin/bash
# Compare descriptive labels and a narrow scan using only owned fixtures.
set -euo pipefail
[[ $# -ge 1 && $# -le 2 ]] || {
    echo "usage: $0 OUTPUT_DIR [BEFORE_REF=origin/main]" >&2; exit 1
}
scripts=$(cd "$(dirname "$0")" && pwd)
if [[ -z ${PWRDISK_FIXTURE_ROOT:-} ]]; then
    exec "$scripts/fixtures.sh" bash "$0" "$@"
fi
[[ $PWRDISK_FIXTURE_ROOT == /Volumes/Fixtures/Demo ]] || {
    echo 'Classification pictures require the contrived Fixtures image.' >&2
    exit 1
}
for relative in .cache/DiagOutputDir .Trash/kept .cache/tmp/session \
    private/tmp/claude-501 .claude/experiments; do
    path="$PWRDISK_FIXTURE_ROOT/$relative"
    mkdir -p "$path"
    dd if=/dev/zero of="$path/notes" bs=1024 count=128 2>/dev/null
done
# Git history clears Trash's cleanup hint; the badge must still say trash.
gitdir="$PWRDISK_FIXTURE_ROOT/.Trash/kept/.git"
mkdir -p "$gitdir/objects" "$gitdir/refs"
printf 'ref: refs/heads/main\n' > "$gitdir/HEAD"
for relative in .cache .Trash private .claude; do
    find "$PWRDISK_FIXTURE_ROOT/$relative" -exec touch -t 202401011200 {} +
done
before=${2:-origin/main}
"$scripts/pair.sh" "$scripts/classification-context.json" "$1/broad" "$before"
PWRDISK_FIXTURE_ROOT="$PWRDISK_FIXTURE_ROOT/private/tmp" \
    "$scripts/pair.sh" "$scripts/narrow-temporary.json" "$1/narrow" "$before"
