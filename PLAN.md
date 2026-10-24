# Implementation Plan — Open Tangerine for Windows (Path B)

**Goal:** Reach parity with the macOS product (tangerineformac.com) on Windows: image, video, audio, PDF, documents, slides, spreadsheets, subtitles, archives conversions; 26 tools; 6 file actions.

**Scope & constraints:**
- Personal / open-source use; GPL/LGPL dependencies acceptable.
- Final app size should be low; app should be fast.
- DOCX / PPTX / XLSX deferred to later (no reliable offline Rust library yet).
- Process phase by phase. Do not skip phases.
- Stop for review before starting document or audio work (review checkpoint = end of Phase B-4).
- Implement only genuinely supported capabilities — no fake formats.
- Do not commit or push; preserve working-tree changes; no remote Git operations.
- Author: Ravi <ravirrajpatil@gmail.com>; Conventional Commits when committing.

**Baseline (verified):** All 12 real-FFmpeg ignored tests pass against `vendor/ffmpeg`. The existing 6-image pipeline (PNG/JPEG/WebP/GIF/TIFF/BMP) works end-to-end.

---

## Phase B-0 — Baseline (DONE)

**Objective:** Prove current image pipeline works against bundled FFmpeg.

**Command:**
```powershell
cargo build --features test-helper --bins
cargo test --features test-helper -- --ignored
```

**Result:** 12 / 12 passed. Foundation is proven.

---

## Phase B-1 — Full FFmpeg vendor build

**Objective:** Replace minimal `--disable-everything` FFmpeg with a full build supporting video/audio muxers, codecs, libheif, libavif, dav1d.

**Files to change:**
- `ffmpeg/package.sh` — rewrite configure flags:
  - Video muxers/encoders: mp4, mkv, webm, avi, wmv, gif
  - Audio encoders/decoders: mp3, aac, flac, opus, wma
  - Video decoders: h264, hevc, vp8, vp9, av1
  - Image: apng, webp_anim, libheif, libavif
  - Enable libheif, libavif, dav1d with Windows pkg-config paths
- `ffmpeg/components.txt` — update to list all enabled parts (single source of truth; build.rs assertions must match)
- `build.rs` — post-build assertions reflect new component list
- Vendor binaries: rebuild `vendor/ffmpeg/ffmpeg.exe` and `ffprobe.exe`

**Risk:** dav1d/pkg-config build failure on Windows. May need pre-built static libs or MSYS2 packages.

**Tests:** `vendor/ffmpeg/ffmpeg.exe -encoders -decoders -formats` lists all needed parts; `cargo test` green.

**Exit criteria:** Full FFmpeg runs and lists all required encoders/decoders/muxers.

---

## Phase B-2 — Manifest + probe expansion

**Objective:** Manifest describes every format/part the new build has; probe reports capabilities correctly.

**Files to change:**
- `ffmpeg/components.txt` — add format entries for heic, avif, svg (write-only input via resvg seam), ico, mp4, mkv, webm, avi, wmv, mp3, aac, flac, opus, pdf, srt, vtt, zip
- `src/media/manifest.rs` — parse new formats; `read` or `write` may be `None` for write-only/read-only formats
- `src/media/probe.rs` — `can_read`/`can_write` handle absent read/write; `environment()` builds processor reads/writes from manifest
- `src/media/command.rs` — builds FFmpeg args for video/audio (no composite graph for video/audio-only outputs)

**Tests:** Manifest parse tests for every new format; probe tests with scripted listings.

**Exit criteria:** Manifest and probe correctly report capabilities for all target formats.

---

## Phase B-3 — Pipeline redesign — job system

**Objective:** Replace single-shot `Converter::convert` with a job system supporting progress, cancellation, multi-input, mixed media types.

**Files to change:**
- `src/media/job.rs` (new) — `Job`, `JobStatus`, `JobResult`, progress reporter, cancellation token
- `src/media/convert.rs` — becomes per-format dispatch: image → FFmpeg image path (existing), video/audio → filtergraph/muxer path, document/archive → external tool or Rust crates
- `src/radial/execution.rs` — handler returns `Succeeded`/`Failed`/`Cancelled` with progress
- Keep original-file preservation and never-overwrite rules from `src/media/publish.rs`

**Tests:** Job cancellation stops FFmpeg process; progress reporting; multi-file batch; same-format rejection; collision handling.

**Exit criteria:** Existing image conversions still work through new job system; cancellation works.

---

## Review checkpoint (STOP HERE)

**Deliverable:** Report to user:
- What works (image formats, job system)
- What is unsupported and why
- Recommendation to proceed into document/audio (Phase B-4+) only with explicit approval

**Do not proceed past this point without user go-ahead.** Required by constraints.

---

## Phase B-4 — Image format completion (HEIC, AVIF, SVG, ICO)

**Objective:** Image formats Tangerine supports.

**Files to change:**
- `src/media/ico.rs` (new) — ICO container wrapper around FFmpeg BMP/PNG output
- `src/media/heic.rs` (new) — libheif binding or FFmpeg with libheif
- `src/media/avif.rs` (new) — libavif + dav1d (fix pkg-config build)
- `src/media/svg.rs` (new) — resvg rasterize to PNG, then existing image pipeline
- `src/media/convert.rs` — route heic/avif/svg/ico through correct seam

**Tests:** Real conversions for each format; transparency preservation; dimension checks; magic-byte validation.

**Exit criteria:** PNG↔HEIC↔AVIF↔SVG↔BMP↔ICO conversions produce valid files.

---

## Phase B-5 — Video & audio conversions

**Objective:** MP4/MOV/MKV/WebM/AVI/WMV/GIF; MP3/M4A/WAV/FLAC/OGG/Opus/AIFF/WMA; audio extraction from video.

**Files to change:**
- FFmpeg filtergraph args for video encode/decode, audio extraction, GIF from video
- New `video` and `audio` processors in registry/probe
- `src/radial/registry.rs` — add convert.mp4, convert.mp3, etc. operations

**Tests:** Real video→video, video→audio, audio→audio conversions; output plays.

**Exit criteria:** A short MP4 converts to WebM and MP3 without error.

---

## Phase B-6 — Documents, PDF, slides, sheets, subtitles, archives

**Objective:** Non-media formats. DOCX/PPTX/XLSX deferred to later.

**Files to change:**
- PDF → images/text: poppler or equivalent (heavy Windows dependency)
- Subtitles SRT↔VTT↔TXT: native Rust text parsing (simple)
- Archives ZIP/TAR/GZIP/RAR input: `zip`, `flate2`, `tar` crates; RAR needs `unrar`

**Risk:** DOCX/PPTX/XLSX may not be feasible offline without LibreOffice. Mark unsupported if infeasible.

**Tests:** PDF→PNG, ZIP extraction, SRT↔VTT.

**Exit criteria:** Feasible formats work; infeasible ones documented as unsupported with reason.

---

## Phase B-7 — Tools wheel (26 operations)

**Objective:** Compress, crop, trim, split, merge, background removal, metadata, QR.

**Files to change:**
- `src/radial/registry.rs` — flip tools from `implemented: false` to real handlers
- Crop/trim: FFmpeg filtergraph on image/video
- Compress: re-encode at lower quality/bitrate
- Merge: FFmpeg concat demuxer (same-kind only)
- Metadata: EXIF/XMP — `exif` crate or FFmpeg metadata
- Background removal / QR: assess size/license; may be out of scope for low-size goal

**Tests:** Crop dimensions, trim duration, merge output plays.

**Exit criteria:** Core tools (crop, trim, compress, merge) work on image/video.

---

## Phase B-8 — File actions wheel (6 operations)

**Objective:** Copy path, duplicate, rename, reveal in Explorer (Windows equivalents of macOS AirDrop/Float/Terminal).

**Files to change:**
- All `Requirement::None` — run in platform layer, no FFmpeg
- Win32 clipboard for copy path; `SHFileOperation`/`MoveFileEx` for duplicate/rename; `ShellExecute` for reveal

**Tests:** Eligibility (folders allowed), collision handling, no FFmpeg needed.

**Exit criteria:** All actions run without FFmpeg.

---

## Phase B-9 — UI & product polish

**Objective:** Settings, tray, shortcuts, notifications, progress.

**Files to change:**
- Settings window: appearance (light/dark), modifier keys, output location
- Tray icon + startup registration
- Progress notifications during conversion; cancellation UI
- Wire existing wheel to new job system

**Tests:** Settings persistence; shortcut parsing; tray show/hide.

**Exit criteria:** App launches to tray, opens wheel on Shift-drag, shows progress during conversion.

---

## Phase B-10 — Packaging & release

**Objective:** Produce installable Windows package.

**Files to change:**
- `ffmpeg/package.sh` — repackage with full FFmpeg + all DLLs
- `dist\OpenTangerine\open-tangerine.exe` rebuilt (current one is stale)

**Exit criteria:** Package runs on clean Windows 11 without dev environment.

---

## Effort estimates

| Phase | Effort |
|---|---|
| B-0 Baseline | Done |
| B-1 Full FFmpeg build | 3–5 days |
| B-2 Manifest/probe | 2–3 days |
| B-3 Job system | 5–8 days |
| Review checkpoint | — |
| B-4 Image formats | 3–5 days |
| B-5 Video/audio | 5–10 days |
| B-6 Documents/archives | 5–15 days |
| B-7 Tools | 5–10 days |
| B-8 File actions | 2–3 days |
| B-9 UI polish | 5–10 days |
| B-10 Packaging | 2–3 days |

**Total realistic range: 30–70 days** of focused work.

---

## Next immediate step

Begin **Phase B-1**: inspect and rewrite `ffmpeg/package.sh` for the full FFmpeg build.

Shall I start B-1?