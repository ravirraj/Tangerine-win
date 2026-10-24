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
- [ ] Assess poppler Windows build feasibility
- [ ] Add pdf to ffmpeg/components.txt
- [ ] Create src/media/pdf.rs (poppler render to PNG seam)
- [ ] Add convert.pdf registry operation
- [ ] Add registry test
- [ ] Add real test: PDF→PNG multi-page
- [ ] Manual verification: PDF pages render correctly

### B-9.5 Documents / Slides / Spreadsheets (REVISED 2026-10-24)
Replaces the old "LibreOffice headless" plan. Engine chain, resolved at runtime
in this order — every engine behind one seam in `src/media/documents2.rs`:

    Word/Excel/PowerPoint COM  ->  LibreOffice (if installed)  ->
    office2pdf (optional ~20 MB download)  ->  clear "how to enable" message

**Engine A — Office COM (zero bytes shipped, perfect fidelity):**
- [ ] Create src/media/documents2.rs with the engine-chain seam
- [ ] Word COM: Documents.Open -> SaveAs2 wdFormatPDF (DOCX->PDF)
- [ ] Excel COM: XLSX->PDF; PowerPoint COM: PPTX->PDF + slide export JPG/PNG
- [ ] Runtime detection (registry lookup of Office install, graceful absence)

**Engine B — LibreOffice (detected, never required, never bundled):**
- [ ] Detect soffice.exe in standard locations; reuse chain seam

**Engine C — office2pdf for users with no MS Office (optional download):**
- [ ] Evaluate output quality on real DOCX/PPTX/XLSX test set (v0.9.1, young)
- [ ] If good: optional in-app download (~20 MB, user consent), store under
      %LOCALAPPDATA%\OpenTangerine\engines\; base installer stays small
- [ ] PPTX->JPG/PNG without Office = office2pdf->PDF then poppler render (B-9.4)

**Native routes (always available, no engine, milliseconds):**
- [ ] DOCX->TXT: zip + XML walk of word/document.xml (zip crate already a dep)
- [ ] XLSX/XLS/ODS -> CSV/TSV: calamine (~1.5 MB)
- [ ] CSV/TSV/XLSX/ODS interconversion via calamine
- [ ] MD/TXT -> PDF via markdown->HTML->PDF (small crates)
- [ ] DOCX->HTML (approximate styling) and -> EPUB (zip of HTML) — later

**Related cheap wins from the website matrix:**
- [ ] Images->PDF via pdf-writer crate (fills the "PDF export" row)
- [ ] PDF->TXT via poppler text layer (free once B-9.4 lands)

**Registry operations:** convert.docx (PDF/TXT targets), convert.pptx
(PDF/JPG/PNG), convert.xlsx (PDF/CSV) — `implemented` reflects engine
availability at runtime; wheel never shows a conversion it cannot run.

**Descoped (registered implemented: false, reason stated):**
- PDF -> DOCX/PPTX/XLSX: layout reconstruction, no lightweight offline engine
- PAGES input: Apple-only format, N/A on Windows

### B-9.6 Audio Formats
- [ ] Add wav, ogg, aiff, m4a to ffmpeg/components.txt
- [ ] Add to ffmpeg/build.sh configure flags
- [ ] Add registry operations (convert.wav, convert.ogg, etc.)
- [ ] Add real-FFmpeg ignored tests (audio→audio conversion)
- [ ] Manual verification: audio files convert correctly

### B-9.7 RAR Input
- [ ] Assess unrar/7z feasibility
- [ ] Add rar to ffmpeg/components.txt (read = -)
- [ ] Create external tool invocation
- [ ] Add registry operation (implemented: false until unrar detected)

---

## Phase B-10: Remaining Tools (19 of 26)

### B-10.1 Background Removal
- [ ] Assess ONNX Runtime/rembg size impact (~50-100 MB)
- [ ] Decision: defer or implement
- [ ] If implement: add tools.background-remove registry operation
- [ ] Add ONNX Runtime dependency

### B-10.2 Remove Metadata (EXIF strip)
- [ ] Add exif crate dependency
- [ ] Create metadata strip implementation
- [ ] Add tools.metadata-strip registry operation
- [ ] Add real test: strip EXIF from JPEG

### B-10.3 Read QR
- [ ] Assess qr2/barcode crate size
- [ ] Decision: defer or implement
- [ ] If implement: add tools.read-qr registry operation

### B-10.4 Split
- [ ] Implement split for video/images using FFmpeg
- [ ] Add tools.split registry operation
- [ ] Add real test: split video at timestamp

### B-10.5 Remaining ~14 Tools
- [ ] Review reference tools page (tangerineformac.com/tools)
- [ ] List all missing tools
- [ ] Implement FFmpeg-filter-based ones (flip, grayscale, etc.)
- [ ] Mark unsupported ones as implemented: false with reason
- [ ] Add registry tests for all implemented tools

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
| Background removal | ~50-100 MB ONNX Runtime — too heavy |
| QR reading | Needs barcode library — assess size first |
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
