# Open Tangerine for Windows

A radial wheel for the files you are dragging. Hold a modifier while you drag
files out of Explorer and a wheel appears around the pointer: move onto a choice,
let go, and the action runs on the files you were dragging.

This is **Milestone 1**: the interaction, the visual design and the Windows
drag integration are real. The picture operations work too: a picture that
is dragged onto one of them is converted, by the FFmpeg program, and the
original is left exactly as it was. The other actions are still
placeholders — nothing is compressed or renamed yet.

## Build and run

```sh
cargo build --release
cargo test
.\target\release\open-tangerine.exe
```

The program needs no installation and no runtime beyond the standard Windows
libraries. It is a single executable of about 7 MB.

### Options

| Option | What it does |
| --- | --- |
| `--dev-wheel` | Opens the wheel straight away, for a look at the design. |
| `--advanced-modifiers` | Also lets `Ctrl` and `Alt` open their own wheels. |
| `--exit-after <seconds>` | Quits after so many seconds. Used by the checks. |
| `--help` | Prints the same list. |

The log level comes from the environment:

```sh
set OPEN_TANGERINE_LOG=debug
```

`debug`, `info`, `warn` or `error`. Without it the log stays at `info`, which
is quiet enough to leave running in the background.

## Using it

1. Select one or more files in Explorer and start dragging them. A file on
   the desktop can be dragged too.
2. Hold **Shift** while you drag. The wheel opens around the pointer.
3. Move onto a choice. The segment under the pointer lifts and lights up.
4. Let go of the mouse over the choice. The action is chosen.
5. **Escape** cancels. **Scroll** moves between the Convert, Tools and File
   actions wheels. **Left** and **Right** move between the segments, and
   **Enter** picks the one that is lit.

`Ctrl+Alt+T` opens the wheel without a drag. That is the development
shortcut: the wheel is marked `DEVELOPMENT MODE`, no files are involved, and
nothing it reports is a real Explorer drag.

## How it is put together

```
src/
  main.rs                  the entry point
  app.rs                   the loop, the wiring, the diagnostics
  diagnostics.rs           the timestamped log
  session.rs               the file list of one drag
  radial/
    actions.rs             what the three wheels offer
    controller.rs          the interaction state machine
    geometry.rs            the wheel's shape and the hit testing
    icons.rs               the little pictures
    registry.rs            the operations, and when they may be used
    selection.rs           what the dragged files are
    state.rs               the states and the events
  platform/windows/
    droptarget.rs          being the target of a drag
    explorer.rs            recognising an Explorer or desktop drag
    input.rs               the pointer, the keys, the scroll wheel
    monitors.rs            where the screens are
    overlay.rs             the transparent window the wheel is drawn in
    shellselection.rs      asking the shell what is being dragged
  ui/
    animation.rs           the one animation model
    wheel.rs               the view model
    wheel.slint            the markup
tests/
  windows-integration.ps1  the tests with a real Explorer window and the
                           real desktop
```

The rule the layout follows: the controller never knows about Windows, the
platform layer never knows about the wheel, and the drawing never decides
anything. Everything the user does arrives as a typed event, and everything the
application wants to do comes back as a typed effect.

## The tests

```sh
cargo test
```

There is also a set of tests that needs a desktop: they open a real Explorer
window, drag a real file with a real mouse and read what the program logged.

```sh
cargo build --release
powershell -ExecutionPolicy Bypass -File tests\windows-integration.ps1
```

They move the mouse and press keys, so they must not be run while somebody
else is using the machine.

### The conversion program

The picture operations need the FFmpeg program, which is not in the
repository: it is built from the sources by the script in `ffmpeg`, and the
package is made from what that script builds. A developer who wants to try
the conversions keeps the program in `vendor\ffmpeg`, which the repository
ignores.

```sh
cargo build --features test-helper --bins   # the stand-in the tests use
cargo test --features test-helper
```

The tests that run against the real program are marked as needing it, and
are run with `cargo test -- --ignored` once it has been built.

## The documents

- [`docs/INTERACTION.md`](docs/INTERACTION.md) - the gesture, the states, the
  keys, the modifier mapping.
- [`docs/CAPABILITIES.md`](docs/CAPABILITIES.md) - what the program can do,
  what the machine can do, and how an operation is run.
- [`docs/DRAG-INTEGRATION.md`](docs/DRAG-INTEGRATION.md) — how a Windows
  Explorer drag is found, what was measured, and what the limits are.
- [`docs/MILESTONE-1-REPORT.md`](docs/MILESTONE-1-REPORT.md) — what was built,
  what was measured, and what is still to do.

## Licence

MIT or Apache-2.0, at your choice.
