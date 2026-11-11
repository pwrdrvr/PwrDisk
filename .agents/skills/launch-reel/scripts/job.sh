#!/bin/bash
# job.sh: the lab job's command, run from the guest checkout after setup has
# built target/release/pwrdisk, target/drive, target/rec and target/mcp.py.
# Dark mode, a contrived home on a Fixtures image, one recorded take; dark
# mode and the image are put back on any exit. Guest only.
set -uo pipefail
here=$(cd "$(dirname "$0")" && pwd)
dark=$(osascript -e 'tell application "System Events" to tell appearance preferences to get dark mode')
img=$PWD/target/fixtures.sparseimage
cleanup() {
  osascript -e "tell application \"System Events\" to tell appearance preferences to set dark mode to $dark"
  hdiutil detach /Volumes/Fixtures -force >/dev/null 2>&1
  rm -f "$img"
}
trap cleanup EXIT
osascript -e 'tell application "System Events" to tell appearance preferences to set dark mode to true'
# 14g, not more: a sparse image's free space is capped by the guest disk
# under it, and an oversized one shows a negative change on Done. The marks
# (about 0.55 GiB) must stay under 5% of it.
hdiutil create -size 14g -type SPARSE -fs APFS -volname Fixtures -attach "$PWD/target/fixtures" || exit 1
home=/Volumes/Fixtures/Users/alex
"$here/fixture.sh" "$home" || exit 1
"$here/shoot.sh" target/release/pwrdisk "$home"
