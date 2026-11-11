#!/bin/bash
# shoot.sh BIN HOME: open BIN on the contrived home HOME (as fixture.sh made it) and record its window as one
# ProRes take, scene by scene, with a marker per scene. Run it from the
# guest checkout, after building target/drive, target/rec and target/mcp.py.
# Fixture data only.
set -uo pipefail
BIN=$(cd "$(dirname "$1")" && pwd)/$(basename "$1")
S=$PWD/target/lab-clips; mkdir -p "$S"; D=$PWD/target/drive; REC=$PWD/target/rec
H=${2:?usage: shoot.sh BIN HOME}; P=$H/Projects
export HOME=$H PWRDISK_LINK=/tmp/pd-clips.sock PWRDISK=$BIN MARKERS=$S/markers.tsv
rm -f "$PWRDISK_LINK" "$S/stop"; : > "$MARKERS"
log() { echo "[$(date +%T)] $*"; }
mark() { python3 -c 'import time,sys; print(f"{time.time():.3f}\t{sys.argv[1]}")' "$1" >> "$MARKERS"; log "MARK $1"; }

# A 16:9 window as large as the guest's screen allows.
read -r SW SH _ <<< "$($D screen)"
W=$(( SW - 60 )); [ $W -gt 1920 ] && W=1920; WH=$(( W * 9 / 16 ))
if [ $WH -gt $(( SH - 70 )) ]; then WH=$(( SH - 70 )); W=$(( WH * 16 / 9 )); fi
WX=$(( (SW - W) / 2 )); WY=$(( 30 + (SH - 30 - WH) / 2 ))
log "screen ${SW}x${SH}, window ${W}x${WH} at $WX,$WY"

"$BIN" "$H" > "$S/app.log" 2>&1 & pid=$!
wid=; for _ in $(seq 150); do wid=$($D wid $pid | head -1 | cut -d' ' -f1); [ -n "$wid" ] && break; sleep 0.1; done
[ -n "$wid" ] || { log "no window"; kill $pid; exit 1; }
$D front $pid; $D resize $pid $W $WH $WX $WY; sleep 0.5
"$REC" "$wid" "$S/pwrdisk-take.mov" "$S/stop" 400 > "$S/rec.log" 2>&1 & rec=$!
for _ in $(seq 100); do grep -q 'rec: start' "$S/rec.log" && break; sleep 0.1; done
cat "$S/rec.log"
# No recording, no shoot: the steps below remove files.
grep -q 'rec: start' "$S/rec.log" || { log "recorder did not start"; kill $rec $pid; exit 1; }
# Window-relative point, as fractions of the window.
pt() { python3 -c "print(int($WX+$W*$1), int($WY+$WH*$2))"; }
glide() { $D glide $(pt $1 $2) $(pt $3 $4) $5; }
click() { $D click $(pt $1 $2) ${3:-1}; }
key() { $D front $pid; $D key "$@"; }

mark launch-scan; sleep 12
# 1. A scan from the start: Rescan (r). Keys act on the hovered tile, so
# park the pointer on the map first.
$D move $(pt 0.40 0.50); sleep 0.5
mark rescan; key 15; sleep 14; mark rescan-end; sleep 1

# 2. Moving around the finished scan: hover, select, open, arrows, back.
mark nav
glide 0.12 0.35 0.30 0.45 1200; sleep 0.4
glide 0.30 0.45 0.50 0.60 1000; sleep 0.3; click 0.50 0.60; sleep 1.0
glide 0.50 0.60 0.28 0.62 900; sleep 0.3; click 0.28 0.62; sleep 0.9
click 0.28 0.62 2; sleep 2.2
key 124; sleep 0.7; key 124; sleep 0.7; key 125; sleep 0.7; key 123; sleep 0.9
key 51; sleep 1.8
glide 0.28 0.62 0.55 0.35 900; sleep 0.3; click 0.55 0.35 2; sleep 2.2
key 125; sleep 0.7; key 124; sleep 0.9; key 51; sleep 2.0
$D move $(pt 0.98 0.98)
mark nav-end; sleep 1

# 3-6. The agent finds junk and marks it; Review; the removal; Done.
cat > "$S/steps.json" <<JSON
[["mark","agent"],["tool","pwrdisk_status",{}],["sleep",0.6],
 ["tool","pwrdisk_find",{"reclaimable":true,"kind":"dir"}],["sleep",0.6],
 ["tool","pwrdisk_show",{"path":"$P"}],["sleep",2.2],
 ["tool","pwrdisk_mark",{"paths":["$P/atlas/target"]}],["sleep",0.45],
 ["tool","pwrdisk_mark",{"paths":["$P/comet/target"]}],["sleep",0.45],
 ["tool","pwrdisk_mark",{"paths":["$P/beacon/node_modules"]}],["sleep",0.45],
 ["tool","pwrdisk_mark",{"paths":["$P/beacon/.next"]}],["sleep",0.45],
 ["tool","pwrdisk_mark",{"paths":["$P/drift/node_modules"]}],["sleep",0.8],
 ["tool","pwrdisk_show",{"path":"$H/Library/Caches"}],["sleep",2.0],
 ["tool","pwrdisk_mark",{"paths":["$H/Library/Caches/Yarn"]}],["sleep",0.45],
 ["tool","pwrdisk_mark",{"paths":["$H/Library/Caches/Homebrew"]}],["sleep",1.2],
 ["tool","pwrdisk_show",{"path":"$H"}],["sleep",2.5],["mark","agent-end"],
 ["mark","review"],["tool","pwrdisk_review",{}],["sleep",3.0],
 ["sh","$D scroll $(pt 0.40 0.55) -6 45; sleep 1.2; $D scroll $(pt 0.40 0.55) 6 45; $D move $(pt 0.98 0.98)"],["sleep",1.5],
 ["mark","review-end"],
 ["mark","remove"],["remove","permanent"],["sleep",10.0],
 ["mark","done"],["tool","pwrdisk_status",{}],["sleep",6.0],["mark","done-end"]]
JSON
python3 "$PWD/target/mcp.py" "$S/steps.json" > "$S/mcp.log" 2>&1; steps=$?
log "steps exit $steps"
touch "$S/stop"; wait $rec; cat "$S/rec.log"
screencapture -o -x -l "$wid" "$S/last-frame.png" 2>/dev/null
kill $pid; wait $pid 2>/dev/null
log done; exit $steps
