# Handoff — Open Tangerine for Windows (B-10 tool windows)

> Picking up a tool batch? Read **`PLAYBOOK.md`** first — it is the full
> method the finished batches (trim, split, crop) followed, in order.
> This file is the state map; the playbook is the procedure.

Written 2026-10-25 after the B-10.5/B-10.6 batch (roadmap reconciliation + Split Video).
Read `todo.md` (the running work log, phases B-0…B-10.6) and `PLAN.md` first; this file
only says what a fresh agent needs to continue without the prior session's memory.

---

## 1. The ground rules (carried from the user's brief — do not renegotiate)

- Authority order when building a tool window:
  **1. Swift source (behavior) → 2. screenshots/mockups from the site (visual) →
  3. DESIGN-SPEC (design system) → 4. MAPPING-ROADMAP (priority only) →
  5. Windows constraints.** When sources conflict, resolve in that order and
  record the conflict in `todo.md` as a "deliberate difference".
- Write access: ONLY `C:\Users\thorfinn\projects\Tangerine-win\`. The directory
  `C:\Users\thorfinn\projects\tempp\reference\tangerine\` is read-only reference.
- Git: Conventional Commits, author **Ravi** (already configured), **never push**,
  work in phases on `phase/*` branches, small reviewable commits, and keep
  `todo.md` / `PLAN.md` current as you go. Non-destructive: preserve user work.
- Honesty rules: never mark a test as passing without running it; never claim
  parity with the reference without a screenshot pair (mockup vs our capture);
  state limitations exactly; don't stop at an audit when implementation can
  safely proceed; no framework swaps, no engine rewrites, no giant
  unreviewable changes.
- **Protect the trim baseline (B-10.4) and the split window (B-10.6).** If you
  touch shared parts (`shell.slint`, `timelinewindow.rs`), re-run the capture
  tests and compare `target/shots/trim-window.png` and `split-window.png`
  against their baselines before committing.

## 2. Where everything is

### Reference material (read-only)

| What | Path |
|---|---|
| Priorities (P0…P3, per-item order) | `C:\Users\thorfinn\projects\tempp\reference\tangerine\MAPPING-ROADMAP.md` (123 lines — read fully at the start of each new tool) |
| Design system | `C:\Users\thorfinn\projects\tempp\reference\tangerine\DESIGN-SPEC.md` |
| Tool-window mockups (visual authority) | `C:\Users\thorfinn\projects\tempp\reference\tangerine\tool-windows\*.webp` — 22 files: cropVideo, cropImage, compress, removeMetadata, trimAudio, normalizeAudio, audioChannels, audioToVideo, changeVideoSpeed, joinVideos, mergePDF, organizePDF, redactVideo/Image/Audio, videoSnapshots, frameImage, editImage, annotateImage, createCollage, trimVideo, splitVideo |
| Swift behavior authority — tool views | `reference\extracted\Open-Tangerine-main\NativeApp\Sources\Tools\…`: `Video\TrimSplitVideoToolViews.swift` (done), `Video\CropSpeedJoinVideoToolViews.swift` (crop **and** speed **and** join), `Image\CropImageToolView.swift`, `Image\EditImageToolView.swift` + `Image\ImageCanvas.swift`, `CompressToolView.swift`, `MetadataToolView.swift`, `Audio\AudioToolViews.swift`, `Video\SnapshotsRedactVideoToolViews.swift`, `Image\RedactImageToolView.swift`, `PDF\PDFToolViews.swift`, `Video\EnhanceVideoToolView.swift` |
| Swift behavior authority — the actual operations | `reference\extracted\Open-Tangerine-main\NativeApp\Sources\Operations\`: `VideoOps.swift`, `ImageOps.swift`, `AudioOps.swift`, `MetadataOps.swift`, `PDFOps.swift` (grep `func split` / `func trim` / `func crop` there — this is where the FFmpeg-mirroring math lives) |
| General UI helpers | `Sources\Tools\Image\ZoomableView.swift`, `MediaPlayer.swift` (read when a mockup needs canvas/zoom behavior) |

All `reference\…` paths above are relative to the repo root
(`C:\Users\thorfinn\projects\Tangerine-win\reference\…` — writable copies, safe to read).

### Our code (the port)

| Role | Path |
|---|---|
| Registry (all ops, eligibility) | `src\radial\registry.rs` |
| Settings/request/outcome contract | `src\radial\execution.rs` |
| Tool handler (FFmpeg args, split, trim) + real tests | `src\radial\tools.rs` |
| Wheel/menu pins + icons | `src\radial\actions.rs` |
| Shared slint skeleton & controls | `src\ui\shell.slint`, `src\ui\tokens.slint` |
| Timeline card (trim + split rows) | `src\ui\trim_window.slint` |
| Timeline window host (layout, hits, keyboard, captures) | `src\platform\windows\timelinewindow.rs` |
| Generic tool window (plain card) | `src\platform\windows\toolwindow.rs` + `src\ui\tool_window.slint` |
| Shared window-host plumbing (DPI, wide words, fixed frame, timecode, fraction + the capture-test helpers) | `src\platform\windows\windowhost.rs` |
| Dispatch (which op opens which window) | `src\app.rs` (`open_tool_window`, `ToolEvent`) |
| FFmpeg binaries for real tests | `vendor\ffmpeg\ffmpeg.exe`, `vendor\ffmpeg\ffprobe.exe` |
| Screenshots (capture-test output) | `target\shots\` (stale `bisect-*`, `crop-*`, `trim-after*` files are old debug artifacts — ignore) |

## 3. State at handoff

- Branch `phase/b-10-tool-windows`, tree clean. Chain ends:
  `… 500d9cc` (metadata window host + card)
  → `453681b` (audio trim backend) → `31d584e` (audio trim timeline skin)
  → `9584c9a` (normalize volume backend) → `df67df1` (shared LabeledSlider control)
  → `bee8c6e` (normalize volume window host + Slint card)
  → `3380dbd` (convert audio channels backend)
  → `42b08d8` (convert audio channels window host + Slint card)
  → `6b5dbd8` (unify normalize volume and audio channels window UI with Windows shell standard)
  → `aeeba44` (docs: HANDOFF unification note) → `1c01ef3` (chrome-aware outer
  size) → `bf62786` (titlebar chrome parity fixes)
  → `0efe153` (docs: Redact Phase A evidence and plan for B-10.10)
  → `95f3f15` (must-use warnings quieted in the new window hosts, dead metadata
  code dropped, three-shot capture probe test added). The 2026-10-26 audit round
  over these twenty commits is recorded in `todo.md` B-10.11; the capture-hash
  protocol it amended lives in PLAYBOOK §3.
- Tools wheel now has **10** implemented ops: compress, crop, trim, split, rotate,
  resize, merge, metadata, normalize, channels. Dedicated windows exist for
  Trim Video, Split Video, Crop Video/Image, Compress, Edit Metadata, Trim Audio,
  Normalize Volume, Convert Audio Channels, Redact Photo, Redact Video, Bleep Audio,
  Change Speed, and Join Videos / Merge PDFs matching reference behavior and visual mockups.
- Tests at handoff: **494 passed; 0 failed; 69 ignored** (`cargo test`:
  `test result: ok. 494 passed; 0 failed; 69 ignored` — real-FFmpeg and real-PDFium operation
  tests, and all window captures including `join-videos-window.png` and `merge-pdf-window.png`).
  Warnings: **43** (exact baseline, 0 new warnings).
- `ToolSettings` fields today: `width, height (u32)`, `start_ms, end_ms (u64)`,
  `points_ms: Vec<u64>`, `crop_x, crop_y, crop_w, crop_h (Option<u32>)`,
  `compress_strong: bool`, `target_bytes: Option<u64>`,
  `max_long_edge: Option<u32>`, `max_fps: Option<u32>`,
  `metadata_tags: Vec<(String, String)>`, `metadata_remove_all: bool`,
  `metadata_remove_sensitive: bool`, `normalize_lufs_x10: Option<i32>`,
  `normalize_range_x10: Option<i32>`, `normalize_true_peak_x10: Option<i32>`,
  `channel_mode: Option<String>`, `channel_left_gain: Option<u32>`,
  `channel_right_gain: Option<u32>`.

## 4. The conventions a new window must follow

1. **Layout-twin pattern.** Slint only *draws*; the Rust window computes every
   rectangle in `Layout::of()` and hands identical numbers to both the card and
   its own hit-testing. Slint component instances are pinned
   `x:0; y:0; width: card-width; height: card-height` and position
   themselves from `Panel` properties (`{x,y,width,height}`), NOT from a
   layout container. Never use slint layout containers inside cards.
   An instance left unmeasured is auto-centred (width 0, `x = card/2`) by
   the compiler and lands nowhere — the crop chips bug (B-10.7); the pin
   above is not a style preference, it is what makes the boxes reachable.
2. **Unit system (scale-independent).** All coordinates are designed in "units"
   for a `CARD_W = 420` card and multiplied by a DPI scale factor:
   `at(|u| u * scale)`. Content stacks: trim `CONTENT_UNITS = 282` +
   `FOOTER_UNITS = 64` → window heights 582/646 (preview 236/300); split
   `SPLIT_CONTENT_UNITS = 308` → 608/672. Pinned by unit tests — keep them
   pinned and give every new window stack its own const + `height_for_*`.
3. **Tokens** (`tokens.slint` — never invent colors locally; theme.rs holds the
   same numbers): accent `#ff5a1f` (hover `#ff7138`, pressed `#e94a12`),
   window bg `#F7F6F4`, preview dark `#1A1A1A`, text `#1D1D1F`, muted `#828782`,
   transport/ink `#3A3A3A`, hairline `#E3E3E1`, field border `#D0D0D0`,
   mono face `"Consolas"` (alias `Tokens.font-mono`), radii 24/12/14/10/6.
   Orange = *chosen*, never *danger/press*; buttons are quiet pills.
4. **Chrome.** Native Windows title bar showing the tool label ("Trim", "Split")
   — the recorded deviation replacing the mockups' frameless centered titles.
   In-card header = file name + one muted detail line. Footer = hairline +
   shortcut line left + quiet Run/Cancel pills right (the footer component
   `CardFooter`, now with `run-label` and `run-enabled`).
5. **Comment voice.** The codebase comments in a deliberate plain-English
   dialect ("the strip is worn with frames", "the moments are said in the face
   where every digit takes the same room", "the caps are drawn, not cast").
   Match it — read a few files before writing. It is part of the review bar.
6. **Registry/menu pin tests break when you add an op.** Update exactly three
   places: the `IMPLEMENTED` list test in `registry.rs` (~line 1507), the
   design-menu string pin in `actions.rs`, and the `segments().len()` count in
   `controller.rs` (was 7→8). Grep for the old count first; there is one site.
7. **Backend pattern (`tools.rs`).** `run_one` maps an op id to args
   (`trim_args` for range/stream-copy work, `build_convert_args` + optional
   video filter otherwise); the run-and-publish tail is the shared `finish()`
   helper. A new special op follows `run_split`'s shape: gate
   `if request.operation == "tools.split"` early in `Handler::run`, do your own
   naming/folder policy, reuse `finish()`. Keep `run_one`'s other branches
   byte-identical.
8. **Capture = proof.** Each window gets an `#[ignore]` capture test in the
   `show_tests` module (see `capture_the_trim_window` / `capture_the_split_window`)
   writing `target/shots/<name>-buffer.png` + `<name>-window.png`, then you READ
   BOTH the webp mockup and the png (both render as images via the read tool)
   and record the comparison in `todo.md`.
9. **The host plumbing lives in `windowhost.rs` — take, never copy.**
   `scale_factor`, `screen_scale`, `to_wide` (every window), `fixed_style`,
   `timecode`, `fraction` (the timeline and crop hosts), and under
   `#[cfg(test)] mod screenshot` the `make_test_video` + `capture` helpers
   every capture test shares (`crate::platform\windows\windowhost\…`).
   What stays local to a window: its geometry and unit consts, its card and
   STATE, its timers, `outer_size`, `window_handle`, the capture tests' own
   pump loops, and crop's `make_test_image`.

## 5. The next plan (evidence-based order came from MAPPING-ROADMAP)

The roadmap's P2 = shared control library built *before* per-tool bodies; P3
items: 6 trim ✅, **7 split ✅**, **8 crop ✅**, **9 compress+metadata NEXT**,
10 audio trio, 11 redact, 12 snapshots, 13 join/merge, 14 organize PDF,
15 annotate/frame/collage. "Resize" is NOT a reference window — its mapping
row routes resize/crop to the Crop window; our Resize ToolCard stands until
that polish naturally covers it.

### B-10.7 — Crop video + image (P3-8) — DONE

Landed as `5f9f3a5` (backend, `src/radial/tools.rs`: the real crop args —
`crop=W:H:X:Y` from `crop_x/y/w/h`, even-rounded; the Rust-native image path;
`finish()` publishing results beside the original by the shared naming) and
`a4ad5bb` (window: `src/platform/windows/cropwindow.rs` +
`src/ui/crop_window.slint`, dispatch, exports, tests). The window keeps the
crop's own geometry and reads the pointer itself — corner reach, track, draw —
with the aspect ratio laid on by the presets, the way Swift's canvas does.

The audit fixed two backend mistakes before they landed: results publish
beside the input through the same `finish()` loop every other tool takes,
and `tools.rotate`/`tools.resize` stay `[.image]`-only (an earlier widening
to video was reverted — Swift pins both to image).

Deliberate differences are recorded at the bottom of the B-10.7 section of
`todo.md` (no W/H fields — keep-size always; no edge-midpoint handles; four
corner squares, not circles; run label "Crop"; a fitted stage instead of the
pinch-zoom canvas; no filmstrip — crop is timeline-neutral).

Acceptance evidence: `target/shots/crop-video-window.png` +
`crop-image-window.png` against the two mockups; `trim-window.png` +
`split-window.png` retaken unchanged; 355 unit + 36 ignored, 0 failures.
Manual drag-and-run verification is the one thing a person still has to do.

### B-10.8 — Compress + Edit metadata (P3-9) — DONE

Landed across four clean commits:
- `5805672` (audit & evidence): detailed audit of Compress & Metadata backends vs Swift sources and mockups.
- `fe97de0` (compress execution backend): `ToolSettings` extended with preset/target_bytes/max_long_edge/max_fps; FFmpeg video/audio compression + native image compression with binary-search target file size; eligibility unblocked.
- `f07f900` (compress window): twin layout host `CompressWindow` + Slint `CompressCard` with file rows, preset chips, video options, target-size numeric entry with KB/MB chips, round checkbox.
- `b4e70da` (metadata execution backend): `MetadataField`, `MetadataTag`, probe reading, native image strip of sensitive EXIF/GPS/TIFF tags, FFmpeg stream-copy tag updates; eligibility unblocked.
- `500d9cc` (metadata window): twin layout host `MetadataWindow` + Slint `MetadataCard` with native child EDIT controls for search and 7 editable tags, changed dots, file system readouts, right sidebar cards (Fields/Undo, Remove checkboxes, Summary count), footer.

Deliberate differences:
- Native title bar with left alignment and close top-right.
- Checkbox is round (`RoundCheckbox` in `shell.slint`), matching `compress.webp` and `removeMetadata.webp`.
- Slider control deferred to future batches (Normalize Audio / Edit Photos).

Acceptance evidence:
- `metadata-window.png`, `compress-window.png`, `compress-video-window.png` against reference mockups.
- 6 baseline capture hashes verified byte-identical.
- 381 unit tests passed, 0 failed, 45 ignored.

### The queue after B-10.8 (each one reads its own evidence first)

- **P3-10 Audio trio** — `trimAudio.webp` (timeline reuse!), `normalizeAudio.webp`,
  `audioChannels.webp`; Swift `AudioToolViews.swift` + `AudioOps.swift`.
  Needs a **dropdown/segmented** control and **slider**. trimAudio should reuse
  the timeline window in trim mode (audio has no frames — preview shows waveform
  or empty state; check the webp).
- **P3-11 Redact** — `redactVideo/Image/Audio.webp`; Swift
  `SnapshotsRedactVideoToolViews.swift`, `RedactImageToolView.swift`.
- **P3-12 Snapshots** — `videoSnapshots.webp`; same Swift file; frame extraction
  reuses the timeline window's filmstrip machinery.
- **P3-13 Join/Merge** — `joinVideos.webp`, `mergePDF.webp`; Swift
  `CropSpeedJoinVideoToolViews.swift` (join part) + `PDFToolViews.swift`; needs
  a reorder-list control. Backends exist (`tools.merge`).
- Later: organize PDF (14), annotate/frame/collage (15), speed, enhance,
  background removal, etc. — each reads MAPPING-ROADMAP first.

Shared controls still missing from `shell.slint` (build them **with** the
window that first needs them, not speculatively): Segmented, Dropdown,
Slider+ticks, SwatchGrid, ValidationNote. (Already present:
IconButton, PillButton, TimelineRange with markers/has-range, RoundCheckbox).

## 6. Testing & how to run

```
cargo check --tests        # must be 0 before committing anything
cargo test                 # unit suite; at handoff: 408 passed, 0 failed, 52 ignored
cargo test -- --ignored    # real-FFmpeg ops + captures; at handoff: 52 passed, 0 failed
cargo test capture -- --ignored --nocapture   # regenerate the window shots
```
- Real tests need `vendor\ffmpeg\*.exe` (committed? no — vendored locally; if
  absent they print "the program is not there" and pass vacuously — never take
  that as success when claiming FFmpeg work).
- After any change touching `shell.slint` or `timelinewindow.rs`, re-run BOTH
  capture tests and eyeball `trim-window.png` + `split-window.png`.
- Window-shot byte diffs confined to the title band are the capture face
  race (grayscale vs ClearType), not a regression — PLAYBOOK §3's amended
  protocol is the rule; buffer shots are the byte gate.
- Shell quirks to avoid noise: PowerShell has **no `&&`**
  (use `;` or separate calls); `Select-String` exits **1** on no-match —
  when a command's success matters, capture it:
  `cmd > $null 2>&1; Write-Output "EXIT=$LASTEXITCODE"`.

## 7. Gotchas learned the hard way (B-10.6 + B-10.7)

- **Slint setters are generated only for the ROOT component's in-properties.**
  To reach a sub-component property (e.g. `CardFooter.run-enabled`), add a
  passthrough in-property on the card and wire it in the instance.
- **Sub-component instances must be measured.** An instance with no
  `x/y/width/height` at the instantiation site is auto-centred by the slint
  compiler (`x = card/2`, width 0): its inner absolutely-positioned content
  draws off-window and its TouchAreas sit outside every hit-test — the
  chips answered nothing until each `Chip {…}` got the four-line pin the
  pills already had. The working double was `PillButton` with the pin; the
  broken one was `Chip` without it (B-10.7).
- **Never re-enter a thread-local that is already borrowed.** A helper
  called from inside a `HOST.with(borrow_mut)` closure must not itself read
  `HOST` — the crop's first `stand_media` re-read the stage through the
  thread-local instead of taking it as an argument and panicked
  (`RefCell already borrowed`) in the plain `show_for` path. Take geometry
  as function arguments; the timeline window's own code never nests Borrows.
- **Do not hold the HOST borrow while dispatching input events.**
  `software.window().dispatch_event(...)` runs slint callbacks *inside* the
  call, and a callback (`choose_aspect`, the pills' answers) reads HOST on
  its own. The window proc clones the `Rc<MinimalSoftwareWindow>` out of
  the CELL, dispatches without the borrow, then draws. Same shape the
  trim window already avoids by never dispatching under a borrow.
- **Array properties (`[float]`)**: from Rust use
  `std::rc::Rc::new(slint::VecModel::from(vec)).into()` — `Vec` alone is not a
  `Model`.
- **Win32 imports** for keyboard live in
  `windows::Win32::UI::Input::KeyboardAndMouse::{VK_DELETE, VK_SPACE, …}`.
- Split-mode facts if you must touch the timeline window: split is a flag in
  `STATE` (`split`, `points`, `parts`, `equal_saying`), the layout branch is in
  `Layout::of()` keyed by `is_split()`, markers hit-testing uses
  `MARKER_REACH = 8` units, Swift guards live as constants
  (`POINT_MARGIN 0.1`, `MIN_POINT_GAP 0.05`, `PARTS_MIN/MAX`).
- One test run exited `0xC0000409` (fail-fast) during a back-to-back
  unit+ignored run; it never reproduced (three clean reruns, incl. the identical
  sequence). If you see it once, rerun; record it if you can make it stable.
- Docs: exactly-phrase test counts from the runner ("355 unit + 36 ignored
  (real-FFmpeg operations and the four window captures), 0 failures").
