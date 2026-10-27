# Bug Ledger — Stabilization Campaign

Statuses: `confirmed` (reproduced or established conclusively) · `probable` (strong evidence, no repro yet) · `potential` (needs evidence) · `design` (recommendation).

| ID | Title | Feature | Severity | Status |
| --- | --- | --- | --- | --- |
| BUG-001 | Tray icon never wired: no way to see, configure, or exit the background app | App shell | P1 | fixed, verified (menu hand-check manual) |
| BUG-002 | Channels window aborts on Tab + arrow keys (nested RefCell borrow) | Audio channels card | P0 | fixed, verified |
| BUG-003 | Timeline card aborts on any press of a card button (nested RefCell borrow across Slint dispatch) | Trim / Split / Snapshots cards | P0 | fixed, verified |
| BUG-004 | Wheel scale frozen at process start: wrong wheel size on a differently-scaled monitor | Radial wheel / DPI | P1 | fixed, verified (single-scale runtime) |
| BUG-005 | Low-level mouse hook never calls CallNextHookEx | Input hooks | P2 | fixed (code-verified) |
| BUG-006 | Hover can light a wedge before the entrance placement is what is drawn | Radial wheel | P2 | not a bug (premise disproven) |
| BUG-007 | Overlay `place()` marks the window placed even when SetWindowPos fails | Radial wheel | P2 | fixed (code-verified) |
| BUG-008 | Drop-target coordinates (0,0) treated as a real pointer position | Drag & drop | P3 | fixed, verified |
| BUG-009 | Documents conversions silently overwrite existing files (and can overwrite the input itself) | Documents wheel | P1 | fixed, verified |
| BUG-010 | LibreOffice export can never succeed: staging removed before the result is checked | Documents wheel | P1 | fixed, verified (stub program) |
| BUG-011 | Office conversions write straight onto the target path (no staging publish) | Documents wheel | P2 | resolved by caller staging; verified live |
| BUG-012 | Archive extraction pours without any limit; a refused extract leaves its half-made `.extracted` folder behind | Documents wheel + file actions | P2 | fixed, verified |
| BUG-013 | Pdfium could be bound out of the folder the program ran in - a library of no known kind | Media (pdf) | P3 | fixed, verified |
| BUG-014 | A failed tool conversion leaves its `.part` beside the user's file | Tools wheel | P3 | fixed, verified (a `SweepOf` guard at every staging site) |
| BUG-015 | Spawned helpers (office COM, LibreOffice, ffmpeg) run with no timeout: a wedged helper parks its worker for ever | Media pipelines | P3 | fixed for program spawns, verified (run cap + job objects); COM wedges said apart |
| BUG-016 | No cancellation, and no per-run staging: two operations on the same file run their pipelines at once and share one `.<ext>.part` name | Tools wheel workers | P2 | staging race fixed, verified (per-run names everywhere); cancellation decided (dispatch-time refusal stands) |
| UI-1 | One accent said three ways: windows hardcode `#FF5C00`/`#FF5A1F`, the tokens say `#ff5a1f`, the wheel's emphasis is `#ff8a3d` | UI tokens | P3 | fixed, verified (the accent uses fill from the token; the panel palettes and the wheel's wash are not accent) |
| BUG-017 | The audio trim's "Play Selection" button and its Space stop answer nothing (born-dead affordance) | Audio trim card | P3 | fixed, verified (selection cut + MCI play + the walk) |
| BUG-018 | Space performed the trim instead of playing the preview: Enter should perform, Space should play (user-reported) | Trim / video tool cards | P1 | fixed, verified |
| BUG-019 | Dragging a trim cap did not move the preview: the playhead stayed where it stood (user-reported) | Trim card | P2 | fixed, verified |

## BUG-001 — Tray icon never wired

- **Feature:** App shell / lifecycle
- **Severity:** P1
- **Evidence:** `src/platform/windows/tray.rs` defines `add_tray_icon`/`show_tray_menu`/`WM_TRAYICON`; a repo-wide search finds **no callers** (`Select-String` over `src/**/*.rs` → definition file only). PLAN.md marks milestone B-13 (tray + context menu + chime) as ✅ Done; HANDOFF.md documents the tray module. The app is a background process with no visible window; without the tray the user cannot see it is running, open settings, or quit it except via Task Manager.
- **Root cause:** module written but never called from the startup path (born dead in commit 1ca7992).
- **Fix:** `src/platform/windows/tray.rs` now owns a hidden `TrayWindow` (its own window class + `tray_wndproc`) that carries the icon; clicks land there:
  - left-click of the icon → `TrayCommand::Open` — the wheel opens at the pointer, exactly as the Ctrl+Alt+T shortcut does;
  - right-click → `show_tray_menu` (Open Tangerine / — / Exit), whose `WM_COMMAND` answers are queued;
  - `App::handle_tray` (new) drains the queue in the run loop's message branch: `Open` → `open_development_wheel()`, `Exit` → `self.quit = true` — the same clean stop `--exit-after` uses;
  - `run()` removes the icon on shutdown, so nothing is left on the tray of a stopped program;
  - a tray that cannot be had is logged (`Level::Warn`) and never fatal — the program works without it, as before.
  - **Scope note:** the menu item *Settings…* was left out on purpose: no settings window is wired yet (see `src/main.rs` module doc lines 7–11 and `pub const TRAY_ID_SETTINGS` retained in tray.rs). A menu item that answers nothing would tell the person something that is not so.
- **Regression test:** `the_tray_hands_what_was_chosen_over_to_the_queue` (default suite) — makes the real tray window/icon, imitates the icon's left click (`WM_TRAYICON` + `WM_LBUTTONUP`), then the menu's answers (`WM_COMMAND` ×2), and asserts the queue reads `[Open]`, `[Open, Exit]` and is empty after draining (nothing said twice).
- **Verification:** default suite EXIT=0; release boot `--dev-wheel --exit-after 2` ERRLVL=0 with no tray complaint in the log (the icon is created and removed in the same run), 63.4 fps.
- **Manual procedure** (needs a hand on the taskbar): start the app → the "Open Tangerine" icon shows in the tray → left-click opens the wheel at the pointer → right-click opens the menu with Open Tangerine and Exit → choosing Exit stops the program cleanly (`the application is stopping` in the log, icon gone).
- **Status:** fixed, verified (icon creation, click and queue plumbing automated; menu encounter and Exit click manual).

## BUG-002 — Channels window aborts on Tab + arrow keys

- **Feature:** Audio channels card — keyboard operation of the left/right gain sliders
- **Severity:** P0 (process abort from a user-shaped interaction: Tab into a slider, press an arrow)
- **Reproduction:** show the channels card → `WM_KEYDOWN VK_TAB` (focus moves to the left slider) → `WM_KEYDOWN VK_LEFT`. The process aborts (exit 0xC0000409).
- **Root cause:** in `src/platform/windows/channelswindow.rs` the `WM_KEYDOWN` arms `0x25`/`0x27` called `refresh_card()` **inside** `STATE.with(|s| { s.borrow_mut() … })`; `refresh_card()` itself does `STATE.with(|slot| slot.borrow())` (line 605) → `RefCell already borrowed` → a panic inside the window body aborts the process.
- **Fix:** speedwindow's template (house pattern): the arrow arm closes the borrow with `let moved = STATE.with(…)` returning whether the gain moved, then calls `refresh_card()` only `if moved` — outside any borrow. Tab/Space arms were already correct.
- **Regression test:** `tabbing_to_a_slider_and_arrowing_it_keeps_the_window_up` (default suite) — Tabs, arrows left ×3 and right ×2, asserts the window stays visible and `left_gain` moved 100.0 → 95.0 — proving the arrow path really ran.
- **Coverage evidence:** with the old arrow code restored, the same test aborts: `panicked at src\platform\windows\channelswindow.rs:605:23` ("already borrowed") and the binary exits `-1073740791` (0xC0000409). Fully quashed with the fix.
- **Verification:** default suite `test result: ok. 559 passed; 0 failed; 77 ignored; 0 measured; 0 filtered out; finished in 5.55s` EXIT=0; ignored suite `test result: ok. 77 passed; 0 failed; …; finished in 8.42s` EXIT=0; zero compiler warnings.
- **Status:** fixed, verified.

## BUG-003 — Timeline card aborts on any press of a card button

- **Feature:** Trim / Split / Snapshots cards — every card button: silence rail, play selection, transport step chips, Add Split, Set Equal, stepper, shot remove
- **Severity:** P0 (the residual stage of the user's "audio trim closes when I touch it": commit c3a2fea fixed the strip press; this fixes every press on a control whose handler re-borrows the host)
- **Reproduction:** show the audio trim card → `WM_LBUTTONDOWN` + `WM_LBUTTONUP` over the silence checkbox (or a step chip). The process aborts (0xC0000409).
- **Root cause:** in `timelinewindow.rs` the `WM_MOUSEMOVE|WM_LBUTTONDOWN|WM_LBUTTONUP` arm borrowed `HOST` mutably and called `host.pointer(event)`, which dispatches Slint events synchronously. A pressed card button fires its callback (`on_silence_toggled` → `toggle_silence` → `HOST.with(borrow_mut)` at 1755; `on_step_*` → `step` → `refresh_times` → `HOST.with(borrow_mut)` at 1385; `on_add_split_chosen` → `refit`/`refresh_times`; same for `turn_parts`, `set_equal`, `remove_shot_at`) — a nested borrow of the same `RefCell` while it is still held.
- **Fix:** cropwindow's discipline (cropwindow.rs:1784–1810): take a **shared** borrow, clone `(software, window)` from the host, release `HOST`, then `software.window().dispatch_event(event)` + `request_redraw()` + `draw_window(told_window)` completely outside any `HOST` borrow. A `draw_window(window: HWND)` helper (same shape as cropwindow.rs:687) was added beside `window_handle()`. The now-orphaned `Host::pointer` was removed to keep the zero-warning baseline. toolwindow.rs:687 has the same shape but its callbacks only touch other thread-locals (`OPEN_FOR`/`EVENTS`) — safe today; left as-is and recorded below as latent.
- **Regression test:** `pressing_the_speaking_controls_of_the_card_leaves_the_window_up` (ignored; needs the conversion program) — opens the real audio trim card, presses the silence rail, play-selection button, and both step chips, asserting the window stays visible after each press.
- **Coverage evidence:** with the old dispatch-inside-borrow restored, the same test aborts: `panicked at src\platform\windows\timelinewindow.rs:1748:34: RefCell already borrowed` (the silence chain) and the binary exits `-1073740791` (0xC0000409).
- **Verification:** default suite (559 tests incl. the new channels test) and ignored suite (77 tests incl. the new press test) both pass EXIT=0; zero warnings.
- **Status:** fixed, verified.

## BUG-004 — Wheel scale frozen at process start

- **Feature:** Radial wheel / per-display DPI
- **Severity:** P1 on multi-monitor deployments with mixed scaling (a single-monitor user sees nothing)
- **Evidence:** `detect_scale(cursor)` is called exactly once, in `App::new` (`src/app.rs` ~2316, call at ~341); the wheel's disc size and buffer are sized from that scale, so re-opening on a monitor with a different scale factor keeps using the startup value. docs/INTERACTION.md:185 promises the wheel is drawn at the display's own scale.
- **Root cause:** the scale was captured at startup only, when the wheel was a single-window single-monitor affair.
- **Fix:** `ensure_placement` (src/app.rs) now re-reads `detect_scale(centre)` every time it is about to place; when the scale differs it logs it, calls the new `WheelUi::resize_for(scale)` (window physical size + buffer re-allocated in `src/ui/wheel.rs`), and continues with the updated `self.scale`, so the placement rectangle, the wedge geometry and the view model all follow on the same frame. The overlay DIB already re-sizes itself in `present()→ensure_surface`, so no change was needed there.
- **Regression test:** `the_window_and_buffer_take_the_size_of_the_display_asked` (default suite): `WheelUi::new(1.0)` → `resize_for(2.0)` doubles window and buffer exactly, and a repeated `resize_for` keeps everything.
- **Verification:** default suite `test result: ok. 561 passed; 0 failed; 77 ignored; …` EXIT=0; boot run `--dev-wheel --exit-after 2` ERRLVL=0 with no scale-change log at 1.00 (correct no-op path), 63.4 fps. **Limitation:** this machine has one 100%-scaled display; the multi-monitor path is code-verified only. Manual check: dual monitors with different scaling → drag files on each → the wheel is the same physical size on both and logs `the wheel is drawn at the scale 1.25/1.5/…`
- **Status:** fixed, verified (single-scale runtime; multi-scale documented manual procedure).

## BUG-005 — Low-level mouse hook never calls CallNextHookEx

- **Feature:** Low-level mouse hook (wheel drag detection)
- **Severity:** P2 (breaks hook chaining etiquette; can disturb other apps' hooks and, depending on timing, starve the system's own input path)
- **Evidence:** `src/platform/windows/input.rs` `wheel_hook` returned `LRESULT(0)` unconditionally — never calling `CallNextHookEx` — while its own doc comment (line 141–143) claims "the next hook in the chain is always called".
- **Fix:** the hook now ends with `unsafe { CallNextHookEx(None, code, wparam, lparam) }`, and the comment says why.
- **Test:** no automated test of global hook chaining is possible in-harness (it needs a second hooking process and live input). Verified by: compilation, full suites EXIT=0, and the boot run in which the hook is installed while the wheel is visible with no errors.
- **Status:** fixed (code-verified; global chaining effect documented manual procedure).

## BUG-006 — Hover lights wedges before the entrance matches the drawing

- **Feature:** Radial wheel entrance animation
- **Severity claimed:** P2 (a transient wrong highlight right at open)
- **Assessment: the premise is disproven; there is no mismatch.**
  - The entrance is an **opacity fade, not a scale-up**: in `src/ui/wheel.slint` the only consumer of `appear` on the disc is `opacity: root.appear` (line 78); the wedges are laid out inside the disc at full fixed geometry through the whole entrance. (`appear` also fades the notice alone, lines 268–269.)
  - The `appear`-scaled things on the Rust side (`build_view_model`: icon/hub/label sizes, `src/ui/wheel.rs`) are contents *inside* a wedge, not hover targets.
  - **Hover and drawing share one geometry source:** the drawing takes the wedges from `self.placement.geometry` (`src/app.rs:2097–2102`), and the hover hit-test is `placement.segment_at(...)` on the very same placement (`src/app.rs:1777–1779`). What lights up is exactly what is drawn, at every frame, faded or not.
  - Bonus coherence: the hover path runs `ensure_placement()` first (`src/app.rs:1289`), which since BUG-004's fix also follows any scale change of the display.
- A wedge *can* be highlighted while the wheel is still fading in — the pointer was simply already over it, and what lights is what is drawn. That is ordinary UI behaviour, not a defect.
- **Status:** not a bug (evidence recorded; no code change).

## BUG-007 — Overlay place() marks placed even when SetWindowPos fails

- **Feature:** Radial wheel placement
- **Severity:** P2 (on a failure the wheel is never retried at the right spot)
- **Evidence:** `src/platform/windows/overlay.rs` `place()` set the `placed` flag without checking the `SetWindowPos` result, and `App::ensure_placement` set its own `self.placed = true` before calling `place()`, so a refused move was never retried and `present()` kept believing the window was where the wheel belongs.
- **Fix:** `place()` only sets `placed` when the move succeeded and logs or leaves it false otherwise; a new `Overlay::is_placed()` getter lets `ensure_placement` detect the failure and report `InteractionEvent::Failed` the same way every other overlay complaint is (mirrors the `present()` error path), instead of silently pretending.
- **Test:** the failure path cannot be reproduced in-harness (SetWindowPos refuses only on OS-level error). Verified by compilation, full suites, and the boot run (the success path still sets `placed` and opens the wheel).
- **Status:** fixed (code-verified; failure path documented manual procedure).

## BUG-008 — (0,0) drop-target pointer position

- **Feature:** Drag & drop target coordinates
- **Severity:** P3
- **Evidence:** `src/platform/windows/droptarget.rs:64` used `(0, 0)` to stand for "no point", so a drag dropped on the primary monitor's top-left pixel was treated as "no point" and fell back to `input::cursor_position`.
- **Fix:** `point` is now `Cell<Option<(i32, i32)>>` — set to `Some` by `DragEnter`/`DragOver`/`Drop`, and `screen_point()` speaks for itself.
- **Regression test:** `the_drop_point_is_whatever_the_drag_said_and_nothing_until_it_does` (default suite): no point before the drag speaks, (0, 0) is a real point, (123, 456) round-trips.
- **Verification:** default suite EXIT=0 (`561 passed`), zero warnings.
- **Status:** fixed, verified.

## BUG-009 — Documents conversions silently overwrite existing files

- **Feature:** Documents wheel (pdf→png / png→pdf / office conversions)
- **Severity:** P1 **data loss**
- **Evidence:** `src/radial/documents.rs` ignored the passed `WindowsPublisher` (field marked `#[allow(dead_code)]`); both write paths handed `input.with_extension(ext)` straight to the converter, which `File::create`/`fs::write` truncates (`src/media/documents.rs:238,269,297,313,392,623`). Any existing file of the target name is destroyed — re-running "make a PDF" silently overwrites yesterday's `note.pdf`, and an input named `note.SRT` can find its own content targeted.
- **Root cause:** the handler was written before the publisher existed and never adopted it, despite the registry declaring `Collision::Rename` for every document operation.
- **Fix:** the handler now stages every conversion beside its file under a `.<ext>.part` name (`staging_name`, added in `src/radial/documents.rs`), then publishes to the first free name from `names_for(...)` through the real `WindowsPublisher` (`MoveFileExW` without overwrite) — the exact discipline the tools use (`src/radial/tools.rs:199–214`). Staging files are removed on every failure path. The spreadsheet→CSV path keeps its direct write because `convert_sheet` already numbers every sheet out of the way of what is there (`src/media/documents.rs:432–447`). The `convert.pdf` group target got the same treatment. The source-extension is also lowercased, so `NOTE.SRT` converts like `note.srt` does.
- **Regression tests** (default suite): `a_conversion_never_writes_over_what_is_there` (taken name → `note (2).vtt`, the old file byte-for-byte unchanged, no staging left), `a_conversion_whose_name_is_free_keeps_the_plain_name` (free name taken as-is), `a_file_called_with_a_capped_extension_converts_all_the_same` (case).
- **Coverage evidence:** while writing the tests, a test publisher built on `std::fs::rename` was found to **overwrite** on Windows (`MOVEFILE_REPLACE_EXISTING` under the hood) — the double was corrected to refuse existing names, mirroring the real `MoveFileExW`-flags-0 publisher; ledgered here because it confirms the overwrite danger was one step away.
- **Verification:** default suite `test result: ok. 564 passed; 0 failed; 77 ignored; …` EXIT=0; release boot ERRLVL=0. A live wheel check with a real file (drag srt → choose VTT twice) remains a documented manual procedure.
- **Status:** fixed, verified.

## BUG-010 — LibreOffice export can never succeed

- **Feature:** Documents wheel — DOCX/XLS/PPT → PDF through LibreOffice, headlessly
- **Severity:** P1 (for every machine without Office the feature is unusable)
- **Evidence:** `src/media/documents2.rs:195` (old line) called `remove_dir_all(&staging)` **before** `made.is_file()` was checked at all; on the success of the export the made PDF had already been deleted with its staging folder, so the success arm could never be taken and every export — a successful one included — came back `OfficeFailed`. `App` module doc line 11 promises "the wheel never shows what cannot be run" while this path could not run.
- **Root cause:** the staging sweep was written above the result check; ordering bug.
- **Fix:** the made file is checked and taken out of the staging folder *while the staging is still there*; the sweep happens after — in both the success and the failure arm — so no staging is left behind either way.
- **Regression tests** (default suite, driven by a stub `fake-soffice.cmd` that honours LibreOffice's `--outdir`/source contract): `a_libreoffice_export_is_taken_out_of_its_staging_before_the_staging_falls_away` (whole PDF at the name, staging gone) and `a_libreoffice_export_that_made_nothing_takes_its_staging_with_it` (Err(OfficeFailed), no half-made file, staging gone).
- **Coverage evidence:** with the old ordering restored, the first test fails exactly as the live bug would: `the export was made whole: Err(OfficeFailed)` — the stub exited 0 and made the PDF, and the old code still said it failed.
- **Verification:** default suite `test result: ok. 566 passed; 0 failed; …` EXIT=0; zero warnings.
- **Limitation:** LibreOffice itself is not installed on this machine, so the real program cannot be exercised from here. The stub contract (`--convert-to pdf`, `--outdir`, `<stem>.pdf`) is the documented LibreOffice CLI contract. Manual check on a LibreOffice machine: drag a DOCX → cherry the "PDF" segment → `letter.pdf` appears beside it.
- **Status:** fixed, verified (with stub program; real-program check documented).

## BUG-011 — Office conversions write straight onto the target

- **Feature:** Documents wheel — office COM conversions
- **Severity:** P2 — destructive only where the target is a user file
- **Evidence:** `office_export` (`documents2.rs:150–165`) exports straight to `target` through `$env:OT_TARGET`. After BUG-009, the handler stages every conversion, so in the production path `target` is the handler's `.<ext>.part` staging name and never a user file; direct callers passing real user paths have none outside tests.
- **Reclassification:** the destruction hazard this finding described came from the handler writing onto user paths (BUG-009), which is fixed at its root. Keeping the COM script itself simple (no double staging) is the right scope: the caller owns publishing now.
- **Verification live on this machine:** Word and Excel installed; the existing ignored tests `a_docx_is_exported_to_pdf_by_word` and `an_xlsx_is_exported_to_pdf_by_excel` **run and pass** (`test result: ok. 2 passed; 0 failed; …; finished in 6.18s`) — real COM export produces real `%PDF` files.
- **Status:** resolved through the caller boundary; office path verified live where the machine allows it.

## BUG-012 - Archives pour without a limit, and a refusal leaves its half-made folder behind

- **Feature:** Documents wheel (archive to archive, in memory, `src/media/documents.rs`) and file actions (extract to a folder beside the original, `src/radial/file_actions.rs`)
- **Severity:** P2 - a "zip bomb" (says it is small, comes out enormous) glasses over the conversion's memory or the disk; an extraction refused part way leaves a `.<ext>.extracted` folder half-filled beside the user's file, which a later look reads as a made result
- **Fix, one share said twice:**
  - `src/media/documents.rs`: `DocError::ArchiveTooLarge`; `ARCHIVE_DECODE_LIMIT` (512 MiB) passed as a parameter to the extractors; `read_capped` cuts the pour at what still fits and asks one byte further; the zip and tar extractors refuse from each entry's claim *before* a byte is poured, and the pour is cut separately for a header that does not say the truth.
  - `src/radial/file_actions.rs`: `FileActionError::ArchiveTooLarge`; `ARCHIVE_EXTRACT_LIMIT` (512 MiB); `copy_capped`, the same take-and-probe shape on the disk side; header claims refused first; and `extract()` sweeps the `.<ext>.extracted` folder on **every** refusal (including "not an archive at all" and an unreadable source), so nothing half-made is left.
- **Tests:** `a_gz_that_comes_out_beyond_its_share_is_refused`, `a_zip_whose_files_claim_beyond_what_fits_is_refused`, `a_tar_whose_entry_is_beyond_the_share_is_refused` (documents), `an_extraction_that_would_come_out_beyond_its_share_is_refused`, `an_extraction_that_refuses_leaves_nothing_beside_the_original` (file actions) - all in the default suite, using a share of 1 MiB the test can fill.
- **Coverage proofs** (old code temporarily restored, observed verbatim, fix re-applied):
  - sweep: `panicked at src\radial\file_actions.rs:768:9: the leftovers of a refused extraction were swept` - `test result: FAILED. 0 passed; 1 failed`.
  - caps: `panicked at src\radial\file_actions.rs:702:9: assertion 'left == right' failed: a zip beyond the share is refused / left: Ok(()) / right: Err(ArchiveTooLarge)` - `test result: FAILED. 0 passed; 1 failed`.
  - The documents-side tests were reasoned rather than revert-proved: the old extractors took no share at all and poured whole, so the new `Err(ArchiveTooLarge)` cases cannot come out of the old code.
- **Suites after:** default `572 passed; 0 failed; 77 ignored ... finished in 5.57s` EXIT=0; ignored `77 passed; 0 failed ... finished in 7.28s` EXIT=0.
- **Status:** fixed, verified.

## BUG-013 - Pdfium could be bound out of the folder the program ran in

- **Feature:** PDF reading/rendering, `src/media/pdf.rs::library()`
- **Severity:** P3 - the candidate list ended with a bare `pdfium.dll` and a `vendor\pdfium\pdfium.dll` relative to the **current directory**, which for a program started by Explorer is whatever the person stood in. A foreign library of an unknown ABI version would bind (the binding checks the file exists, not its kind) and fault natively instead of refusing.
- **Fix:** the two CWD candidates are gone. What stays is exe-derived only: beside the program, and - through the build folders - the repository's own `vendor\pdfium` when the program was built by cargo. The walk now also covers `target\debug\deps` (where the test programs live), so the tests find the developer's library without the CWD look.
- **Verification:** `cargo test --no-run` EXIT=0; the whole ignored suite with real pdfium on this machine passes (`77 passed; 0 failed ... 7.13s`), including every `#[ignore = "needs pdfium.dll"]` test, which find the library through the new walk.
- **Status:** fixed, verified.

## BUG-014 - A failed tool conversion leaves its `.part` beside the user's file

- **Feature:** Tools wheel (cut/trim/compress/re-time and the rest), `src/radial/tools.rs`
- **Severity:** P3 - no data is destroyed (publishing never writes over), but every staging site (`finish` at tools.rs:172-197, and the inline `compress_picture` sites) leaves the `.<ext>.part` temporary behind when the process fails, the run refuses, or the publish cannot move. A wedged ffmpeg therefore leaves a large half-file beside the user's media, and a repeat of the failure piles them up.
- **Evidence:** `fn finish` (tools.rs:192-194) returns `Err(ToolError::ToolFailed)` with no removal of `temporary`; `fn publish` (tools.rs:205-214) returns through both failure arms the same way. Contrast `src/media/convert.rs`, which sweeps `.part` on every path and whose tests assert nothing is left (`temporaries_left`), and the documents wheel (BUG-009) and the extract action (BUG-012), which sweep too.
- **Fix:** a sweep guard, said once and worn at every site: `SweepOf(PathBuf)` (tools.rs) takes its work file back when the owning function returns — whichever page the return ends on. A run that got as far as publishing has nothing under the staged name any more (the publish moved it), so the sweep is a nothing then; a run that failed anywhere else leaves the half-made work taken back. The guard now stands beside every staging file the wheel makes: the outer per-run staging of `run`, the redact image and redact video `redact` stages, the bleep / normalize / channels stage wavs and outer stages, the split's per-clip work files, the speed flow's whole family (`speed`, `speed-raw.bin`, `speed-frames.bin`, `speed-sound.wav` — the raw plane never got swept even on the run's *success* page before this), and the merge and organize PDF unions. `documents.rs`'s two "no staging file was left behind" assertions were upgraded to a whole-folder leftover scan (`a_leftover_in`) so they stay true whatever a staging name is.
- **Honest note:** a sweep is what a run can owe while it still runs. A program killed by a crash or a machine leaves its debris where its last page stood — as `convert.rs`'s scan-for-`.part` notes say apart — and that is not what this fix can reach.
- **Regression test:** `a_run_that_cannot_publish_sweeps_its_work` (default): a picture redaction where the publisher can never move (`StandingPublisher`) — the run says failed, the folder holds nothing of the run's.
- **Coverage evidence:** with the redact guard taken out, the test fails verbatim:
  `panicked at src\radial\tools.rs:6933:9: nothing of the run was left beside the file, yet C:\Users\thorfinn\AppData\Local\Temp\open-tangerine-sweep-test\photo.6152.1.redact.png stood`
  `test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 654 filtered out; finished in 0.02s`.
- **Verification:** default suite `test result: ok. 579 passed; 0 failed; 80 ignored; 0 measured; 0 filtered out; finished in 5.78s` EXIT=0; with the guard restored, the test goes green again. Shared with the whole battery at the end of this unit.
- **Status:** fixed, verified.

## BUG-015 - Spawned helpers run with no timeout

- **Feature:** every pipeline that starts a program: office COM export (`documents2.rs`), LibreOffice export (`documents2::libreoffice_export`), ffmpeg/ffprobe everywhere (`locator::Tools`)
- **Severity:** P3 potential - the processes run to natural completion; a wedged helper (a hung COM server is a classic) parks its worker thread for ever. The wheel itself keeps drawing (the work runs on workers), but the action never reports, and the worker (and its files) leak.
- **Fix:** the real runner waits, but not for ever (`src/media/runner.rs`). `SystemRunner::run_capped` spawns under a **run cap** (`RUN_CAP_MS: u64 = 600_000` — a generous cap, not a pace; `WAIT_STEP_MS = 50` polling), and the helper is put under a **job of its own** (`Win32_System_JobObjects`, kill-on-close): standing a helper down ends its whole tree, and this program dying takes the tree with it. The piped sides are drained on side threads the way a shell-less wait must, so a talkative ffmpeg can never wedge itself on a full pipe while the wait stands on the process. A helper stood down says `status: None` (it did not end by its own word — `RunResult`'s own saying) and takes a line into its stderr: `the helper was stood down after {cap_ms} ms of standing`. The trait's `run` signature is untouched (it delegates with the cap), so every stub runner and test kept its shape.
- **Honest note (COM):** a COM server runs in the worker's own process, and a wedged COM node cannot be stood down from outside it without putting the office work in a separate program — that isolation is not built here. The spawn families (ffmpeg, ffprobe, LibreOffice) are fully covered; the COM wedge remains possible and is said so here.
- **Regression tests:** `a_helper_beyond_its_cap_is_stood_down` (default): a 30-second sleeper stands against a 500 ms cap — it answers `None` and the saying says why. `a_helper_inside_its_cap_stands_whole` (default): a quick helper under a 30 s cap ends whole (`Some(0)`), because a cap is not a pace.
- **Coverage evidence (direct exercise, not a revert):** with the old body restored the tester itself would hang for ever — the old behaviour *is* the failing mode this fixes — so the proof of this unit is the cap firing in the lived run above: `test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 655 filtered out; finished in 0.52s`.
- **Verification:** part of the end-of-unit battery (suites, release, boot all green).
- **Status:** fixed for program spawns, verified; COM said apart.

## BUG-016 - Two operations on one file share a staging name, and nothing can stop a run

- **Feature:** the worker layer, `src/app.rs::start_operation` (a thread per request), staging in `src/radial/tools.rs` and the two picture compressors
- **Severity:** P2 potential (silent corruption), the trigger being ordinary use: a long job (a 4K compress can take half a minute) and the same file dragged again in that time
- **Evidence:**
  - Every request spawns its own worker with no gate, one at a time check, or queue (`std::thread::spawn` at app.rs:1128); results are only said afterwards (`Message::Finished`).
  - Two workers on the same input compute the **same** staging path (`input.with_extension("{source_ext}.part")`, tools.rs:1782 and the other inline sites), so a second run's `File::create` truncates the file the first run is pouring into; both ends publish corrupted output that looks whole. The media converter avoids this by building its staging from a process id and a number (`.{stem}.{ext}.{pid}.part`, convert.rs:181); the tools wheel never had that.
  - `Outcome::Cancelled` is only ever stated by tests; no production handler says it, and the `ProcessRunner` trait (runner.rs:33-45) has no way to be told to stop. The tests that say "was not asked to stop" are guarding that a normal run never *reports* cancelled - not that a run *can* be stopped.
- **Fix (the staging race, closed):** one name-sayer for the whole program, `staging_name(original, words)` (tools.rs, `pub(crate)`): the stem, the program's own id, and one running number (`AtomicU64`, the counter `convert.rs::temporary_file` carries in the same spirit), the words of the kind of work last so every site keeps the extension its own tools read. Every inline site now says its staging through it — the outer `run` site, the bleep / normalize / channels outer stages *and* their stage wavs, the redact image and video `redact` stages, the speed flow's whole family (`speed`, `speed-raw.bin`, `speed-frames.bin`, `speed-sound.wav`), the merge and organize PDF unions — and `documents.rs`'s staging says it too. Inner unique-folder sites (split's clips folder) keep their names: two runs already braid no buckets there. Each site's work files also wear the BUG-014 sweep guard, so a stopped run takes everything back.
- **The cancellation half, decided:** "cancel" stays **dispatch-time refusal** — the wheel says an operation once, and a run already given does its page. What BUG-015 built (a job of the run's own, a cap, a stand-down) is the honest machinery for *wedged helpers*; killing a helper that is *working* is a different promise than the row asked for, and it is not claimed here. Said so, plainly.
- **Regression tests:** `a_staging_name_is_one_runs_own` (default): two name-sayings of one file are never the same, each carries the program's id. `two_runs_of_one_file_stand_beside_each_other` (ignored, real ffmpeg): two concurrent `tools.trim` runs of the same mp4 (a real move publisher, the production way) both succeed, both results stand, both carry near their asked halves of a second.
- **Coverage evidence, honestly said:** with the old shared name restored, four concurrent tiny-clip races still completed green — at test sizes the two runs' writes are milliseconds apart and the collision window nearly never lands; exactly the **flaky-race territory** this row recorded before the fix. The race's existence stands by the code reading (one name, two writers); the fix's proofs are the name's own uniqueness (the unit test) and both whole runs standing (the integration test). A timed live reproduction would want minutes-long jobs and a wider window than a test rig should ask for.
- **Verification:** part of the end-of-unit battery (suites, release, boot all green).
- **Status:** staging race fixed, verified; cancellation decided (dispatch-time refusal stands) and said so.

## UI-1 - The accent is said three ways

- **Feature:** the shared tokens and every window's fills, `src/ui/tokens.slint`, `src/ui/wheel.slint`, the per-tool `.slint` windows
- **Severity:** P3 (design improvement; the look is coherent today, but the accent cannot be retuned from one place, which the tokens file's own comment calls a rule)
- **Evidence (grep, this pass):** `tokens.slint:25` accent `#ff5a1f`; `settings_window.slint:257` fill `#FF5C00`; `channels_window.slint:102`, `frame_image_window.slint:248`, `normalize_window.slint:102` hardcode `#FF5A1F` instead of the token; `wheel.slint:62-64` emphasis/fading washes `#ff8a3d*` (a lighter wash by intent, said apart here).
- **Evidence, corrected where the reading went deeper:** the reference `theme.rs` says the accent once — `(0xff, 0x5a, 0x1f)` in both themes (`theme.rs:60`/`77`) — so `tokens.slint`'s `#ff5a1f` **is the reference's own value** and needs no re-deciding. The three window tiles named above split honestly in two:
  - **Accent uses (drift):** `channels_window.slint:102` and `normalize_window.slint:102` — the audio centre-lines drawn in a hardcoded `#FF5A1F`, a fill the token owns. Fixed: both now say `background: Tokens.accent`.
  - **Not accent (palette content):** `frame_image_window.slint:248` is the first entry of the frame tool's **gradient-swatch palette** (its neighbour at :284 is another entry, `#FFB75E`), and `settings_window.slint:257` (`#FF5C00`) is the first option of the settings' **accent chooser** — both are lists of colours a person picks among, the same way `annotate_window`'s and `collage`'s palettes are. They are not fills of the accent and they would break as a tool's choices if bound to it. Left as they were, said so here.
  - The wheel's `#ff8a3d` family stays, as before: a wash by intent.
- **Verification:** grep now finds no window fill drawn in a hardcoded accent colour; the two tokenized windows build (`cargo build` EXIT=0, part of the end-of-unit battery).
- **Status:** fixed, verified (the accent uses fill from the token; the palettes and the wheel's wash are content, said apart). The full look-and-polish plan stays with the mapping roadmap (`MAPPING-ROADMAP.md` in the reference folder).

## BUG-017 - The audio trim's Play Selection answers nothing

- **Feature:** the audio trim card - the "Play Selection" button and the Space stop that triggers it, `timelinewindow.rs::play_selection`
- **Severity:** P3 (a born-dead affordance: a person trusting the button is told nothing at all)
- **Evidence:** `unsafe fn play_selection(_window: HWND) { // Affordance for media playback }` - an empty body. Both the button's callback and the `is_audio() && VK_SPACE` keyboard stop (FOCUS 3) land in it. Nothing anywhere in the program plays audio.
- **Fix:** the play is real now, on the Windows MCI as the row's fix direction said:
  - **Toggle:** saying play while a play stands puts it down (the sound with it); otherwise it begins.
  - **The stretch is cut first:** the conversion program is asked for the selection (`-ss <in> -i <file> -t <span> -vn -c:a pcm_s16le`) into **a wav of the play's own** — the temp folder, named with the program's id and a per-play counter (`tangerine-trim-play-{pid}.{n}.wav`), so two windows' plays can never pour into one another's bucket. Every kind of sound a card can stand is heard this way, not only what MCI itself has a driver for. The cut is real work the window waits on (a moment — the honest precedent is the card's own full-PCM decode); a failed cut is said in the log and the play does not begin.
  - **The speaking:** `mciSendStringW` opens the wav `type waveaudio alias {one play's name}` and plays it `from 0 to {span_ms}`. The alias is per-play (program id + the counter again), because MCI hears every window's names the same; a wave device that cannot be heard does not stop the walk — the log says the play could not be heard, and the minutes row still runs, because the play of a card is the preview of it. The helpers live in windows 0.62's `Win32_Media_Multimedia`, whose `mciSendStringW` takes the answer buffer as an `Option<&mut [u16]>`.
  - **The walk:** the same bell the video frames walk by (`PLAY_TIMER_ID 55`, a tick of 250 ms, a step of 0.25 s). An audio card's `refresh_frame` early-outs, so the walk carries exactly one thing: the minutes row. At the stretch's out end the walk puts itself down — and takes the sound and its cut with it.
  - **The putting down:** `close_play_sound` (stop, close, and the cut file taken back) is said by `halt_play` — which every playhead-taker already called: the press-arm of a touch, the keyboard steps, run, cancel — and by the walk's own end branch of `WM_TIMER`. `TrimState` gains `mci_alias` and `play_sound` for exactly this.
- **Honest notes:** the cut moment blocks the UI thread, as the card's own decode already did; a program killed mid-play leaves its cut wav in the temp folder (the temp folder is where work files wait to be taken; nothing of the user's folders is touched).
- **Regression tests:** `saying_play_on_an_audio_card_begins_a_walk_of_the_selection` (default, `make_test_audio`'s wav is written by pure Rust and the vendored ffmpeg does the real cut): play says walking and stands the playhead on the stretch's in end; play again puts it down, the cut taken back, nothing of the play left in the folder. `a_play_of_the_selection_is_heard_and_walking` (ignored — needs the conversion program and **plays real audio**): the interface's own word is `playing`, the bell carries the playhead on within the stretch, and the put-down makes the interface stop saying `playing` and the cut gone.
- **Coverage evidence:** with the stub restored, the default test fails verbatim:
  `panicked at src\platform\windows\timelinewindow.rs:3032:13: the walk is walking`
  `test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 658 filtered out; finished in 0.01s`.
- **Verification:** part of the end-of-unit battery (suites, release, boot all green; the ignored play ran and was heard on this machine).
- **Status:** fixed, verified.

## BUG-018 - Space performed the trim instead of playing the preview

- **Feature:** the keyboard of the timeline family - `timelinewindow.rs` `WM_KEYDOWN` (Trim Video above all; Split / Snapshots / Trim Audio have their own Space semantics that came before the shared tail)
- **Severity:** P1 (user-reported from real use: a plain press of Space, the key every player teaches to mean "play", ran the operation and put the card away)
- **Reproduction:** open Trim Video on a clip → press Space → `run()` fires: a `ToolEvent::Run { tools.trim }` is said and the card hides. Expected: Space plays the preview; Enter says go.
- **Root cause:** the `WM_KEYDOWN` arm `key == VK_RETURN || key == VK_SPACE` fell through to a shared tail `else { run(window) }`. The audio / shots / split cards were special-cased above it, so the tail is reached exactly by the plain video trim - where Space performed the operation.
- **Fix:** the tail is split - `VK_RETURN` → `run(window)`; everything else (Space) → `toggle_play(window)`. A new `playing: bool` on `TrimState`; `PLAY_TIMER_ID (55)`, `PLAY_TICK_MS (250)`, `PLAY_STEP (0.25)` (a quarter of a second of video every quarter of a second; every step passes `refresh_frame`'s ≥ FRAME_STEP gate). On start, a playhead standing at or past the out point jumps back to the in point. A `WM_TIMER` step advances the playhead toward the card's out point and takes the frame there; at the out point the bell is let go of and the playhead stays. Everything that takes the playhead back halts the walk first: the press-arm of `timeline_touched` (before SetCapture), the keyboard steps of the caps, `run`, `cancel`. The audio / shots / split Space semantics above the arm are untouched.
- **Honest note:** the play is a slideshow of frames asked of the conversion program with a fast seek - there is **no sound**. Sound is BUG-017's own unit.
- **Regression tests:** `space_on_a_trim_card_says_nothing_and_enter_says_go` (default): Space says nothing to the handler and begins no walk (no video); Enter says `ToolEvent::Run`. `space_plays_the_preview_of_a_real_trim_card` (ignored, real video made with the vendored ffmpeg): Space begins the walk, the playhead advances across pumps, no event; a second Space stops the walk where it stood; Enter says the run.
- **Coverage evidence:** with the old tail restored, the default test fails: `panicked at src\platform\windows\timelinewindow.rs:3139:9: the space key said something to the handler` / `test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 651 filtered out; finished in 0.45s`.
- **Verification:** default suite `test result: ok. 574 passed; 0 failed; 78 ignored; 0 measured; 0 filtered out; finished in 5.73s` EXIT=0; ignored suite `test result: ok. 78 passed; 0 failed; 0 ignored; 0 measured; 574 filtered out; finished in 7.35s` EXIT=0 (the real-video plays ran inside it); release build zero warnings EXIT=0; boot `--dev-wheel --exit-after 2` ERRLVL=0.
- **Status:** fixed, verified.

## BUG-019 - Dragging a trim cap did not move the preview

- **Feature:** the drag of the timeline - `timelinewindow.rs::timeline_touched` moved arm (Trim card)
- **Severity:** P2 (user-reported: the point of the preview is to see the cut being made, and the pointer stood still while the cap moved)
- **Reproduction:** duration 10.0, start 2.0, end 8.0 → press on the in cap and slide it to 4.0: the start moved, but the playhead and the preview frame stayed where they stood; no frame was asked for either.
- **Root cause:** the drag's moved arm only refreshed the frame when `dragging == Drag::Playhead`. A Start / End drag moved the cap and wore the range, and the playhead - the preview's pointer - never followed the thing being changed.
- **Fix:** a dragged cap stands the playhead on itself (in cap → playhead = the clamped start; out cap → playhead = the clamped end), and the frame-refresh "scrubbing" condition became `dragging != Drag::Nothing`, so a cap drag asks for its frame the way a playhead drag does. For symmetry, the keyboard's steps of the caps (`step()` focused 0 / 1) stand the playhead the same way.
- **Regression test:** `dragging_a_cap_of_the_trim_stands_the_playhead_on_it` (default): press / slide the in cap 2.0 → 4.0 and the out cap 8.0 → 6.0; asserts `playhead == start` and `playhead == end` exactly (the cap's own moment tolerates the one-pixel quantization of a synthetic drag).
- **Coverage evidence:** with the follow reverted, the test fails: `panicked at src\platform\windows\timelinewindow.rs:3215:13: assertion 'left == right' failed: the playhead did not follow the in cap` / `test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 651 filtered out; finished in 0.43s`.
- **Verification:** as BUG-018 (suites, release, boot all green).
- **Status:** fixed, verified.

## Latent (not live) - recorded for the record

- `toolwindow.rs` pointer arm dispatches while the host is held (line 687) — safe today only because its callbacks touch other thread-locals; if a future handler re-borrows the host it dies exactly like BUG-003. Left untouched per the no-unrelated-edits rule.
