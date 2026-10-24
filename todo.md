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
- [ ] Manual verification: drag audio files, wheel offers the targets

### B-9.7 RAR Input
- [ ] Assess unrar/7z feasibility
- [ ] Add rar to ffmpeg/components.txt (read = -)
- [ ] Create external tool invocation
- [ ] Add registry operation (implemented: false until unrar detected)

---

## Phase B-10: Tool Windows (the reference product IS its windows)

Verified against tangerineformac.com/tools: every one of the 21 tools opens
as a DEDICATED CONFIGURATION WINDOW (preview, controls, Cancel/action).
Our tools currently run immediately with hardcoded defaults — without this
phase there is no tool parity, whatever the operations can do.

### B-10.0 Tool window shell (TW-1)
- [ ] Create src/ui/tool_window.slint — shared chrome: title, file strip,
      preview area, controls column, Cancel + action footer
- [ ] Controller effect: OpenToolWindow(op, files) instead of immediate Run
- [ ] First window: Compress or Resize (image preview, no timeline)

### B-10.1 Time-range template (TW-2) — the Trim video window
- [ ] Frame preview via ffmpeg -ss seek (~50-150 ms per scrub update)
- [ ] Timeline with trim handles, timecodes, frame-step buttons
- [ ] Template reused by: Trim audio, Split video, Save frames, Bleep

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
