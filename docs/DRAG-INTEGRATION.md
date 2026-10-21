# Finding an Explorer drag on Windows

This is the hard part of the Milestone 1 brief, and it is the part that took
the most measuring. Everything below was tested on the machine the Milestone 1
work was done on (Windows 11, 1920×1080, 100 % scaling).

## The short version

An Explorer drag is found, the files that are being dragged are found, the
drop is taken over, and the drag is not cancelled.

The one thing that does **not** work the way the macOS product works is that
the payload of a Windows drag is not on the clipboard, so it cannot simply be
read. The files are read from the shell instead: the shell knows what is
selected in the Explorer window the drag started in, and that selection is
what is being dragged.

## What was tried, and what was found

### 1. The global clipboard — does not carry the drag

The first idea was the obvious one: read the clipboard. Explorer has always
published a file drag under a registered format called `DragDrop`, holding a
`DROPFILES` structure with the list of files.

The clipboard was watched while a real Explorer drag was running, with the
formats enumerated every frame:

```
--- before (seq 435) ---
  49161, 49492, 49407, 49817, 49818, 13, 1, 49368, 49171, 16, 7
--- during the drag (seq 435) ---
  49161, 49492, 49407, 49817, 49818, 13, 1, 49368, 49171, 16, 7
  DragDrop (49903) present: False
  CF_HDROP (15) present: False
--- after the drop (seq 435) ---
  49161, 49492, 49407, 49817, 49818, 13, 1, 49368, 49171, 16, 7
```

The clipboard did not change at all: the same sequence number, the same
formats, before, during and after. The drag was real — the ghost image of the
file was on the screen and the details pane showed the file — but the
clipboard was left alone.

That is deliberate on Windows 8 and later: a drag no longer replaces what the
user has copied.

There was a second trap here. The `DragDrop` format *does* turn up on the
clipboard, but it can be the **leftovers of an earlier drag**. A first
version of the application read it and reported the wrong file: the folder the
user had dragged a moment before.

The clipboard is no longer read at all. The list of files comes from the
shell, which is the only place the payload of a drag can be read from.

### 2. The OLE clipboard — also does not carry the drag

The next idea was `OleGetClipboard`, which is the way to read the data object
of an OLE drag. It works, and it returns the clipboard of the last copy
operation, not the drag:

```
[ole] the OLE clipboard holds: ["text/html", "HTML Format", "text/_moz_htmlcontext",
       "text/_moz_htmlinfo", "#13", "#1", "text/x-moz-url-priv"]
[ole] the OLE drag does not offer CF_HDROP
```

(Those are the formats of a web page copied out of a browser.) The OLE
clipboard is not where a Windows drag keeps its payload either.

### 3. Being the target of the drag — the drop target still does its job

A drag hands its payload to the drop targets it passes over, and to nobody
else. So the application registers its own window as a drop target
(`RegisterDragDrop`), and the payload arrives in the `DragEnter` and `Drop`
callbacks of `IDropTarget`.

That is what `src/platform/windows/droptarget.rs` does. The file list comes
out of the data object as `CF_HDROP`, with the shell's own `DragDrop` format
as a second chance.

It works, and the whole gesture runs:

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

The drop target is still needed for the **drop**: it is what takes the drop
over, so that a file dropped on a choice does not land in the folder
underneath. What it is *not* used for any more is the list of files. The list
comes from the shell, and the list the drop target reports is only used as a
check: when the two disagree, the log says so.

### 4. The shell's own view of the window — this is where the files come from

The drop target cannot be the only source, for two reasons.

- It only fires once the drag passes **over** the wheel, which is well after
  the wheel has opened. The wheel would show "No files" until then.
- It does not always fire at all. In one long series of runs, `DragEnter`
  never fired, on any run, although `RegisterDragDrop` always succeeded. The
  shell may simply not offer a drag to a third party drop target on Windows 11.

What the shell *does* publish is the selection of the window the drag started
in, and that selection is what is being dragged. The shell has had this
automation interface for twenty years: it is the same `Shell.Application`
object that a script uses.

The chain is:

```
Shell.Application
  -> Windows()            the open Explorer windows
     -> Item(n)           one of them
        -> HWND           its window handle
        -> Document       the shell's view of that window
           -> SelectedItems()
              -> Item(n)
                 -> Path   the full path of the file
```

This was measured with a real Explorer window, with a real mouse drag, before
anything was written in Rust:

```
BEFORE the drag:
  the window is showing : C:\Users\thorfinn\AppData\Local\Temp\opencode\dragtest
    path = 'C:\Users\thorfinn\AppData\Local\Temp\opencode\dragtest\file1.txt'
  the selection has     : 1 item(s)
DURING the drag (the button is down):
  the window is showing : C:\Users\thorfinn\AppData\Local\Temp\opencode\dragtest
    path = 'C:\Users\thorfinn\AppData\Local\Temp\opencode\dragtest\file1.txt'
  the selection has     : 1 item(s)
AFTER the drop:
  the window is showing : C:\Users\thorfinn\AppData\Local\Temp\opencode\dragtest
    path = 'C:\Users\thorfinn\AppData\Local\Temp\opencode\dragtest\file1.txt'
  the selection has     : 1 item(s)
```

The shell answers before, during and after the drag, and it answers with the
full path of every file.

In Rust this is `src/platform/windows/shellselection.rs`. It uses the late
bound `IDispatch` interface of the `windows` crate, on the apartment thread
the application already has (`OleInitialize`).

Three things about the shell's own answers had to be found by trying:

- **`Item` is a method, not a property.** Asking for it as a property fails
  with `DISP_E_MEMBERNOTFOUND`. Both ways of asking are tried.
- **The handle is not always a 32 bit number.** Every kind of whole number is
  understood.
- **The shell has to be asked while Explorer is idle.** While a drag is going,
  Explorer is in its own drag loop and the shell has to wait for it: the first
  read during a drag takes about 50 ms, the next one takes 2 ms. The window
  and its document are therefore found on the **press**, when Explorer is still
  idle, and only the selection is read when the drag has begun.

The result is that the wheel opens with the right number of files on it, from
the first moment it is on the screen.

## How the drag is found, step by step

Everything is polled from the application's own message loop. There is **no
global mouse hook** for the drag, so there is no hook to install, no hook to
leak and no input for anyone else to swallow.

1. **The press.** The pointer is read with `GetCursorPos` and the button with
   `GetAsyncKeyState`. When the button goes down, the window under the
   pointer is found with `WindowFromPoint` and its root is asked for its class
   name. Only `CabinetWClass` and `ExploreWClass` — Explorer's file lists —
   are accepted. A press anywhere else is not a candidate, and the log says
   which window it was on.
2. **The movement.** The pointer has to travel at least 8 pixels from the
   press. A click does not move that far, so a click is never a drag.
3. **The modifier.** The modifier keys are read with `GetAsyncKeyState` and
   mapped to a wheel with `ModifierMap`. `Shift` opens the Convert wheel.
4. **The payload.** The files are read from the shell, using the window the
   drag started in. The window is found on the press, when Explorer is idle,
   and the selection is read when the drag has begun.
5. **The choice.** The drop arrives in the `Drop` callback, and the segment
   under the point of the drop is taken.

### The release, and the place of the wheel

The release is the one moment where three things happen at once: the mouse
comes up, Windows hands over the drop, and the wheel goes away. They have to
come in the right order, or the wheel moves.

**The wheel does not move.** It is put in one place when it opens, and it stays
in that place until it is gone. The place comes from the gesture, which the
controller knows. When the gesture is over the controller forgets it, and the
wheel must not fall back on the pointer: the pointer is somewhere else by
then, and the wheel would jump there on its way out. This was a real bug: the
wheel shifted by the distance between its middle and the point of the drop,
and then faded away.

**The drop is handled before the end of the drag.** Windows says where the
mouse was let go of, and that is a better answer than the pointer, which the
drag itself may have moved about. So the drop is looked at first.

**The end of the drag waits for the drop.** Windows hands the drop over a
moment after the mouse comes up, and the application only notices the mouse
coming up when it looks, which is up to a frame later. If the wheel hid itself
right away, it would stop being a drop target before the drop arrived, and
the drop would be lost. So the end of the drag waits a few frames for it. The
drop is normally there on the very next frame.

**The drop is taken once.** It has a flag of its own, and that flag is taken
exactly once, so that a second callback cannot choose a second time. A drop
that arrives after the wheel has gone is ignored, and the log says so.

**The choice is made with the files of the drag.** The segment and the file
list are taken before the wheel is hidden, and the action runs on the files
that were being dragged.

What the log says when a file is released over a choice:

```
[drag] the drag was dropped at (301, 130)
[drag] the drag was dropped on the wheel
[drag] the drop landed at Some(Point { x: 301.0, y: 130.0 }), on segment Some(0)
[state] WheelVisible -> SelectionPending (event)
[action] placeholder: PNG (convert.png) in the convert wheel on 1 file(s) ...
[overlay] the overlay was hidden
```

And what it says when the file is released on the hub, where there is no
choice:

```
[drag] the drag was dropped at (271, 225)
[drag] the drag was dropped on the wheel
[drag] the drop landed at Some(Point { x: 271.0, y: 225.0 }), on segment None
```

Nothing is run, and the wheel stays up and takes clicks.

### Every drag is a session

A drag is a **session** with a number, and the number is in the log:

```
[drag] drag 1 started at Point { x: 207.0, y: 177.0 } with 0 file(s), foreground window Some("CabinetWClass")
[drag] drag 1 ended
```

A session is opened by a press on an Explorer window and closed by the button
coming up, and it can only be opened again after the button has been seen up in
between. That latch is what stops one gesture from being counted as several:
without it, a button that is reported as up for a single frame ends the drag
and starts a new one, and the wheel opens and closes again and again.

A session that has ended cannot bring the wheel back. A **new** session can:
if a drop missed every choice and the wheel is still up, the next drag takes
the wheel with it:

```
[state] WheelVisible -> CandidateDrag (event)
[state] CandidateDrag -> WheelVisible (event)
```

## Where the wheel is put

The wheel is placed **before** it is ever drawn, and it is never moved unless
the place really changes.

Two things had to be got right, and the first one was a real bug:

- **`UpdateLayeredWindow` must not be given a destination device context.**
  When it is, the point that goes with it is the place the window is moved
  to, and a point of zero means the top left corner of the screen. The wheel
  was therefore yanked from the pointer to the corner of the screen on every
  frame, and `place` moved it back on the next one. The wheel flickered
  between the two places at 60 frames a second, which is what the bug looked
  like: one wheel in the corner, one on the pointer, and no way to catch
  either. With no destination context the window keeps the place it was given.
- **The window is made off the screen**, at `(-32000, -32000)`, and it is
  placed before the first pixel is drawn. A window that has never been placed
  would be drawn in the corner.

The pointer is read with `GetCursorPos`, which is in **physical** screen
pixels, which is what `SetWindowPos` wants. The wheel is drawn at the scale of
the monitor it is on, and the program asks Windows to be per monitor aware so
that the wheel is not stretched on a screen that is set to anything but
100 %.

The wheel is kept on the monitor the pointer is on, and it is clamped so that
all of it fits. A pointer in the corner of the screen therefore puts the wheel
against the corner, and not half off the screen:

```
pointer (400, 300) -> the wheel is placed at (210, 110)
pointer (1200, 700) -> the wheel is placed at (1010, 510)
pointer (960, 540) -> the wheel is placed at (770, 350)
pointer (100, 100) -> the wheel is placed at (0, 0)
```

## The drag is not cancelled

This was checked rather than assumed.

- The overlay window is `WS_EX_NOACTIVATE`, so it never takes the focus and
  the foreground window does not change.
- The overlay is only shown once the drag has been recognised, so it never
  appears under the pointer in the middle of Explorer's own drag.
- While the drag is running, the wheel takes the drop over: the drop comes to
  the application instead of landing in the folder underneath, and the
  application runs the chosen action on the files that were being dragged.
- The registration is undone as soon as the wheel closes, so a drag that is
  still going is left to the program underneath.

The one case worth knowing: because the application takes the drop over, a
drop on the **hub** or on a **gap** between the segments is taken by the
application and does not move the files. That is the same trade the macOS
product makes, and it is the reason the wheel is only registered while it is
on screen.

## What was ruled out, and why

| Idea | Why not |
| --- | --- |
| A low level mouse hook | It sees movement, but not the payload, and it is a global hook that has to be trusted. The polling and the drop target do the job without one. |
| UI Automation on Explorer's list | It would give the selection rather than the drag, and it is a large amount of COM code for a worse answer. The shell's own automation gives the same answer with far less code. |
| `OleGetClipboard` | Measured: it does not carry the drag. |
| The `DragDrop` clipboard format | Measured: it is either absent or the leftovers of an earlier drag. |
| Owning the whole drag | The macOS product registers a drop destination and takes the drop. The same is done here, but only while the wheel is on screen. |

## Limits that are still there

- **A source that is not Explorer is not recognised.** Only Explorer's file
  lists are looked at. Another program that drags files (a browser, an
  editor) will not open the wheel.
- **The files are known when the wheel opens.** They are read from the shell
  before the wheel is put on the screen, so the number of files on the wheel
  is right from the start. The read takes about 50 ms while Explorer is in
  its drag loop, which is the one price of this way of reading them.
- **A drop on the hub or a gap is taken by the application**, and the files
  are not moved by Explorer.
- **The list of files is what the drag carries**, so it is the selection the
  user made. A drag of a folder carries the folder, and folders are left for
  the file actions wheel.
- **Only one drag at a time.** A second drag during an interaction is
  ignored, on purpose.

## The one hook there is

There is a single low level mouse hook, and it is only there for the scroll
wheel. The wheel overlay never takes the focus, so the scroll messages go to
the program underneath and never reach it; a low level hook is the only way to
see them.

Its scope is small and it is cleaned up:

- It is installed when the wheel opens and removed when the wheel closes.
- It only looks at `WM_MOUSEWHEEL`.
- It never stops a message: the next hook in the chain is always called.
- It is removed by `Drop` as well, so it goes away even if the application is
  ended the other way.

Everything else — the pointer, the modifier keys, the foreground window, the
key presses — is polled.
