---
name: launch-reel
description: Make a short launch or sizzle video of PwrDisk, with real app footage recorded in the lab VM, a fast voiceover, music, and numbers that match the README. Use when asked for a reel, a demo video, a launch clip or fresh app footage; not for PR screenshots (docs/pr-screenshots.md covers those).
---

# PwrDisk launch reel

A 25–35 s video: real PwrDisk footage on contrived data, a fast narrated
voiceover, a music bed, and captions whose numbers come from measurements.
Three parts, usually in this order: record the footage, write and voice the
script, compose and render.

## Before you start

- **Work files go in `.local/reel/` in the worktree.** `.local/` is
  gitignored and survives across sessions; system temp folders get cleaned
  and have lost a reel's scripts before. Hand the finished video to the
  operator somewhere they can open from Finder, such as the OS videos
  folder, and give the full path.
- **Nothing is posted** anywhere without the operator. No public uploads,
  tweets or PR attachments of the reel unless asked.
- **Contrived data only.** Footage of a real disk shows user, project and
  volume names. Read the "Scans are private data" rule in `AGENTS.md`.
- **Never record on the operator's desktop.** Footage comes from the lab's
  headed macOS VM; see [Testing on a desktop](../../../AGENTS.md#testing-on-a-desktop)
  and [docs/headed-e2e-pwrsuitelab.md](../../../docs/headed-e2e-pwrsuitelab.md).
  Keep lab details out of anything committed.
- **Permissive tools only.** No source-available video frameworks
  (Remotion, HyperFrames and the like). PwrSnap, ffmpeg, a headless browser,
  and your own code are fine.

## 1. Record the footage

`scripts/` holds a working recorder. One lab job builds PwrDisk, makes a
contrived home on a `Fixtures` disk image, opens PwrDisk on it and records
one continuous take of its window, scene by scene, with a marker file.

| script | what it does |
| --- | --- |
| `fixture.sh HOME` | the contrived home: projects with `target/` and `node_modules`, caches, models, movies, a 300k-file archive |
| `job.sh` | the job's whole command: dark mode, the `Fixtures` image, `fixture.sh`, `shoot.sh`, and putting both back on exit |
| `shoot.sh BIN HOME` | launches PwrDisk on HOME, sizes its window to 16:9, records, and drives the scenes |
| `rec.swift` | ScreenCaptureKit → ProRes 422 LT `.mov`, up to 60 fps, cursor shown |
| `drive.swift` | keys, clicks, glides, scrolls and window placement, in the guest only |
| `mcp.py` | one `pwrdisk mcp` session that runs the agent's steps and marks scenes |

The scenes, in order: a rescan running from zero; moving around the
finished scan (hover, select, double-click, arrows, Backspace); an agent
over `pwrdisk mcp` following its finds with `show` and marking paths one by
one; Review with the list scrolled; a permanent removal with rows checking
off; Done with the measured free-space change.

Copy `scripts/` into a throwaway clone of the commit to record (a clean,
committed worktree, as the lab requires), commit it there, and run one
`lab_e2e_run` against it:

- `setup`: install rustup if missing, `cargo build --release --locked -p
  disktree-app`, then `swiftc -O` `drive.swift` and `rec.swift` into
  `target/` and copy `mcp.py` there.
- `command`: `job.sh`. Change the image size or the scenes in the scripts,
  not by retyping the command, so the next take starts from what worked.
- `artifacts`: `target/lab-clips`. The take is about 300 MB for 96 s.

The whole job takes 7–8 minutes. Then collect it and check it before use:
a contact sheet (`ffmpeg -vf fps=1,scale=480:-1,tile=6x17`) and full-size
frames of the scan, removal and Done.

Lessons from the takes that went wrong:

- **Keep the disk image smaller than the guest's free space.** A sparse
  image's free space is capped by the disk under it, so deleting inside an
  oversized image left Done showing a *negative* observed change.
- **Stay under the 5% limit.** The marked paths must total less than 5% of
  the image's capacity, or the agent's marks are refused. `fixture.sh`
  marks about 0.55 GiB against a 14 GiB image.
- **Pace the removal with empty files.** They cost no bytes but each takes
  an unlink: 10k per marked path checks rows off in about 1.5 s in all.
- **Size the scan to its slot.** 300k small files scan in about 2.6 s in
  the guest.
- **Keys act on the hovered tile.** Park the pointer on the map before the
  first key.
- **The recording is variable frame rate.** ScreenCaptureKit sends a frame
  only when the window changes, and stream-copy cuts of it get wrong
  durations. Resample the whole take once (`-vf fps=60 -c:v prores_ks
  -profile:v 1`), then cut that with `-c copy`; ProRes cuts are
  frame-exact.
- **The recorder needs the window server.** A command-line tool must touch
  `NSApplication.shared` and `CGMainDisplayID()` first and keep the main
  run loop turning, or ScreenCaptureKit asserts.
- **`mcp.py` answers the removal confirmation itself.** That is only
  acceptable on the lab fixture. Never use it against real data or a
  window outside the guest.

`shoot.sh` and `mcp.py` exit non-zero, and stop before filming Done, when
the recorder never starts or the removal is refused: a take that removed
nothing is not footage.

Write a `markers.json` beside the take: each scene's in/out seconds, what
happens when, and the facts captions may use. `markers.tsv` holds wall-clock
epochs; subtract the epoch on `rec.log`'s `rec: start` line to get seconds
into the take. The facts are the marked path count, the listed bytes, the
removed count and the observed change from `pwrdisk_status` in `mcp.log`.

## 2. Script and voice

- **Fast and punchy.** About 9 short lines for 30 s. The operator wants a
  high-energy delivery, not a newsreader. OpenAI `gpt-4o-mini-tts` with a
  hype-narrator instruction works. Trim silence from both ends, then speed
  it up about 1.2× (`atempo=1.2`).
- **Spell names the way they are said.** "PwrDisk" → "Power Disk",
  "disktree" → "Disk Tree"; the model reads "disktree" as "DiskFree".
- **Check every take by transcribing it** (`gpt-4o-mini-transcribe`), and
  check the final mix the same way.
- **One continuous take timed to phrases** sounds better than separate
  lines butted together. Fit visuals to the measured phrase boundaries,
  not the other way round.
- Fetch API keys from the operator's secret store at run time and never
  print them. OpenAI's usage policy asks that listeners be told the voice
  is AI-generated, so suggest a short note in the post.

## 3. Compose and render

**Prefer PwrSnap's Sizzle timeline**, version 1.3.0 or later, through its
MCP tools. Its `docs/sizzle-agent-reels.md` is the reference. It covers:
- importing files (`pwrsnap_sizzle_import`) or captures
- an editable timeline (`_get`, `_update`, `_apply_launch_template`) with
  layered images and video, captions, counters, typing and terminal reveals
- narration (`_prepare_narration`, `_align_narration`)
- frame inspection (`_inspect_frames`)
- cached scene renders (`_render_preview`, `_render_full`)
- mixed-audio QA (`_audio_qa`)

It writes the movie under the OS videos folder's `PwrSnap` directory.

Without PwrSnap, the fallback is to hand-build the reel:
- An HTML page with a `window.seek(t)` timeline, stepped frame by frame by
  a headless Chromium and encoded with ffmpeg (libx264 crf 17, AAC,
  `+faststart`).
- Music from your own synthesis code, with no samples.
- The voice ducked under it with `sidechaincompress`, and loudness
  normalized to −14 LUFS.

Either way:

- **Numbers come from measurements.** Benchmark figures come from the
  measured comparison with disktree (the README and `docs/performance/`),
  never from memory, and the README's figure is the one to quote. Demo figures come from the take's `markers.json`, as the window showed
  them, such as "7 of 7 removed · +581 MiB observed". Never present demo
  numbers as the benchmark, or the reverse, and never claim a saving the
  app did not measure.
- **When a number changes**, change the caption, any counter and the voice
  line together, and re-render only the scenes they touch.
- **Check the result**: a contact sheet of the render, a few full-size
  frames, and a transcript of the mixed audio.
