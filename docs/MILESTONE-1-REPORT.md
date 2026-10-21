# Milestone 1 report

Open Tangerine for Windows, Milestone 1: the radial wheel, the interaction
and the Windows drag integration.

Everything in this report was measured on the machine the work was done on
(Windows 11, 1920×1080, 100 % scaling, one monitor). Where something was not
measured, it says so.

---

## 1. What was built

A native Windows program, in Rust, with no browser in it. It waits in the
background, notices a file drag coming out of Explorer, and puts a radial
wheel around the pointer when the user holds `Shift` down while dragging.

The whole gesture works end to end:

```
[drag] the press started on an Explorer window at Point { x: 199.0, y: 171.0 }
[drag] Explorer drag started with 0 file(s), foreground window Some("CabinetWClass")
[state] Idle -> CandidateDrag (event)
[state] CandidateDrag -> WheelVisible (event)
[wheel] the wheel is opening in the convert mode with 8 segment(s) on 0 file(s)
[wheel] the wheel was on screen 36.5 ms after the gesture
[drag] the drag carries 1 file(s)
[wheel] the pointer is over segment Some(2)
[drag] the drag ended
[state] WheelVisible -> SelectionPending (event)
[action] placeholder: WebP (convert.webp) in the convert wheel on 1 file(s), source ExplorerDrag.
```

The actions themselves are placeholders. Milestone 1 does not convert,
compress, rename or zip anything, and says so in the log every time a choice
is made.

---

## 1a. The wheel was in the wrong place, and it flickered

This was reported after the first Milestone 1 build: the wheel appeared in the
top left corner of the screen whatever the pointer was doing, a second wheel
appeared near the pointer, and the wheel flickered so fast that it could not
be caught in a screenshot.

### The root cause

`UpdateLayeredWindow` was called with a **destination device context** and a
destination point of `(0, 0)`.

When a destination context is given, that point is not a hint: it is where the
window is **moved to**. A point of zero is the top left corner of the screen.
So every frame did this:

1. `place` moved the window to the pointer.
2. `present` moved it back to the corner of the screen.
3. The next frame's `place` moved it to the pointer again.

At 62 frames a second the window therefore oscillated between the pointer and
the corner of the screen. That is all three symptoms at once: the wheel in the
corner, the wheel on the pointer, and the flickering.

Two smaller faults were found and fixed at the same time:

- The overlay window was made at `(0, 0)` with a size of 16 by 16. A layered
  window that is given pixels by `UpdateLayeredWindow` is **resized to fit
  them and made visible**, so a window that was never placed would put a
  whole wheel in the corner. The window is now made off the screen at
  `(-32000, -32000)`, at the right size, and it is placed before the first
  pixel is drawn. `present` refuses to draw into a window that has not been
  placed.
- The centre of the wheel was taken from the pointer *before* the drag was
  announced, so when the modifier was already held when the drag began, the
  controller opened the wheel before it had been told where the pointer was.
  The pointer is now read after the drag is announced, so the wheel always
  opens around the pointer.

### The other things that were tightened

- **One program, one wheel.** A second copy of the program now says so and
  stops, rather than putting a second wheel on the screen.
- **The window is placed once.** It is only moved when the place really
  changes, and the monitor is not asked about on every frame.
- **Showing and hiding happens once.** The overlay knows whether it is on the
  screen, and the wheel is not hidden and shown again while it is fading.
- **The drop target is registered once.** Registering twice would fail and
  leave the window with no drop target at all.
- **Every drag is a session.** A session has a number, it is opened by a press
  on an Explorer window, and it is closed by the button coming up. A new
  session can only start after the button has been seen up, so a button that
  is reported as up for a single frame can no longer end one drag and start
  another. That was the cause of the repeated "the drag ended" messages.
- **A press that is not on an Explorer window is refused**, and the log says
  which window it was on.
- **The scale of the monitor is read properly** and the program asks Windows
  to be per monitor aware, so the wheel is not stretched on a screen that is
  set to anything but 100 %.

### The checks

| Check | Result |
| --- | --- |
| The pointer at four places on the screen | The wheel is placed exactly on the pointer at each of them |
| Shift held for five seconds | The overlay is shown once and hidden never; the drag ends once |
| The same drag three times over | Three sessions, three endings, the wheel opened three times, one window |
| The pointer moved around the wheel | Five different segments lit up, one after the other |
| Escape | `WheelVisible → Cancelling → Idle`, and the overlay is hidden |
| A second copy of the program | It says another copy is running and stops |

---

## 1b. The wheel lit up the wrong segment, and the choice flickered

The pointer was reported to move between the segments and back to nothing:
`Some(0) -> Some(1) -> None`. The wheel was interactive, but the segment
under the pointer could not be seen.

### The root causes

There were four of them, and they added up.

**1. The drawing and the hit test used different centres.** The wheel is
*drawn* at the centre the pointer was clamped to, so that it stays on the
screen. The hit test used the pointer's **raw** position. Near the edge of
the screen the two are not the same place, so the wheel lit up a segment that
was not under the pointer — and the lit segment moved as the pointer moved.

**2. The drawing and the hit test used different radii.** The drawing scales
the wheel by the scale of the display; the hit test used the unscaled radii.
On a screen set to anything but 100 % every hit test was wrong.

**3. Two things decided which segment was chosen.** The pointer decided, and
the drag that might be passing over the wheel also decided — with a different
point, in a different set of numbers, and with the drag's point treated as
though it were in the window's own coordinates when it is in screen
coordinates. The two took turns, which is the `Some(n) -> None -> Some(n)`
pattern in the log.

**4. The choice had no patience.** The gap between two segments belongs to
the one on its clockwise side, so a pointer resting on the line between two
of them made the choice jump from one to the other. The same was true of the
edge of the hub and the edge of the rim.

### What was changed

- **One placement for both.** `WheelPlacement` holds the centre and the
  shape of the wheel in physical pixels. The drawing, the pointer test and
  the click test all use it, so they cannot disagree.
- **One source for the pointer.** The pointer is the truth. The drag is only
  asked when it is over the wheel, and its point is used as the screen
  coordinate it is, with no conversion.
- **A little patience.** The choice the pointer already had is kept while
  the pointer is inside the gap between two segments, and inside a dead zone
  at the edge of the hub and at the edge of the rim. The dead zone is the
  gap measured at the inner edge, so it is the same size everywhere. The
  patience never keeps a segment the pointer has clearly left, and a pointer
  that goes well into the hub still gives the choice up.
- **The highlight is unmistakable**, and it no longer covers the picture or
  the name. The lit segment is drawn with a brighter fill and a two pixel
  orange border, and the picture and the name are drawn **over** it, so they
  stay crisp and readable.
- **The log says what changed**, and only when it changed: the name of the
  segment, its number, and that the wheel will light it up.

### A bug that was found on the way

The modifier keys were read with the *general* `VK_SHIFT`, `VK_CONTROL` and
`VK_MENU`. A key that is injected, or a keyboard that only reports the
general key, was missed — and the wheel then opened only sometimes, which
looked like a fault in the drag detection. The left and the right keys are
now asked for as well.

### The checks

| Check | Result |
| --- | --- |
| The pointer at the four sides of the wheel | The right segment is lit at each of them |
| The wheel pushed against the edge of the screen | The segment under the pointer is lit, not another one |
| The pointer creeping across the line between two segments | **One** change, not five |
| The pointer creeping across the edge of the hub | **One** change, not five |
| The pointer creeping across the edge of the rim | **One** change, not five |
| The pointer going from a wedge into the hub | The choice is given up |
| The pointer walking around the ring | Five segments lit in turn, in order |
| A drag in Explorer, the pointer on a wedge | The wedge is lit and the drop chooses it |
| Escape | `WheelVisible → Cancelling → Idle` |

The frame rate is unchanged: 62.7 frames a second, 3.59 ms of work a frame
with the wheel on the screen.

---

## 1c. The wheel did not know what was being dragged

This was reported after the first Milestone 1 build: the wheel opened on a
drag, but it said it had **no files** on it, and the choice that was made was
made on nothing.

### The root cause

There were two sources of the file list, and **neither of them was the drag**.

- The **clipboard** was read, and Explorer does not put a drag on the
  clipboard. The format list and the sequence number were measured before,
  during and after a real drag: they did not change at all. The one time the
  `DragDrop` format is on the clipboard, it is the leftovers of an earlier
  drag.
- The **OLE drop target** was registered, and `RegisterDragDrop` always
  succeeded, but `DragEnter` did not fire. Not once, in a long series of
  runs. The wheel therefore never learned anything from it either.

So the count that was shown was accidental: the "0 file(s)" at the start was
the honest answer, and the "1 file(s)" that turned up later was the drop
target having been told about the drag by the system rather than by the
program.

The one thing that does publish the payload of a drag is the shell itself: it
knows what is selected in the Explorer window the drag started in, and that
selection is what is being dragged.

### What was changed

- `src/platform/windows/shellselection.rs` is new. It asks the shell, through
  the automation interface the shell has always had, what is selected in an
  Explorer window, and gives back the **full path** of every file.
- `src/session.rs` is new. It owns the file list of one drag, the window the
  drag started in, and where the list came from. It refuses an answer that
  belongs to another drag, and a drag that could not be resolved has no files
  at all rather than the files of an earlier drag.
- `src/platform/windows/explorer.rs` no longer reads the clipboard. It finds
  the Explorer window the press was on and hands it over.
- `src/app.rs` reads the files once per drag, at the start of the drag, and
  the list is the only one the wheel uses from then on. The list the drop
  target reports is a check and nothing more: when the two disagree, the log
  says so.
- `src/radial/controller.rs` takes the shell's answer as the list, even when
  it is empty, so that a drag with no files cannot keep the files of the drag
  before it.

The window and its document are found on the **press**, while Explorer is
still idle, and only the selection is read when the drag has begun. While a
drag is going, Explorer is in its own drag loop and the shell has to wait for
it: the first read during a drag takes about 50 ms, the next one takes 2 ms.

### The checks

`tests/windows-integration.ps1` drives a real Explorer window with a real
mouse and reads what the program logged. Nothing is mocked.

| Case | Result |
| --- | --- |
| One file | 1 file, and the shell names the file that was dragged |
| Three files | 3 files, and the choice is made on 3 files |
| Two drags in a row, with different files | The first carries 2 files, the second 1, and the second does not keep the files of the first |
| A drag that is cancelled with Escape | The files are read, and nothing is run |
| Dragging a file that was not selected before | Only the file that was dragged is carried |
| A drag that does not come from Explorer | The press is rejected, and no drag is started |

---

## 1d. The wheel jumped to one side as it went away

This was reported after the file list was fixed: when a file is released over
a choice, the wheel shifts slightly to one side immediately before it
disappears.

### The root cause

The wheel is put in one place when it opens, and the place comes from the
gesture. When the choice is made, the controller finishes the interaction and
**forgets the place**: `reset()` clears the centre.

The wheel is still fading out at that point, and the fading is drawn every
frame. Every frame the wheel was put in place again, and the place came from
the controller — which no longer knew it — so the **pointer** was used
instead. The pointer was at the point of the drop, and the wheel was in the
middle. So the wheel was moved there, and then it faded away.

The log said so, plainly:

```
[overlay] the wheel is placed at (49, 11), 380 by 380 pixels, centre Point { x: 239.0, y: 201.0 }
[action] placeholder: JPEG (convert.jpeg) in the convert wheel on 1 file(s) ...
[overlay] the wheel is being placed at Point { x: 309.0, y: 143.0 } (the controller says None, ...)
[overlay] the window was moved to (119, 0), 380 by 380 pixels, and is now at (119, 0, 499, 380)
[overlay] the overlay was hidden
```

The wheel was in the middle at 239, 201 and was moved to 309, 143: 70 pixels
to the right and 58 pixels up, which is where the file was let go of.

There was a second fault behind it. The `Drop` callback clears the "a drag is
over the wheel" flag, and the application looked for the drop behind that
flag, so **the drop was never seen at all**. The choice was always made from
the segment that was lit, and the drop was lost.

And a third: the application only notices the mouse coming up when it looks,
which is up to a frame later, and by then it had already stopped being a drop
target. Explorer's call to `Drop` then found nothing to talk to, and the drop
was lost for good.

### What was changed

- `src/app.rs` — the wheel is put in one place and stays there. The place
  comes from the gesture; when the gesture is over, the wheel keeps the place
  it has, and the pointer is never used to move it.
- `src/app.rs` — the drop is handled before the end of the drag, and the
  segment is found from the point of the drop, in screen pixels, with the
  wheel's own geometry. The drop is taken once and only once.
- `src/app.rs` — when the mouse comes up over the wheel, the end of the drag
  waits a few frames for the drop, so that the wheel is still a drop target
  when Windows hands it over. The drop is normally there on the next frame.
- `src/app.rs` — a drop that lands on the hub lights nothing, so the end of
  the drag cannot choose the segment that was lit before it.
- `src/radial/controller.rs` — the choice and the file list are taken before
  the wheel is hidden, and a drop on the hub runs nothing.

### The checks

`tests/windows-integration.ps1` drives a real Explorer window with a real
mouse. Two cases were added for this:

| Case | Result |
| --- | --- |
| A drag released over a choice | The wheel is moved **once**, when it opens, and never again; the drop chooses the segment under the point of the drop, and the action runs on the files of the drag |
| A drag released on the hub | Nothing is run, and the wheel stays up |

The whole suite was run three times: ten checks, all of them passed, and the
fallback — taking the segment that is lit when the drop does not come — was
never needed.

---

## 2. Files created or modified

Nothing existed before this milestone except `reference/Open-Tangerine-main.zip`.
Everything below is new.

| File | Lines | What it is |
| --- | --- | --- |
| `Cargo.toml` | 28 | The manifest and the three dependencies |
| `build.rs` | 6 | Compiles the Slint markup |
| `src/main.rs` | 30 | The entry point and the command line |
| `src/app.rs` | 781 | The loop, the wiring, the diagnostics |
| `src/diagnostics.rs` | 162 | The timestamped log |
| `src/radial/mod.rs` | 6 | |
| `src/radial/actions.rs` | 321 | The three wheels and the modifier mapping |
| `src/radial/controller.rs` | 614 | The interaction state machine |
| `src/radial/geometry.rs` | 443 | The shape of the wheel and the hit testing |
| `src/radial/icons.rs` | 88 | The little pictures |
| `src/radial/state.rs` | 100 | The states, the events and the effects |
| `src/platform/mod.rs` | 5 | |
| `src/platform/windows/mod.rs` | 6 | |
| `src/platform/windows/droptarget.rs` | 213 | Being the target of a drag |
| `src/platform/windows/explorer.rs` | 354 | Recognising an Explorer drag |
| `src/platform/windows/input.rs` | 191 | The pointer, the keys, the scroll wheel |
| `src/platform/windows/monitors.rs` | 88 | Where the screens are |
| `src/platform/windows/overlay.rs` | 407 | The transparent window the wheel is drawn in |
| `src/platform/windows/shellselection.rs` | 508 | Asking the shell what is being dragged |
| `src/session.rs` | 263 | The file list of one drag |
| `src/ui/mod.rs` | 3 | |
| `src/ui/animation.rs` | 204 | The one animation model |
| `src/ui/wheel.rs` | 296 | The view model |
| `src/ui/wheel.slint` | 193 | The markup |
| `tests/windows-integration.ps1` | 552 | The Windows tests with a real Explorer |
| `README.md` | 75 | |
| `docs/INTERACTION.md` | 119 | |
| `docs/DRAG-INTEGRATION.md` | 200 | |
| `docs/MILESTONE-1-REPORT.md` | | This file |

6 300 lines of Rust, of which about 1 500 are tests.

---

## 3. Architecture and dependency decisions

### The framework: Slint, but not the way Slint usually draws

The brief asks for Slint, and Slint 1.18.1 was used. The interesting part is
**how** it draws.

Slint's own Windows backend does not give a per-pixel transparent window.
The source says so plainly: `wants_transparent` is only computed for macOS,
and on Windows the surface is configured with `CompositeAlphaMode::Opaque`.
There is no supported way to get a see-through window out of it.

So the window is a plain Win32 layered window
(`WS_EX_LAYERED`, `UpdateLayeredWindow`), and **Slint's software renderer
draws into a buffer of ours** (`MinimalSoftwareWindow` plus a custom
`TargetPixel` that writes 32-bit premultiplied BGRA, which is the order
`UpdateLayeredWindow` wants). The result is a real, per-pixel transparent
overlay that floats over everything, with no browser and no GPU needed.

The cost of that choice is that Slint's element transforms are not available:
the software renderer's `scale` and `rotate` are no-ops. The arrival animation
therefore scales the **geometry** — the radii, the corner radius, the label
sizes and the icon sizes all grow together — rather than scaling an element.
It is also one animation model instead of several, so nothing fights over the
same value.

The rest of Slint is used as intended: the markup, the text layout, the
system fonts, the properties and the bindings.

### The dependencies

| Crate | Why |
| --- | --- |
| `slint` 1.18 | The wheel's markup, its text and its layout |
| `slint-build` 1.18 | Compiles the markup at build time (build dependency) |
| `windows` 0.62 | The overlay window, the drop target, the clipboard, the pointer, the keys |
| `windows-core` 0.62 | The `#[implement]` macro for the COM drop target needs it in scope |

Four crates, and `windows-core` is the same crate `windows` already uses, so
nothing extra is compiled.

### The shape of the program

```
main.rs ─▶ app.rs ─┬─▶ radial/controller.rs   (the rules; no Windows, no drawing)
                    ├─▶ platform/windows/      (the world; no wheel, no rules)
                    └─▶ ui/                    (the drawing; decides nothing)
```

The controller is handed typed events and returns typed effects. It never
knows what a window is; the platform layer never knows what a segment is; the
drawing never decides anything. That is what makes the interaction testable
without a mouse and a screen.

---

## 4. The commands that were run

All of them were run on the machine described above.

| Command | Result |
| --- | --- |
| `cargo fmt --check` | **Passed**, no differences |
| `cargo check` | **Passed**, no warnings |
| `cargo clippy` | **Passed**, no warnings |
| `cargo test` | **Passed**, 65 tests, 0 failed |
| `cargo build --release` | **Passed**, 7.4 MB, 16 s from cold |
| `cargo tree --depth 1` | 3 dependencies, 1 build dependency |

Nothing was skipped to make them pass. Two build errors and a set of API
mismatches with `windows` 0.62 came up along the way and were fixed rather
than worked around.

---

## 5. The tests

65 tests, all passing, all automatic.

### The state machine (22 tests)

| What is checked | Where |
| --- | --- |
| Idle → CandidateDrag | `controller.rs` |
| CandidateDrag → WheelVisible | `controller.rs` |
| The wheel stays up when the modifier is let go | `controller.rs` |
| The mode changes when the modifier changes | `controller.rs` |
| A valid choice runs the action | `controller.rs` |
| An invalid choice runs nothing | `controller.rs` |
| Escape cancels and returns to Idle | `controller.rs` |
| Five interactions in a row | `controller.rs` |
| A second drag does not open a second wheel | `controller.rs` |
| A failure cleans up | `controller.rs` |
| The file set does not change under the user | `controller.rs` |
| The development wheel is never an Explorer drag | `controller.rs` |
| The wheel's placement on the screen | `geometry.rs` |
| The animation never overshoots | `animation.rs` |

### The geometry (17 tests)

| What is checked | Where |
| --- | --- |
| The four cardinal directions | `geometry.rs` |
| The eight diagonal directions | `geometry.rs` |
| A boundary belongs to the clockwise side | `geometry.rs` |
| The hub is never a segment | `geometry.rs` |
| The rim is never a segment | `geometry.rs` |
| A wheel with no segments never hits | `geometry.rs` |
| Coordinates that are not numbers do not panic | `geometry.rs` |
| The gap between two segments | `geometry.rs` |
| The labels sit between the hub and the rim | `geometry.rs` |
| The outline is a closed shape and every segment has its own | `geometry.rs` |
| The outline stays between the hub and the rim | `geometry.rs` |
| The wheel is kept on the screen | `geometry.rs` |
| A screen smaller than the wheel does not break the maths | `geometry.rs` |

### The rest

The platform layer has tests that do not touch the screen: the drag detector
ignores movement while the button is up, the monitor lookup works, the
modifier reading is consistent.

The file list of a drag has its own tests, in `session.rs` and in
`controller.rs`:

| What is checked | Where |
| --- | --- |
| A new drag drops the files of the drag before it | `session.rs` |
| An answer about another drag is refused | `session.rs` |
| A drag that could not be resolved has no files at all | `session.rs` |
| An empty selection is resolved, and empty | `session.rs` |
| The files of one drag are never mixed with another's | `session.rs` |
| The wheel takes the shell's answer, even when it is empty | `controller.rs` |
| The files of a drag stay with the wheel while it is up | `controller.rs` |

### The Windows tests

`tests/windows-integration.ps1` is the other half of the checking. It drives
a real Explorer window with a real mouse and reads what the program logged.
The six cases are listed in section 1c; all six pass.

---

## 6. The manual tests

These were run by driving the program with synthetic input on the same
machine, and by looking at the screen.

| # | Test | Result |
| --- | --- | --- |
| 1 | The program starts and stays running | **Passed** |
| 2 | The development wheel opens and is marked `DEVELOPMENT MODE` | **Passed** |
| 3 | The wheel has eight segments, a hub and three mode dots | **Passed** |
| 4 | The wheel is see-through; the program behind it shows through | **Passed** |
| 5 | The wheel arrives with a fade and a growth | **Passed** |
| 6 | The pointer lights a segment and lifts it | **Passed** |
| 7 | Every segment can be pointed at | **Passed** |
| 8 | Scrolling moves Convert → Tools → File actions and back | **Passed** |
| 9 | The tools and file actions wheels have their own segments | **Passed** |
| 10 | The hub shows the name and the explanation of the choice | **Passed** |
| 11 | The arrow keys move the choice, and the pointer does not steal it back | **Passed** |
| 12 | The up and down arrows move between the wheels | **Passed** |
| 13 | Escape closes the wheel and the program goes back to waiting | **Passed** |
| 14 | The wheel can be opened again straight away | **Passed** |
| 15 | An Explorer file drag is recognised | **Passed** |
| 16 | `Shift` during the drag opens the wheel | **Passed** |
| 17 | The drag is not cancelled by the wheel appearing | **Passed** |
| 18 | The files being dragged are found, and they are the right ones | **Passed** |
| 19 | The drop chooses the segment under the pointer | **Passed** |
| 20 | The wheel does not open for ordinary mouse movement | **Passed** |
| 21 | The wheel does not open while typing | **Passed** |
| 22 | The program cleans up and exits | **Passed** |

**Not tested, because the machine has one monitor and one scale setting:**

- A second monitor. The code asks Windows for the monitor a point is on and
  clamps the wheel to that monitor's bounds, and the work was written with
  that in mind, but it has not been run on two screens.
- A display scale other than 100 %. The wheel is drawn at the display's own
  scale, and the scaling code reads the DPI of the window, but it has not been
  run at 125 % or 150 %.
- A drag that is dropped on the hub or in a gap. The behaviour is defined
  (the wheel keeps the drop and the files are not moved) but it has not been
  watched by hand.

---

## 7. What was measured

### Responsiveness

| What | Measured |
| --- | --- |
| Time to the first frame after the program starts | **4.7 ms** to 11.4 ms |
| From the gesture to the wheel on the screen | **36.5 ms** (an Explorer drag), **47.0 ms** (the development wheel, whose first frame also loads the fonts) |
| Frames a second with the wheel on the screen | **62.7** |
| Frames a second while waiting | **62.6** |
| Work a frame with the wheel on the screen | **4.48 ms** on average |
| Work a frame while waiting | **0.03 ms** on average |

The target was under 50 ms between a verified gesture and the wheel being
visible. The measured figure is **36.5 ms**.

The 62.7 frames a second is the real number, not an estimate. It is worth
saying how it was got: the first attempt used `WM_TIMER`, and the wheel only
managed 40 frames a second, because Windows throttles the timer of a window
that never comes to the front. The program now waits on a **waitable timer**
instead, and the frame rate is 62.7.

### Memory and the background

| What | Measured |
| --- | --- |
| Working set, wheel on the screen | **14.9 MB** |
| Private memory | **4.6 MB** |
| Handles | **135** |
| Threads | **4** |
| Work a frame while waiting | **0.03 ms** |

The process accounting on this machine reports 0 ms of processor time for the
program even while it is running, so the idle load is given as the work a
frame does rather than as a share of a core: at 62.6 frames a second that is
0.03 ms × 62.6, about **0.2 %** of one core.

### The frame rate against the animation

The arrival is 180 ms long, so at 62.7 frames a second it is drawn in
**11 frames**. The dismissal is 130 ms, so **8 frames**. Neither is a number
that has been counted by eye; both follow from the measured frame rate.

---

## 8. What works, and how it was checked

- **The wheel is a real, see-through, always-on-top window** with no browser
  and no web view. Checked by looking at it over another program.
- **The state machine is explicit** and every transition is a typed event and
  a typed effect. 22 tests cover it.
- **The hit testing is exact**: the cardinal and the diagonal directions, the
  hub, the rim and the boundaries between two segments are all tested.
- **The keyboard works**: the arrows move the choice, Enter takes it, Escape
  cancels, and the pointer does not take the choice back until it moves.
- **The scroll wheel moves between the three wheels**, in both directions.
- **The three wheels are separate from the drawing**, so a later milestone can
  fill them in without touching the renderer.
- **The development wheel is clearly a test**: it is marked on screen, it is
  marked in the log, and the controller can never report it as an Explorer
  drag.
- **An Explorer drag is found, the files in it are found, the drop is taken
  over, and the drag is not cancelled.** Checked end to end, with a real
  Explorer window and a real mouse.
- **The program is a good citizen**: no global keyboard or mouse hook except
  one narrow one for the scroll wheel, no input is swallowed, Explorer is
  never touched outside a drag, and the hook and the window are cleaned up
  when the program ends.

---

## 9. What is not done, and what is risky

### Not done

- **The actions are placeholders.** Nothing is converted, compressed, renamed
  or zipped. That was the brief for this milestone.
- **The wheel only opens for Explorer.** A drag from a browser or an editor
  is not recognised. The check is for the `CabinetWClass` window, which is
  Explorer's file list.
- **There is no tray icon.** The program runs in the background but has no
  icon to click, and no way to be quit by hand. `Ctrl+Alt+Q` and the
  `--exit-after` option are what exist for now.
- **There is no settings, no output folder and no login item.**
- **There is no "the files are being converted" progress.** The macOS product
  shows the work in a notch; there is nothing of that here yet.
- **There is no help file, no right-click entry in Explorer and no shell
  extension.**

### Risks

1. **The payload of a drag is only visible to a drop target.** This is the
   biggest one. The application takes the drop over, so a drop on the hub or
   in a gap between the segments is taken by the application and the files do
   not move. The macOS product has the same trade, but it is worth knowing
   about before this is given to anyone else. The smallest thing that would
   take the risk away: when the drop lands on the hub or in a gap, hand the
   drop back to the program underneath instead of taking it.

2. **The file list is read while Explorer is in its drag loop.** The shell
   cannot be asked quickly at that moment: the first read takes about 50 ms
   and the wheel waits for it, so the wheel opens about 50 ms later than it
   would otherwise. The number of files on the wheel is right from the start,
   which is the trade that was made.

3. **Only one drag at a time.** A second drag during an interaction is
   ignored. That is deliberate, but it is a limitation.

4. **The scroll wheel needs a low level hook.** The overlay never takes the
   focus, so the scroll messages never reach it. The hook is installed only
   while the wheel is on screen, only looks at the scroll, and never stops a
   message — but it is still a global hook, and it is the one thing in the
   program that a security tool might ask about.

5. **The software renderer is slower than the GPU one.** At 4.48 ms a frame
   for a 380 pixel wheel it is comfortable, but a much larger wheel, or a much
   slower machine, would feel it. Moving to the GPU renderer would mean
   giving up the per-pixel transparency, or finding a different way to get it.

6. **The wheel is not yet on a second monitor or at another scale.** The code
   is written for it and the maths is tested, but it has not been seen.

7. **The synthetic input used for the tests is not a human hand.** The drag
   was driven with `mouse_event` and `SetCursorPos`. The behaviour was
   confirmed on a real Explorer window with a real ghost image, but nobody has
   yet held a mouse and dragged a file while holding `Shift`.

8. **A file that has no path of its own is left out.** A virtual item — a
   search result, a library file, a OneDrive placeholder that has not been
   fetched — can have a name but no path. Such an item is not in the list,
   and the list is shorter than the number of items on the screen. It is
   never guessed at.

---

## 10. The next milestone

**Milestone 2: the first action that really works, and the shell integration.**

The order that is suggested:

1. **Make one action real** — `Copy path` is the smallest one that touches
   nothing on disk, and it proves the whole chain: the files are found, the
   choice reaches the action, the action does something. Then `Reveal in
   Explorer`, which opens the folder of the file that was dragged.
2. **Put the wheel in Explorer's right-click menu**, through the shell's
   "Services" mechanism, so the wheel can be opened on a selection without a
   drag. This is the Windows answer to the macOS product's Finder Services
   entry, and it is the way to reach the wheel without a gesture at all.
3. **Fix the drop on the hub**, so that a drop that is not on a choice goes
   back to the program underneath instead of being taken.
4. **Add a tray icon**, so the program can be found, quit and configured.
5. **Then the conversion engine**, with the first real format route (image to
   image) behind it, and the progress shown where the macOS product shows it.

The one thing that should not be started before the second and third points
are done is the conversion engine itself: without the shell integration and
without the drop being safe, a wrong choice could still move or lose files,
and that is the one failure the macOS product has had to be careful about
too.

---

## 11. A note on the reference

The macOS product in `reference/Open-Tangerine-main.zip` was read before any
of this was written. `NativeApp/Sources/Radial/DragMonitor.swift`,
`RadialController.swift`, `RadialView.swift`, `RadialMode.swift` and
`FinderSelection.swift` were read in full, together with the parts of the
README that describe the drag, the wheels, the keyboard, the cancellation and
what happens to the output.

None of it was ported directly. The macOS product reads the drag from the
pasteboard, watches the global mouse events and puts the wheel in a
non-activating panel that takes the drop. The Windows product cannot do the
first of those at all, and the last one is a very different API. What was
kept is the **shape** of the interaction: the wheel around the pointer, the
modifier that opens it, the segment that lifts, the scroll that moves between
the three wheels, the keyboard, the escape, and the rule that a drop that is
not on a choice leaves the wheel up.
