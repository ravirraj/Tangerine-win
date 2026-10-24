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

### B-9.3 SVG Input (IN PROGRESS)
- [x] Assess resvg integration (vendored via Slint 1.18)
- [ ] Add svg to ffmpeg/components.txt (read = -)
- [ ] Create src/media/svg.rs (resvg rasterize to PNG seam)
- [ ] Add convert.svg registry operation
- [ ] Add registry test
- [ ] Add real test: SVG→PNG→JPEG chain
- [ ] Manual verification: SVG rasterizes correctly

### B-9.4 PDF Support
- [ ] Assess poppler Windows build feasibility
- [ ] Add pdf to ffmpeg/components.txt
- [ ] Create src/media/pdf.rs (poppler render to PNG seam)
- [ ] Add convert.pdf registry operation
- [ ] Add registry test
- [ ] Add real test: PDF→PNG multi-page
- [ ] Manual verification: PDF pages render correctly

### B-9.5 DOCX/PPTX/XLSX
- [ ] Assess LibreOffice headless feasibility (user-installed, not bundled)
- [ ] Add docx/pptx/xlsx to ffmpeg/components.txt (read = -)
- [ ] Create src/media/documents2.rs (LibreOffice shell invocation)
- [ ] Add registry operations (implemented: false until LibreOffice detected)
- [ ] Document as optional/external dependency

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
| DOCX/PPTX/XLSX bundled LibreOffice | Too large — detect at runtime only |
| AirDrop | No Windows API — use Nearby Share fallback |
| Code signing | Optional for personal use |

---

## Progress Tracking

Update this file as each item is completed. Mark with [x] when done.

**Current branch:** feature/image-conversion
**Phase branches:** phase/b-9-*, phase/b-10-*, etc. (create as needed)
**Merge to master:** Only after user approval at the end

---
