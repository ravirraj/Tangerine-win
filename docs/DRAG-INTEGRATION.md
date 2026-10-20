# Finding an Explorer drag on Windows

This is the hard part of the Milestone 1 brief, and it is the part that took
the most measuring. Everything below was tested on the machine the Milestone 1
work was done on (Windows 11, 1920×1080, 100 % scaling).

## The short version

An Explorer drag is found, the files that are being dragged are found, the
drop is taken over, and the drag is not cancelled.

The one thing that does **not** work the way the macOS product works is that
the payload of a Windows drag is not on the clipboard, so it cannot simply be
read. It has to be picked up by being the target of the drag.

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

There is a second trap here. The `DragDrop` format *does* turn up on the
clipboard, but it can be the **leftovers of an earlier drag**. A first
version of the application read it and reported the wrong file: the folder the
user had dragged a moment before. The fix is in `explorer.rs`: the clipboard
is only believed when its sequence number has changed since the press.

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

### 3. Being the target of the drag — this is the one

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

The file list arrives about 40 ms after the wheel opens, which is why the
wheel can open with no files and pick them up a moment later.

## How the drag is found, step by step

Everything is polled from the application's own message loop. There is **no
global mouse hook** for the drag, so there is no hook to install, no hook to
leak and no input for anyone else to swallow.

1. **The press.** The pointer is read with `GetCursorPos` and the button with
   `GetAsyncKeyState`. When the button goes down, the window under the
   pointer is found with `WindowFromPoint` and its root is asked for its class
   name. Only `CabinetWClass` and `ExploreWClass` — Explorer's file lists —
   are accepted. A press anywhere else is not a candidate.
2. **The movement.** The pointer has to travel at least 8 pixels from the
   press. A click does not move that far, so a click is never a drag.
3. **The modifier.** The modifier keys are read with `GetAsyncKeyState` and
   mapped to a wheel with `ModifierMap`. `Shift` opens the Convert wheel.
4. **The payload.** The wheel registers itself as a drop target, and the file
   list arrives with the `DragEnter` callback.
5. **The choice.** The drop arrives in the `Drop` callback, and the segment
   under the pointer is taken.

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
| UI Automation on Explorer's list | It would give the selection rather than the drag, and it is a large amount of COM code for a worse answer. Kept as the next thing to try if a source that does not publish a drop target has to be supported. |
| `OleGetClipboard` | Measured: it does not carry the drag. |
| The `DragDrop` clipboard format | Measured: it is either absent or the leftovers of an earlier drag. |
| Owning the whole drag | The macOS product registers a drop destination and takes the drop. The same is done here, but only while the wheel is on screen. |

## Limits that are still there

- **A source that is not Explorer is not recognised.** Only Explorer's file
  lists are looked at. Another program that drags files (a browser, an
  editor) will not open the wheel.
- **The files arrive a moment after the wheel.** The wheel can open with no
  files, and picks them up about 40 ms later.
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
