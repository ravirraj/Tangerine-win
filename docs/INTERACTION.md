# The interaction

This is the Windows version of the gesture the macOS product uses. The
reference product was read in full before any of this was written; this
document says what was kept, what was changed, and why.

## The gesture

1. The user selects files in Explorer and starts dragging them.
2. The user holds a modifier down while dragging.
3. A wheel appears around the pointer.
4. Moving over a segment lights it up.
5. Letting go over a segment chooses it.
6. Escape cancels.
7. The application keeps running in the background when nothing is happening.

Releasing the modifier does **not** close the wheel: the drag is still going
and the user may still want to choose something. This matches the reference
product, where the modifier is a switch rather than a button.

## The three wheels

| Wheel | What it offers | How it is reached |
| --- | --- | --- |
| Convert | PNG, JPEG, WebP, PDF, MP4, GIF, TIFF, BMP | The modifier, or scrolling |
| Tools | Compress, Crop, Trim, Merge, Edit Metadata | Scrolling |
| File actions | Copy path, Duplicate, Rename, ZIP, Show in Explorer | Scrolling |

The Convert wheel has the eight segments the Milestone 1 brief asks for. The
other two have five each. These are **placeholders**: they show the design and
the interaction, and choosing one logs the choice. Nothing is converted,
compressed, renamed or zipped yet.

The three wheels are fixed, but what is on them is not. The wheel shows the
operations that may really be done with the files that were dragged, and the
registry decides which those are. A wheel with nothing to offer is empty, and
that is what a selection that nothing can be done with looks like.

The mode definitions live in `src/radial/actions.rs` and the operations in
`src/radial/registry.rs`, away from the drawing, so that a later milestone can
fill each wheel in from the real conversion matrix and the real tool list
without touching the renderer.

The **development wheel** (`Ctrl+Alt+T`) is a look at the design, and it shows
every operation there is, whether or not it can be run. It is marked on screen
and in the log, and nothing it reports is a real Explorer drag.

## The modifier mapping

The macOS product uses `Shift` for Convert, `Command` or `Option` for Tools and
`Control` for File actions. `Command` has no meaning on Windows, and the other
two are already spoken for inside Explorer's own drag:

| Key | Inside Explorer | Verdict |
| --- | --- | --- |
| `Shift` | Move the file to another folder | Taken by the wheel for Convert |
| `Ctrl` | Copy the file | Left to Explorer |
| `Alt` | Make a shortcut to the file | Left to Explorer |

So Milestone 1 claims **only `Shift`**, and the other two wheels are reached by
scrolling. The mapping is configuration, not a rule in the code
(`ModifierMap` in `src/radial/actions.rs`), and `--advanced-modifiers` turns on
the macOS mapping for comparison.

This is a deliberate trade: taking `Ctrl` and `Alt` would break Explorer's own
copy and shortcut gestures for every drag, which is a worse price than two
scrolls to reach another wheel.

## The states

```
Idle
  │  a drag was seen
  ▼
CandidateDrag ──── the modifier was let go ───▶ Idle
  │  the modifier was held
  ▼
WheelVisible ──── Escape ───▶ Cancelling ──▶ Idle
  │  a segment was chosen
  ▼
SelectionPending ──▶ Executing ──▶ Idle
```

- **Idle** — nothing is happening. The application is waiting.
- **CandidateDrag** — a drag was seen in Explorer and the wheel has not opened
  yet. The file list may still be empty: the shell publishes the payload a
  moment after the drag begins.
- **WheelVisible** — the wheel is on screen.
- **SelectionPending** — a choice was made and the action is about to run.
- **Executing** — the action is running.
- **Cancelling** — the wheel is on its way out.

The rules the machine keeps:

- A drag that is not in an Explorer window never opens the wheel.
- A press that does not move far enough is a click, not a drag, and never opens
  the wheel.
- `Shift` on its own, with no drag, never opens the wheel.
- A second drag never opens a second wheel.
- The file list stays the same for the whole interaction, apart from the one
  documented step where the shell's payload arrives.
- Every path back to `Idle` clears the file list, the highlight and the centre.

## How a segment is chosen

The segment under the pointer is chosen with one test, and the drawing and
the test use the **same numbers**: the same centre, the same radii, the same
number of segments and the same gap.

The centre is the place the wheel is actually drawn, which is not always the
place the pointer was when the wheel opened: a pointer near the edge of the
screen pushes the wheel back onto the screen, and the test follows the wheel
there, not the pointer.

The test has a little patience, so that the choice does not flicker:

- The gap between two segments is four degrees wide and belongs to the
  segment on its clockwise side. A pointer resting on the line keeps the
  choice it already had.
- The edge of the hub and the edge of the rim have a dead zone, as wide as
  the gap is at the inner edge. A pointer resting on one of those lines
  keeps its choice.
- The patience never keeps a segment the pointer has clearly left. A pointer
  that goes well into the hub, or well past the rim, gives the choice up.

The lit segment is drawn with a brighter fill and a two pixel orange
border, and it is lifted four pixels out of the wheel. The picture and the
name are drawn **over** the lit segment, so that they stay crisp.

## The keys

| Key | What it does |
| --- | --- |
| `Shift` (held) | Opens the wheel during a drag |
| Scroll | Moves between the three wheels |
| `Left`, `Right` | Moves between the segments |
| `Up`, `Down` | Moves between the three wheels |
| `Enter` | Chooses the segment that is lit |
| `Escape` | Cancels |
| `Ctrl+Alt+T` | Opens the development wheel |

While the keyboard is choosing, the pointer does not take the choice back
until it moves. Without that, the two would fight over the same segment.

## The development wheel

`Ctrl+Alt+T` (or `--dev-wheel`) opens the wheel without a drag. It is a
**test**, and it says so:

- The wheel carries a `DEVELOPMENT MODE` mark.
- The log says `the development wheel was opened. This is a test, no files are
  involved.`
- The controller keeps the two apart: `InteractionSource::Development` is
  never `InteractionSource::ExplorerDrag`, and `is_explorer_drag()` is false.
- The file list is always empty. No path is ever invented.
- The actions it reports are marked with the source, so a report can always say
  where a choice came from.

## The animation

One model drives everything (`src/ui/animation.rs`), so no two animations ever
fight over the same value.

| Motion | How long | What it does |
| --- | --- | --- |
| Arriving | 180 ms | The wheel grows out of the pointer and fades in |
| Leaving | 130 ms | The wheel fades away |
| A segment lighting up | 100 ms | The segment lifts 4 px and takes the accent colour |
| A segment letting go | 90 ms | The accent fades away |

The software renderer that draws the wheel does not support element
transforms, so the arrival is done by scaling the geometry itself: the radii,
the corner radius, the label sizes and the icon sizes all grow together with
the pointer, and the whole thing fades in on top.

## The look

- Near black: the segments are `#141419`, the hub `#131317`.
- A hairline between the segments: `#2c2c35`.
- The accent: orange `#ff8a3d`.
- Names: white when the segment is lit, `#c4c4cf` otherwise.
- The hub carries the name of the choice, a line of explanation, and three
  dots that show which of the three wheels is showing.

The wheel is drawn at the display's own scale, so it is the same size on the
screen whatever the scaling of the monitor is set to.
