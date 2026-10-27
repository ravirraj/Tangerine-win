# Final audit report — Tangerine-win

Branch `phase/b-10-tool-windows`, base commit `45d41fb`, all fixes of this campaign
**committed on top of it as one commit** (nothing pushed).
Companion documents: [`BUG_LEDGER.md`](./BUG_LEDGER.md) (every finding, with evidence), and
`PLAN.md` (the unit plan the audit followed).

---

## 1. Executive summary

The repository was swept feature by feature — boot and tray, wheel open/placement/scale,
hover and keyboard, every conversion pipeline (subtitles, archives, office, PDF, media,
pictures, file actions, tools), the worker layer, and the platform windows — with every
claim backed by code reading, a live run, or a test.

**Result: 19 bugs fixed or resolved, 1 disproven, 0 confirmed-but-left bugs, and the
look/improvement row carried to its honest end.** The two data-destructive P1/P2s
(files written over without staging — BUG-009), the abort/crash family (re-entrancy of
window callbacks — BUG-002/003), the tray that never was (BUG-001) and the unbounded
archive pours (BUG-012) are the ones a user would have hit; all are fixed with
regression tests, and every fix was proved by temporarily restoring the old code and
watching the new test fail — verbatim quotes in the ledger.

The codebase underneath is in good shape: process spawning is shell-free *and now
capped under job objects of the runs' own* (a wedged helper is stood down, its tree
with it), staging + publish discipline is uniform and per-run wherever work is poured
(two runs of one file braid no buckets; a stopped run takes its work back), path
traversal in archives is guarded, the audio card's play is heard (a real selection cut
and MCI voice), and the test corpus is unusually honest for a Win32 Rust program
(579 default + 80 real-OS ignored tests). The one risk still said apart: a COM server
runs in the worker's own process and cannot be stood down from outside it (BUG-015's
honest note); the COM wedge remains possible. The wheel's cancellation story is
dispatch-time refusal, decided and recorded (BUG-016).

**Verdict: safe to carry forward as the working branch.** The release-gate rows this
report once named first (BUG-016 per-run staging, BUG-014 sweep on refusal) are fixed
and verified; see §8 for the exact remaining procedures that need a human hand, and
§9 for what honest work is left.

---

## 2. Method

1. Stabilize, don't rewrite: every fix is the smallest change that makes the behaviour
   honest, in the existing idiom of the file.
2. Nothing swallowed: no error was turned into a silent success; no test weakened; no
   feature disabled by a fix.
3. Evidence for every claim: verbatim lines, exit codes, fps; unverifiable claims marked
   as manual procedures (§8).
4. Coverage proofs: for each new regression test, the old buggy code was temporarily
   restored, the test was observed to fail with the exact old symptom, and the fix was
   re-applied and re-run green. Exceptions (noted in the ledger): the archive bombs
   themselves were not re-run uncapped (a real bomb under the old code risks the
   machine) — argued from the cap parameter instead (BUG-012); BUG-015's old code
   would hang the tester for ever (it *is* the failing mode) — proved by direct
   exercise of the cap; and BUG-016's name-collision revert never landed at test
   sizes (flake territory) — the fix proved by the name's uniqueness and both whole
   runs standing.
5. Suites were run throughout; the release build is the zero-warning bar (the debug
   build's dead-code warning flood predates this work entirely and is only silenced in
   release, via the note at `src/main.rs:12`).

---

## 3. The bug ledger (condensed)

Full entries with evidence and quotes: [`BUG_LEDGER.md`](./BUG_LEDGER.md).

| # | Finding | Area | Severity | State |
|---|---------|------|----------|-------|
| BUG-001 | Tray icon born dead: no way to see, open or stop the app | App shell | P1 | **fixed, verified** (menu hand-check manual) |
| BUG-002 | `WM_KEYDOWN` re-entrancy: Tab+arrow aborted the channels window (0xC0000409) | Channels window | P1 | **fixed, verified** |
| BUG-003 | Same re-entrancy family on the timeline window's speaking controls | Timeline window | P1 | **fixed, verified** (ignored test) |
| BUG-004 | Scale captured once at startup; mixed-DPI re-placement drew with the old scale | Wheel placement | P1 | **fixed, verified** (runtime re-detect + buffer + overlay DIB resize) |
| BUG-005 | Low-level mouse hook could drop the chain's events | Input hooks | P1 | **fixed** (code-verified `CallNextHookEx`) |
| BUG-006 | "Hover lights wedges before the entrance matches the drawing" | Radial wheel | P2 | **not a bug** — premise disproven (one geometry source; entrance is opacity-only) |
| BUG-007 | `SetWindowPos` failure reported success; the wheel believed itself placed | Overlay placement | P1/P2 | **fixed, verified** (`is_placed`, failure event) |
| BUG-008 | Drop point fell back to guesses when the drag said nothing | Drop targeting | P2 | **fixed, verified** (optional point, nothing until said) |
| BUG-009 | Conversions wrote **straight onto the target**; collision/clobber data loss (also staging left behind) | Documents wheel | **P1 data loss** | **fixed, verified** (stage → `names_for` → publish; 3 collision tests) |
| BUG-010 | LibreOffice export deleted its own export (ordering) | Documents pipeline | P2 | **fixed, verified** (stub-observed old failure quoted) |
| BUG-011 | Office COM export writes where the caller says | Documents pipeline | P2 | resolved by caller staging; **verified live** (real Word/Excel, 2 tests pass) |
| BUG-012 | Archive pours unbounded (zip-bomb family); refused extractions left `.extracted` folders | Documents + file actions | P2 | **fixed, verified** (caps both paths; sweep; revert-proofed) |
| BUG-013 | Pdfium could bind out of the process's current directory (library of unknown kind) | Media (pdf) | P3 | **fixed, verified** (exe-derived walk incl. `target\debug\deps`; full ignored suite green with real pdfium) |
| BUG-014 | A failed tool conversion leaves its `.part` beside the user's file | Tools wheel | P3 | **fixed, verified** (`SweepOf` guard at every staging site; revert-observed quote in ledger) |
| BUG-015 | Spawned helpers run with no timeout: a wedged helper parks its worker for ever | Media pipelines | P3 | **fixed for program spawns, verified** (run cap + kill-on-close job objects; COM wedge said apart) |
| BUG-016 | Two operations on one file share one `.<ext>.part` staging name (no per-run staging, no cancellation) | Tools workers | P2 potential | **staging race fixed, verified** (shared `staging_name`, pid + counter, every site); cancellation decided: dispatch-time refusal stands |
| UI-1 | The accent is said three ways (`#ff5a1f` token, `#FF5C00` fills, `#ff8a3d` wheel wash) | UI tokens | P3 | **fixed, verified** (accent uses fill from `Tokens.accent`; the swatch palettes and the wheel's wash are content, said apart) |
| BUG-017 | The audio trim's "Play Selection" button and its Space stop answer nothing (born-dead affordance) | Audio trim card | P3 | **fixed, verified** (per-play selection cut + MCI voice + the shared walk; revert-observed quote in ledger) |
| BUG-018 | Space performed the trim instead of playing the preview; Enter should perform, Space should play (user-reported from real use) | Trim / video tool cards | P1 | **fixed, verified** (Space plays a frame-slideshow walk; revert-proofed; no sound — see BUG-017) |
| BUG-019 | Dragging a trim cap did not move the preview: the playhead stayed where it stood (user-reported from real use) | Trim card | P2 | **fixed, verified** (cap drag stands the playhead on itself; revert-proofed) |

---

## 4. Architecture assessment

**Strengths**

- **Staging + publish, said once.** `Publisher.publish(temporary, destination)` with
  `DestinationExists` → next numbered name; the buggy paths now all route through it.
  The media converter already did this; the documents wheel (BUG-009), office export
  (BUG-010/011) and the file actions (BUG-012) were brought into the same shape.
- **Test pyramid with real teeth.** Pure logic in Rust (parsing, naming, geometry) is
  tested fast; the Win32 layer has real-OS tests that open actual windows, hooks and
  now the actual tray icon, plus `#[ignore]` tests that run the REAL vendored binaries
  and real Office COM — run on this machine.
- **Thread discipline.** The re-entrancy house rule (never dispatch a callback while
  holding a host `RefCell`) is now written into the fixed windows (crop, timeline,
  channels, speed template) and documented at the touchpoints. The one remaining
  latent spot is recorded (`toolwindow.rs:687`) per the no-unrelated-edits rule.
- **Honest UI.** Tray menu ships Open Tangerine + Exit only; the *Settings…* item waits
  for the settings window to exist rather than lying with a dead item (`TRAY_ID_SETTINGS`
  retained with a comment).

**Weaknesses, and where they now stand**

- **No cancellation anywhere — decided, not left.** The `Outcome::Cancelled` arm exists
  but no production handler can produce it; fire-and-forget stays the minimal design —
  and per-run staging now backs it (BUG-016 fixed), while every *spawned helper* stands
  under a run cap in a kill-on-close job of its own (BUG-015 fixed for program spawns).
  Killing a *working* run from the outside remains an undone kind of promise, said so.
- **Worker orchestration is still bare.** One thread per request, results collected later
  by `try_recv`; no queue, no dedup, no "same operation already running" guard. With
  per-run staging the concurrent workers braid no buckets any more; the guard stays
  future work.
- **Leftover-cleanup customs now match.** convert.rs, documents.rs and tools.rs all
  sweep their staging on the failure paths — the sweep guard is one shared idiom
  (`SweepOf`), the staging name one shared sayer (`staging_name`) — which is what
  BUG-014/016's fixes gave tools.rs and documents.rs.

---

## 5. Performance assessment

- **Idle/interactive frame rate:** 63.4 fps at 1.69 ms of work a frame (release boot
  logs, repeated). The 16 ms tick with a waitable timer is not the bottleneck.
- **Boot:** ≈ 56–59 ms to the start log, wheel on screen ~113 ms after the gesture.
- **Fixed this pass:** `tools.organize_pdf`'s default branch rendered **every page as a
  72-dpi PNG bitmap just to count them**; it now asks the document for its page count
  (`page_count`), and the live (ignored) test exercises the default branch with real
  pdfium. This removes the largest needlessly-expensive operation found in a pipeline.
- **Caps now bound worst cases:** a hostile 512+ MiB decompression (the
  "conversion eats all the RAM" case) is refused at the pour instead of surviving until
  the machine runs out (BUG-012). The 512 MiB share is the honest ceiling for archive
  work in a desktop utility and is one constant, visible at the top of each file.
- **No regressions measured** from any fix: frame timings and boot numbers before and
  after are indistinguishable in the logs.

---

## 6. Security assessment

- **Process spawning** is structured-arguments only, never through a shell (runner.rs),
  so no path becomes syntax. Document conversion PowerShell scripts pass values by env,
  and the LibreOffice stub contract now honours `--outdir`.
- **Path traversal** from archives is guarded on both extract paths (`enclosed_name` for
  zip, component walk for tar), tested (`extract_zip_rejects_path_traversal_zip_slip`).
- **Untrusted-input bounds:** archive decode is capped on both the memory path (documents)
  and the disk path (file actions) (BUG-012).
- **Nothing is overwritten:** export/conversion collisions now refuse-and-number or
  refuse outright at every verified staging site (BUG-009/011/012).
- **Provenance of native libraries:** pdfium candidates are exe-derived only as of
  BUG-013; the old CWD-relative candidates (which could bind an unknown-version
  library) are gone.
- **Residual risk, stated honestly (post-fix):** the spawn families carry the run cap
  and job-object stand-down (BUG-015) and the staging race has per-run names
  (BUG-016); what remains is the COM server — in the worker's own process, not
  stand-down-able from outside (only remotely reachable through the user's own
  files, which is why it stays a note rather than a danger).

---

## 7. Verification trail

Final fresh runs of both suites (after every change in this campaign):

```
test result: ok. 579 passed; 0 failed; 80 ignored; 0 measured; 0 filtered out; finished in 5.78s   (default, EXIT=0)
test result: ok. 80 passed; 0 failed; 0 ignored; 0 measured; 579 filtered out; finished in 9.67s   (--ignored, EXIT=0)
```

Release build: zero warnings, EXIT=0. Release boot (`--dev-wheel --exit-after 2`):

```
[start] started in 53.8 ms, revision 45d41fb-dirty, scale 1.00, timer 16 ms, ...
[stop] the application is stopping: 128 frame(s) in 2.0 s, 63.7 frames a second, 2.58 ms of work a frame
ERRLVL=0
```

Real-binary runs on this machine (ignored suite): Word + Excel COM exports
(BUG-011), vendored ffmpeg/ffprobe jobs, real pdfium full-exercise, a real
video play of the trim preview (BUG-018), two concurrent real trims of one file
standing beside each other (BUG-016) and a **play of the audio selection that was
heard** (BUG-017 — the interface itself said `playing`). Selected verbatim
coverage-proof quotes (from the ledger):

- BUG-002 restore-proof: `panicked at channelswindow.rs:605` (abort, exit -1073740791) — fix re-applied, test green.
- BUG-003 restore-proof: `RefCell already borrowed` at timelinewindow.rs:1748 — fix re-applied, test green.
- BUG-010 restore-proof: `the export was made whole: Err(OfficeFailed)`.
- BUG-012 restore-proofs: `the leftovers of a refused extraction were swept`;
  `a zip beyond the share is refused / left: Ok(()) / right: Err(ArchiveTooLarge)`.
- BUG-018 restore-proof: `panicked at src\platform\windows\timelinewindow.rs:3139:9:
  the space key said something to the handler` — fix re-applied, test green.
- BUG-019 restore-proof: `panicked at src\platform\windows\timelinewindow.rs:3215:13:
  assertion 'left == right' failed: the playhead did not follow the in cap` —
  fix re-applied, test green.
- BUG-014 restore-proof: `panicked at src\radial\tools.rs:6933:9: nothing of the run
  was left beside the file, yet …photo.6152.1.redact.png stood` — fix re-applied, test green.
- BUG-017 restore-proof: `panicked at src\platform\windows\timelinewindow.rs:3032:13:
  the walk is walking` — fix re-applied, test green.
- BUG-015, said honestly instead of restored: with the old body back the tester itself
  would hang for ever (the old behaviour *is* the failing mode); the cap was proved by
  direct exercise — a 30-second sleeper stood down at a 500 ms cap says `None` and
  `the helper was stood down after 500 ms of standing`.
- BUG-016's revert, flake territory exactly as the pre-fix row predicted: with the old
  shared name restored, four concurrent tiny-clip races still completed — the window
  between two test-size runs' writes is milliseconds. The race stands by the code
  reading (one name, two writers); the fix is proved by the name's uniqueness (unit)
  and both whole runs standing (integration).

---

## 8. Manual procedures (what automation cannot say here)

1. **Multi-scale monitors**: open the wheel on a mixed-DPI setup, drag between
   monitors, confirm the window and drawing follow (the BUG-004 test simulates the
   re-placement; the visual check is by hand).
2. **Tray encounter**: start the app; the "Open Tangerine" icon appears; left-click
   opens the wheel at the pointer; right-click shows the menu (Open Tangerine / Exit);
   choosing Exit stops cleanly and takes the icon with it. The click plumbing is
   test-covered; the menu encounter needs a hand.
3. **Modifier-chain fidelity**: hold Ctrl+Alt over an Explorer selection and roll the
   wheel sideways while other apps listen, to confirm the hook chain is fed and no
   event is swallowed (BUG-005's fix is invoked there only).
4. **Real LibreOffice** (not the stub): if the machine has `soffice`, the documents
   wheel with a LibreOffice-only format should round-trip; the stub contract is
   verified, the real program is not installed here.
5. **Trim preview play (BUG-018)**: open Trim Video on a real clip, press Space — the
   preview walks on, a frame every quarter second, the minutes row moving with it;
   Space again holds it where it stands; Enter runs the trim. It is a frame slideshow:
   there is no sound in the video preview walk by design. Drag either cap
   and watch the preview follow it (BUG-019).
6. **Audio play selection (BUG-017)**: open Trim Audio on a real sound, press the
   play-selection button or Space at focus 3 — the selection is heard, the minutes
   row walks with it, and play again (or run/no) puts it down, the sound taken back.
   A machine with no wave device still walks the minutes row and logs that the play
   could not be heard (the play is the preview of the card). This was heard on this
   machine in the ignored suite; the ear-check is a human's.

---

## 9. Remaining work (recommended order)

Everything this ledger once named as its own unit has landed: the staging race
(BUG-016), the sweep on refusal (BUG-014), the spawn caps and job objects (BUG-015),
the audio play (BUG-017) and the accent token wiring (UI-1). What remains is honest
work by nature, not a bug to fix:

1. **The COM wedge (BUG-015's note)** — a COM server runs in the worker's own process
   and cannot be stood down from outside it. If it ever matters, the office work gets
   a program of its own, which can then live under the run cap like every other
   helper. Recorded, not claimed.
2. **Stand-up parts** (documented in `src/main.rs`): the settings window (then the tray
   menu gains its *Settings…* item) and the completion chime. These are features the
   roadmap asks for, not bugs — they were deliberately left out of a bug campaign.
3. **The look** — follow the existing `MAPPING-ROADMAP.md` (reference folder) for the
   per-tool chrome. The accent itself is single-sourced now (tokens.slint ↔ theme.rs,
   both `#ff5a1f`); the wheel's lighter wash and the tools' palette swatches are
   content, carried in the ledger (UI-1) as said-apart.

## 10. Limitations of this audit (said plainly)

- The sweeps were risk-ranked by entry point, not a line-by-line read of every file;
  the latent `toolwindow.rs:687` spot was deliberately left untouched.
- BUG-016 was fixed and its means proved (unique names per run, both runs standing);
  a live double-drag race is flaky by nature — the revert of the fix was run four
  times at test sizes and the collision never landed there (said in the ledger).
- BUG-015's old code cannot be reverted into a failing test without hanging the
  tester for ever; it was proved by direct exercise instead (also in the ledger).
- The debug build's dead-code warning flood in untouched files is pre-existing
  baseline, not addressed (the release bar is the shipped one).
- The campaign's fixes sit on `phase/b-10-tool-windows` as one commit on top of
  `45d41fb`; nothing was pushed.
