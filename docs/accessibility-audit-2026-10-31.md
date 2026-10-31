# PwrDisk accessibility audit — 2026-10-31

Tracking: [PWR-27](https://linear.app/pwrdrvr/issue/PWR-27).

## Result and scope

**PwrDisk's core graphical workflow is not independently usable through
the inspected macOS accessibility tree.** Controls from GPUI Base expose
some useful semantics, but the file tree, selected-path details, marked
list, document text and much of the application state are missing. This
audit records failures and explicit validation gaps; it is not a claim of
WCAG conformance or a successful three-platform screen-reader test.

The audited production source is **v0.14.0,
`4d68f6cb3ca6742529faf3f4436278da11d486d2`**. The audit PR adds only this
report, a fixture generator, test-only contrast measurements and keyboard
regressions. It does not change the production UI or port upstream UI.
Re-run against a new commit after remediation; do not carry these outcomes
forward as proof about a later build.

Repository guidance and the full PWR-27 description were read first. Its
comments were empty. The earlier history investigation, PwrRoute audits
and PwrDisk PR #26's cleanup-advice inspection are not accessibility
evidence for this audit.

## Environments and evidence limits

| Environment | Build and tools | Result and limitation |
| --- | --- | --- |
| macOS | 27.0.1, build 26A434, Apple Silicon; local `cargo build --locked -p disktree-app`; debug v0.14.0; native CUA AX inspection and keyboard input | Native AX and selected keyboard/visual scenarios tested. The isolated `/tmp/PwrDisk-A11y.app` launcher executes this worktree's binary. An already-running installed PwrDisk was inspected only to identify it, then left untouched; its tree is not baseline evidence. |
| VoiceOver | System VoiceOver app; version not obtained | **BLOCKED.** Cmd-F5 did not produce a running VoiceOver. Direct launch through computer use timed out with `-10005: timeoutReached`; subsequent app inventory reported `isRunning: false`. No spoken output, rotor, speech cadence or VoiceOver navigation result is claimed. |
| Windows | Discovered WindowsVM peer: Windows 11 build 26200, x64 | **BLOCKED.** Peer was disconnected. No audited Windows binary, UIA snapshot, Narrator/NVDA version or runtime result. A toolkit Windows adapter in source is not Windows verification. |
| Linux | Dedicated Sandboxer `sb_ypcbvzmzum`, Ubuntu 24.04.5 LTS, x86_64, kernel 6.18.56-sandboxer; base `pwrdisk-base-us-east`, us-east | Build, lint, harness tests and isolated SSH integration ran at the exact baseline commit. These are not native AT tests. |
| Orca / AT-SPI | Same Linux VM | **BLOCKED.** `DISPLAY` and `WAYLAND_DISPLAY` unset, no X11 socket, `command -v orca` empty, no installed Orca/pyatspi version. Xvfb and dbus-run-session exist, but there is no usable desktop/AT session in the available controller. Neither X11 nor Wayland screen-reader behavior was verified. |

Native AX is direct semantic evidence, not a substitute for speech testing.
Harness focus is GPUI focus, not VoiceOver focus. The controller consistently
reported the native window as AX focused even after application keyboard
selection changed. A missing AX focused item is a finding; it does not prove
that no internal focus handle exists.

Live inspection used a 2880×1800 capture (1440×900 logical surface), default
dark appearance, 100% UI zoom and the supported maximum 175% UI zoom.
Light appearance and small/zoomed layouts have harness/source coverage as
specified below. OS display/text scaling and OS high-contrast modes were
not changed or verified. Screenshots include the real volume meter even
with a contrived scan, so live captures were not published as wholly
contrived evidence. No scan of the user's home was initiated.

## Reproduce

Use a fresh worktree/destination, with no real data in the fixture. Python
3 is required. The generator refuses an existing root and never removes
or overwrites an existing tree.

```sh
git checkout --detach 4d68f6cb3ca6742529faf3f4436278da11d486d2
# Use scripts/a11y-fixture.py from the audit PR with the baseline build.
python3 scripts/a11y-fixture.py /tmp/pwrdisk-a11y-new
cargo build --locked -p disktree-app
target/debug/pwrdisk --apparent-size
```

On Windows choose an unused temporary path and run
`target\debug\pwrdisk.exe --apparent-size`. On macOS `/tmp` resolves to
`/private/tmp`; compare canonical paths. The default fixture has 135 files,
including hidden data, an empty file/directory, nested cache data, 128 small
files, a long name and `Résumé 日本語.txt`. Payloads and timestamps are fixed;
directory timestamps and allocated block sizes are not. Use apparent size
for comparable figures. The live root was `/tmp/pwrdisk-a11y-20261031`.
Use `--entries 50000` in a separate fresh fixture for a longer scan; the
default fixture completes too quickly for a live progress announcement
test. Permission-denied and successful SSH discovery require separate
controlled fixtures; do not infer their AT behavior from this local scan.

For each native run record the commit, binary/build profile, OS/AT version,
theme, logical dimensions, display scale and UI/text scale. Inspect with
VoiceOver/native AX, Narrator or NVDA/UIA, and Orca/AT-SPI respectively:

1. Launch without a path. Traverse forward/backward with Tab/Shift-Tab;
   inspect host/path field names, values, root rows, selected source,
   disabled states and instructions. Try arrows and Escape. Reopen with
   `v`. Enter `-audit-invalid` in the host field and Return: validation
   must occur without an SSH connection. Compare the visible error with
   the accessibility tree and announcements.
2. Enter only the fresh fixture path in the path input and Return. In the
   observed build this required a pointer click to seed field focus;
   keyboard-only startup did not provide a working Tab route. Observe
   scan progress, indeterminate/busy state, Escape cancellation and retry
   using a larger fixture. Read progress/error/completion without a stream
   of sampled-path announcements.
3. Explore with Tab/Shift-Tab, arrows, Enter and Backspace; filter with `/`.
   Compare names, sizes, hierarchy, current root, selection and active
   descendant with the visual result. Mark using Space, review with `c`,
   change modes with `m`/`p`, return with Escape. Check marked/blocked
   reasons and free-space projections. **Do not press Enter or an approval
   key in live Review; do not activate its removal control.**
4. Open Help with `?`, inspect all shortcuts and modal boundaries, try
   background navigation keys, dismiss with Escape. Open Settings with
   Cmd+, on macOS (Ctrl+, elsewhere); inspect every pane and each copy
   action. Open Changelog, License, Third-Party Notices and Logs. Read,
   search, select, scroll and copy without a pointer. Close auxiliary
   windows and check focus restoration. Do not change release channels or
   removal limits as part of reading Settings.
5. Check native menus on macOS; on Windows/Linux test F10/Alt, arrows,
   activation and Escape. Check disabled items, submenu order and focus
   return. Repeat relevant screens in light/dark/high-contrast appearance,
   at minimum window sizes and large UI/OS text/display scale, with long
   paths, and with reduced motion enabled.

Removal confirmation and destructive outcomes are **harness-only** here.
Existing tests operate on temporary trees and verify unmarked neighbors.
Any future interactive destructive test needs a human operator; never use
computer use, AX, UIA or AT-SPI to approve removal.

## Audit matrix

`PASS` means only the stated assertion passed. `FAIL` means an observed or
source-demonstrated defect. `BLOCKED` means an unavailable runtime or
controller capability. `NOT TESTED` identifies an unexercised case.
Evidence: **M** = live macOS native AX/keyboard/visual observation;
**H** = GPUI window harness (macOS and Linux); **S** = source inspection;
**C** = computed production-color diagnostic. H/S/C do not establish native
screen-reader interoperability. **Every native Windows and Linux row below
is BLOCKED** for the environment reasons above, including menu, contrast,
scale and motion runtime checks. Spoken VoiceOver results are also BLOCKED
for every row. See [PWR-36](https://linear.app/pwrdrvr/issue/PWR-36).

| Area / promise | macOS native semantics / visual result | Keyboard / additional evidence | Finding |
| --- | --- | --- | --- |
| Startup has no implicit scan | NOT TESTED through AT | PASS H: `startup_chooser_never_scans_before_explicit_acceptance` | Preserve |
| Source input names, values and relationships | FAIL M: two unnamed text fields; entered host value not observable in AX snapshot | FAIL M: repeated Tab/Shift-Tab left native focus on window; pointer needed for path input | F2, F3 |
| Mount/source choices and reading order | FAIL M: visible mount rows and source explanation absent; no selected root row exposed | PASS H: arrows and explicit acceptance; full keyboard reachability FAIL M | F2, F3 |
| Source modal containment/restoration | FAIL M: background toolbar still exposed, no dialog in AX snapshot | PASS H: background mark/review/filter/rescan keys contained; Escape restores application root focus | F3 |
| SSH invalid destination/error | FAIL M: visible `invalid SSH destination`, unchanged AX | PASS H: invalid host cannot start scan/change existing source; M field activation worked after pointer focus | F2 |
| Successful SSH chooser and authentication failures | NOT TESTED M/AT | PASS H: simulated remote source/error cases; PASS Linux SSH integration, not native AT | PWR-36 |
| Scan progress, busy, cancel and completion | NOT TESTED native progress: default tree completed before capture; FAIL S: progress/notice strings have no semantic status/progress nodes | PASS H: initial/widening scan Escape cancel and retry; preview/progress remain bounded and non-actionable | F2 |
| Explore file hierarchy and selection | FAIL M: no file/tree nodes, sizes, hierarchy or selected-item state | PASS M/H: Tab/arrows move visual selection; Enter/Backspace handled in harness | F1 |
| Filter / search | FAIL S: explore filter is custom key-fed text without input/result semantics | PASS H: typing, Enter and Escape; no native speech result | F1, F2 |
| Marks and review rows | FAIL M: marked paths, row reasons and summary text absent; Unmark button alone does not name the row | PASS M/H: Space and c open review; Escape returns | F1 |
| Review mode names/selected state | PASS M: `Move to trash` / `Delete permanently` radio buttons and selected state track m/p | PASS M/H: mode keys; no live approval attempted | Preserve |
| Removal alert names/description/actions | NOT TESTED native AX or speech | PASS H: open, contain background shortcuts, Tab/Shift-Tab remain in application dialog, Escape restores Review/root focus, files retained | F3; PWR-36 |
| Destructive completion and unmarked neighbors | NOT TESTED interactively by design | PASS H: existing permanent-removal and link tests operate on temporary trees | Preserve |
| Toolbar/control names and actions | FAIL M: `<`, `>`, `−`, `+` names lack purpose/context; radio/checkbox names and state present | PASS M: field/button activation where exercised; all safe control actions not individually tested | F2, F3 |
| Disabled-state metadata | FAIL M/S: visually disabled Scan controls still ordinary AX buttons; Base test records missing disabled flag | Click action removal in Base is not a disabled-state announcement; native disabled-action invocation NOT TESTED | F3 |
| Native macOS menus | PASS M: named File/Edit/View/Go/Window/Help headings exposed | BLOCKED: native Help menu click did not complete; controller required refresh, Ctrl-F2 made no observable change. Popup keyboard operation not verified | PWR-36 |
| Off-macOS app/menu popovers | BLOCKED native | FAIL S: custom clickable Div rows without roles; root menu handling only dismisses on Escape; full platform keyboard behavior NOT TESTED | F3 |
| Settings pane order/content | FAIL M: pane selectors/titles/descriptions absent; Tab changes panes with AX focus at window | PASS M/H: pane cycling, updates shortcuts; control-by-control traversal FAIL M | F3, F4 |
| About / copy names | FAIL M: two indistinguishable `Copy` buttons, no surrounding identity text | Safe actions exposed; complete keyboard-only copy path NOT TESTED | F4 |
| Help shortcut overlay | FAIL M: no added AX content or modal role; underlying controls remain | PASS H: background navigation/action keys contained, Escape restores operation; FAIL M: sheet clipped vertically | F3, F4, F6 |
| Changelog / License / Notices | FAIL M: only Copy and native window controls; no text, headings or document reading order | PASS M: close returns to Settings; Tab/PageDown did not expose document navigation; scrolling outcome NOT TESTED | F4 |
| Logs | FAIL M: unnamed search, no log text; Wrap/Mark/Live exposed as buttons without observed state semantics | PASS H: filtering/rotation/selection/copy; Cmd-F exposed no AX focused search item in live check | F3, F4 |
| Progress/error/selection/mark/free-space announcements | FAIL M/S for missing current readable state; speech/cadence BLOCKED | No measured announcement frequency; cannot claim throttling or silence is correct | F1, F2 |
| Light/dark text and non-text contrast | FAIL C: ratios below thresholds, table below | PASS H only means both appearances draw; light native appearance NOT TESTED | F5 |
| Color-independent meaning | PASS M/S: reclaimable hatch, visible Marked text, sizes, selection outline; FAIL S: color/fading carries age/filter state without equivalent semantic file content | Combined selected+marked, unreadable and all color-vision simulations NOT TESTED | F1, F5 |
| Main UI zoom / clipping | PASS M: zoom grows UI through 175%; FAIL M: Help clipped at 100% and 175%; 200% unsupported by configured steps | PASS H: chooser/progress bounds at small dimensions/large rem; all-screen minimum-size sweep NOT TESTED | F6 |
| Auxiliary zoom / OS scaling | FAIL S: main zoom handler not shared with auxiliary windows | OS text/display scale and high-contrast runtime checks NOT TESTED on macOS; BLOCKED Windows/Linux | F6; PWR-36 |
| Long/Unicode names and tiny targets | PASS M: Unicode name drawn; long tile name clipped; FAIL for accessible full-path alternative (F1) | Target pixel/spacing sweep NOT TESTED; small controls/tiles require measurement | F1, F6 |
| Reduced motion | FAIL S: custom LayoutTransition creation/sampling does not read `cx.reduce_motion()` | OS preference runtime toggle NOT TESTED; no claim that Base preference import disables custom animation | F7 |

### Native AX evidence excerpts

These are transcribed relevant portions, not complete raw dumps. Window
controls and menu headings are omitted below. Indices are transient.

```text
Source chooser:
  container
    button <
    button >
    radio button (selected, settable, boolean) Size
    radio button (selectable, settable, boolean) Files
    radio button (selectable, settable, boolean) Age
    checkbox (settable, boolean) Hidden files, Value: on
    checkbox (settable, boolean) Apparent size, Value: on
    button −
    button +
    button Power Efficiency: Balanced
    button This computer
    text field
    button Connect SSH
    text field
    button Scan directory
    button Cancel
    button Scan selected root
  focused UI element: standard window

Explore after Tab:
  toolbar controls as above
  button Open
  button Mark for removal
  button Source…
  no file nodes or selected path/size; focused UI element: standard window

Review after p:
  button Unmark
  radio button (selectable, settable, boolean) Move to trash
  radio button (selected, settable, boolean) Delete permanently
  button Save list…
  button Copy as prompt
  button Save delete script…
  button Copy delete script
  button Back
  button Delete 0 items…

License / Third-Party Notices / Changelog after frame settles:
  button Copy
  no document text; focused UI element: standard window

Logs:
  button Copy path
  button Reveal
  text field
  button Wrap
  button Mark
  button Live
  no log text; focused UI element: standard window
```

The live review mark was a hidden fixture directory held back by the plan;
the zero executable item count is not evidence of a deletion-count bug.
The held-back explanation was visible but missing from AX, which is the
finding. The harness separately marks a removable `junk` directory.

## Severity-ranked findings and remediation

High means a required task or essential information is unavailable to an
affected user. Medium means a serious preference/usability failure with a
potential alternative. Severity is task impact, not a conformance ruling.
Each issue is in the **PwrDisk** Linear project, linked under PWR-27.

| ID | Severity | Observed failure / reproduction | Remediation |
| --- | --- | --- | --- |
| F1 | High | Scan fixture, Tab/arrows, Space, c: visual files/selection/marks/review exist without corresponding accessible data | [PWR-29](https://linear.app/pwrdrvr/issue/PWR-29): semantic scalable file navigation, selection, marks and review; stable IDs and safe actions |
| F2 | High | Startup two unnamed inputs; `-audit-invalid` + Return shows an error absent from AX; progress/status lack semantic nodes | [PWR-30](https://linear.app/pwrdrvr/issue/PWR-30): named inputs/values/relationships, selected source, progress/errors/free-space status and measured announcement throttling |
| F3 | High | Tab does not traverse startup controls; overlays expose background controls; application focus is not an AX item; disabled metadata incomplete | [PWR-31](https://linear.app/pwrdrvr/issue/PWR-31): traversal, visible focus, modal/menu containment, useful restoration and disabled semantics |
| F4 | High | ? / Settings / documents / Logs expose action fragments but omit readable content and context | [PWR-32](https://linear.app/pwrdrvr/issue/PWR-32): structured text/panes/documents/logs, contextual names and keyboard reading/scrolling |
| F5 | High | Run contrast diagnostic: default palettes fail text/state thresholds; semantic alternatives to color-carried age/filter meaning absent | [PWR-33](https://linear.app/pwrdrvr/issue/PWR-33): production contrast thresholds and non-color state equivalents |
| F6 | High | Help overflows at 100% and 175%; main zoom capped at 175%, auxiliary windows lack shared zoom route | [PWR-34](https://linear.app/pwrdrvr/issue/PWR-34): scrolling/reflow/scaling, long paths and target-size measurements; verify capture-specific sidebar clipping independently |
| F7 | Medium | Source: descend/history transitions always create/sample LayoutTransition irrespective of reduced motion | [PWR-35](https://linear.app/pwrdrvr/issue/PWR-35): honor preference in custom navigation animation and test final geometry/history |
| G1 | High validation gap | VoiceOver unavailable, Windows peer disconnected, Linux without desktop/Orca; native scaling/menu/AT scenarios incomplete | [PWR-36](https://linear.app/pwrdrvr/issue/PWR-36): finish native platform and AT matrix, keep unobserved cells explicit |

### Contrast measurements

Run against the audit test code (using the unchanged production palettes):

```sh
cargo test --locked -p disktree-app audit_contrast_report -- --ignored --nocapture
```

The diagnostic computes sRGB relative luminance after alpha compositing
the text over an opaque fill. It samples all **11** categories, all **5**
age buckets and depths **0–4**. Numbers are minima per pair. Text uses a
4.5:1 reference; selection ring versus adjacent fill uses 3:1. Exit 0 means
the report ran, **not** that contrast passed. The output intentionally
reports known failures outside the normal regression gate.

| Pair | Tokyo Night | Flexoki Light | Required |
| --- | ---: | ---: | ---: |
| Normal tile name | **3.856 FAIL** | 6.162 PASS | 4.5 |
| Dim tile size | **2.083 FAIL** | **2.393 FAIL** | 4.5 |
| Selected ring versus tile fill | 3.610 PASS | **2.285 FAIL** | 3.0 |
| Marked name on marked fill | 5.460 PASS | **3.967 FAIL** | 4.5 |
| Secondary caption at 75% on background | 5.131 PASS | **3.872 FAIL** | 4.5 |
| Text on highlighted action | 8.547 PASS | **3.184 FAIL** | 4.5 |

These are flat-fill calculations, not raster measurements of glyph edges
or hatch intersections. Not every view uses every pair. Hover, filtered,
unreadable, combined marked/selected, disabled, borders, focus indicators,
custom Omarchy themes and OS high-contrast modes need additional rendered
checks. Distinct hue tests in the existing palette suite do not establish
contrast or color-independent meaning.

## Toolkit bridge investigation

The lockfile selects GPUI Kit/Base **0.6.6**, GPUI pre **0.3.6**, Omarchy
**0.1.3**, and AccessKit **0.24.1**. Adapter implementations exist for
macOS (`SubclassingAdapter`), Windows (`Adapter`, WM_GETOBJECT handling),
and Linux X11/Wayland (`accesskit_unix::Adapter`). macOS buttons/radio
buttons/checkboxes observed above demonstrate that the bridge is active.
It is incorrect to describe GPUI as having no accessibility support.

The GPUI accessibility guide requires IDs and roles; ordinary string
children are not automatically a readable document. PwrDisk chiefly uses
plain Div/string content and a painted canvas; inspection found only one
explicit app accessibility label override (Power Efficiency). The canvas
needs a semantic navigation path. GPUI supports synthetic accessibility
children, but their existence does not make this canvas accessible.
`debug_selector` identifiers used by harness tests are not accessible
names or roles.

There is also a concrete toolkit gap: GPUI Base's
`button::tests::accessibility_role_label_and_disabled_action_surface`
explicitly asserts `!disabled.is_disabled()` and explains that the GPUI
interface lacks a disabled-state setter even though AccessKit supports
one. Removing the Click action is not equivalent to conveying disabled
state. Input names/values/focus and modal semantics need investigation in
both the app and toolkit, without attributing every missing app role to
the platform bridge. Windows and Linux adapter correctness remains
unverified here. PWR-31 tracks the owning-layer work.

Relevant baseline source: [views](../crates/disktree-app/src/views.rs),
[state](../crates/disktree-app/src/state.rs),
[treemap](../crates/disktree-app/src/treemap_view.rs),
[Settings](../crates/disktree-app/src/settings.rs),
[documents](../crates/disktree-app/src/doc_window.rs),
[Logs](../crates/disktree-app/src/logs_window.rs),
[appearance](../crates/disktree-app/src/appearance.rs),
[palette](../crates/disktree-app/src/palette.rs), and
[scale tokens](../crates/disktree-app/src/ui.rs).

## Upstream remediation candidates

Inspected PR descriptions and full diffs on 2026-10-31; both were open.
Neither was built or run in this audit, and neither was imported.

* [tobi/disktree #51](https://github.com/tobi/disktree/pull/51), head
  `b7879541c4dfc709009725b376bc0d138a8f2e3c`: virtualized row view,
  expansion, arrows, shared selection/marks/filter and offscreen-row
  composition test. Useful groundwork for a scalable alternative to
  spatial tiles. The reviewed diff has no tree/list roles, accessibility
  item states/relationships or AT results. Virtualization must not make
  offscreen files unreachable to AT. Evaluate selected pieces in PWR-29,
  coordinated with [PWR-28](https://linear.app/pwrdrvr/issue/PWR-28).
* [tobi/disktree #58](https://github.com/tobi/disktree/pull/58), head
  `c9f454d5e8cac841be265c01f11eba18984060d5`: fixed themes, persistence,
  and alpha-composited contrast tests. Useful calculation/test patterns.
  Its `tile_names_stay_readable_on_every_fill` covers nine categories,
  normal labels, category/age/marked fills and theme variants. It does not
  cover PwrDisk's two extra categories, 50%-opacity size text, all status
  captions/actions, focus/state boundaries, high-contrast OS behavior or
  screen-reader usability. Evaluate narrowly in PWR-33; theme picker and
  persistence are independent product work.

## Regression coverage and verification

New `audit_` regressions in [tests.rs](../crates/disktree-app/src/tests.rs)
verify Help background-key containment and resumed marking, source-picker
background-key containment and focus restoration, and removal-dialog
background-key containment plus Tab/Shift-Tab containment and Escape
cancellation. The confirmation test never sends the final approval action.
Files and marks remain after cancellation. These preserve existing safety
and keyboard promises; they do not encode missing semantics as acceptable
behavior.

The palette suite adds a regression for alpha-composited contrast math
(black/white 21:1, transparent 1:1, half-white on black about 5.281:1) and
an explicitly ignored diagnostic for the table above. Future color fixes
need threshold assertions over corrected production colors. No production
fix is claimed by passing the math or keyboard tests.

Existing harness tests cover scan/cancel/retry, explicit chooser
acceptance, invalid SSH input, metric/selection preservation, filtering,
marks/review, removal guards, Settings, documents, Logs, both appearances,
and bounded chooser/progress layouts. They draw frames or inspect state,
not native AX/UIA/AT-SPI. The headless platform does not activate native AT;
native semantic regressions still need to be added with the remediation.

```sh
cargo test --locked -p disktree-app audit_ -- --include-ignored --nocapture
cargo xtask lint
cargo xtask test
```

Baseline Linux verification in the dedicated VM used
`scripts/sandboxer/verify.sh all`:

| Step | Exit | Seconds |
| --- | ---: | ---: |
| setup | 0 | 2 |
| lint | 0 | 16 |
| test | 0 | 30 |
| portable build | 0 | 84 |
| SSH integration | 0 | 7 |
| all | 0 | — |

Final audit-commit verification is recorded in the PR, including the
commit SHA and platform result. Documentation/test-only changes do not
have a before/after production window change. The audit publishes text
and numeric evidence; future visible remediations require contrived
before/after PR attachments under [the screenshot procedure](pr-screenshots.md).
