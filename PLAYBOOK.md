# Playbook — how a tool batch is built here

**Read this file before anything else if you are picking up a tool batch
(the next one is P3-9: Compress + Metadata).** It is the distilled method
behind Trim (B-10.4), Split (B-10.6) and Crop (B-10.7). Companion docs:
`HANDOFF.md` (state + conventions + gotchas), `todo.md` (per-batch logs),
`PLAN.md` (baseline table). When these conflict with this file, trust the
newer `todo.md`/`HANDOFF.md` entries.

---

## 0. Read order — before any code

1. `HANDOFF.md` fully (ground rules §1, file map §2, state §3, conventions
   §4, remaining plan §5, testing §6, gotchas §7).
2. `reference/tangerine/MAPPING-ROADMAP.md` fully (123 lines) — read it
   again at the start of **every** new tool, per the ground rules.
3. `reference/tangerine/DESIGN-SPEC.md` — the section the tool needs.
4. The tool's **mockup webp** (`reference/tangerine/tool-windows/*.webp`)
   — open it with the read tool; render it as an image, do not guess.
5. The tool's **Swift authority**
   (`reference/extracted/Open-Tangerine-main/NativeApp/Sources/Tools/…`
   and `Sources/Operations/*Ops.swift`) — this is where the real behavior
   lives; layout comes from the webp, behavior from Swift.
6. `todo.md` — the previous batch's section is the template for yours.

`C:\Users\thorfinn\projects\tempp\reference\tangerine\` is a **read-only
reference**; never edit it. All edits happen under the repo.

---

## 1. The batch shape — five phases, five-ish commits

### Phase A — Evidence (no code)

- Read the mockup + Swift + spec (§0). Note **every** difference between
  what our stack can do and what the reference shows.
- Audit the backend for the tool in `src\radial\tools.rs` +
  `radial\registry.rs`; check `ToolSettings` fields for missing data.
- Write the batch section in `todo.md` (`### B-10.x <tool> (roadmap P3-y)
  — reference: <webp> + <Swift file>`) with checkboxes and a
  **deliberate-differences list you extend as decisions happen**.
- Decisions that are genuinely a choice (control style, what a slider
  means for center-zero, what a mockup's odd control maps to): **ask the
  user** with the question tool. Record the answer in todo.md.

### Phase B — Backend (one `feat:` commit)

- Args builder (`*_args` fn), gate early in `Handler::run`, do your own
  naming rule only if the tool needs one (follow `run_split`'s shape),
  reuse the shared `finish()` publisher and the beside-input convention.
- Real-FFmpeg proof tests (`#[ignore = "needs the conversion program…"]`)
  in `src\radial\tools.rs` — they run only with `vendor\ffmpeg\*.exe`
  present; if absent they pass vacuously, so **never** report success from
  an unrun ignored test.
- Run `cargo check --tests`, then `cargo test`; quote the runner lines
  exactly ("N passed; 0 failed; M ignored"). Never claim unrun tests.
- Commit: conventional, message in the house voice, author
  `Ravi <ravirrajpatil@gmail.com>`, never push.

### Phase C — Window (one or two `feat:` commits)

This is where the layout-twin pattern applies. Slint only *draws*; the
Rust window computes every rectangle in `Layout::of()` and hands the same
numbers to the card (via `Panel` properties) and to its own hit-testing.
Files, in order:

1. **Slint card** — a new `src\ui\<tool>_window.slint` composed of the
   shell components (`CardHeader`, `Hairline`, `IconButton`, `PillButton`,
   `CardFooter`) and tokens (`tokens.slint` — never invent colors; the
   theme's numbers are pinned by tests). Export it through
   `src\ui\tool_window.slint` (named export, as crop did) and check
   whether `build.rs` needs the new file.
2. **Every sub-component instance is pinned at its instantiation site**:
   `x: 0; y: 0; width: <card_w>; height: <card_h>;` and positions itself
   from the `Panel` `{x,y,width,height}` properties. **An unmeasured
   instance is auto-centred by the compiler (`x = card/2, width 0`) and
   lands nowhere** — invisible, unreachable TouchAreas. This was the crop
   chips bug; the pin is not a style preference, it is what makes the
   boxes reachable (see windowhost/HANDOFF §4.1).
3. **Rust window host** — new `src\platform\windows\<tool>window.rs`:
   - Take shared plumbing from `windowhost.rs`
     (`scale_factor`, `screen_scale`, `to_wide`; plus `fixed_style`,
     `timecode`, `fraction` where relevant) — **take, never copy**.
     Under tests, import `windowhost::screenshot::{capture,
     make_test_video}` — do not re-copy them.
   - Keep local: `CARD_W` (your window's own), the unit consts for your
     content stack + `FOOTER_UNITS = 64`, `height_for*` functions, your
     `STATE` (a `thread_local!` RefCell holding the window's facts),
     your `outer_size`, your `window_handle`, timers, the pump loops of
     your capture tests, the window procedure with your OWN hit-testing,
     keyboard walk and message sending (`ToolEvent` via the shared
     channel pattern).
   - The window sizes once from what it holds (width fixed = CARD_W,
     height = content units + preview units + footer units, scaled by
     DPI), follows `WM_DPICHANGED`, fades in on its own timer id.
4. **Dispatch** — `src\app.rs` `open_tool_window()` maps the op id to the
   new window; `tools.crop` used this as the relay pattern (the window is
   opened once per run request; the send side speaks `ToolEvent`).

### Phase D — Proof

- Unit pins, in `mod tests` of the window file (see existing ones):
  stack heights (`height_for*`), the layout stacks top-to-bottom,
  hit-test answers, timecode/fraction edges (already in windowhost),
  keyboard-walk rules that are pure.
- Pump tests (`show_tests`): making the window, showing + pumping
  messages must not panic — these run on every `cargo test`.
- **Capture test** (the parity proof): an `#[ignore]` test in
  `show_tests` that makes the real media (`make_test_video` or your own
  image maker), opens the REAL window, pumps until the fade is done,
  then writes BOTH `target\shots\<tool>-buffer.png` (the renderer's own
  buffer — catches pure-drawing bugs) and `<tool>-window.png` (the
  system-composited window — catches chrome/rendering bugs). Use the
  shared `windowhost::screenshot::capture` (PrintWindow with
  `PW_RENDERFULLCONTENT`, GetDIBits, B/R swap).
- **Then you personally read** the generated png AND the reference webp
  (both via the read tool) and record the comparison in todo.md —
  parity and deliberate differences. This step is yours, not the tests'.
- Full suite: `cargo test` → quote exact numbers.
- **Never re-take an existing baseline without stashing hashes first**
  (see §4). Baseline shots are evidence; a silent change is a regression.

### Phase E — Docs

- `todo.md`: tick checkboxes, finish the deliberate-differences list.
- `HANDOFF.md`: §3 chain + reviewable range, §4 conventions if something
  new was learned, §5 DONE box + next batch, §6 counts, §7 gotchas for
  anything hard-won.
- `PLAN.md`: baseline table row + the test-count line.
- One `docs:` commit. Tree clean, nothing pushed.

---

## 2. The hard rules (break these and the window lies)

1. **Measured instances or nothing.** Every sub-component instance pinned
   `x:0; y:0; width; height` at its instantiation site (rule above).
2. **Slint setters exist only for the ROOT component's in-properties.**
   To reach a sub-component property (e.g. `CardFooter.run-enabled`),
   route it through root in-properties/Panel plumbing.
3. **Never re-enter a borrowed thread-local.** When a window proc /
   callback borrows `HOST` (or `STATE`), take the geometry you need as
   plain arguments to the helper; calling back into the same
   `thread_local` borrow panics and was crop's hardest lesson.
4. **Never hold `HOST` while dispatching slint events.** Clone
   `Rc<MinimalSoftwareWindow>` out of the host, drop the borrow, then
   dispatch. Slint event handlers can re-enter your window code.
5. **The card only draws; the window reads the pointer.** All
   interaction logic (hit-testing, dragging, keyboard) lives in the
   window's window procedure, holding the same geometry the card was
   given. Do not split the truth between card and window.
6. **One accent = chosen.** Orange `#ff5a1f` family marks choice/hover,
   never danger. Tokens are pinned by `ui/theme.rs` tests; change both
   places or neither.
7. **The strip/thread pattern for long work** (filmstrip, waveform,
   thumbnails): many queued seeks answered behind a channel, worn by the
   window's timer (40 ms); NEVER spawn or block on the draw thread;
   opening the window again replaces the inbox so late results land in a
   closed channel harmlessly.
8. **Comment and commit voice.** Plain-English house dialect
   ("the strip is worn with frames", "the moments are said in the face
   where every digit takes the same room"). It is part of the review
   bar; read a few files before writing yours.
9. **PowerShell quirks.** No `&&` (use `;`); `Select-String` exits 1 on
   no-match and stderr redirections can lie about cargo's exit — when a
   command's success matters: `cmd > $null 2>&1; Write-Output
   "EXIT=$LASTEXITCODE"` and trust `test result:` lines, not pipe noise.
10. **Never claim tests passed unrun.** Counts come from the runner line,
    verbatim: "358 passed; 0 failed; 36 ignored" is the current hand state.
11. **Manual verification stays human.** Drag → wheel → tool → run is
    checked by a person, never ticked programmatically.

---

## 3. Verification protocol (use for every batch)

```
cargo check --tests        # exit 0 before anything else
cargo test                 # quote the "test result:" line
cargo test capture -- --ignored --nocapture   # regenerate shots
```

Hash-stash trick (do it BEFORE touching window files):

```
$before = Get-ChildItem "target\shots\*.png" | ForEach-Object {
    "$($_.Name) $((Get-FileHash $_.FullName -Algorithm SHA256).Hash)" }
$before | Out-String | Set-Content "$env:LOCALAPPDATA\Temp\opencode\shots-before.txt"
```

…then compare after: every baseline png must be byte-identical unless you
*intended* to change that window's pixels. `bisect-*`, `trim-after*`,
`crop-strip/footer/artifact` are old debug artifacts — ignore them.

`target\shots\<tool>-buffer.png` = what our renderer drew (pure card
truth); `<tool>-window.png` = what Windows composited (real chrome). Both
must match the mockup; read both.

---

## 4. What is left, in order (18 of the reference's 22 tools)

Done: trim (B-10.4), split (B-10.6), crop video + crop images (B-10.7), compress + edit metadata (P3-9 / B-10.8), audio trio (P3-10 / B-10.9: trim audio, normalize volume, convert audio channels).
Remaining, in the agreed batch order:

| Batch | Tools | Backends | New controls to build |
|---|---|---|---|
| **P3-11 NEXT** | Redact video, audio (bleep), photos | — | LaneClip editor |
| P3-12 | Save video frames (snapshots) | — | FrameChip |
| P3-13 | Join videos, Merge PDFs | merge **done** | ItemRow list/reorder |
| P3-14 | Organize PDF pages | — | PageCard grid |
| P3-15 | Annotate photos, Add background, Create collage (+ Edit photos family) | — | canvases/swatches — biggest, last |
| loose | Change video speed, Audio visualizer | — | pin their batch when their evidence is read |

Swift authority per batch: `NativeApp/Sources/Tools/…` +
`Operations/MetadataOps.swift` (metadata), `CompressToolView.swift`,
`AudioToolViews.swift`, `SnapshotsRedactVideoToolViews.swift`,
`PDFToolViews.swift`, `CropSpeedJoinVideoToolViews.swift` (speed + join),
`EnhanceVideoToolView.swift`, `RedactImageToolView.swift`,
`EditImageToolView.swift` + `ImageCanvas.swift` + `ZoomableView.swift`.

Two Windows-originals (`tools.rotate`, `tools.resize`) are NOT reference
tools — they run via the plain/generic cards and need no batches.

Registry note: compress + metadata are ALREADY among the 8 implemented
wheel ops, so P3-9 changes no registry/menu pins. If a batch ever adds an
op, three places must be updated together (registry IMPLEMENTED test
~line 1507, actions.rs design-menu pin, controller.rs segment count).

---

## 5. Commit & authorship discipline

- Conventional Commits (`feat:`, `refactor:`, `docs:`), author
  **Ravi <ravirrajpatil@gmail.com>**, **never push**, merge only when
  the user says. Small, reviewable commits: backend → window → docs.
- Every batch records its reviewable range in HANDOFF §3
  (`git diff <docs-hash>..<code-hash>`), so a reviewer reads code only.
- Current head after the windowhost refactor:
  `0c67b51` (docs) ← `3352a14` (refactor) ← `2f29107` (crop docs).

## 6. When stuck

- The three finished windows are the working examples: trim/split share
  `timelinewindow.rs`, crop is the newest, compress/metadata will be the
  plain-card form family. Copy the SHAPE, take the shared plumbing from
  `windowhost.rs`, and keep each window's own geometry in its own file.
- If a decision is the user's (sizes, chrome, meaning of a control),
  ask with options + a recommendation; do not silently decide.
- If evidence conflicts (mockup vs Swift vs spec), the decision rule used
  so far: behavior from Swift, layout from the mockup, and the conflict
  is resolved per tool with the user and recorded in todo.md.
