#!/bin/bash
# Compare inherited labels using only invented temporary and agent work.
set -euo pipefail
[[ $# == 1 ]] || { echo "usage: $0 OUTPUT_DIR" >&2; exit 1; }
scripts=$(cd "$(dirname "$0")" && pwd)
if [[ -z ${PWRDISK_FIXTURE_ROOT:-} ]]; then
    exec "$scripts/fixtures.sh" bash "$0" "$@"
fi
[[ $PWRDISK_FIXTURE_ROOT == /Volumes/Fixtures/Demo ]] || {
    echo 'Temporary-label pictures require the contrived Fixtures image.' >&2
    exit 1
}
for relative in private/tmp/claude-501 private/tmp/session-demo \
    .claude/experiments; do
    path="$PWRDISK_FIXTURE_ROOT/$relative"
    # Git history clears the ancestor's reclaim hint. Review then shows
    # the inherited category, the path that exposed the misleading badge.
    mkdir -p "$path/.git/objects" "$path/.git/refs"
    printf 'ref: refs/heads/main\n' > "$path/.git/HEAD"
    dd if=/dev/zero of="$path/notes" bs=1024 count=128 2>/dev/null
    find "$path" -exec touch -t 202401011200 {} +
done
exec "$scripts/pair.sh" "$scripts/temporary-labels.json" "$1"
