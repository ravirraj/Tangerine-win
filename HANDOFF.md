# Handoff — Open Tangerine for Windows (B-10 tool windows)

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
| Dispatch (which op opens which window) | `src\app.rs` (`open_tool_window`, `ToolEvent`) |
| FFmpeg binaries for real tests | `vendor\ffmpeg\ffmpeg.exe`, `vendor\ffmpeg\ffprobe.exe` |
| Screenshots (capture-test output) | `target\shots\` (stale `bisect-*`, `crop-*`, `trim-after*` files are old debug artifacts — ignore) |

## 3. State at handoff

- Branch `phase/b-10-tool-windows`, tree clean. Chain ends:
  `… c3be929` (B-10.4 trim) → `e9476e1` (docs reconciliation) → `a37ae61` (split backend)
  → `5d167db` (split window) → `90da4f9` (docs tally). Reviewable range for the whole
  split batch: `git diff c3be929..HEAD` (11 files, +1332 −66).
- Tools wheel now has **8** implemented ops: compress, crop, trim, **split**, rotate,
  resize, merge, metadata. Trim (B-10.4) and Split (B-10.6) have real windows.
- Tests at handoff: **330 unit + 31 ignored, 0 failures** (the ignored suite = real-FFmpeg
  operation tests incl. the split one + the two window-capture tests).
  These numbers will grow as you work; state real numbers after running.
- `ToolSettings` fields today: `width, height (u32)`, `start_ms, end_ms (u64)`,
  `points_ms: Vec<u64>` (only `tools.split` reads points).

## 4. The conventions a new window must follow

1. **Layout-twin pattern.** Slint only *draws*; the Rust window computes every
   rectangle in `Layout::of()` and hands identical numbers to both the card and
   its own hit-testing. Slint component instances are pinned
   `x:0; y:0; width: card-width; height: card-height` and position
   themselves from `Panel` properties (`{x,y,width,height}`), NOT from a
   layout container. Never use slint layout containers inside cards.
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

## 5. The next plan (evidence-based order came from MAPPING-ROADMAP)

The roadmap's P2 = shared control library built *before* per-tool bodies; P3
items: 6 trim ✅, **7 split ✅**, **8 crop NEXT**, 9 compress+metadata,
10 audio trio, 11 redact, 12 snapshots, 13 join/merge, 14 organize PDF,
15 annotate/frame/collage. "Resize" is NOT a reference window — its mapping
row routes resize/crop to the Crop window; our Resize ToolCard stands until
that polish naturally covers it.

### B-10.7 — Crop video + image (P3-8), the immediate batch

Evidence to read first (in this order):
1. `tool-windows/cropVideo.webp` and `cropImage.webp` (visual layout);
2. `Sources\Tools\Video\CropSpeedJoinVideoToolViews.swift` (video crop behavior —
   same file also carries speed + join, which you will need later; extract only
   the crop part now);
3. `Sources\Tools\Image\CropImageToolView.swift`, `Image\ImageCanvas.swift` +
   `Operations\VideoOps.crop` / `Operations\ImageOps.crop` for the actual math;
4. `DESIGN-SPEC.md` for the crop controls (aspect presets, numeric fields).

Known starting points in our code:
- Backend today is a **center-half placeholder**: `tools.rs` `run_one` maps
  `"tools.crop"` to `crop=iw/2:ih/2:(iw-iw/2)/2:(ih-iw/2)/2`. The registry row is
  `categories: [Image]` — a discrepancy to resolve: the reference crops BOTH
  video and image, and the roadmap marks cropVideo P3-8. Expected outcome:
  either widen the category list per reference eligibility, or keep two
  behaviors routed by input kind (the handler already branches by op id).
- `ToolSettings` will need crop fields (e.g. `crop: {x, y, w, h}` or aspect +
  preset). Follow the B-10.6 pattern: the window computes what it can; the
  backend stays "cut to the asked rectangle".
- The window: crop is a *timeline-neutral* tool — do NOT assume it belongs in
  the timeline window. Decide after reading the mockups; the generic
  `toolwindow.rs`/`tool_window.slint` plain-card shell is the likely host, with
  the image crop needing a canvas to drag on (ZoomableView in Swift is the
  behavior hint). The card stack + hit-testing twin pattern applies either way.

Plan the batch IN `todo.md` FIRST as B-10.7 with checkboxes and a
deliberate-differences area (B-10.5/B-10.6 are the template), then implement
backend → registry/dispatch → window → tests → captures, committing in
2–4 reviewable conventional commits authored Ravi, never pushed.

Acceptance criteria:
- cropVideo.webp / cropImage.webp parity with a screenshot pair each;
- real-FFmpeg test(s) cutting a real fixture to an asked rectangle (and image
  crop real test);
- crop output preserves input format beside the original (`File::BesideAsInput`
  semantics, collision Rename) with Swift's naming rules if it names files;
- trim + split captures re-verified unchanged;
- unit + ignored suites green with refreshed counts in PLAN.md/todo.md.

### The queue after B-10.7 (each one reads its own evidence first)

- **P3-9 Compress + Metadata** — `compress.webp`, `removeMetadata.webp`;
  Swift `CompressToolView.swift`, `MetadataToolView.swift`, `MetadataOps.swift`.
  Needs shared **Slider with quality presets** and **Checkbox** controls. Our
  compress/metadata backends partially exist — audit first.
- **P3-10 Audio trio** — `trimAudio.webp` (timeline reuse!), `normalizeAudio.webp`,
  `audioChannels.webp`; Swift `AudioToolViews.swift` + `AudioOps.swift`.
  Needs a **dropdown/segmented** control. trimAudio should reuse the timeline
  window in trim mode (audio has no frames — preview shows waveform or empty
  state; check the webp).
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
Slider+ticks, Checkbox, SwatchGrid, ValidationNote. (Already present:
IconButton, PillButton, TimelineRange with markers/has-range, stepper —
the split card's is inline; extract to a shared `Stepper` when P3-9 needs one.)

## 6. Testing & how to run

```
cargo check --tests        # must be 0 before committing anything
cargo test                 # unit suite; at handoff: 330 passed, 31 ignored
cargo test -- --ignored    # real-FFmpeg ops + window captures; at handoff: 31 passed
cargo test capture -- --ignored --nocapture   # regenerate the two window shots
```
- Real tests need `vendor\ffmpeg\*.exe` (committed? no — vendored locally; if
  absent they print "the program is not there" and pass vacuously — never take
  that as success when claiming FFmpeg work).
- After any change touching `shell.slint` or `timelinewindow.rs`, re-run BOTH
  capture tests and eyeball `trim-window.png` + `split-window.png`.
- Shell quirks to avoid noise: PowerShell has **no `&&`**
  (use `;` or separate calls); `Select-String` exits **1** on no-match —
  when a command's success matters, capture it:
  `cmd > $null 2>&1; Write-Output "EXIT=$LASTEXITCODE"`.

## 7. Gotchas learned the hard way (B-10.6)

- **Slint setters are generated only for the ROOT component's in-properties.**
  To reach a sub-component property (e.g. `CardFooter.run-enabled`), add a
  passthrough in-property on the card and wire it in the instance.
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
- Docs: exactly-phrase test counts from the runner ("330 unit + 31 ignored
  (real-FFmpeg operations and the two window captures), 0 failures").
