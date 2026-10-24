# Open Tangerine for Windows — Implementation Plan (Path B)

**Goal:** Reach parity with tangerineformac.com on Windows.
**Author:** Ravi <ravirrajpatil@gmail.com>
**Conventional Commits.** No commit/push until approved.
**Personal/open-source use.** GPL/LGPL dependencies acceptable (FFmpeg, poppler, libheif).
**Low app size and fast performance** are priorities.
**Do not implement until approved.** This document is the plan only.

---

## Baseline (Verified)

| Phase | Status | What |
|---|---|---|
| B-0 | ✅ Done | Real FFmpeg baseline — 12/12 ignored tests pass |
| B-1 | ✅ Done | Full FFmpeg build (video/audio codecs) merged to feature/image-conversion |
| B-2 | ✅ Done | Manifest/probe updated for new formats |
| B-3 | ✅ Done | Job system skeleton (src/media/job.rs) |
| B-4 | ✅ Done | ICO output + HEIC/AVIF/SVG documented as unsupported |
| B-5 | ✅ Done | Video/audio operations (MP4/MKV/AVI/WMV registered + real-tested) |
| B-6 | ✅ Done | Documents/subtitles/archives (SRT/VTT/TXT/ZIP/TAR/GZ) |
| B-7 | ✅ Done | Tools wheel (compress, crop, trim, rotate, resize, merge, metadata) |
| B-8 | ✅ Done | File actions (copy path, duplicate, rename, reveal, terminal, extract) |

**Current branch:** `feature/image-conversion` (33 commits ahead of origin, all local, no push)
**Tests:** 297 unit + 16 real-FFmpeg, 0 failures

---

## Reference Product (tangerineformac.com)

**Product:** Tangerine for Mac by Leap Studio, $29.99, macOS 13+, Apple silicon
**Key behaviors:**
- Drag a file, tap Shift → conversion wheel opens under cursor
- Tap Command → file tools wheel
- Tap Control → file actions wheel
- Wheel stays open until drop/cancel
- Escape cancels
- Each tool opens as a **dedicated modal window** with its own UI (e.g., "Merge PDFs" shows file ordering + Merge button)
- Output beside original, never overwrites
- 275 conversions, 26 tools, 6 file actions
- Local processing, no upload

---

## What's Missing (Gap Analysis)

### 1. Image Formats (blocked — no decoder in toolchain)
| Format | Reference | Status | Blocker |
|---|---|---|---|
| HEIC input/output | Supported | ❌ | No libheif/libde265 |
| AVIF input | Supported | ❌ | No libavif/dav1d |
| AVIF output | Supported | ❌ | No libavif |
| SVG input | Supported | ❌ | No resvg integration |
| PDF export from images | Supported | ❌ | No PDF library |

### 2. Video/Audio Formats
| Format | Status |
|---|---|
| MP4, MKV, AVI, WMV | ✅ Done |
| MOV, WebM | ⚠️ Registered, untested |
| MP3, AAC, FLAC, Opus | ⚠️ Some registered, untested |
| WAV, OGG, AIFF, M4A | ❌ Not registered |

### 3. Documents / Slides / Subtitles
| Format | Reference | Status |
|---|---|---|
| PDF → images/text | Supported | ❌ No poppler/LibreOffice |
| DOCX, PPTX, XLSX | Supported | ❌ No offline Rust library |
| PPT/PPTX → PDF/JPG/PNG | Supported | ❌ Missing |
| SRT, VTT → TXT | Supported | ✅ Done |

### 4. Advanced Tools (26 in reference — 7 implemented, 19 missing)
**Implemented:** Compress, Crop, Trim, Rotate, Resize, Merge, Metadata

**Missing (19):** Background removal, Remove metadata, Read QR, Split, plus ~13 others from reference tools page

### 5. File Actions (Windows equivalents)
| Reference | Windows Equivalent | Status |
|---|---|---|
| AirDrop | Nearby Share / Smart Share | ❌ No public API — fallback |
| Float | None | ❌ N/A |
| Copy (duplicate) | ✅ | Done |
| Move/rename | ✅ | Done |
| Copy Path | ✅ | Done |
| Terminal | ✅ | Done |
| Reveal in Explorer | ✅ | Done |
| Extract archive | ✅ | Done |

### 6. Interaction Model
| Feature | Status |
|---|---|
| Escape cancels | ❌ |
| Enter/arrow keys on file | ❌ |
| Auto-retract wheel on success | ❌ |
| Progress indicator | ❌ |
| Notification on completion | ❌ |
| Sound toggle | ❌ |

### 7. Windows Integration
| Feature | Status |
|---|---|
| System tray | ❌ |
| Launch at login | ❌ |
| Windows notifications | ❌ |
| Right-click Explorer integration | ❌ |

### 8. Settings
| Feature | Status |
|---|---|
| Appearance (light/dark) | ✅ Exists |
| Output location | ❌ |
| Shortcut customization | ❌ |
| Auto-retract toggle | ❌ |
| Sound toggle | ❌ |
| Persistence | ❌ No settings storage |

### 9. Packaging
| Item | Status |
|---|---|
| `dist\` binary is stale | ❌ Needs repackaging |
| Installer (Inno Setup/NSIS) | ❌ |
| Code signing | ❌ |

---

## Tool Windows Decision

**Reference behavior:** Each tool on tangerineformac.com/tools opens as a **dedicated modal window** with its own configuration UI (e.g., "Merge PDFs" shows file reordering + Merge button).

**Decision:** Option A — Keep the radial wheel as the primary interaction, but when a tool segment is clicked, open a **tool configuration window/modal** with format-specific controls. This matches the reference product's interaction model.

**Tool windows to implement (one per tool):**
- Compress: quality slider, output format
- Crop: drag crop box, aspect ratio options
- Trim: start/end time sliders for video/audio
- Rotate: 90°/180°/270° buttons, flip options
- Resize: width/height inputs, percentage, maintain aspect ratio
- Merge: file list with drag-to-reorder, output format
- Metadata: EXIF editor fields
- Background removal: (deferred — too heavy)
- Read QR: (deferred — needs barcode library)
- Split: split point inputs for PDF/video

---

## Implementation Phases (in order)

| Phase | What | Est. Effort | Dependencies |
|---|---|---|---|
| **B-9.1** | HEIC input/output | 3–5 days | libheif/libde265 Windows build |
| **B-9.2** | AVIF input/output | 3–5 days | libavif/dav1d (previous build failed) |
| **B-9.3** | SVG input | 2–3 days | resvg (vendored via Slint) |
| **B-9.4** | PDF support | 5–10 days | poppler (Windows build) |
| **B-9.5** | Documents/slides/spreadsheets | 5–10 days | Engine chain: Office COM → LibreOffice (detect) → office2pdf (optional download); native TXT/CSV routes |
| **B-9.6** | Audio formats (WAV, OGG, AIFF, M4A) | 2–3 days | Native FFmpeg codecs |
| **B-9.7** | RAR input | 2–3 days | unrar/7z external |
| **B-10** | Remaining 19 tools | 10–15 days | FFmpeg filters + Rust crates |
| **B-11** | Terminal + Extract (done) + Nearby Share | 2–3 days | Windows share contract fallback |
| **B-12** | Progress UI, Cancellation, Escape/Keyboard | 5–8 days | None |
| **B-13** | Tray, Startup, Notifications | 3–5 days | Win32 APIs |
| **B-14** | Settings UI + persistence | 5–10 days | serde + serde_json |
| **B-15** | Packaging | 3–5 days | Inno Setup/NSIS |

**Total estimated: 40–85 days**

---

## Dependencies & Licensing

| Dependency | Size | License | Notes |
|---|---|---|---|
| libheif + libde265 | ~2 MB | LGPL/GPL | Static link |
| libavif + dav1d | ~3 MB | BSD | Previous build failed |
| poppler (PDF) | ~10–20 MB | GPL | Optional/external |
| calamine (XLSX/XLS/ODS read) | ~1.5 MB | MIT | Native spreadsheet routes |
| pdf-writer (images→PDF) | ~0.5 MB | Apache/MIT | Cheap win, fills "PDF export" |
| office2pdf (DOCX/PPTX/XLSX→PDF) | ~20 MB | Apache-2.0 | Optional download for no-Office users, not bundled |
| exif-rs | ~0.5 MB | MIT | Small |
| ONNX Runtime (background removal) | ~50–100 MB | MIT | **Too heavy** — defer |
| serde + serde_json | ~0 | MIT | For settings |

**Total estimated added size:** ~5–15 MB (excluding optional background removal)

---

## Files to Create (New)

```
src/media/heic.rs           — HEIC wrapper
src/media/avif.rs           — AVIF wrapper
src/media/svg.rs            — SVG rasterize seam
src/media/pdf.rs            — PDF via poppler seam
src/media/documents2.rs     — DOCX/PPTX/XLSX via LibreOffice shell
src/media/audio.rs          — Audio format handlers
src/platform/windows/tray.rs       — System tray
src/platform/windows/shortcut.rs   — Startup registry
src/platform/windows/notifications.rs — Toast notifications
src/ui/progress.rs          — Progress UI
src/ui/settings.rs          — Settings logic
src/ui/settings.slint       — Settings UI
src/ui/sound.rs             — Optional sound
src/radial/tools.rs         — EXISTING (add remaining tools)
src/radial/file_actions.rs  — EXISTING (already has terminal/extract)
```

## Files to Modify (Existing)

```
Cargo.toml                  — Add heif, avif, serde, serde_json, exif
ffmpeg/components.txt       — Add heic, avif, svg, pdf, docx, pptx, xlsx, wav, ogg, aiff, rar, ppt
ffmpeg/build.sh             — Add new library configure flags
ffmpeg/package.sh           — Include new DLLs and licenses
src/media/command.rs        — Already updated (video_filter param)
src/media/convert.rs        — Add SVG/seam routing, document routing
src/media/probe.rs          — Already updated (document processor)
src/media/manifest.rs       — No change (generic)
src/radial/registry.rs      — Add all new operations
src/radial/actions.rs       — Add IconKind::Terminal, IconKind::Extract; update menu tests
src/radial/icons.rs         — Add TERMINAL, EXTRACT paths
src/radial/controller.rs    — Add keyboard navigation, Escape, progress
src/app.rs                  — Wire Documents, Tools, FileActions handlers; settings
src/platform/windows/mod.rs — Add clipboard, shell, tray modules
```

---

## Stop Conditions

**Do not implement:**
- Background removal (ONNX Runtime ~50-100 MB — too heavy)
- QR reading (needs barcode library — assess size first)
- PDF → DOCX/PPTX/XLSX (layout reconstruction — no lightweight offline engine)
- Bundled LibreOffice or bundled office2pdf — engines are detected or
  optionally downloaded, never shipped in the installer

**Do not implement AirDrop** — replace with Windows Nearby Share fallback (Explorer selection + manual share).

---

## Approval Checklist

- [ ] Plan documented in PLAN.md
- [ ] todo.md created with actionable items
- [ ] User approves starting implementation
- [ ] User confirms tool windows (Option A) as interaction model
- [ ] User confirms which phase to start with

---

## Next Steps (Pending Approval)

1. User approves plan
2. User selects starting phase (recommend B-9.1 HEIC or B-12 Progress UI)
3. Implementation begins on `phase/b-9-...` branch
4. Merge to `feature/image-conversion` after each phase
5. Merge to `master` only at the end

---
