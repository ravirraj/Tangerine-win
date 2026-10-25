# Open Tangerine for Windows — Todo List

## Status Key
- [ ] Not started
- [x] In progress
- [ ] Completed

---

## Phase B-9: Missing Format Engines

### B-9.1 HEIC Input/Output (BLOCKED)
- [x] Assess libheif/libde265 Windows static build feasibility — NO MSYS2 packages available
- [ ] Decision: Document as blocked until pre-built libs available
- [ ] Add to ffmpeg/components.txt when libs available
- [ ] Add convert.heic registry operation (implemented: false until backend available)

### B-9.2 AVIF Input/Output (BLOCKED)
- [x] Assess libavif/dav1d Windows static build feasibility — previous build failed, no MSYS2 packages
- [ ] Decision: Document as blocked until pre-built libs available

### B-9.3 SVG Input (DONE 2026-10-24, corrected after 0ac9064)
- [x] Assess resvg integration (vendored via Slint 1.18)
- [x] Add svg to ffmpeg/components.txt (read = -, input-only seam)
- [x] Create src/media/svg.rs (resvg rasterise to PNG seam)
- [x] Fix: SVG is INPUT ONLY — removed bogus "convert.svg" target op
      (nothing can write SVG; registry/controller/actions tests reverted to 15)
- [x] Fix: rasterised PNG staged in its own `.staging.png` file, never the
      same path as FFmpeg input and output; staging removed on all paths
- [x] Fix: `.svg` classified as FileKind::Image in selection.rs — the wheel
      now offers PNG/JPEG/… for SVG drags (previously classified Unknown,
      so SVG input never reached the converter at all)
- [x] Unit test: an_svg_is_rasterised_and_converted (convert.rs)
- [x] Real test: SVG→PNG and SVG→JPEG with real FFmpeg (real.rs)
- [ ] Manual verification: drag a real .svg in Explorer, wheel offers image
      conversions, output looks right (needs the running app)

### B-9.4 PDF Support
Engine decision (2026-10-24): **PDFium, not poppler** — verified.
- [x] Assess engine: pdfium-render crate (v0.9.4) renders pages to bitmaps
      AND extracts text; prebuilt pdfium.dll ~7 MB (chromium/8111) from
      bblanchon/pdfium-binaries; Apache-2.0/BSD. Poppler rejected (C++ MSYS2
      build — the HEIC rabbit hole).
- [x] Vendor pdfium.dll: vendor/pdfium/ (gitignored); ffmpeg/package.sh
      copies it beside the exe with licence + NOTICE when present
- [x] Create src/media/pdf.rs: bind-once seam (library() lookup beside exe/
      cwd/vendor), render_pages(dpi) -> PNG/JPEG bytes per page,
      extract_text; DPI = scale on the 72-pt page
- [x] PDF input ops: convert.pdf-png / pdf-jpeg / pdf-txt for FileKind::Pdf,
      routed through the documents handler at 300 DPI; every page becomes
      its own file (name, name (2), ...), nothing overwritten
- [x] Honest gating: probe exposes a "pdf" processor ONLY while the DLL is
      found — the wheel never offers PDF conversions it cannot run.
      Note: pdf deliberately NOT in components.txt (it is not an FFmpeg
      component; unlike SVG nothing needs a manifest lookup for it)
- [x] Real tests (ignored, need dll): 2-page render at exact pixel size,
      text extraction, not-a-pdf refused, page naming with collision
- [ ] convert.pdf output side (images -> PDF): PDFium can create PDFs, or
      pdf-writer (~0.5 MB) — pick when B-9.5 touches it
- [x] Manual verification: PDF drag works — wheel offers PNG/JPG/TXT, pages
      render correctly (verified 2026-10-24 on the release build)

### B-9.5 Documents / Slides / Spreadsheets (REVISED 2026-10-24)
Replaces the old "LibreOffice headless" plan. Engine chain, resolved at runtime
in this order — every engine behind one seam in `src/media/documents2.rs`:

    Word/Excel/PowerPoint COM  ->  LibreOffice (if installed)  ->
    office2pdf (optional ~20 MB download)  ->  clear "how to enable" message

**Engine A — Office COM (zero bytes shipped, perfect fidelity): DONE 2026-10-24**
- [x] src/media/documents2.rs — the engine-chain seam
- [x] Word: Documents.Open -> ExportAsFixedFormat (docx AND legacy doc)
- [x] Excel: Workbooks.Open -> ExportAsFixedFormat (xlsx, xls, ods, csv)
- [x] PowerPoint: Presentations.Open -> ExportAsFixedFormat (pptx, ppt)
- [x] Driven via PowerShell (-ComObject), paths passed in environment
      variables — no path ever inside a script; detection via reg query
      HKCR\<ProgId> (fast, ~20 ms)
- [x] Registry: convert.docx-pdf / convert.xlsx-pdf / convert.pptx-pdf,
      'office' processor present only for apps actually installed
- [x] Real tests green on this machine: Word exported a fixture DOCX to
      PDF, Excel exported a fixture XLSX to PDF (pptx test skipped — a
      valid minimal PPTX fixture is disproportionately complex)

**Engine B — LibreOffice (detected, never required, never bundled): DONE**
- [x] Detect soffice.exe in standard locations; headless
      --convert-to pdf into a staging folder, then moved beside original
- [x] Serves the same 'office' processor reads when Office is absent
- [ ] Verified on a machine that has LibreOffice (none here — self-skips)

**Engine C — office2pdf for users with no MS Office (optional download):**
- [x] Evaluated on this machine (v0.9.1, 2026-10-24):
      - Windows x64 binary is 68 MB — not the ~20 MB once guessed
        (Typst engine + bundled fonts)
      - Converts a genuine Word-made DOCX to a valid PDF (%PDF-1.7)
      - Font substitution observed: Word's default "Aptos" -> "Noto Sans";
        layout can shift from what Word shows
      - PDF as input is rejected by the tool (PDFium covers that side)
      - DECISION: not bundled for now — Engine A (Office COM) covers most
        users at zero size; keep the binary in vendor/office2pdf/
        (gitignored) and revisit if/when the no-Office path matters more
- [ ] If ever bundled: optional in-app download (~68 MB, user consent),
      stored under %LOCALAPPDATA%\OpenTangerine\engines\; PPTX->JPG
      without Office = office2pdf -> PDF then PDFium render

**Native routes (always available, no engine, milliseconds):**
- [x] DOCX->TXT: zip + quick-xml walk of word/document.xml (2026-10-24;
      run text kept verbatim, paragraphs apart; deps: quick-xml)
- [x] XLSX/XLS/ODS/CSV/TSV -> CSV: calamine 0.37; one file per sheet
      (name.csv, name - Sheet.csv), cells quoted as CSV asks, nothing
      overwritten (deps: calamine)
- [ ] TSV/other spreadsheet targets (only CSV output so far)
- [ ] MD/TXT -> PDF via markdown->HTML->PDF (small crates)
- [ ] DOCX->HTML (approximate styling) and -> EPUB (zip of HTML) — later
- [ ] RTF input: assess a Rust RTF reader (reference accepts RTF input;
      RTFD is an Apple bundle, skip)

**Related cheap wins from the website matrix:**
- [x] Manual verification of engines A+B: docx/xlsx/pptx drags offer PDF and
      export correctly on a machine with Office (2026-10-24)
- [x] Images->PDF via pdf-writer 0.15 (2026-10-24): one page per picture,
      page sized to the picture; JPEG embedded whole (DCTDecode); others
      deflated; transparency kept as an SMask; SVG input rasterised first.
      convert.pdf now implemented via the document processor; verified by
      reading the made PDF back with PDFium (page == picture size)
- [x] PDF->TXT — done as part of B-9.4 (PDFium text extraction)
- [x] Many images on PDF -> ONE PDF, a page each, named after the first
      picture (user-requested; previously one PDF per picture) (d045e12)

**Registry operations:** convert.docx (PDF/TXT targets), convert.pptx
(PDF/JPG/PNG), convert.xlsx (PDF/CSV) — `implemented` reflects engine
availability at runtime; wheel never shows a conversion it cannot run.

**Descoped (registered implemented: false, reason stated):**
- PDF -> DOCX/PPTX/XLSX: layout reconstruction, no lightweight offline engine
- PAGES input: Apple-only format, N/A on Windows

### B-9.6 Audio Formats
Reference audio set (homepage): MP3 M4A WAV FLAC OGG Opus AIFF WMA.
- [x] Add wav, ogg, aiff, m4a, opus, wma to ffmpeg/components.txt (+webm
      as video input; mp3 write filled with libmp3lame) — FFmpeg rebuilt
      from source with lame 3.100 / libopus 1.6.1 / libvorbis 1.3.7
      (MSYS2 ucrt64) and ALL build assertions pass; exe 13 MB
- [x] aresample filter added — audio encoders open only after the sound
      is put in the shape they take (found by running the real command)
- [x] Registry ops: convert.mp3 (Video AND Audio categories — video sound
      extraction), convert.wav/.ogg/.opus/.aiff/.m4a/.wma; new Music icon
- [x] probe: processors divided by kind (image/video/audio) — also FIXED
      phantom ops: convert.mp4 required a "video" processor and
      convert.webp a "webp" processor that never existed, so video/webp
      conversions were never offered by the wheel until now; svg added to
      image reads (svg drags were never offered either)
- [x] command.rs: audio detected by ENCODER not muxer (asf holds wmv and
      wma); no white-matte filter on audio conversions; ipod muxer
      writes to a file
- [x] Real-FFmpeg tests green: hand-crafted WAV -> mp3/ogg/opus/aiff/m4a/
      wma/flac/aac (magic verified, incl. ID3-headed mp3), and m4a -> mp3
      as the sound-extraction path. 308 unit + 27 real, 0 failures.
- [x] Manual verification passed (2026-10-24): audio drags offer targets, video offers MP3

### B-9.7 RAR Input
- [ ] Assess unrar/7z feasibility
- [ ] Add rar to ffmpeg/components.txt (read = -)
- [ ] Create external tool invocation
- [ ] Add registry operation (implemented: false until unrar detected)

---

## Phase B-10: Tool Windows (the reference product IS its windows)

### User's design notes (notes.md, 2026-10-25) — address in the B-12 UX pass
- [ ] Wheel transition animation is sloppy — tighten the state-change
      animation (ui/animation.rs)
- [ ] The wheel looks too big — try a smaller disc radius (DISC_SIZE /
      geometry.rs), compare against reference screenshots
- [ ] Fonts and segment icons look off — audit typeface/size and the icon
      stroke paths against the reference (radial/icons.rs, wheel.slint)

Verified against tangerineformac.com/tools: every one of the 21 tools opens
as a DEDICATED CONFIGURATION WINDOW (preview, controls, Cancel/action).
Our tools currently run immediately with hardcoded defaults — without this
phase there is no tool parity, whatever the operations can do.

### B-10.0 Tool window shell (TW-1) — DONE 2026-10-25
- [x] The shell is a real Win32 window (src/platform/windows/toolwindow.rs):
      title/detail, file name, picture preview of the first dragged image
      (decoded + thumbnailed via the image crate, SVG rasterised first),
      width/height fields for the tools that take a size (resize for now),
      Cancel + Run (Enter runs, Esc closes), DPI-aware layout. Native
      controls — keyboard/focus/accessibility are Windows' own.
      NOTE: the app's Slint setup is a custom software platform for the
      wheel overlay, so a second Slint window would mean hand-rolled input
      plumbing; a native window is the honest seam. Visual polish (comctl6
      manifest) is a later pass.
- [x] Picking any tools.* segment opens the window instead of running
      immediately (app.rs); the window's answer comes back through the
      message channel and runs on a worker like everything else
- [x] execution::Request carries ToolSettings (width/height); tools.resize
      uses them: scale=W:H, one side alone keeps the shape, no window =
      the old half-size default
- [x] Tests: resize filter shapes, number parsing; 312 unit + 27 real green
- [x] Manual verification: drag an image → Tools wheel → Resize → window
      with preview; enter width → Run → resized file appears (the audit
      below opened, sized, focused and keyboard-walked the window; the
      polish pass re-verified with the pump test)

### B-10.0.1 TW-1 polish pass — make the window look designed (audit 2026-10-25) — DONE 2026-10-25 (1-9)
Side-by-side with the 21 reference windows (Screenshots.rar, extracted to
%TEMP%\ref-ui). The shell is right — preview-first card, Cancel/Run, Enter
runs, Esc closes — but the skin reads as a default Win32 form wearing two
rounded rectangles. Ordered by payoff: items 1-9 are small diffs, 10 is the
structural one.

- [x] 1. DPI sizing — created size now scales by the display factor
      (screen_scale() before CreateWindowExW); the card stretches its
      words with a ui-scale property the host feeds the same factor.
- [x] 2. One header — tool name in the native title bar only; first
      in-window line is the file name (15px semibold, left) with the
      detail line (12px dim) under it, hairline below the header. All
      content left-aligned.
- [x] 3. Fields — persistent "Width"/"Height" labels with a dim "px"
      suffix drawn by the card inside white 1px-#d0d0d0 6px-radius tiles;
      the real EDIT (ES_NUMBER, left-aligned) is laid between label and
      unit; the tile border turns accent on EN_SETFOCUS (bool into the
      card); cue banners and ES_CENTER dropped.
- [x] 4. Keyboard — Tab/arrows (and Up/Down) cycle fields → Cancel →
      Run via a focus index driven from both window procs; Space/Enter
      activate what the keyboard is on; Esc cancels; focus rings drawn
      (accent tile border, accent ring around Run, accent border on
      Cancel); both buttons get pressed states.
- [x] 5. One token set, one orange — src/ui/tokens.slint is the single
      set (accent #ff8a3d, pressed, surface, canvas, field-border, text,
      text-dim, hairline, outline); the wheel's orange was already
      #ff8a3d everywhere, so the product has exactly one orange.
- [x] 6. Preview — single frame: edge-to-edge in its own 8px-radius tile
      on #f5f5f5, thumbnail rendered at box size x scale (show_for reads
      the laid-out box before making the image).
- [x] 7. Body structure — header / hairline / body / hairline / footer,
      hairlines full-width #e3e3e1.
- [x] 8. Footer — Run 120x40 accent filled (near-black words), Cancel
      96x40 quiet 1px outline, helper "Enter to run · Esc to close" left
      of the buttons.
- [x] 9. Chrome — no maximize box, WM_GETMINMAXINFO clamps 420x460 x
      scale, explicit white class brush + white WM_CTLCOLOREDIT brush (no
      black flicker under dark themes), Segoe UI with DPI-scaled sizes,
      ~100ms fade-in via WS_EX_LAYERED + timer (0→255 in 51-alpha steps).
- [ ] 10. ToolShell component — header / hairline / body slot / hairline /
      footer in one Slint component; every later window (rows, timeline,
      canvas, reorder list) fills the body slot. This is how the reference
      makes 21 windows feel like one product: copy the skeleton, wear our
      own Windows skin — left-aligned, native title bar, hairlines, one
      tangerine orange, visible keyboard focus everywhere. Build during
      B-10.1 so TW-2 is the first window to use it.
      DONE 2026-10-25 as shell.slint, in the shape Slint honestly allows:
      CardHeader / Hairline / FieldTile / CardFooter are shared
      components in src/ui/shell.slint; ToolCard and TrimCard both wear
      them, so the chrome cannot drift between windows. (A true @children
      body slot was not needed: the body is the part that differs, and
      each window's file says only its body. The re-export chain is
      wheel.slint -> tool_window.slint -> trim_window.slint because a
      file may re-export with * only once.)

### B-10.1 Time-range template (TW-2) — the Trim video window — DONE 2026-10-25
- [x] ToolShell extraction (audit item 10, above) — shell.slint is the
      skeleton every tool window wears
- [x] The moments of a video: src/media/frames.rs — duration_of (the
      probe says format=duration) and frame_at (a fast -ss seek before
      the input, one frame, PNG over the pipe; the pipe muxer is
      image2pipe in our build, image2 is not there)
- [x] The trim itself is real now: tools.trim builds its own arguments
      (-ss before the input, -t the kept length, -map 0 -c copy — no
      decoding, no re-encode; without a range it re-muxes the whole
      file). Before this it went through the single-frame conversion
      path, which kept ONE FRAME of the video.
- [x] ToolSettings carries the range (start_ms/end_ms); the plain card
      sends none, the timeline sends what its handles hold
- [x] TimelineWindow (src/platform/windows/timelinewindow.rs): the frame
      at the playhead, the bar with start/end handles, three time codes,
      -1s/+1s step buttons (Shift = 5s), the shared footer. Pressing near
      a handle takes that handle, elsewhere scrubs the playhead (the
      frame is re-drawn when it moves ≥0.25s); Tab/Up/Down walk handles →
      Cancel → Run, Left/Right step, Enter/Space/Esc do what they say.
      Same chrome as the card: DPI-scaled size, no maximize box, min
      size, white brushes, ~100ms layered fade-in. tools.trim is now
      video-only (gated on the video processor) — a timeline needs a
      length.
- [x] Tests: 320 unit + 28 real green; the real test makes a 3s video
      (rawvideo → mpeg4/mov), trims to 1s and checks the length, and
      takes a frame from the middle
- [ ] Manual verification: drag a video → Tools wheel → Trim → timeline
      with frame preview; drag handles or Tab+arrows → Run → trimmed
      file appears

Notes for the windows that reuse this template (Trim audio, Split video,
Save frames, Bleep): the frame seek costs ~50-150ms and runs on the
thread that draws — fine for one frame, but a waveform or a scrub-happy
preview should move to a worker + posted message before shipping.

### B-10.4 P0 shared design foundation + P1 trim window to reference parity — DONE 2026-10-25
Scope per the user's direction: P0 (shared design foundation) and the Trim
Video window only; the other windows keep their skin until said so. Reference:
DESIGN-SPEC.md, MAPPING-ROADMAP.md (P0-1/2, P1-4/5/6) and
tool-windows/trimVideo.webp in reference/tangerine/ (external, read-only).

- [x] P0-1 tokens (src/ui/tokens.slint): the reference palette measured in —
      window-bg #F7F6F4, surface #F1F0EE, elevated #ECECEA, content #FFFFFF;
      preview-black #1A1A1A; text #1D1D1F, text-muted #828782, transport
      #3A3A3A; hairline #E3E3E1, field-border #D0D0D0; danger #E0483E;
      font-mono "Consolas"; radii 24/12/14/10/6. The accent stays the
      application's own #ff5a1f (hover #ff7138, pressed #e94a12, tint 30%) —
      within one step of the original app's #FF5C00, and theme.rs (the wheel)
      already said the same numbers, so the whole product wears one orange.
- [x] Window chrome decided (native first, per the roadmap's cheapest-first
      rule): native frames kept (WS_OVERLAPPEDWINDOW minus thickframe and
      maximize box); the tool's name stands in the native title bar; the
      in-card header says the file's name and what the tool does. The
      reference's frameless look with a circled close top-left is NOT copied
      (deliberate divergence, recorded here).
- [x] THE ROOT OF EVERYTHING-MISSING FOUND AND FIXED: a custom component
      instance placed inside one of these Slint windows without explicit
      geometry draws at a displaced spot — the CardHeader probe drew at
      (+272,+319) instead of (0,0). Fix: pin every shell instance with
      x: 0; y: 0; width: card-width; height: card-height (trim_window.slint
      and tool_window.slint). Verified by probes landing exactly where asked
      (magenta at 0,0; cyan at 124,16) before the probes were removed. This
      had silently broken TW-1/TW-2's committed designs — headers, hairlines,
      tiles and footers never rendered.
- [x] TimelineRange (shell.slint): a real filmstrip under the two orange caps
      — caps 28 wide, full strip height, each carrying a white grip a third
      of it; a thin white playhead standing on the moment; what is cut away
      darkened in the picture's own black (#1A1A1A @ ~55%), not a daylight
      wash. Empty filmstrip, the strip is a plain well.
- [x] The filmstrip is REAL: the vendored FFmpeg build carries no tile/fps
      filters (checked -filters), so a one-pass mosaic is impossible.
      Instead ~23 pillars are asked for one by one with the same fast -ss
      seek the playhead's frame takes (frames.rs pillar_of crops the frame's
      middle to the tile's shape and squeezes it), inside a background
      thread; the finished strip arrives as plain bytes through a channel
      and is worn by the window's timer (40 ms), which also paints the
      window. Opening a new trim replaces the inbox, so a strip of an old
      video can land nowhere.
- [x] Trim card body to the reference's own pattern (trim_window.slint +
      timelinewindow.rs): the picture on the picture's own dark, rounded
      12, no doubled letterbox (the frame keeps its own bars inside);
      caption row "Drag the ends to trim" left and "N selected" mono right;
      the strip 72 units tall (the mockup's 73½ at the content width); the
      moments said once in mono 13px ("Start 0:00.00" / "End 0:04.00");
      the transport as bare glyphs; one quiet action.
- [x] Window sized once from the video's shape (preview clamp 190..300),
      DPI changes followed (WM_DPICHANGED); tests pin the stack numbers
      (282 content + preview + 64 footer).
- [x] DELIBERATE DIFFERENCES from trimVideo.webp, and why:
      - Transport is TWO glyphs, not the mockup's three: ⏮/⏭-style bare
        glyphs nudge the keyboard's handle ±1 s (±5 with Shift), matching
        the original app's frame-stepping; there is no media-player
        backend, and a dead play button was refused.
      - The closing hairline is 2 units nearer the footer than a rounded
        count would put it (the transport row adds 2), invisible at rest.
      - Native title bar (tool name) instead of frameless centered title +
        circled close; helper "Enter to run · Esc to close" kept, matching
        the Apple-original keyboard flow rather than the site mockup.
- [x] Tests green: 324 unit + 29 real-FFmpeg, 0 failures. The capture check
      (ignored) opens the window on a real 3s video, waits up to 12 s for a
      strip to be worn, and writes target/shots/trim-buffer.png (the
      renderer's own buffer) + trim-window.png (the system's composited
      window) — compared by eye and by pixel sampling against
      trimVideo.webp: filmstrip, caps, grips, hairlines, footer all in
      place.
- [ ] Manual verification by a person: run dist, drag a REAL video (landscape
      and portrait), check the strip fills in over ~1-2 s, drag the caps,
      keyboard-walk (Tab/arrows/Enter/Esc), run the trim, confirm the output
      starts/ends where the caps stood.

Notes for the windows that reuse this template (Trim audio, Split video,
Save frames, Bleep): the strip's thread is the pattern to copy — many
seeks behind a channel, worn by the window's timer; never spawn it on the
draw thread. Also: TimelineRange's caps' hit-test still keys on
HANDLE_REACH=16, valid for 28-wide caps.

### B-10.5 Repo ↔ reference traceable mapping (audit round 2026-10-25)

Reconciled against the external `reference/tangerine/MAPPING-ROADMAP.md`
(123 lines, read in full) and `DESIGN-SPEC.md`; the Swift authority is
`reference/extracted/Open-Tangerine-main/NativeApp/Sources/` inside THIS repo.
Every `tools.*` operation in our registry, its reference window, its Swift
source and its state:

| Our op | Reference window (webp) | Swift source | Status |
|---|---|---|---|
| tools.compress | compress.webp (roadmap P3-9) | Compress tool views | Backend done (quality/slider window pending) |
| tools.crop | cropVideo/cropImage (roadmap P3-8) | Crop views | Backend = center-half crop (placeholder); window = plain card, fields only |
| tools.trim | trimVideo.webp (P3-6) | TrimSplitVideoToolViews.swift | DONE B-10.4, at parity |
| tools.split | splitVideo.webp (P3-7) | TrimSplitVideoToolViews.swift | **This batch (B-10.6)** |
| tools.rotate | — (no reference window; fold/rotate lives in editImage there) | — | Windows-original; runs via plain card |
| tools.resize | — (NOT a reference tool; the external mapping routes "resize/crop" to cropVideo/cropImage, P3-8) | — | Windows-original ToolCard (B-10.0), already wears the B-10.4 tokens/shell |
| tools.merge | joinVideos/mergePDF (P3-13) | — | Backend done; reorder-list window pending |
| tools.metadata | removeMetadata (P3-9) | — | Backend partial; window pending |

Evidence for the batch order after Trim: the external roadmap's P2 says the
shared control library (Segmented, Dropdown, Stepper, Checkbox, Slider+ticks,
IconButton, SwatchGrid, ValidationNote) is built BEFORE per-tool bodies, and
its P3 list puts **Split video as item 7** (next after Trim, item 6) because
it reuses TimelineRange + Stepper; Crop video/image is item 8. "Resize" is
NOT one of the reference's 21 windows, so there is no reference window batch
for it: our Resize keeps its working ToolCard (already pinned to the B-10.4
shell) and the reference-parity work it touches lives under Crop (P3-8).
The shared controls Split needs (aStepper with up/down chevrons, ghost-pill
buttons, marker lines on the timeline) are built here and will be reused by
the P3 tools that follow.

### B-10.6 Split Video window (roadmap P3-7) — reference: tool-windows/splitVideo.webp + TrimSplitVideoToolViews.swift

Original behavior (Swift, authoritative): mode `points`/`equal`; points are
added at the playhead only when `0.1 < t < duration - 0.1` and no point stands
within 0.05 s; equal parts computes `duration * i / parts` for `i in 1..<parts`
(parts 2..50); the strip is the SAME TimelineRange track with markers instead
of a dragged range (unusedStart/unusedEnd stay pinned); Save writes segments
`[0] + points + [duration]`, skipping any part shorter than 0.05 s, into a
`"{baseName} clips"` folder beside the original, each clip
`"{base} part N.{ext}"`, stream-copied (no re-encode); progress per segment.
The mockup (webp) carries a few lines the Swift does not: an equal-parts
stepper with a `Set` commit button and "0 split points" count text. Both are
taken (mockup is the visual authority; Swift is the behavior authority).

- [x] Backend: `tools.split` operation (video-only, Batch::One) — splits at
      the points carried in ToolSettings (milliseconds); segments mirror the
      Swift math (0 and duration as the outer boundary, parts < 0.05 s
      skipped); each part is stream-copied by the same trim_args a trim
      takes; the folder is `"{base} clips"` beside the input, made unique
      with " 2", " 3"… if it exists (ConversionMatrix.uniqueDirectory's
      behavior); clips named `"{base} part N.{ext}"`.
- [x] ToolSettings: `points_ms: Vec<u64>`; the window sends explicit points —
      the manual ones exactly as placed, the equal ones computed by the
      window from parts — so the backend stays small and the registry row
      does not need a second op.
- [x] Shared controls (shell.slint): PillButton — the quiet pill the footer
      buttons are, one pressable with a label (and glyph) any body row can
      take; TimelineRange gained `markers` (accent-coloured split markers on
      the strip) and `has-range` (a split has no dragged range: no caps, no
      dimming outside a stretch).
- [x] Timeline window hosts the split card (timelinewindow.rs): the same
      shell, exposure, filmstrip thread and transport; a split layout takes
      its own stack — points row ("N split points" + Add Split pill),
      the strip with markers, the transport, the equal row ("Equal clips" +
      parts stepper + Set pill) — and the footer's Run says "Split Video"
      (the mockup's label). Trim layout and behaviour unchanged (height tests
      pin 582/646 as before; the split stack has its own pinned heights).
- [x] Interaction: Add Split at the playhead (the Swift guards); the
      stepper 2..50 (Swift's range), `Set` replaces the points with
      N-1 equal ones; clicking a marker seeks; Delete removes the nearest
      point within 0.5 s of the playhead (documented Windows replacement for
      the Swift per-point trash rows — the mockup has no list); keyboard
      walk: playhead → Add Split → Set → Run (Tab/Up/Down), arrows scrub,
      Enter runs, Esc closes.
- [x] Dispatch: app.rs routes `tools.split` to the timeline window like trim;
      the plain card keeps the other tools.
- [x] Tests green: 330 unit + 31 ignored (real-FFmpeg operations and the two
      window captures), 0 failures (the split stack's
      layout pins, the segment-math and folder-naming unit tests, the real
      3 s video split into "three clips" with the right lengths, and the
      registry/menu pins updated — tools wheel now 8 segments). The capture
      check writes target/shots/{split-buffer,split-window}.png, compared
      against splitVideo.webp: preview, points row + Add Split, markers on
      the strip (quarters for four parts), transport + timecode, the
      Equal clips stepper with Set, and the enabled Split Video footer all
      in place; trim-window.png retaken and unchanged from the B-10.4 shot.
- [ ] Manual verification: drag a real video → Tools wheel → Split → add
      points, Set equal parts, run; folder of numbered clips appears
      beside the original; trim window unchanged.

Deliberate differences from splitVideo.webp, and why:
- Native title bar ("Split") instead of the mockup's frameless header — the
  recorded B-10.4 chrome decision, kept for every tool window.
- Footer runs "Split Video" always; Swift swaps the title to "Split into N
  clips" — our footer label is static, the count is said in the points row.
- The mockup's two transport glyphs are ours from B-10.4 (Step ±1 s, Shift
  ±5 s); the Swift adds none for split, do the same job here.
- The Add Split pill says its words only — the mockup draws a scissors on
  it — because at 14 px the wheel's own cut mark would not read; the label
  is the affordance, per the shared PillButton.
- No per-point rows with a trash button (Swift) — the mockup shows no list;
  markers + click-to-seek + Delete stand in for it, and "N split points"
  says what exists.
- The equal caption "Each clip lasts about X" (Swift) is said by appending
  "· each about T" to the points row after Set, instead of a second caption
  line the mockup does not carry.

### B-10.2 FFmpeg-backed windows (1-3 days each once template exists)
- [ ] Change video speed (setpts/atempo, preserve pitch)
- [ ] Split video (custom points or equal parts, one output folder)
- [ ] Save video frames (frame-step picker -> full-resolution PNGs)
- [ ] Join videos (file list reorder -> single MP4; our merge, windowed)
- [ ] Crop video (exact dimensions, keep audio)
- [ ] Crop images (pixel dims + aspect presets; replaces hardcoded center-crop)
- [ ] Compress (target exact file size, resize while compressing)
- [ ] Edit metadata (inspect/change/remove; search by file/track/chapter)
- [ ] Normalize volume (loudness range + true peak, in/out comparison)
- [ ] Convert audio channels (mono/stereo, per-channel volume + preview)
- [ ] Trim audio (waveform via showwavespic, trim silence at ends)
- [ ] Merge PDFs (file list reorder) — gated on B-9.4 poppler
- [ ] Windows for our own Rotate/Resize ops (not in reference set, keep)

### B-10.3 Heavy/custom windows (assess each before committing)
- [ ] Edit photos (live preview: dehaze, clarity, grain, denoise, light/
      color/detail — FFmpeg eq curves get partway; assess fidelity)
- [ ] Add background (canvas, blur, rounded corners, shadow — pad+gblur)
- [ ] Create collage (grid/row/column/featured layouts — Rust compositing)
- [ ] Audio visualizer (animated waveform -> MP4, orientation options)
- [ ] Bleep audio (movable time ranges, original-vs-bleep comparison)
- [ ] Redact photos / Redact video (movable solid/blur/pixelate boxes)
- [ ] Organize PDF pages (rotate/duplicate/remove pages — poppler)

### Reference notes
Headline says "26 file tools" but /tools documents 21 windows. Old todo's
"19 of 26 missing" count was wrong; this section is the verified list.

All 21 reference windows analysed 2026-10-25 (screenshots in %TEMP%\ref-ui):
one skeleton — header (close + small centered title) / hairline / body /
hairline / footer (primary action right). The bodies reduce to three
archetypes plus two specials, which map 1:1 onto our templates:
- Form rows (label left, control right, caption under): Normalize volume
  (3 LUFS sliders), Compress, Change speed, Convert audio channels
  (mono/stereo + L/R sliders), Convert format, Edit metadata, Rotate
  (row of 4 icon buttons) — a rows template covers ~7 of B-10.2.
- Timeline (filmstrip or waveform + range handles + transport): Trim
  video/audio, Bleep video/audio (adds a chip lane), Redact video (lanes
  list per box) — the B-10.1 time-range template, reused four-plus times.
- Canvas (preview + overlay boxes): Crop image/video, Redact photo, Edit
  photo, Annotate, Add background, Collage, Organize PDF (thumbnails) —
  the B-10.3 heavy work.
- Specials: reorder list (Merge PDFs — document rows with up/down
  chevrons; Join videos — thumbnail order list) and a style picker
  (Audio visualizer — preview cards + option rows).
Bonus captures from overlapping page sections: Join videos (stacked
thumbnail order list) and Rotate video (rotate L/R, flip H/V icon buttons)
— both on our B-10.2 list, reference detail already in hand.
Our windows deliberately do NOT copy the Mac skin (circular close
top-left, centered titles, 16px-radius floating cards, SF Pro): native
title bar with close top-right, left alignment, <=8px radius, Segoe UI
Variable, one tangerine orange. Keyboard paths for every drag interaction
the reference only offers by mouse (see B-12.3).

---

## Phase B-11: Windows-Specific File Actions

### B-11.1 Terminal (already implemented)
- [ ] Verify cmd.exe opens at correct folder
- [ ] Add real test

### B-11.2 Extract (already implemented)
- [ ] Verify ZIP/TAR/GZ extraction
- [ ] Add real test

### B-11.3 Nearby Share / Smart Share (replaces AirDrop)
- [ ] Assess Windows share contract API
- [ ] Implement ShellExecuteExW with "share" verb or Explorer fallback
- [ ] Add actions.nearby-share registry operation
- [ ] Add test

### B-11.4 Copy Path / Duplicate / Rename / Reveal (already implemented)
- [ ] Verify all work correctly
- [ ] Add tests if missing

---

## Phase B-12: Interaction & UX

### B-12.1 Progress UI
- [ ] Create src/ui/progress.rs
- [ ] Create src/ui/progress.slint
- [ ] Integrate into conversion flow
- [ ] Show progress during multi-file batch

### B-12.2 Cancellation
- [ ] Add CancellationToken to Converter
- [ ] Propagate to FFmpeg subprocess (kill on cancel)
- [ ] Add Cancel button to progress UI
- [ ] Add test: cancel mid-conversion

### B-12.3 Escape / Keyboard Navigation
- [ ] Add Escape key handling in controller
- [ ] Add Enter/arrow key navigation in wheel
- [ ] Add tests

### B-12.4 Auto-retract on Success
- [ ] Add setting: auto-retract wheel after conversion
- [ ] Implement auto-close logic

### B-12.5 Sound Toggle
- [ ] Assess sound library size
- [ ] Decision: implement or defer
- [ ] If implement: add settings toggle + click/completion sounds

---

## Phase B-13: Windows Integration

### B-13.1 System Tray
- [ ] Create src/platform/windows/tray.rs
- [ ] Implement Shell_NotifyIcon
- [ ] Tray icon click shows/hides wheel
- [ ] Right-click tray: Settings, Exit

### B-13.2 Launch at Startup
- [ ] Create src/platform/windows/shortcut.rs
- [ ] Registry HKCU\...\Run entry
- [ ] Settings toggle

### B-13.3 Windows Notifications
- [ ] Create src/platform/windows/notifications.rs
- [ ] Toast notification on conversion complete/failure
- [ ] Settings toggle

### B-13.4 Right-Click Explorer Integration
- [ ] Document registry shell extension
- [ ] Manual installation instructions

### B-13.5 Nearby Share / Smart Share
- [ ] Implement ShellExecuteExW with "share" verb
- [ ] Add actions.nearby-share registry operation

---

## Phase B-14: Settings UI

### B-14.1 Settings Storage
- [ ] Add serde + serde_json dependencies
- [ ] Create src/settings.rs (read/write %APPDATA%\Open Tangerine\settings.json)
- [ ] Default values for all settings

### B-14.2 Settings UI
- [ ] Create src/ui/settings.slint
- [ ] Appearance (light/dark/system)
- [ ] Modifier keys for each wheel
- [ ] Output location
- [ ] Auto-retract toggle
- [ ] Sound toggle
- [ ] Launch at login toggle
- [ ] Show notifications toggle

### B-14.3 Wire Settings to App
- [ ] Load settings on startup
- [ ] Save settings on change
- [ ] Apply modifier keys to wheel activation

---

## Phase B-15: Packaging & Release

### B-15.1 Repackaging
- [ ] Update ffmpeg/package.sh for all new DLLs
- [ ] Rebuild dist\OpenTangerine\open-tangerine.exe
- [ ] Include all licenses

### B-15.2 Installer
- [ ] Evaluate Inno Setup or NSIS
- [ ] Create installer script
- [ ] Test clean Windows 11 install

### B-15.3 Code Signing
- [ ] Document code signing requirements
- [ ] Optional for personal use

---

## Deferred Items (Not Implemented)

| Item | Reason |
|---|---|
| Background removal | Not in the reference tool set, and ONNX ~50-100 MB too heavy |
| QR reading | Not in the reference tool set, and needs a barcode library |
| PDF -> DOCX/PPTX/XLSX | Layout reconstruction — no lightweight offline engine |
| PAGES input | Apple-only format, irrelevant on Windows |
| Bundling LibreOffice | ~700 MB — detect if present, never bundle (B-9.5 chain) |
| Bundling office2pdf in installer | Optional ~20 MB download instead, user consent |
| AirDrop | No Windows API — use Nearby Share fallback |
| Code signing | Optional for personal use |

---

## Progress Tracking

Update this file as each item is completed. Mark with [x] when done.

**Current branch:** feature/image-conversion
**Phase branches:** phase/b-9-*, phase/b-10-*, etc. (create as needed)
**Merge to master:** Only after user approval at the end

---
