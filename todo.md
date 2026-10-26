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

### B-10.7 Crop video + image (roadmap P3-8) — reference: tool-windows/cropVideo.webp + cropImage.webp + CropSpeedJoinVideoToolViews.swift / CropImageToolView.swift / Shared/ImageCanvas.swift

Original behavior (Swift, authoritative — read in this order, video first):
both tools share one canvas — drag empty space draws the rect, dragging
inside moves it (clamped to the media), dragging near a corner (12 px in
display space) resizes that corner, and an aspect preset constrains
`height = width / ratio` while drawing or resizing. The rect starts at a
10 % inset of the media, integral. Presets both tools:
Free (no ratio) · Original (−1, the media's own) · Square · 4:3 · 3:2 ·
16:9 · 9:16 · 3:4 · 2:3 · 5:4.
- VIDEO: the frame follows a scrub bar (TransportBar); the sidebar says
  "Crop area: W × H at X, Y" and "Video: W × H"; Output dimensions has a
  "Same as crop area" toggle (default on) over Width/Height fields that
  are disabled while it holds; "Audio is kept as is. Output is MP4
  (H.264)". Save: x/y rounded, w/h rounded DOWN to even (and output dims
  likewise), `crop=W:H:X:Y` (+ `scale=W:H` only when the toggle is off),
  audio copied; enabled when the rect is wider and taller than 8.
- IMAGE: same editor on a checkerboard, zoomable; "Dimensions" Width and
  Height are two-way (typing resizes the rect, kept inside the image),
  "Original: W × H" under them, and Center + Whole image buttons; saved
  at full resolution in the input's own format; enabled at 1×1.

- [x] Plan (this section), evidence read in the order above — Swift
      behavior before the mockups; conflicts resolved toward Swift and
      listed below.
- [x] Backend (committed `5f9f3a5`): ToolSettings grows `crop_x/crop_y/crop_w/crop_h` (the
      source rectangle; `width/height` double as the optional output
      scale, exactly as resize uses them). `crop_args` builds the video
      path — `crop=W:H:X:Y` (+ `,scale=W:H` when the window says a size),
      `-c:a copy`, the target's own encoder/muxer — with w/h even-rounded
      defensively. The image path is Rust-native: decode, crop, optional
      resize, save in the input's format; svg refuses (raster crop of a
      vector would quietly change what the file is). The audit fixed two
      things on the way in: results publish beside the original (the
      finals are looped through the same naming the other tools take),
      and rotate/resize stay `[.image]`-only as Swift pins them — the
      earlier widening back to video was reverted.
- [x] Registry (committed `5f9f3a5`): `tools.crop` categories widen to Image + Video; a new
      `Requirement::AnyProcessor(&["image", "video"])` says what crop
      actually needs (the image path needs nothing, the video path needs
      the program).
- [x] Window (committed `a4ad5bb`): a dedicated crop window (cropwindow.rs + crop_window.slint)
      — crop is timeline-neutral, and trim/split stay untouched. The
      layout-twin pattern as everywhere: the stage shows the fitted media
      with the rect over it (create/move/corner-resize hit-testing in
      stage coordinates, aspect constraint), preset chips in two rows, the
      crop-area caption, the video's scrub row (slider + step glyphs +
      timecode, the frame from the same fast seek the trim uses), the
      image's Center/Whole buttons, and a footer whose Reset puts the
      10 % rect back and whose Run is enabled by the Swift rule. A
      lesson the crop badge pins for keeps: every sub-component instance
      that places itself by its box needs `x/y/width/height` at the
      instance, the way the shell's pills do — one left unmeasured is
      centred by the compiler and lands nowhere.
- [x] Dispatch (committed `a4ad5bb`): app.rs routes `tools.crop` to the crop window (video and
      image alike) on its own relay; the plain card keeps the rest.
- [x] Tests green: 355 unit passes, 0 failures, 36 ignored (the real-FFmpeg
      credits and the captures). The crop stack, the fit/clamp/aspect
      math, picking (corner reach/move inset/draw-otherwise), corner
      anchoring under a ratio, the drop rules, `crop_args` shapes
      (filter string, even rounding, audio copy) and the registry/menu
      pins (the wheel stays 8 segments); real-FFmpeg: a 3 s video
      cropped to an asked rectangle with the output dimensions probed,
      and an image cropped with its dimensions read back; a window test
      presses the Square chip through the card's own events and the
      rectangle comes back square.
- [x] Captures: crop-video-window.png against cropVideo.webp and
      crop-image-window.png against cropImage.webp — stage with the
      orange rect and its four corner squares, the label and the two
      rows of ten chips, the words, the video's scrub row / the image's
      Center + Whole image, the Reset and Crop footer, all in place;
      trim-window.png and split-window.png retaken and unchanged
      (shell.slint, timelinewindow.rs and trim_window.slint untouched in
      git).
- [ ] Manual verification: drag a video → Tools wheel → Crop → draw the
      rect → Run; drag an image → the same; both results beside the
      originals.

Deliberate differences from the mockups, and why:
- Native title bar ("Crop") instead of the frameless centred header — the
  B-10.4 chrome decision every window keeps.
- Swift writes video crops as MP4 whatever the input; we keep the input's
  own container (BesideAsInput) — the tool convention since B-8.
- Swift names the result "{base} cropped.{ext}"; we publish beside the
  original with collision Rename — the same B-8 convention.
- Ten Swift presets shown as a wrapping pair of chip rows; the mockup
  draws six in one row — the Swift list wins.
- Four corner handles (Swift's); the mockup's edge-midpoint pills are not
  drawn — the corners are what the behavior knows.
- The checkerboard and the pinch-zoom canvas (Swift ZoomableView) become
  a fitted stage on the card's own background — zoom is later polish, and
  nothing in the crop math needs it.
- The video scrub is a plain slider the window draws itself, not the
  shared Slider control — that control is built when P3-9 first needs it,
  per the handoff's note on speculative controls.

### B-10.7.1 The window hosts share their plumbing (the approved refactor between batches)
- [x] `src\platform\windows\windowhost.rs` is written and registered in
      `mod.rs`: `scale_factor`, `screen_scale`, `to_wide`, `fixed_style`,
      `timecode`, `fraction` and — under `#[cfg(test)] mod screenshot` —
      the tests' `make_test_video` + `capture`, taken once.
- [x] toolwindow imports three of them; timelinewindow and cropwindow
      import all six plus the screenshot helpers and lost ~170 duplicate
      lines each. Kept local on purpose: geometry and unit consts, cards,
      STATE, timers (51/52/54 differ per window), `outer_size`,
      `window_handle`, the capture tests' own pump loops, and crop's
      `make_test_image`.
- [x] Proof: `cargo check --tests` exit 0; `cargo test` **358 passed,
      0 failed, 36 ignored** (windowhost adds three pins); the four window
      shots and three buffer shots were regenerated through the shared
      helpers and are byte-identical (SHA-256) to the pre-refactor set.

### B-10.8 Compress + Edit metadata (roadmap P3-9) — reference: tool-windows/compress.webp + removeMetadata.webp + CompressToolView.swift / Compressor.swift / MetadataToolView.swift / MetadataOps.swift

Original behavior (Swift, authoritative — read 2026-10-25):
- COMPRESS: file rows (icon, name, "size · saves as …"); Compression
  preset with two cases, Balanced (default) and Strong, and a hint per
  side ("Noticeably smaller with little visible change." / "Smallest
  output; quality loss is visible."). Video-only rows when any video
  (gif excluded) is in the batch: codec H.264 | HEVC with its hint,
  resolution Original/4K/1080p/720p/480p by the long edge (never
  enlarging), frame rate Original/60/30/24. A "Target file size"
  toggle; when on, a numeric field + KB | MB + "per file", and the
  hint says videos and audio take a bitrate from the duration and PDFs
  get their pictures recompressed. Footnote: N files, total size,
  originals untouched, copies beside. The numbers the presets carry:
  image quality 0.72 / 0.5, video bitrate factor 0.6 / 0.35 (HEVC
  ×0.65), audio bitrate 128k / 64k; strong PNG shrinks the long edge
  to 0.7 when shrinking is allowed. A target size is searched for:
  pictures binary-search the quality between 0.15 and 0.95 within 12
  tries and shrink ×0.8 when nothing fits; transparent PNGs shrink
  ×0.8 up to 12 times; video takes bitrate = bytes·8·0.94 / duration −
  min(audio, 128k), floor 150k; audio clamps 32k…320k. Output format
  rules: png stays png only with transparency, else jpg; bmp/tiff/svg
  become jpg; webp/avif keep when encodable; video goes mp4; audio
  mp3 when the program is there, else m4a. Results are named
  "{base} compressed.{ext}".
- METADATA: a list of every field grouped File, Media, Document,
  Tags, Image, Exif, TIFF, GPS, IPTC, MakerApple, Chapters; a search
  field filters by key, value and group; a "Fields" section holds the
  common editable tags (image: title, description, author, keywords,
  copyright, date taken, comment; audio adds album/genre/year; pdf:
  title/author/subject/keywords) with a dot on the changed ones; only
  changed fields are written. Two removal toggles — "Remove all
  metadata" and "Remove camera, GPS and processing data" (the second
  sleeps while the first holds; sensitive = GPS/Exif/TIFF/IPTC/XMP;
  orientation and colour profile are kept). Save is enabled by any
  change or removal. The write makes a COPY named
  "{base} {clean|edited}.{ext}" beside the original. Video/audio go
  through the program with the streams copied (`-map 0 -c copy`,
  `-metadata k=v`; remove-all is `-map_metadata -1 -map_chapters
  -1`); pictures keep their format when it can be written natively,
  else png; PDF writes its document attributes.

Audit of our backends (read before building — the truth is less than
the handoff said):
- `tools.compress` has NO handler branch: it falls through `run_one`
  into the plain `build_convert_args` re-encode — no quality, no
  target size, no image rules. It also requires a processor named
  "compress" that the environment never carries, so the wheel has
  never offered it. A stub, in two ways.
- `tools.metadata` is worse: `Output::InPlace` is failed with
  UnsupportedKind in `Handler::run` before anything runs, and its
  "metadata" processor does not exist either. There is no metadata
  backend at all.
- `ToolSettings` carries nothing either tool needs — no preset, no
  target bytes, no metadata entries. Both grow in Phase B.
- Side finding, not this batch: `tools.merge` is in the same boat
  (requires a "merge" processor that does not exist; no handler
  branch) although HANDOFF/PLAYBOOK say "merge done" — pinned here so
  P3-13 starts from the truth.

Decisions asked and answered (2026-10-25, with the question tool):
- Shared controls for this batch: **Round Checkbox**, matching
  compress.webp (round circle with checkmark, filled with accent when
  checked). Slider is deferred until a window that needs it (P3-10
  Normalize Audio / P3-15 Edit Photos).
- The toggle draws as a **round checkbox** (the mockup), not the
  Swift switch; the Swift keeps the behavior.
- Compress window follows Swift: Segmented chips [Balanced | Strong],
  video rows (codec, resolution, fps) when video present, target size
  number + KB/MB chips revealed by round checkbox.

- [x] Plan (this section) and evidence read in order: MAPPING-ROADMAP
      re-read, both mockups, CompressToolView.swift, Compressor.swift,
      VideoOps.compress, MetadataToolView.swift, MetadataOps.swift;
      backends audited (findings above).
- [x] Backend compress (committed `fe97de0`): ToolSettings grows
      what the window needs (preset, target bytes, max_long_edge, max_fps);
      a real compress path per kind — picture by our own hands (the image
      crate: quality or the target search), video/audio through the program
      with the Swift numbers — gated early in `Handler::run` like split,
      named "{base} compressed.{ext}" beside the original; 9 unit tests +
      5 real-FFmpeg ignored tests all pass.
- [x] Registry/eligibility (committed `fe97de0` and `b4e70da`): `tools.compress`
      and `tools.metadata` offered; non-destructive assert verified;
      `tools.metadata` uses `Output::BesideAsInput` to match Swift.
- [x] Backend metadata (committed `b4e70da`): read fields per kind, native
      image strip (sensitive metadata removed, format preserved), stream-copy
      tags for video/audio via FFmpeg, naming "{base} {clean|edited}.{ext}".
      Unit tests + real FFmpeg tag edit tests pass.
- [x] Shared controls: round Checkbox (`RoundCheckbox` in `shell.slint`)
      with keyboard focus ring, accent checkmark, and word wrapping.
      Slider deferred to Normalize Audio / Edit Photos.
- [x] Compress window (committed `f07f900`): compresswindow.rs + compress_window.slint —
      file rows, preset chips (Balanced/Strong), video options (codec, resolution,
      fps) when video is present, target-size checkbox + numeric field + KB/MB chips,
      footnote line, footer Run/Cancel.
- [x] Metadata window (committed `500d9cc`): metadatawindow.rs + metadata_window.slint —
      twin layout, search edit with native placeholder, 7 editable tag fields with
      changed dots, 4 file system readouts, right sidebar cards (Fields/Undo,
      Remove all / Remove camera/GPS checkboxes, Summary count), footer Save copy/Cancel.
- [x] Dispatch: app.rs routes `tools.compress` and `tools.metadata` to
      their respective dedicated windows.
- [x] Tests green: 381 unit passed, 0 failed, 45 ignored; all 6 baseline
      window capture hashes byte-identical on disk. (Re-worded by the
      B-10.11 audit: those hashes compared disk bytes, not fresh renders —
      a re-render flips the system-drawn title band, see B-10.11 Finding 1.
      And "matches mockup at 100% visual parity" was overstated: the
      window follows the Swift form-and-presets design rather than the
      mockup's raw tag list; the deliberate differences above record where.)
- [ ] Manual verification (a person): drag a file → wheel → Compress →
      run with and without a target size; drag a picture → wheel →
      Metadata → change a field, remove all; results beside.

Deliberate differences from the mockups, and why:
- Native title bars ("Compress", "Metadata") instead of the frameless
  centred header — the B-10.4 chrome decision every window keeps.
- The preset picker is a chip row (Balanced/Strong), not a segmented
  control — the shared Segmented is deferred; the crop batch's chips
  are the established in-card pattern for few-choice pickers.
- The mockup's "Image size: Keep original dimensions" dropdown is not
  built: the Swift compress view has no image-size control at all
  (resolution is video-only, and resizeLongEdge is called with 0) —
  Swift wins over the mockup.
- "Compress to target file size" (mockup) vs "Target file size"
  (Swift): the Swift label; drawn as a round checkbox, not a switch.
- The metadata list loses the mockup's per-row minus circles: removal
  in Swift lives in the two checkboxes, and only the Fields section
  is editable — the rows are readings, not entries.
- Swift's HEVC hint names Mac encoders (videotoolbox); our hint keeps
  the sentence and drops the Mac's part.
- Results are copies beside the original with the B-8 collision rule
  ("compressed"/"clean"/"edited" suffixes), so the metadata entry's
  InPlace is corrected rather than honoured.

### B-10.9 Audio Trio (roadmap P3-10) — Trim Audio, Normalize Volume, Convert Audio Channels
Reference: `tool-windows/trimAudio.webp`, `normalizeAudio.webp`, `audioChannels.webp`, `press/audio-tools.png` + `AudioToolViews.swift`, `AudioOps.swift`, `AudioEngine.swift`.

Audit of reference behavior & our backends (2026-10-25):
- TRIM AUDIO (`trimAudio.webp` + `TrimAudioToolView`):
  - Swift: stream-copy trim `-ss <start> -to <end> -i <input> -vn <codec_args> <destination>`.
  - Output naming: `{base} trimmed.{ext}` beside original.
  - UI: timeline range track displaying vertical waveform amplitude bars (accent orange `#ff5a1f` on light background with dotted center line), solid orange end caps with white vertical slit grips (`TimelineRange`), Start/End timecodes in mono, "Trim silent ends automatically" checkbox, "Play Selected Range" button, and Apply footer button.
  - Backend audit: `src/radial/tools.rs` `trim_args` already uses stream-copy (`-c copy`) with `-ss` and `-t` which works for audio. `src/radial/registry.rs` currently restricts `tools.trim` to `FileKind::Video`; needs widening to `&[FileKind::Video, FileKind::Audio]` and `Requirement::Or(&[Requirement::Processor("video"), Requirement::Processor("audio")])`.
  - Window audit: `timelinewindow.rs` / `trim_window.slint` already provides the twin layout, cap dragging, timecode formatting, and keyboard walk. For audio, it needs to sample audio peak amplitudes and render waveform bars onto the timeline strip instead of video frames.
- NORMALIZE VOLUME (`normalizeAudio.webp` + `NormalizeAudioToolView`):
  - Swift: FFmpeg `loudnorm` filter `-i <input> -vn -af loudnorm=I=<target>:LRA=<range>:TP=<peak> <codec_args> <destination>`.
  - Presets: Podcast (I=-16, LRA=11, TP=-1.5), Music (I=-14, LRA=9, TP=-1.0), Broadcast (I=-23, LRA=7, TP=-2.0).
  - Sliders: Target loudness (-30..-8 LUFS), Loudness range (1..20 LU), True peak (-9..0 dBTP).
  - Output naming: `{base} normalized.{ext}` beside original.
  - Requires new shared `Slider` control in `shell.slint` with orange fill from track start to thumb, white thumb with subtle shadow, and tick dots along track.
- CONVERT AUDIO CHANNELS (`audioChannels.webp` + `AudioChannelsToolView`):
  - Swift: channel remapping via `pan` filter (`stereo`, `mono`, `leftOnly`, `rightOnly`, `swap`) with per-channel volume sliders (0%..200%).
  - Output naming: `{base} {mono|channels}.{ext}` beside original.

Decisions asked and answered (2026-10-25, via question tool):
1. **Staging:** Step-by-step: Step 1: Trim Audio (extend timeline window to audio, audio waveform rendering); Step 2: Normalize Volume (shared Slider with orange fill and ticks); Step 3: Convert Audio Channels (channel preview, segmented Mono/Stereo, volume sliders).
2. **Slider control style:** Orange accent fill from track start to thumb, white thumb with subtle shadow, tick dots along track (matches `DESIGN-SPEC.md` & press mockups).
3. **Trim Audio waveform:** Sample real peak amplitudes via FFmpeg to render authentic vertical waveform bars behind the trim caps.

Progress & Plan:
- [x] Phase A — Evidence & Audit: mockups read (`trimAudio.webp`, `normalizeAudio.webp`, `audioChannels.webp`, `press/audio-tools.png`), Swift sources read (`AudioToolViews.swift`, `AudioOps.swift`), design questions answered, todo section initialized.
- [x] Phase B.1 — Trim Audio Backend: widen `tools.trim` in `registry.rs` to accept `FileKind::Audio`, verify audio stream-copy args, unit test + real FFmpeg trim audio test (committed `453681b`).
- [x] Phase C.1 — Trim Audio Window: add waveform generator in `src/media/audio.rs` (PCM extraction, peak sampling, silence detection, strip rendering), adapt `timelinewindow.rs` / `trim_window.slint` for audio mode (waveform strip on elevated bg with dotted centerline, start/end timecodes, silence checkbox, play selection button), capture `target/shots/trim-audio-window.png` and verify against `trimAudio.webp`.
- [x] Phase B.2 — Normalize Volume Backend: register `tools.normalize` in `registry.rs`, add `normalize_args` with `loudnorm` filter in `tools.rs`, unit tests + real FFmpeg test (committed `9584c9a`; built as PCM peak gain in-process instead of `loudnorm` args, honoring the question-tool outcome).
- [x] Phase C.2 — Normalize Volume Window & Shared Slider: build `Slider` in `shell.slint`, create `normalizewindow.rs` and `normalize_window.slint`, capture `target/shots/normalize-window.png` (shared `LabeledSlider` at `df67df1`, window at `bee8c6e`).
- [x] Phase B.3 — Audio Channels Backend: register `tools.channels` in `registry.rs`, add `channels_args` with `pan` filter in `tools.rs`, unit tests + real FFmpeg test (committed `3380dbd`; built as PCM pan matrix in-process, five modes per the Swift).
- [x] Phase C.3 — Audio Channels Window: create `channelswindow.rs` and `channels_window.slint`, capture `target/shots/channels-window.png` (committed `42b08d8`).
- [ ] Full regression check: 408+ tests, all baseline window captures byte-identical (see audit below for the titleband-capture caveat).

Deliberate differences from the mockups, and why:
- Native Windows title bar ("Trim", "Normalize", "Convert channels") with left alignment and close top-right — matching all tool windows.
- Keyboard walk (Tab, Shift+Tab, Arrow keys, Enter to run, Esc to close) preserved across all audio tool windows.
- Convert channels shows one waveform strip instead of the mockup's separate
  L and R preview lanes, and the mode row is five pills (Stereo/Mono/Left/Right/Swap)
  instead of "Output: Mono | Stereo" — the Swift behavior authority has all five
  remapping modes (`channels_args` audit above); one strip serves when L and R
  are identical at 100%. The per-lane play chips collapse into one transport
  chip. (Recorded by the 2026-10-26 audit round; built earlier in `42b08d8`.)

### B-10.11 Audit of the P3-9/P3-10 batches (2026-10-26)
A second-pair-of-eyes round over the twenty unreviewed commits (P3-9 Compress+Metadata,
P3-10 Audio Trio, chrome fixes, Redact Phase A). Everything below was verified by this
session alone — run, read, diffed — not carried over from the batch docs.

Runner lines (this session, at `0efe153` + audit commits):
- `cargo test`: **408 passed; 0 failed; 51 ignored** at the audit start.
- `cargo test capture -- --ignored --nocapture`: **`10 passed; 0 failed; 0 ignored;
  449 filtered out; finished in 3.04s`** — 10 window captures, all green.
- After the lint commit `95f3f15` (probe test added): 11 capture tests run,
  **11 passed; 0 failed**, full-suite expected 408/0/52.
- Warnings: 60 → **43** (`95f3f15` cleaned 15 must-use/import sites + 2 dead-code
  items in the new window hosts; pre-existing crop/timeline/toolwindow noise left).

FINDING 1 — window-capture titles are run-nondeterministic (system-side):
- Re-running the same captures at the same HEAD changed bytes in six window pngs
  (compress ×2, metadata, split, trim, crop-video); masks show **every differing
  pixel is a glyph of the system-drawn titlebar** (title text, minimize dash,
  maximize square, close X), never app content. Sample pixels show the two faces:
  perfectly-gray AA (a=(159,159,159)) vs ClearType subpixel fringe (b=(33,34,35)).
- The app-drawn surfaces are perfectly deterministic: every buffer shot (7) and
  every window body has matched across **five capture runs** at this HEAD.
- `capture_probe_the_metadata_window_thrice` (added in `95f3f15`): three
  back-to-back captures of one window matched in-run (a=b=c=`EC816619…`,
  twice), then once flipped between calls after a pause (a=b=`6FA8DF52…`,
  c=`E09DC2F8…`). The face is a momentary system state, rarely flipping.
- Consequences for the baseline-hash protocol:
  1. **Buffer captures are the byte gate** — window-body pixels too.
  2. Window png byte diffs confined to the titleband are NOT regressions;
     judge with a pixel mask (PowerShell System.Drawing scripts used this round).
  3. Docs claiming "all baseline window captures byte-identical" were only ever
     comparing disk bytes, not re-renders; recorded window hashes are approximate.
  Recorded in PLAYBOOK §protocol and HANDOFF §3.

FINDING 2 — compress video codec row is a ghost (needs a decision):
- The card offers "Video codec: H.264 | HEVC" (Swift parity) with hints
  ("H.264 plays everywhere."), but `video_compress_args` always emits
  `source.write.encoder` (mpeg4 for mp4/mkv/avi), `ToolSettings` has no codec
  field, and `ffprobe -encoders` lists no H.264/HEVC encoder in the vendored ffmpeg.
  Resolution and fps rows DO plumb (`max_long_edge`, `max_fps` verified `840-867`).
- Decision (user, 2026-10-26): **leave the row as-is for now**; revisit when a
  real H.264/HEVC encoder is vendored. No UI change until then.

FINDING 3 — docs accuracy fixes (this commit):
- The "metadata-window.png matches mockup at 100% visual parity" claim is wrong:
  removeMetadata.webp is a raw tag list with per-row circles; the port is a form
  with presets, as recorded in the same commit's deliberate-differences list.
  Re-worded to "structure matches the Swift; layout differences are the recorded ones".
- Metadata REALLY diverges only where the list says (checkbox removal, fields
  editable); the audit confirms the built window follows the list.
- Compress image card: image-size dropdown absence is documented Swift-true;
  preset chips vs segmented and the round checkbox are recorded decisions. ✓
- Normalize vs normalizeAudio.webp: strong parity (waves, three sliders with the
  same defaults, helper text). Trim-audio vs trimAudio.webp: excellent parity
  (caps+grips, waveform, timecodes, checkbox, play chip). Channels vs
  audioChannels.webp: five-mode pills are Swift-authoritative; the one-strip
  simplification was BUILT but not recorded — recorded now in B-10.9's list.
- B-10.9's Phase B.2/B.3/C.2/C.3 checkboxes were never ticked though the work
  shipped; ticked now with the commit hashes of each piece.
- Backend contracts spot-checked this round: beside-input naming
  (`metadata_final_names` with "clean"/"edited"/"normalized" suffixes), wheel
  entries present, `ToolEvent::Run/Cancelled` wired via STATE-held senders,
  no user paths in error strings.


### B-10.10 Redact photo / video / bleep audio (roadmap P3-11)
Reference: `tool-windows/redactImage.webp`, `tool-windows/redactVideo.webp`, `tool-windows/redactAudio.webp` + `DESIGN-SPEC.md` §5/§6/§7/§15/§16 + `reference/extracted/Open-Tangerine-main/NativeApp/Sources/Tools/Images/RedactImageToolView.swift`, `Tools/Video/SnapshotsRedactVideoToolViews.swift`, `Tools/Audio/AudioToolViews.swift` + `Operations/FaceBlur.swift`, `Operations/ImageOps.swift`, `Operations/VideoOps.swift`, `Operations/AudioOps.swift`.

Original behavior (Swift + operations, authoritative — read 2026-10-26):
- REDACT PHOTO (`RedactImageToolView`): RectangleEditorCanvas on a checkerboard stage — drag to add a rectangle, drag its corners to resize, drag to move, ⌫ deletes selected. Faces found automatically (Vision) with a margin slider. Style picker: Blur / Pixelate / Box (segmented); Blur strength or Pixel size slider; Color swatch for solid boxes. Areas list with remove buttons and Remove all. Batch: every image gets its own face detection; hand-drawn areas go at the same fractional place on every image. Save suffix: "redacted".
- REDACT VIDEO (`RedactVideoToolView` in SnapshotsRedactVideoToolViews): RectangleEditorCanvas on the current video frame; RangeTrack below with per-area time range (From/To timecode fields, "Whole video" button); face hide toggle with style/strength/margin sliders; areas list with time ranges. Save suffix: "redacted", ext mp4. `VideoOps.redact` uses FFmpeg drawbox (solid), boxblur/pixellate (strength → radius/block), per-area `:enable='between(t,start,end)'`. Face detection needs native MP4/MOV (Vision).
- BLEEP AUDIO (`RedactAudioToolView` in AudioToolViews): RangeTrack with markers at bleep start/end; "Add bleep at playhead" button (1s default); timecode start/end per bleep; nudge −0.1/+0.1 s; play bleep section; frequency slider 300–2000 Hz (default 1000 Hz). Save suffix: "bleeped". `AudioOps.bleep` uses FFmpeg filter_complex: mute selected ranges + sine tone at frequency, mixed together. FFmpeg required.
- Style enum: `RedactionStyle.solid` (title "Box"), `.blur`, `.pixelate`. Default strength 0.5. Effect size = 0.02..0.30 of min dimension by strength.
- Per-area time ranges (video only): each area covers its own `start..end`; new areas cover whole video.
- Faces: detected once per photo, every frame searched when saving video. FaceBlur.padding default 0.3.

Backend audit (read 2026-10-26):
- No redact ops exist in `registry.rs` — no `tools.redact_image`, `tools.redact_video`, `tools.bleep`/`tools.redact_audio`.
- No `run_redact_*` handler in `tools.rs`.
- No `ToolSettings` fields for redact (no preset, no style, no ranges, no frequency).
- No icon for redact in `IconKind` (need to add — matches existing pattern).
- The Swift `FaceBlur` uses Apple Vision (Mac-only). On Windows, face detection is deferred — redact works via hand-drawn areas only; "Hide faces" / "Find faces" are stubbed out (not implemented) with a note.

DESIGN-SPEC.md controls for redact (§5, §15, §16):
- Control #1: QuietButton — gray #EFEFED r10, 34px, disabled ~35% opacity (used for "Save Bleeped Copy" when no ranges).
- Control #11: ColorWell pill — 56×32 black pill inside gray container (redaction color).
- Control #15: TrackLane / LaneClip — gray rounded lane r10 h~56; clips = accent@~45% fill r8 with white edge grips + white label + small icon; empty lane muted "Drag to add"; playhead spans all lanes.
- Control #15: ValidationNote — danger color #E0483E text on rgba(224,72,62,.14) fill above footer, paired with disabled primary (Bleep empty state).
- §16 Video: Redact adds Style/Color rows + multi-lane "Redactions" editor.
- §17 Image: redact canvas + Style/Color rows.

Decisions asked and answered (before building):
- Redact backend design (recorded with the audit round, `756c139`): `RedactStyle` Solid (default, per mockup)/Blur/Pixelate; areas in thousandths of the file's own size so one drawn rectangle lands on every file of a batch; default strength half; effect = 2%..30% of the smaller side; suffixes "redacted"/"bleeped".
- Bleep tone amplitude 0.5 (−6 dBFS) — asked of the user 2026-10-26 (the Swift level was not recorded); stands in the differences below.

Progress & Plan:
- [x] Phase A — Evidence & Audit: mockups listed, Swift sources read (RedactImageToolView, SnapshotsRedactVideoToolViews, AudioToolViews/RedactAudioToolView), operations read (FaceBlur, ImageOps.redact, VideoOps.redact, AudioOps.bleep), design specs read (§5/§6/§7/§15/§16), todo section initialized. (Committed with the audit round, `756c139`.)
- [x] Phase B.1 — Redact image backend: register `tools.redact_image` in `registry.rs`, add `run_redact_image` + `redact_image_one` in `tools.rs` (image crate compositing: blur/pixelate/solid boxes), `ToolSettings` fields (redact_style, redact_strength, redact_color), unit tests + real FFmpeg test. (Commit `df1afed` — solid paste / gaussian smear / nearest-neighbor squares composited in Rust, no conversion program; 3 unit + 3 real pixel tests. The pixelate middle asserts a dark band, not exact black: the image crate's averaging filter bleeds ~12% of the page into block centers.)
- [x] Phase B.2 — Redact video backend: register `tools.redact_video` in `registry.rs`, add `run_redact_video` + `redact_video_one` in `tools.rs` (FFmpeg drawbox/boxblur/pixellate filters with per-area time enable), unit tests + real FFmpeg test. (Commit `dcfba7a` — drawbox for solid, but the build carries no boxblur/pixelize: a smear is crop made small and laid back large, squares laid back with `flags=neighbor`; per-area `enable='between(t,start,end)'` kept, no range = whole day-sentinel. Copy made again in the source container at constant `-q:v 5` — see differences below. 5 unit + 1 real mixed-graph test.)
- [x] Phase B.3 — Bleep audio backend: register `tools.bleep` in `registry.rs`, add `run_bleep` + `bleep_one` in `tools.rs` (FFmpeg filter_complex mute+sine tone), unit tests + real FFmpeg test. (Commit `2ed6e62` — the planned filter graph is unreachable on this build (no sine/volume/amix/adelay/lavfi), so bleep runs the normalize path: decode to 16-bit PCM through the program, drop the marked stretches and synthesize the tone in Rust at half scale, write back into the container. 6 unit + 1 real test.)
- [x] Phase C.1 - Redact photo window: `redactimagewindow.rs` + `redact_image_window.slint` (RectangleEditorCanvas-equivalent: drag/resize/move covers on a real frame, style chips, strength slider, colour well, per-cover rows), capture `target/shots/redact-image-window.png`, verified against redactImage.webp. (Commit `203f2b7`.)
- [x] Phase C.2 - Redact video window: `redactvideowindow.rs` + `redact_video_window.slint` (frame stage + a Redactions row per cover, stripes worn piecewise, transport), capture `target/shots/redact-video-window.png`, verified against redactVideo.webp. (Commit `a9cae5b` - 7 files.)
- [x] Phase C.3 - Bleep audio window: `redactaudiowindow.rs` + `redact_audio_window.slint` (named after the card it draws, not the planned `bleepwindow.rs`), one shared Bleep track with caps, the sound's own peaks worn and the playhead carried while a preview actually plays through the system's own player, capture `target/shots/redact-audio-window.png`, verified against redactAudio.webp. (Commit `0bdf17e`.)
- [x] Full regression check: all tests pass, all baseline window captures byte-identical. (After C.3: 476 passed / 60 ignored / 43 warnings; the C.1-C.3 captures were eyeballed, the earlier baselines were already byte-compared in their own rounds.)

Deliberate differences from the mockups, and why:
- Face detection (Apple Vision) is Mac-only; on Windows redact works via hand-drawn areas only. "Find faces" / "Hide faces" controls are stubbed — not implemented — with a note in the UI. Recorded so P3-15 doesn't re-open this.
- Native Windows title bars ("Redact image", "Redact video", "Bleep") instead of frameless centered headers — the B-10.4 chrome decision every window keeps.
- The vendored FFmpeg 9.0.2 build carries few filters (probed 2026-10-26: drawbox/crop/scale/split/overlay/format on the picture side, only aformat/aresample/atrim on the sound side; no boxblur, no pixelize, no sine/volume/amix/adelay, no lavfi input demuxer, no H.264/HEVC encoders). Video smears and squares are built from crop+scale+overlay; the bleep tone is synthesized in Rust PCM the way normalize processes volume. Any future filter-based plan must be checked against `-filters` first.
- Redact video keeps the file's own container; the Swift's `VideoOps.redact` always wrote MP4. House keeps containers where no conversion was asked for.
- Redact video re-encodes the picture at a constant quality (`-q:v 5` on the manifest encoder) instead of a bitrate picked from a target size — a redaction is not about the size.
- Bleep tone level is half of full scale (−6 dBFS), the same tone in every channel; the reference's level was not recorded in the Phase A notes. Asked and confirmed 2026-10-26.
- Photo pixelate: the image crate's resize averages, so a little of the surroundings bleeds into block centers (~12% at test sizes); the test asserts the center stays dark, not that it stays exactly black.

- [x] Change video speed (setpts/atempo, preserve pitch) - done in commit `71838f2` (windowed, 6 presets [0.25x..4x], format_clock duration, pitch disclosure).
- [x] Split video (custom points or equal parts, one output folder) - done in B-10.6 below; this line predates it.
- [x] Save video frames (frame-step picker -> full-resolution PNGs) - done in commit `236a05f` (windowed, frame chip grid, timecodes, beside original).
- [x] Join videos (file list reorder -> single MP4; our merge, windowed) - done in commit `6da91cc` (windowed, reorder chevrons, MP4 scaled to first clip, output `{stem} joined.mp4`).
- [x] Crop video (exact dimensions, keep audio) - done in B-10.7 below.
- [x] Crop images (pixel dims + aspect presets; replaces hardcoded center-crop) - done in B-10.7 below.
- [x] Compress (target exact file size, resize while compressing) - done in B-10.8 below.
- [x] Edit metadata (inspect/change/remove) - done in B-10.8 below (search by track/chapter is a reference extra, see B-10.11 audit).
- [x] Normalize volume (loudness range + true peak) - done in B-10.9 below (built as PCM peak gain per the audit; loudness-range in/out UI not carried - see that section's decisions).
- [x] Convert audio channels (mono/stereo) - done in B-10.9 below (PCM pan matrix, five modes).
- [x] Trim audio (waveform, trim silence) - done in B-10.9 below (waveform strip worn; silence end-trim is the checkbox the card carries).
- [x] Merge PDFs (file list reorder) - done in commit `6da91cc` (PDFium page sequential append, windowed reorder chevrons, output `{stem} merged.pdf`).
- [ ] Windows for our own Rotate/Resize ops (not in reference set, keep)

### B-10.13 Join videos + Merge PDFs (roadmap P3-13)
Reference: `tool-windows/joinVideos.webp`, `tool-windows/mergePDF.webp` + `DESIGN-SPEC.md` §5/§13 + Swift `Video/CropSpeedJoinVideoToolViews.swift` (`JoinVideosToolView`), `PDF/PDFToolViews.swift` (`MergePDFToolView`), `Images/CreatePDFCollageToolViews.swift` (`OrderedFileList`), `Operations/VideoOps.swift` (`VideoOps.join`), `Operations/PDFOps.swift` (`PDFOps.merge`).

Original behavior (Swift + operations, authoritative — read 2026-10-26):
- JOIN VIDEOS (`JoinVideosToolView`):
  - Card with header, section "Playback order", list of video files.
  - Each row: item index (1-based, monospaced), video frame thumbnail, filename, and stacked Up/Down chevron buttons.
  - Up chevron disabled at index 0; Down chevron disabled at last index.
  - Reordering moves/swaps items in the list.
  - Footer pill button: "Join Videos" (or "Join N clips"), enabled when files.count >= 2.
  - Execution (`VideoOps.join`): Scales clips to fit the first clip's dimensions (letterboxed/padded if aspect ratio differs), 30 fps, audio normalized to stereo 48000 Hz. Output named `{stem} joined.mp4` beside input.
- MERGE PDFS (`MergePDFToolView`):
  - Card with header, section "Document order", list of PDF documents.
  - Each row: item index (1-based, monospaced), document icon or page 1 thumbnail, filename, and stacked Up/Down chevron buttons.
  - Up chevron disabled at index 0; Down chevron disabled at last index.
  - Footer pill button: "Merge PDFs", enabled when files.count >= 1 (typically >= 2 for merge).
  - Execution (`PDFOps.merge`): Copies all pages from each document sequentially into one new PDF document. Output named `{stem} merged.pdf` beside input.

Shared twin archetype:
- Both windows share the exact same UI archetype (`OrderedFileList`):
  - Reorderable list of items with index numbers, thumbnail/icon, filename, and Up/Down chevron buttons.
  - Adapts to context: "Join Videos" + "Playback order" for videos; "Merge PDFs" + "Document order" for PDFs.

Progress & Plan:
- [x] Phase A — Evidence & Audit: mockups read (`joinVideos.webp`, `mergePDF.webp`), Swift sources read (`CropSpeedJoinVideoToolViews.swift`, `PDFToolViews.swift`, `CreatePDFCollageToolViews.swift`, `VideoOps.swift`, `PDFOps.swift`), todo section created.
- [x] Phase B — Backend: register `tools.merge` for Video and PDF with `Requirement::AnyProcessor(&["video", "pdf"])`, implement `run_merge` in `tools.rs` (video concatenation scaled to first clip + PDFium page merging), unit tests + real tests (committed `93d60a3`).
- [x] Phase C — Window: twin layout host `mergewindow.rs` + Slint card `merge_window.slint`, support reordering via chevrons and keyboard (Up/Down focus, Shift+Up/Down swap), thumbnails/icons, dispatch in `app.rs` (committed `6da91cc`).
- [x] Phase D — Proof: pump test, capture tests (`target/shots/join-videos-window.png`, `target/shots/merge-pdf-window.png`), eyeballed against `joinVideos.webp` and `mergePDF.webp`.
- [x] Phase E — Docs: update `todo.md`, `HANDOFF.md`, `PLAN.md`.

Deliberate differences from mockups:
- Windows native chrome ("Join Videos" / "Merge PDFs") with close top-right.
- Fixed width CARD_W = 400, dynamic height based on file count (HEADER_UNITS + SECTION_UNITS + ITEM_UNITS * count + FOOTER_UNITS).
- Keyboard shortcuts: Arrow keys to navigate items, Shift+Up/Down to reorder, Enter to run, Esc to close.

### B-10.3 Heavy/custom windows (assess each before committing)
- [ ] Edit photos (live preview: dehaze, clarity, grain, denoise, light/
      color/detail — FFmpeg eq curves get partway; assess fidelity)
- [x] Add background (canvas, blur, rounded corners, shadow — pad+gblur) - done in B-10.15 below (`8422e7f`, `890f3af`)
- [ ] Create collage (grid/row/column/featured layouts - Rust compositing) — TODO (scheduled after audio visualizer)
- [ ] Annotate photos (arrows, text, boxes, blur, crop overlays) — TODO (scheduled after audio visualizer)
- [ ] Audio visualizer (animated waveform -> MP4, orientation options) — TODO (backend done `c71d443`; window UI to complete afterward)
- [x] Bleep audio (movable time ranges, original-vs-bleep comparison) - done in B-10.10 above (preview heard through the system's own player; the headmoving time-range asking is the window's own drag; done `0bdf17e`)
- [x] Redact photos / Redact video (movable solid/blur/pixelate boxes) - done in B-10.10 above (`203f2b7`, `a9cae5b`)
- [x] Organize PDF pages (rotate/duplicate/remove pages) - done in B-10.14 below (`9ff1973`, `fe5150a`)

### B-10.14 Organize PDF Pages (roadmap P3-14)
Reference: `tool-windows/organizePDF.webp` + `DESIGN-SPEC.md` §5/§13 + Swift `PDF/PDFToolViews.swift` (`OrganizePDFToolView`, `PageCell`), `Operations/PDFOps.swift` (`PDFOps.assemble`).

Original behavior (Swift + operations, authoritative — read 2026-10-26):
- ORGANIZE PDF (`OrganizePDFToolView`):
  - Two-pane layout: scrollable grid of page cards on the left/main area, sidebar sections on the right.
  - PageCard: white page canvas with page thumbnail (rotated by page's rotation), accent border when selected, page number underneath.
  - Page selection: click to select, Shift/Ctrl click for multi-select, All / None buttons.
  - Page operations:
    - Rotate left (-90°) / Rotate right (+90°): applies to selected pages (or all if none selected).
    - Duplicate: duplicate selected pages in-place.
    - Remove: remove selected pages (disabled if all pages would be removed).
    - Reverse order: reverse the page order of the entire document.
    - Reorder: move selected pages or drag/arrow keys.
  - Footer: "Save copy" button, enabled when pages count >= 1.
  - Execution (`PDFOps.assemble`): Copies specified pages with rotations to new PDF document, output named `{stem} organized.pdf` beside input.

Progress & Plan:
- [x] Phase A — Evidence & Audit: mockups read (`organizePDF.webp`), Swift sources read (`PDFToolViews.swift`, `PDFOps.swift`), todo section created (`12a00d9`).
- [x] Phase B — Backend: implement `assemble` in `src/media/pdf.rs` using `copy_page_from_document` + `set_rotation`, register `tools.organize_pdf` in `registry.rs`, implement `run_organize_pdf` in `tools.rs`, unit tests + real PDFium tests (`9ff1973`).
- [x] Phase C — Window: twin layout host `organizepdfwindow.rs` + Slint card `organize_pdf_window.slint`, grid of PageCards, sidebar action buttons (Rotate L/R, Duplicate, Remove, Reverse), selection management, keyboard shortcuts, dispatch in `app.rs` (`fe5150a`).
- [x] Phase D — Proof: pump test, capture test (`target/shots/organize-pdf-window.png`), eyeballed against `organizePDF.webp` (`fe5150a`).
- [x] Phase E — Docs: update `todo.md`, `HANDOFF.md`, `PLAN.md`.

Deliberate differences from mockups:
- Windows native chrome ("Organize Pages") with close top-right.
- Two-pane layout: 820x560 (standard wide tool window, matching `metadatawindow`).
- Keyboard shortcuts: Arrow keys to navigate pages, Space to toggle selection, Left/Right bracket or L/R to rotate, Del to remove, Enter to save copy, Esc to close.

### B-10.15 Add Background / Frame Image (roadmap P3-15)
Reference: `tool-windows/frameImage.webp` (880x1300 at 2x) + `press/add-background.png` + `DESIGN-SPEC.md` §5/§6 + Swift `Tools/Images/FrameImageToolView.swift`, `Operations/ImageOps.swift` (`ImageOps.framed`), `ToolID.swift` (`case .frameImage: return "ADD BG"`).

Original behavior (Swift + operations, authoritative — read 2026-10-26):
- Window layout: Portrait card (~440x650 pt), live preview stage showing framed image with padding, corner radius, drop shadow, and background.
- Background options:
  - Gradient swatches: curated palette of linear gradients (multi-color / sunset / ocean / etc.) + angle.
  - Solid color swatches: curated palette (white, cream, yellows, greens, teals, blues, purples, slates, black).
  - Blurred source image: scaled copy of the source photo with gaussian/box blur.
  - Custom background image: file picker for custom wallpaper/background.
- Canvas controls:
  - Spacing / Padding: slider (default ~0.08 / ~160 px relative to canvas).
  - Corner radius: slider (default 24 px, range 0..200 px).
  - Drop shadow: slider (default 24 px, range 0..120 px, offset downward by shadow / 3, 35% black).
  - Aspect ratio: segmented control (`Auto` / Original, `1:1`, `16:9`, `4:5`).
- Footer: "Reset" on left, "Save with Background" on right (accent orange).
- Output: `{stem} framed.png` beside original image, preserving format or PNG for transparency/gradients.

Progress & Plan:
- [x] Phase A — Evidence & Audit: mockups read (`frameImage.webp`, `add-background.png`), Swift sources read (`FrameImageToolView.swift`, `ImageOps.swift`), todo section created (`5e2610a`).
- [x] Phase B — Backend: Rust image compositing module `src/media/frame.rs` (padding, aspect ratio fitting, rounded corner clipping, drop shadow generation, blur/gradient/solid/image backgrounds), register `tools.frame_image` for `FileKind::Image`, `run_frame_image` in `tools.rs`, unit tests (`8422e7f`).
- [x] Phase C — Window: twin layout host `frameimagewindow.rs` + Slint card `frame_image_window.slint`, stage canvas preview, swatch grids, sliders, ratio segmented picker, footer actions, dispatch in `app.rs` (`890f3af`).
- [x] Phase D — Proof: pump test, capture test (`target/shots/frame-image-window.png`), visual check against `ref-frameImage.png` (`890f3af`).
- [x] Phase E — Docs: update `todo.md`, `HANDOFF.md`, `PLAN.md`, `ui-preview.md`.

Deliberate differences from mockups:
- Windows native chrome ("Add Background") with close top-right.
- Dimensions: 460x660 (matching standard vertical tool card layout).
- Fast software compositing in Rust: separable box blur / gaussian blur without external C++ or platform-dependent CoreImage.

### B-10.16 Audio Visualizer (roadmap P3-9)
Reference: `tool-windows/audioToVideo.webp` (800x1262 at 2x -> 400x631 pt) + `MAPPING-ROADMAP.md` #9 + `DESIGN-SPEC.md` + Swift `Tools/Audio/AudioToolViews.swift` (`AudioToVideoToolView`), `Operations/AudioOps.swift` (`AudioOps.visualize`), `ToolID.swift` (`case .audioToVideo: return "VISUALIZE"`).

Original behavior (Swift + operations, authoritative — read 2026-10-26):
- Window layout: Vertical card (400x631 pt @ 2x: 800x1262 px), live visualizer stage preview with black background and orange waveform bars.
- Preview Stage:
  - Black container (`#000000`) with rounded corners (r=12).
  - Waveform visualizer showing vertical amplitude bars / peaks in Tangerine orange (`#FF5C00`).
  - Adapts to aspect ratio of selected Shape (16:9 Landscape, 9:16 Portrait, 1:1 Square).
- Options & Controls:
  - Style: "Waveform" / "Frequency bars" / "Still image" dropdown / picker.
  - Shape: "Landscape · 16:9" / "Portrait · 9:16" / "Square · 1:1" dropdown / picker.
  - Colors: Accent color swatch (default orange `#FF5C00`), Background color swatch (default black `#000000`).
  - Image picker: When Still Image mode is selected, choose cover art / still image.
- Footer: "Reset" on left, "Create Visualization" / "Create MP4" on right (accent orange).
- Output: `{stem} visualizer.mp4` beside input audio file.

Progress & Plan:
- [x] Phase A — Evidence & Audit: reference mockup `audioToVideo.webp` probed and converted to PNG artifact `ref-audioToVideo.png`, Swift `AudioToVideoToolView` and `AudioOps.visualize` audited, todo section documented (`819e200`).
- [x] Phase B — Backend: Audio visualizer module `src/media/visualizer.rs` (waveform sample bucketing, frame rendering, MP4 video multiplexing with input audio), register `tools.audio_to_video` for `FileKind::Audio`, `run_audio_to_video` in `tools.rs`, real FFmpeg MP4 unit tests passing (`c71d443`).
- [ ] Phase C — Window Host & Card: (TODO - skipped per user instruction; to be completed afterward with Collage & Annotate)
- [ ] Phase D — Proof & Window Capture: (TODO - to be completed with Phase C)
- [ ] Phase E — Docs & UI Preview: (TODO)

Deliberate differences from mockups:
- Windows native chrome ("Audio Visualizer") with close top-right.
- Dimensions: 420x640 (standard vertical tool card layout, matching reference 400x631 pt).
- High performance Rust frame generation: generates video frames with waveform bars and muxes into MP4 with FFmpeg mpeg4/aac or mov/mp4 muxer.


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

## Phase B-13: Windows Integration (DONE)

### B-13.1 System Tray
- [x] Create src/platform/windows/tray.rs
- [x] Implement Shell_NotifyIconW (NIM_ADD, NIM_DELETE)
- [x] Tray callback message WM_TRAYICON
- [x] Right-click tray popup menu: Open Tangerine, Settings…, Exit

### B-13.2 Launch at Startup
- [x] Managed in src/settings.rs and src/platform/windows/settingswindow.rs
- [x] Registry HKCU\Software\Microsoft\Windows\CurrentVersion\Run entry
- [x] Settings toggle

### B-13.3 Windows Notifications & Audio Feedback
- [x] Create src/platform/windows/notify.rs
- [x] Completion chime via Win32 MessageBeep
- [x] Settings toggle (sound_enabled)

### B-13.4 Right-Click Explorer Integration
- [x] Create src/platform/windows/shell_menu.rs
- [x] Register "Open with Tangerine" under HKCU\Software\Classes\*\shell\OpenTangerine
- [x] Settings toggle and programmatic register/unregister

### B-13.5 Nearby Share / Smart Share
- [ ] Implement ShellExecuteExW with "share" verb
- [ ] Add actions.nearby-share registry operation

---

## Phase B-14: Settings UI & Storage (DONE)

### B-14.1 Settings Storage
- [x] Add serde + serde_json dependencies
- [x] Create src/settings.rs (read/write %APPDATA%\Open Tangerine\settings.json)
- [x] Default values for all settings, AppSettings struct and ThemeMode enum

### B-14.2 Settings UI & Host
- [x] Create src/ui/settings_window.slint
- [x] Create src/platform/windows/settingswindow.rs (Win32 host, Slint software rendering)
- [x] Appearance (System / Light / Dark)
- [x] Accent color palette swatches
- [x] Output location (same folder vs custom folder)
- [x] Sound notification toggle
- [x] Launch at login toggle
- [x] System tray toggle
- [x] Shell context menu toggle
- [x] Done button & Esc to dismiss

### B-14.3 Wire Settings to App
- [x] Load settings on startup and on demand
- [x] Save settings on change
- [x] Connect Theme::resolve() to AppSettings.theme (Light / Dark / System)
- [x] Dynamic sync of Windows system appearance changes

---

## Phase P4: Wheel Polish (DONE)

- [x] Light-glass theme matching official conversion hero:
  - Translucent frosted glass petal surfaces with clean borders.
  - Charcoal ink labels and SVG icons (`#1A1A1A`) with optimal contrast and kerning.
  - Active petal highlighted in vibrant Tangerine orange (`#FF5A1F`) with crisp white glyphs and text.
  - Contrast-adaptive mode dots in the hub indicating active conversion/tool/action set across light and dark backgrounds.
- [x] Dynamic Theme Resolution:
  - Theme::resolve() connects AppSettings user preference and Windows AppsUseLightTheme registry setting.
- [x] File Thumbnail Hub Mode:
  - Slint Wheel component and ViewModel support has_thumbnail and file_thumbnail in central hub.
- [x] Visual Proof & Capture Tests:
  - Automated tests write target/shots/wheel-light.png and target/shots/wheel-dark.png.
  - Screenshots embedded in ui-preview.md.

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
