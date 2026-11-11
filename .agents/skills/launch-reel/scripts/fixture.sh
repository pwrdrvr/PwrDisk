#!/bin/bash
# fixture.sh HOME_DIR: a contrived home folder on the Fixtures image. Real
# allocated bytes (mkfile without -n), since sizes come from blocks. The
# marked paths add up to about 0.55 GiB, under the 5% removal limit of a
# 14 GiB image.
set -euo pipefail
H=$1
f() { mkdir -p "$(dirname "$1")"; mkfile "$2" "$1"; }
many() { # dir count minK maxK ext
  mkdir -p "$1"; for i in $(seq 1 "$2"); do mkfile "$(( $3 + (RANDOM % ($4 - $3 + 1)) ))k" "$1/f$i.$5"; done; }
P=$H/Projects
f $P/atlas/target/release/atlas 80m; f $P/atlas/target/debug/incremental/a.bin 50m; f $P/atlas/target/debug/deps/libserde.rlib 20m
many $P/atlas/src 40 4 40 rs
f $P/comet/target/release/comet 60m; f $P/comet/target/debug/build.bin 50m; many $P/comet/src 30 4 30 rs
for m in react next webpack typescript esbuild lodash @swc @babel sharp prisma; do many "$P/beacon/node_modules/$m" 6 400 2400 js; done
f $P/beacon/.next/cache/webpack.pack 40m; many $P/beacon/app 25 4 20 tsx
for m in vite vitest rollup eslint; do many "$P/drift/node_modules/$m" 5 600 2600 js; done
many $P/drift/src 20 4 20 ts
L=$H/Library
f $L/Caches/Homebrew/downloads/llvm.bottle.tar.gz 90m; f $L/Caches/Yarn/v6/cache.bin 60m
f "$L/Caches/com.apple.Safari/WebKitCache" 30m
f "$L/Application Support/Slack/IndexedDB.bin" 90m; f "$L/Developer/Xcode/DerivedData/Atlas/Build.bin" 260m
f $H/Models/llama-3.1-8b-q4.gguf 1500m; f $H/Models/whisper-large-v3.bin 600m
f $H/Movies/keynote-rehearsal.mov 700m; f $H/Movies/drone/flyover.mp4 400m
for y in 2024 2025; do many $H/Pictures/$y 120 1200 3500 heic; done
f $H/Downloads/Xcode_26.xip 900m; many $H/Downloads/misc 15 100 4000 pdf
many $H/Documents/notes 40 4 200 md; f $H/Documents/taxes-2025.pdf 12m
python3 - "$H" <<'PY'
import os, sys
H = sys.argv[1]
# Empty files inside what the reel removes: they cost no bytes, so the
# marks stay under the 5% limit, but each takes an unlink, so the removal
# lasts long enough to watch its rows check off (about 1.5 s at 10k each).
for rel in ["Projects/atlas/target/debug/.fingerprint", "Projects/comet/target/debug/deps",
            "Projects/beacon/node_modules/.cache", "Projects/beacon/.next/cache/swc",
            "Projects/drift/node_modules/.vite", "Library/Caches/Yarn/v6/npm",
            "Library/Caches/Homebrew/api"]:
    d = os.path.join(H, rel); os.makedirs(d, exist_ok=True)
    for i in range(10000):
        os.close(os.open(os.path.join(d, f"f{i:05d}"), os.O_CREAT | os.O_WRONLY, 0o644))
# An archive of small files, so a scan runs long enough to watch: 300k
# files scan in about 2.6 s in the guest.
for a in range(300):
    d = os.path.join(H, "Archive", f"mail-{2010 + a % 15}", f"box{a:03d}"); os.makedirs(d, exist_ok=True)
    for i in range(1000):
        fd = os.open(os.path.join(d, f"m{i:04d}.eml"), os.O_CREAT | os.O_WRONLY, 0o644); os.write(fd, b"x"); os.close(fd)
PY
