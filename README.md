# Open Tangerine for Windows

A radial wheel for the files you are dragging. Hold a modifier while dragging files out of File Explorer (or Desktop) and a fluid, hardware-composited radial wheel appears under the cursor: move onto an action, release the mouse, and Tangerine executes the operation with high performance.

## Key Features

- **Fluid Radial Menu**:
  - Three distinct contextual wheels: **Convert**, **Tools**, and **File Actions**.
  - Dynamic wheel switching using modifier keys (**Shift**, **Ctrl**, **Alt**) or mouse scroll / Up & Down arrows.
  - Per-pixel alpha transparency and native Win32 layered window composition.
- **18 Interactive Tool Cards & Dedicated Windows**:
  - **Compress**: Smart video, audio, and image compression with target size computation.
  - **Crop & Resize**: Interactive aspect ratios, freeform cropping, and dimension adjustment.
  - **Trim & Cut**: Dual-handle timeline scrubbing with sub-second precision.
  - **Split Video**: Multi-cut timeline markers with equal segment splitting.
  - **Redact (Image, Audio, Video)**: Visual and temporal blur/pixelate/blackout redaction.
  - **Edit Metadata**: Interactive Exif, ID3, and media metadata editor with tag inspection.
  - **Normalize Audio**: EBU R128 loudness normalization and peak limiting.
  - **Channels & Mix**: Mono/Stereo matrix remapping and channel isolation.
  - **Frame Image**: Framed presentation with drop shadows, padding, and gradient backgrounds.
  - **Organize PDF**: Multi-page reordering, rotation (90°/180°/270°), and page extraction.
  - **Merge PDF / Join Videos**: Drag-and-drop file reordering, thumbnail inspection, and lossless concatenation.
  - **Adjust Speed**: Speed multiplier adjustments with pitch correction.
  - **Audio to Video**: Audio visualizer waveforms, frequency bars, and animated video generation.
  - **Annotate Photos**: Freehand drawing, callout text, bounding boxes, and blur annotations.
  - **Collage Maker**: Multi-photo layouts (Grid, Split, Featured) with corner rounding and custom backgrounds.
  - **Image Adjustments**: Exposure, brightness, contrast, saturation, temperature, and tint controls.
- **Native Document & Archive Conversions**:
  - In-process lossless conversions for CSV, TSV, XLSX, HTML, Markdown, PDF, and archive extraction/creation (.zip, .tar, .gz) without external shell dependencies.
- **High-DPI & Multi-Monitor**: Full per-monitor DPI awareness (`WM_DPICHANGED`) with dynamic UI re-scaling.
- **Privacy & Security First**: Sandboxed argument vectors, zip slip protection, streaming archive handling, and zero telemetry.

## Build and Run

```powershell
cargo build --release
cargo test
.\target\release\open-tangerine.exe
```

The program requires no external runtimes beyond Windows 10/11 and bundled media tooling.

### CLI Options

| Option | What it does |
| --- | --- |
| `--dev-wheel` | Opens the radial wheel directly for UI testing and inspection. |
| `--exit-after <seconds>` | Automatically closes after specified seconds (used by automated checks). |
| `--help` | Prints help and command-line usage. |

The log level can be set via environment variable:

```powershell
$env:OPEN_TANGERINE_LOG = "debug"  # debug, info, warn, error
```

## Shortcuts & Controls

- **Drag + Shift**: Open **Convert** wheel.
- **Drag + Ctrl**: Open **Tools** wheel.
- **Drag + Alt**: Open **File Actions** wheel.
- **Scroll Wheel** or **Up / Down**: Switch between wheels.
- **Left / Right**: Cycle highlighted action segments.
- **Enter**: Execute selected action.
- **Escape**: Cancel and dismiss menu.
- **Ctrl+Alt+T**: Launch development wheel preview without active drag.

## Architecture

```
src/
  main.rs                  Application entry point and CLI parsing
  app.rs                   Message loop, window orchestration, and event dispatch
  diagnostics.rs           Structured timestamped logging
  session.rs               Active drag session tracking and file resolution
  settings.rs              Persistent configuration and user preferences
  media/                   Core media processing engines
    adjustments.rs         Color balance and exposure adjustments
    annotate.rs            Photo annotation rendering
    audio.rs               PCM waveform analysis and peak detection
    collage.rs             Multi-photo grid and layout compositor
    command.rs             Safe external process argument generation
    documents.rs           PDF and archive generation
    frame.rs               Framed image and background composition
    metadata.rs            Exif and multimedia tag inspection/editing
    pdf.rs                 PDF parsing and page rendering via PDFium
    visualizer.rs          Audio-to-video visualizer graphics
  platform/windows/        Win32 platform integration
    clipboard.rs           Native Win32 clipboard provider
    droptarget.rs          OLE IDropTarget implementation
    explorer.rs            Explorer drag detection
    input.rs               Global low-level input hooks
    monitors.rs            Multi-monitor enumeration and DPI tracking
    overlay.rs             Layered radial wheel host window
    windowhost.rs          Shared Win32 host helpers and DPI scaling
    *window.rs             Dedicated Win32 host implementations for each tool
  radial/                  Wheel business logic
    actions.rs             Wheel definition and segment configuration
    controller.rs          State machine for drag interaction
    execution.rs           Tool execution coordinator
    file_actions.rs        File system and archive action handlers
    geometry.rs            Wheel trigonometry, layout, and hit-testing
    registry.rs            Tool capabilities and registration
    tools.rs               FFmpeg and tool command invocations
  ui/                      Slint UI templates and software renderer bindings
    animation.rs           Segment transition and glow animation easing
    theme.rs               Light and Dark mode design tokens
    wheel.slint            Radial menu Slint presentation
tests/                     Integration tests, automated UI captures, and verification
```

## Running Tests

```powershell
cargo test
```

Integration tests requiring external desktop interaction:

```powershell
powershell -ExecutionPolicy Bypass -File tests\windows-integration.ps1
```

## Packaging

To produce a production-ready standalone distribution archive:

```powershell
powershell -ExecutionPolicy Bypass -File scripts\package_dist.ps1
```

The resulting zip archive will be generated in `dist/OpenTangerine.zip`.

## License

Dual-licensed under MIT OR Apache-2.0.
