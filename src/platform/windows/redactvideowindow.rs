//! The window a video is covered in.
//!
//! The covering does not happen the moment it is picked: it opens a
//! window with the video working in it — its frame fitted on a dark
//! stage, with the numbered rectangles of cover standing over the
//! frame, a square hand at each corner of the chosen one and a pill on
//! each of its edges. Below the stage the look is said in rows: which
//! of the two coverings the boxes take and what colour a solid box
//! wears. Then the Redactions: one row for every cover, dragging the
//! stretch of the video that cover stands over onto a track, the way
//! a trim's range is dragged. Under the rows the strip of the video's
//! own moments is worn in piece by piece, the playhead runs down
//! through the rows and across it, and under that the transport stands:
//! to the start, to the end, and play in between.
//!
//! A cover that no stretch was dragged for stands for the whole video.
//! One style and one colour are said for every cover together, the way
//! the covering is made — the coverings are laid on the video in one
//! taking, so boxes cannot wear different looks.
//!
//! The card is drawn by the same renderer that draws the wheel, and the
//! skeleton it wears is the shell's. The card only draws: the pressing,
//! the corners, the edges, the rows and the seeking are read by the
//! window itself, which holds the same geometry the card was given —
//! the way the crop and the trim do. The covers are kept whole in the
//! media's own pixels and the stretches in the video's own seconds, so
//! what the person sees and what the file is covered by can never
//! disagree; on the save the covers are said in thousandths of the
//! media's own size, and the stretches in thousandths of a second.
//!
//! The window's width is fixed; its height grows by a row for every
//! cover standing in it, and is re-fit when the covers come and go.
//!
//! What the window is asked comes back as a message, the same message
//! the plain tool window sends: the window never runs anything itself.

use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc::Sender;

use slint::platform::software_renderer::MinimalSoftwareWindow;
use slint::platform::{PointerEventButton, WindowAdapter, WindowEvent};
use slint::{LogicalPosition, ModelRc, PhysicalSize, SharedPixelBuffer, VecModel};
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{BeginPaint, CreateSolidBrush, EndPaint, InvalidateRect};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::AdjustWindowRectExForDpi;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, ReleaseCapture, SetCapture, VK_BACK, VK_DELETE, VK_ESCAPE, VK_RETURN, VK_SHIFT,
    VK_TAB,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, GetClientRect, GetWindowRect, KillTimer, LoadCursorW,
    RegisterClassW, SetCursor, SetForegroundWindow, SetLayeredWindowAttributes, SetTimer,
    SetWindowPos, SetWindowTextW, ShowWindow, SW_HIDE, SW_SHOW, SWP_NOACTIVATE, SWP_NOZORDER,
    CW_USEDEFAULT, IDC_ARROW, IDC_CROSS, IDC_SIZEALL, IDC_SIZENESW, IDC_SIZENS, IDC_SIZENWSE,
    IDC_SIZEWE, LWA_ALPHA, SWP_NOMOVE, WNDCLASSW, WM_CLOSE, WM_DESTROY, WM_DPICHANGED,
    WM_ERASEBKGND, WM_KEYDOWN, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_PAINT, WM_SIZE,
    WM_TIMER, WS_EX_LAYERED,
};

use crate::media::frames::{duration_of, frame_at, pillar_of};
use crate::media::locator;
use crate::platform::windows::toolwindow::ToolEvent;
use crate::platform::windows::windowhost::{
    fixed_style, fraction, scale_factor, screen_scale, timecode, to_wide,
};
use crate::radial::execution::{RedactArea, RedactStyle, ToolSettings};
use crate::ui::wheel::{
    window_for_next_component, Bgra8, LaneData, Panel, RectData, RedactVideoCard,
};

/// The timer the window comes in softly on.
const FADE_TIMER_ID: usize = 52;
/// The timer the window asks the strip's own worker by, whether some
/// of its moments have come.
const STRIP_TIMER_ID: usize = 54;
/// The timer that carries a playing video on, beat after beat.
const PLAY_TIMER_ID: usize = 56;
/// How far a playing video moves on in one beat of its timer, in
/// seconds. The beats are asked of the window eighty times a second.
const PLAY_STEP: f64 = 0.08;

/// The width of the card, in the drawing's own units.
const CARD_W: i32 = 420;
/// How tall the stage is, in the drawing's own units.
const PREVIEW_UNITS: f32 = 236.0;
/// How tall the strip of moments is, in the drawing's own units.
const STRIP_UNITS: f32 = 40.0;
/// One tile of the strip wants to stand about this wide, in the
/// drawing's own units, before another moment is asked of the video.
const PILLAR_UNITS: f32 = 16.0;

/// How far from a corner of a cover a press still takes its handle,
/// in pixels of the media.
const CORNER_REACH: f64 = 12.0;
/// How far from the middle of an edge its pill still takes the press,
/// in pixels of the media.
const EDGE_REACH: f64 = 6.0;
/// How far from the cap at the end of a row's stretch a press still
/// takes the cap, in pixels of the window.
const CAP_REACH: f64 = 8.0;
/// The least a cover may be, in pixels of the media along either
/// side, before a drawn one is dropped.
const MIN_COVER: f64 = 8.0;
/// The least a stretch of a row may be, in seconds of the video,
/// before a drawn one is dropped.
const MIN_WINDOW: f64 = 0.25;
/// How much a cover is pulled in from the media's edges when the
/// window arrives: a tenth of each.
const INSET: f64 = 0.1;
/// How deep inside a cover a press must stand to carry it along
/// instead of drawing past it.
const MOVE_INSET: f64 = 4.0;
/// How far off the stage or the media a press may land and still be
/// read as touching it, in window pixels.
const GRACE: f64 = 8.0;
/// How far the playhead must move before a new frame is asked of the
/// video, in seconds. Asking for every beat of a drag would ask for
/// more frames than a person can see.
const FRAME_STEP: f64 = 0.25;

/// The rows' own places, in the drawing's own units: where the first
/// row's track begins, how much it is tall, how much a row stands
/// above the one after it, and where the rows begin under the words
/// of the section.
const LANE_LEFT_UNITS: f32 = 44.0;
const LANE_HEIGHT_UNITS: f32 = 26.0;
const LANE_STEP_UNITS: f32 = 32.0;
const LANES_TOP_UNITS: f32 = 454.0;

/// The colours a solid box takes on, in order: the well cycles them.
const PALETTE: [&str; 6] = ["000000", "FFFFFF", "808080", "C0392B", "2ECC71", "3B82F6"];

/// A rectangle said in four numbers: where it stands, and how much it
/// covers.
pub type Box4 = (f64, f64, f64, f64);

/// Where a press took hold on a cover: the corners carry their
/// numbers the way the crop counts them — TL, TR, BR, BL — and the
/// edges were given the numbers after.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
enum Spot {
    #[default]
    TopLeft,
    TopRight,
    BottomRight,
    BottomLeft,
    Top,
    Right,
    Bottom,
    Left,
}

impl Spot {
    /// Whether the spot is a pill on an edge rather than a handle on
    /// a corner: an edge drag holds one side and lets the other run.
    fn is_edge(self) -> bool {
        matches!(
            self,
            Spot::Top | Spot::Right | Spot::Bottom | Spot::Left
        )
    }
}

/// What a press on the stage lands on: a handle of the chosen cover,
/// the face of some cover, or the space where a new one is drawn.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Pick {
    Corner(Spot),
    Track(usize),
    Create,
}

/// The ways the pointer can be busy on a row: nothing taken, a fresh
/// stretch being drawn between the press and now, a whole stretch
/// being carried along, one of its caps being pulled, or the playhead
/// being brought to where the pointer stands.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
enum LaneDrag {
    #[default]
    Nothing,
    /// A fresh stretch: which row it is drawn on, and at what second
    /// of the video the press took.
    New { lane: usize, from: f64 },
    /// A whole stretch comes along: which row, at what second the
    /// press took beside its start, and how much the stretch spans.
    Move { lane: usize, grab: f64, span: f64 },
    /// One cap of the stretch is pulled: which row, and whether it is
    /// the one at the start.
    Edge { lane: usize, start: bool },
    /// The playhead is brought to the pointer, on the strip.
    Seek,
}

/// What a press on a row's track lands itself on: an empty track to
/// draw on, the cap at an end of the stretch, a stretch to carry
/// along, or a stretch's own track away from it — where a press does
/// nothing, so a person's slip is not taken for a wish to redraw.
#[derive(Clone, Copy, Debug, PartialEq)]
enum LanePick {
    EmptyTrack,
    StartCap,
    EndCap,
    Inside,
    Apart,
}

/// One cover of the video: where it stands in the media's own pixels,
/// and — when a stretch was dragged for it — the stretch of the video
/// it stands over, in the video's own seconds.
#[derive(Clone, Copy, Debug, PartialEq)]
struct RedactBox {
    cut: Box4,
    window: Option<(f64, f64)>,
}

/// What the window remembers between the frames: the media and where
/// it stands, the covers and their stretches, which one stands
/// chosen, the look they take, where the video stands, and what the
/// pointer is doing.
#[derive(Debug)]
struct RedactState {
    /// The media's own size, in pixels.
    media_w: f64,
    media_h: f64,
    /// Where the media sits on the stage, and how many window pixels
    /// one pixel of the media takes.
    media_box: Box4,
    fit: f64,
    /// The covers and their stretches.
    rects: Vec<RedactBox>,
    /// Which cover the pointer's last choice left chosen.
    selected: Option<usize>,
    /// The look the covers take: zero a solid box, one a smear.
    style: usize,
    /// Which colour of the wheel the solid boxes stand at.
    color: usize,
    /// The stage's own drag, when a cover is the business at hand.
    dragging: Drag,
    /// The row's own drag, when a stretch is.
    lane_drag: LaneDrag,
    /// The handle a resizing drag took. A drag begins with nothing.
    grab: Spot,
    /// The box a drag began from.
    anchored: Box4,
    /// How long the video runs, and where its playhead stands, in
    /// seconds; which second the frame showing was taken at, so it is
    /// only asked again when the playhead has truly moved; and
    /// whether the video is playing.
    duration: f64,
    playhead: f64,
    frame_at: f64,
    playing: bool,
    /// The programs and the file the frames come of.
    ffmpeg: Option<locator::Tools>,
    input: PathBuf,
}

/// The ways a stage drag is standing.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
enum Drag {
    #[default]
    Nothing,
    /// A cover moves with the pointer: which one, and where the press
    /// took it, in the media's own pixels.
    Moving { index: usize, press: (f64, f64) },
    /// A handle holds the chosen cover: its side stays put, the one
    /// away from it follows the pointer.
    Resizing { index: usize },
    /// A cover is being drawn: where the press took the media, and
    /// which slot of the covers it stands in while it is drawn.
    Drawing { press: (f64, f64), index: usize },
}

impl Default for RedactState {
    fn default() -> Self {
        RedactState {
            media_w: 0.0,
            media_h: 0.0,
            media_box: (0.0, 0.0, 0.0, 0.0),
            fit: 1.0,
            rects: Vec::new(),
            selected: None,
            style: 0,
            color: 0,
            dragging: Drag::Nothing,
            lane_drag: LaneDrag::Nothing,
            grab: Spot::TopLeft,
            anchored: (0.0, 0.0, 0.0, 0.0),
            duration: 0.0,
            playhead: 0.0,
            frame_at: f64::NEG_INFINITY,
            playing: false,
            ffmpeg: None,
            input: PathBuf::new(),
        }
    }
}

thread_local! {
    static HOST: std::cell::RefCell<Option<Host>> = std::cell::RefCell::new(None);
    static WINDOW: std::cell::Cell<isize> = const { std::cell::Cell::new(0) };
    static STATE: std::cell::RefCell<RedactState> =
        std::cell::RefCell::new(RedactState::default());
    static OPEN_FOR: std::cell::RefCell<Option<(String, Vec<PathBuf>)>> =
        const { std::cell::RefCell::new(None) };
    static EVENTS: std::cell::RefCell<Option<Sender<ToolEvent>>> =
        const { std::cell::RefCell::new(None) };
    /// The keyboard's stop: 0 nothing, 1 and 2 the style chips, 3 the
    /// colour well, 4 play, 5 save.
    static FOCUS_INDEX: std::cell::Cell<i32> = const { std::cell::Cell::new(0) };
    static FADE_ALPHA: std::cell::Cell<u8> = const { std::cell::Cell::new(255) };
    /// The strip's own moments are worked out elsewhere; what comes of
    /// the work is buried here for the window to collect.
    static STRIP_INBOX: std::cell::RefCell<Option<std::sync::mpsc::Receiver<StripPiece>>> =
        const { std::cell::RefCell::new(None) };
    /// Whether the finished strip has been put on the card already.
    static STRIP_WORN: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// One finished strip: its pixels laid one colour after another, and
/// how wide and tall it should be said.
type StripPiece = (Vec<u8>, u32, u32);

/// The window handle, if there is one.
fn window_handle() -> HWND {
    HWND(WINDOW.get() as *mut _)
}

/// How many rows the card is saying: one for every cover.
fn lane_count() -> usize {
    STATE.with(|slot| slot.borrow().rects.len())
}

/// The window, its card, and the buffer the card is drawn into.
struct Host {
    window: HWND,
    card: RedactVideoCard,
    software: Rc<MinimalSoftwareWindow>,
    buffer: Vec<Bgra8>,
    width: u32,
    height: u32,
    /// How many rows the card was last laid out for: when a cover
    /// comes or goes, the window is re-fit for the rows it now has.
    last_lanes: usize,
}

/// The window a video is covered in.
pub struct RedactVideoWindow {
    window: HWND,
}

// The window belongs to the thread that made it and is us
unsafe impl Send for RedactVideoWindow {}

impl RedactVideoWindow {
    /// Makes the window, hidden until a cover is asked for.
    pub fn new(events: Sender<ToolEvent>) -> Result<RedactVideoWindow, String> {
        EVENTS.with(|slot| *slot.borrow_mut() = Some(events));

        // A window is put aside for the card before the card is made, so
        // the two find each other.
        let software = window_for_next_component();
        let card = RedactVideoCard::new().map_err(|error| error.to_string())?;

        let instance = unsafe { GetModuleHandleW(None).map_err(|error| error.to_string())? };
        let class_name = w!("OpenTangerineRedactVideo");
        // The card's own white is what fills it while it comes in, so a
        // dark theme's idea of "window colour" never shows through.
        let scale = screen_scale();
        let (outer_w, outer_h) = outer_size(scale, 1);
        let window = unsafe {
            let class = WNDCLASSW {
                lpfnWndProc: Some(window_procedure),
                hInstance: instance.into(),
                lpszClassName: class_name,
                hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
                hbrBackground: CreateSolidBrush(COLORREF(0x00FF_FFFF)),
                ..Default::default()
            };
            RegisterClassW(&class);

            CreateWindowExW(
                WS_EX_LAYERED,
                class_name,
                w!("Open Tangerine"),
                fixed_style(),
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                outer_w,
                outer_h,
                None,
                None,
                Some(instance.into()),
                None,
            )
        }
        .map_err(|error| error.to_string())?;

        // The corners of the window are round on a system that rounds
        // its windows; on one that does not, they stay as they are.
        unsafe {
            use windows::Win32::Graphics::Dwm::{
                DwmSetWindowAttribute, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND,
            };
            let preference = DWMWCP_ROUND;
            let _ = DwmSetWindowAttribute(
                window,
                DWMWA_WINDOW_CORNER_PREFERENCE,
                &preference as *const _ as *const core::ffi::c_void,
                std::mem::size_of::<i32>() as u32,
            );
        }

        // The buttons of the card say what they were pressed for, the
        // way the wheel's choices do. They read the handle on its own,
        // not the host they are drawn by, because a button can be
        // pressed while a drawing is still holding the host.
        {
            card.on_run_chosen(move || {
                let window = window_handle();
                if !window.0.is_null() {
                    unsafe { run(window) };
                }
            });
            card.on_style_chosen(move |chip| {
                let window = window_handle();
                if !window.0.is_null() {
                    unsafe { choose_style(window, (chip.max(1) - 1) as usize) };
                }
            });
            card.on_color_chosen(move || {
                let window = window_handle();
                if !window.0.is_null() {
                    unsafe { choose_color(window) };
                }
            });
            card.on_play_chosen(move || {
                let window = window_handle();
                if !window.0.is_null() {
                    unsafe { toggle_play(window) };
                }
            });
            card.on_skip_back_chosen(move || {
                let window = window_handle();
                if !window.0.is_null() {
                    unsafe { jump(window, false) };
                }
            });
            card.on_skip_forward_chosen(move || {
                let window = window_handle();
                if !window.0.is_null() {
                    unsafe { jump(window, true) };
                }
            });
        }

        let mut host = Host {
            window,
            card,
            software,
            buffer: Vec::new(),
            width: 0,
            height: 0,
            last_lanes: 1,
        };
        host.resize();
        HOST.with(|slot| *slot.borrow_mut() = Some(host));
        WINDOW.with(|slot| slot.set(window.0 as isize));

        Ok(RedactVideoWindow { window })
    }

    /// Asks the window to hide itself.
    pub fn hide(&mut self) {
        unsafe {
            let _ = ShowWindow(self.window, SW_HIDE);
        }
    }

    /// The first frame of the video is the one asked about: how long
    /// it runs, how big it is. All of it is asked before the window is
    /// shown, so what appears is already the video's own card at its
    /// own shape.
    pub fn show_for(&mut self, _operation: &str, label: &str, detail: &str, inputs: &[PathBuf]) {
        OPEN_FOR.with(|slot| {
            *slot.borrow_mut() = Some((_operation.to_string(), inputs.to_vec()));
        });

        let said = if inputs.len() == 1 {
            inputs[0]
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| inputs[0].to_string_lossy().into_owned())
        } else {
            format!("{} files", inputs.len())
        };

        let input = inputs
            .first()
            .cloned()
            .unwrap_or_else(|| PathBuf::from("nothing"));
        let extension = input
            .extension()
            .and_then(|part| part.to_str())
            .map(|part| part.to_lowercase())
            .unwrap_or_default();

        // A cover-up works on a video and nothing else: when what was
        // dragged is not one, the window does not open at all.
        if crate::media::probe::kind_of(&extension) != Some(crate::media::probe::Kind::Video) {
            OPEN_FOR.with(|slot| *slot.borrow_mut() = None);
            return;
        }

        let ffmpeg = locator::find();
        // The fold it opens on answers for its shape and it is asked
        // how long it runs. Without the conversion program the window
        // still opens: the person's covers are placed by hand, and the
        // first frame is the only one missing.
        let first = ffmpeg
            .as_ref()
            .and_then(|tools| frame_at(tools, &input, 0.0));
        let (media_w, media_h) = first
            .as_ref()
            .map(|frame| (frame.width(), frame.height()))
            .unwrap_or((16, 9));
        let duration = ffmpeg
            .as_ref()
            .and_then(|tools| duration_of(tools, &input))
            .unwrap_or(0.0);
        let (media_w, media_h) = (media_w as f64, media_h as f64);

        STATE.with(|slot| {
            let mut state = slot.borrow_mut();
            *state = RedactState {
                media_w,
                media_h,
                // One cover stands drawn as the window arrives, a tenth
                // in from every edge, and it stands chosen. It is asked
                // for the whole video until a stretch is dragged for it.
                rects: vec![RedactBox {
                    cut: inset_cover(media_w, media_h),
                    window: None,
                }],
                selected: Some(0),
                style: 0,
                color: 0,
                dragging: Drag::Nothing,
                lane_drag: LaneDrag::Nothing,
                grab: Spot::TopLeft,
                anchored: (0.0, 0.0, 0.0, 0.0),
                duration,
                playhead: 0.0,
                frame_at: 0.0,
                playing: false,
                ffmpeg: ffmpeg.clone(),
                input: input.clone(),
                fit: 1.0,
                media_box: (0.0, 0.0, 0.0, 0.0),
            }
        });

        // Everything the card needs is set first, while the host is
        // held only for that and nothing is shown yet. Where the media
        // stands is told after the host is let go, because telling it
        // draws a frame, and drawing must never find the card still
        // held.
        let card_window = HOST.with(|slot| {
            let mut borrowed = slot.borrow_mut();
            let Some(host) = borrowed.as_mut() else {
                return HWND::default();
            };
            let lanes = lane_count();
            host.last_lanes = lanes;
            host.card.set_file_name(said.into());
            host.card.set_detail(detail.into());
            host.card.set_playing(false);
            host.card.set_has_strip(false);
            host.card.set_strip_image(slint::Image::default());
            host.lay_out();
            // The video's own first frame, at the size of its own box.
            if let Some(frame) = first {
                let scale = scale_factor(host.window);
                let layout = Layout::of(host.width as i32, host.height as i32, scale, lanes);
                let (box_w, box_h) = media_standing_box(&layout);
                let small = frame.thumbnail(box_w.max(1), box_h.max(1));
                let rgba = small.to_rgba8();
                let buffer = SharedPixelBuffer::clone_from_slice(
                    rgba.as_raw(),
                    small.width(),
                    small.height(),
                );
                host.card.set_frame(slint::Image::from_rgba8(buffer));
                host.card.set_has_frame(true);
            } else {
                host.card.set_has_frame(false);
            }
            host.software.request_redraw();
            host.window
        });
        // The rows' own playhead and time are said with the host let
        // go: the saying takes the host again for its own telling.
        refresh_times();

        // The strip of moments is worked out elsewhere: the window asks
        // it every few beats whether some of it has come.
        let strip_window = card_window;
        if strip_window.0.is_null() {
            return;
        }
        {
            STATE.with(|slot| {
                let state = slot.borrow();
                if state.duration > 0.0 {
                    if let Some(tools) = state.ffmpeg.as_ref() {
                        let edges = strip_edges(card_window);
                        ask_for_strip(
                            tools.clone(),
                            state.input.clone(),
                            state.duration,
                            edges.0,
                            edges.1,
                        );
                        unsafe {
                            let _ = SetTimer(Some(card_window), STRIP_TIMER_ID, 40, None);
                        }
                    }
                }
            });
        }

        unsafe {
            // The window is made the size the card wants, where it
            // already stands.
            let mut stands = RECT::default();
            let _ = GetWindowRect(card_window, &mut stands);
            let scale = scale_factor(card_window);
            let (outer_w, outer_h) = outer_size(scale, lane_count());
            let _ = SetWindowPos(
                card_window,
                None,
                stands.left,
                stands.top,
                outer_w,
                outer_h,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
            let _ = SetWindowTextW(card_window, PCWSTR::from_raw(to_wide(label).as_ptr()));
            let _ = InvalidateRect(Some(card_window), None, true);
            // The window comes in softly, from nothing to whole in a
            // few beats of the timer.
            FADE_ALPHA.with(|slot| slot.set(0));
            let _ = SetLayeredWindowAttributes(card_window, COLORREF(0), 0, LWA_ALPHA);
            let _ = SetForegroundWindow(card_window);
            let _ = ShowWindow(card_window, SW_SHOW);
            let _ = SetTimer(Some(card_window), FADE_TIMER_ID, 16, None);
            // The keyboard starts with nothing under it: the chips are
            // one Tab away, and the covers take the pointer first.
            FOCUS_INDEX.with(|slot| slot.set(0));
        }
        draw_window(card_window);
    }
}

/// Where the strip is being worn, in window pixels of the window the
/// strip is being worked for: how wide and tall it will be worn as.
fn strip_edges(window: HWND) -> (u32, u32) {
    HOST.with(|slot| {
        slot.borrow().as_ref().map(|host| {
            if host.window != window {
                return (352, 40);
            }
            let scale = scale_factor(host.window);
            let lanes = lane_count();
            let layout = Layout::of(host.width as i32, host.height as i32, scale, lanes);
            (
                (layout.strip.right - layout.strip.left).max(1) as u32,
                (layout.strip.bottom - layout.strip.top).max(1) as u32,
            )
        })
    })
    .unwrap_or((352, 40))
}

/// Asks the strip of moments elsewhere, one moment of the video at a
/// time, laying the tiles' answers together into one strip.
fn ask_for_strip(tools: locator::Tools, input: PathBuf, duration: f64, width: u32, height: u32) {
    let (sender, receiver) = std::sync::mpsc::channel::<StripPiece>();
    std::thread::spawn(move || {
        let tiles = ((width as f32 / PILLAR_UNITS).round() as u32).clamp(6, 30);
        let tile_w = (width / tiles).max(1);
        let tile_h = height.max(1);
        let step = (duration / tiles as f64).max(0.05);
        let mut strip = image::RgbaImage::new(tile_w * tiles, tile_h);
        for index in 0..tiles {
            let moment = ((index as f64 + 0.5) * step).min(duration);
            let Some(frame) = frame_at(&tools, &input, moment) else {
                continue;
            };
            let pillar = pillar_of(&frame, tile_w, tile_h);
            image::imageops::overlay(&mut strip, &pillar, (index * tile_w) as i64, 0);
        }
        let (wide, tall) = (strip.width(), strip.height());
        let _ = sender.send((strip.into_raw(), wide, tall));
    });
    STRIP_INBOX.with(|slot| {
        *slot.borrow_mut() = Some(receiver);
    });
    STRIP_WORN.with(|slot| slot.set(false));
}

/// Takes one turn of the strip's arriving, without waiting.
fn strip_tick() -> Option<StripPiece> {
    STRIP_INBOX.with(|slot| {
        let mut buried = slot.borrow_mut();
        let Some(receiver) = buried.as_ref() else {
            return None;
        };
        match receiver.try_recv() {
            Ok(piece) => Some(piece),
            Err(std::sync::mpsc::TryRecvError::Empty) => None,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                *buried = None;
                None
            }
        }
    })
}

/// Puts a finished strip on the card and asks the window to paint it,
/// the way the trim wears its own strip.
unsafe fn wear_strip(piece: StripPiece) {
    let (pixels, width, height) = piece;
    if width == 0 || height == 0 {
        return;
    }
    let buffer = SharedPixelBuffer::clone_from_slice(&pixels, width, height);
    let image = slint::Image::from_rgba8(buffer);
    HOST.with(|slot| {
        if let Some(host) = slot.borrow_mut().as_mut() {
            host.card.set_strip_image(image);
            host.card.set_has_strip(true);
            host.software.request_redraw();
        }
    });
    let window = window_handle();
    if !window.0.is_null() {
        let _ = unsafe { InvalidateRect(Some(window), None, true) };
    }
    STRIP_WORN.with(|slot| slot.set(true));
}

/// How wide a window of this card must be made so that what a person
/// uses of it is the size the drawing asked for, at this stretch. The
/// height grows by a row for every cover standing in it.
fn outer_size(scale: f32, lanes: usize) -> (i32, i32) {
    let client_w = (CARD_W as f32 * scale) as i32;
    let client_h = (Layout::height(lanes) * scale) as i32;
    let mut rect = RECT {
        left: 0,
        top: 0,
        right: client_w,
        bottom: client_h,
    };
    unsafe {
        let _ = AdjustWindowRectExForDpi(
            &mut rect,
            fixed_style(),
            false,
            Default::default(),
            (scale * 96.0) as u32,
        );
    }
    (rect.right - rect.left, rect.bottom - rect.top)
}

impl Host {
    /// Makes the buffer the size the window is, and tells the card.
    fn resize(&mut self) {
        let mut rect = RECT::default();
        let _ = unsafe { GetClientRect(self.window, &mut rect) };
        let width = rect.right.max(1) as u32;
        let height = rect.bottom.max(1) as u32;
        if width != self.width || height != self.height {
            self.width = width;
            self.height = height;
            self.last_lanes = lane_count();
            self.buffer = vec![Bgra8::default(); (width as usize) * (height as usize)];
            // The card's own size is said here, directly: the renderer
            // draws exactly what these two properties say, and a size
            // that only travelled through the window's events has been
            // lost between the first frame and the next.
            let scale = scale_factor(self.window);
            self.card.set_card_width(width as f32 / scale);
            self.card.set_card_height(height as f32 / scale);
            self.software.set_size(PhysicalSize::new(width, height));
            self.lay_out();
        }
    }

    /// Says every box the card asks for, and puts the media and the
    /// covers where they stand. The same numbers are read by the
    /// window when the stage or a row is pressed, so what is drawn and
    /// what is taken can never disagree.
    fn lay_out(&mut self) {
        let scale = scale_factor(self.window);
        let lanes = lane_count();
        let layout = Layout::of(self.width as i32, self.height as i32, scale, lanes);
        let panel = |rect: RECT| Panel {
            x: rect.left as f32,
            y: rect.top as f32,
            width: (rect.right - rect.left) as f32,
            height: (rect.bottom - rect.top) as f32,
        };
        self.card.set_ui_scale(scale);
        self.card.set_header_box(panel(layout.file_name));
        self.card.set_detail_box(panel(layout.detail));
        self.card.set_hairline_top_box(panel(layout.hairline_top));
        self.card.set_hairline_mid_box(panel(layout.hairline_mid));
        self.card.set_hairline_bottom_box(panel(layout.hairline_bottom));
        self.card.set_stage_box(panel(layout.stage));
        self.card.set_style_label_box(panel(layout.style_label));
        self.card.set_solid_box(panel(layout.solid));
        self.card.set_blur_box(panel(layout.blur));
        self.card.set_color_label_box(panel(layout.color_label));
        self.card.set_well_box(panel(layout.well));
        self.card.set_redactions_label_box(panel(layout.redactions_label));
        self.card.set_lanes_box(panel(layout.lanes));
        self.card.set_strip_box(panel(layout.strip));
        self.card.set_skip_back_box(panel(layout.skip_back));
        self.card.set_play_box(panel(layout.play));
        self.card.set_skip_forward_box(panel(layout.skip_forward));
        self.card.set_time_text_box(panel(layout.time_text));
        self.card.set_run_box(panel(layout.run));
        stand_media(&mut self.card, box4_of(layout.stage));
        stand_covers(self);
        stand_lanes(self);
    }

    /// Draws the card if it changed, and puts what was drawn on the
    /// window.
    fn draw(&mut self) {
        let stride = self.width as usize;
        let mut buffer = std::mem::take(&mut self.buffer);
        self.software
            .draw_if_needed(|renderer| {
                renderer.render(&mut buffer, stride);
            });
        self.buffer = buffer;
        unsafe { self.put_on_window() };
    }

    /// Puts the pixels of the last drawing on the window.
    unsafe fn put_on_window(&self) {
        unsafe {
            let dc = windows::Win32::Graphics::Gdi::GetDC(Some(self.window));
            let mut info = windows::Win32::Graphics::Gdi::BITMAPINFO::default();
            info.bmiHeader.biSize = std::mem::size_of::<
                windows::Win32::Graphics::Gdi::BITMAPINFOHEADER,
            >() as u32;
            info.bmiHeader.biWidth = self.width as i32;
            // Negative height: the first row of the buffer is the top row.
            info.bmiHeader.biHeight = -(self.height as i32);
            info.bmiHeader.biPlanes = 1;
            info.bmiHeader.biBitCount = 32;
            info.bmiHeader.biCompression =
                windows::Win32::Graphics::Gdi::BI_RGB.0 as u32;
            windows::Win32::Graphics::Gdi::SetDIBitsToDevice(
                dc,
                0,
                0,
                self.width,
                self.height,
                0,
                0,
                0,
                self.height,
                self.buffer.as_ptr() as *const core::ffi::c_void,
                &info,
                windows::Win32::Graphics::Gdi::DIB_RGB_COLORS,
            );
            let _ = windows::Win32::Graphics::Gdi::ReleaseDC(Some(self.window), dc);
        }
    }
}

/// Draws what the card has, if the window the drawing was made for is
/// the one asked.
fn draw_window(window: HWND) {
    HOST.with(|slot| {
        let mut borrowed = slot.borrow_mut();
        if let Some(host) = borrowed.as_mut() {
            if host.window == window {
                host.draw();
            }
        }
    });
}

/// Where every part of the window is, in pixels of the screen.
///
/// The card is a stack: a header, a hairline, the stage, the two rows
/// the look is said in, an opening for the Redactions, a row for
/// every cover, the strip of moments, the transport, a closing
/// hairline, and the footer. The Rust side works the stack out once
/// and both sides read it; one row more or less moves everything
/// under the Redactions down by one step.
#[derive(Clone, Copy, Debug, Default)]
struct Layout {
    file_name: RECT,
    detail: RECT,
    hairline_top: RECT,
    stage: RECT,
    style_label: RECT,
    solid: RECT,
    blur: RECT,
    color_label: RECT,
    well: RECT,
    hairline_mid: RECT,
    redactions_label: RECT,
    lanes: RECT,
    strip: RECT,
    skip_back: RECT,
    play: RECT,
    skip_forward: RECT,
    time_text: RECT,
    hairline_bottom: RECT,
    run: RECT,
}

impl Layout {
    /// Everything of the stack except the header's own band and the
    /// footer, in the drawing's own units, for a card holding no
    /// covers. A cover adds one step of a row to it.
    const LANES_BASE_UNITS: f32 = 486.0;
    /// The header's band.
    const HEADER_UNITS: f32 = 62.0;
    /// How much a row of the Redactions adds, in the drawing's own
    /// units: the row's own height and the air below it.
    const LANE_STEP_UNITS: f32 = 32.0;
    /// The footer.
    const FOOTER_UNITS: f32 = 64.0;

    /// How tall a window of this card is, holding this many covers.
    fn height(lanes: usize) -> f32 {
        Self::HEADER_UNITS
            + Self::LANES_BASE_UNITS
            + lanes as f32 * Self::LANE_STEP_UNITS
            + Self::FOOTER_UNITS
    }

    fn of(whole: i32, side: i32, scale: f32, lanes: usize) -> Layout {
        let at = |value: f32| (value * scale) as i32;
        let margin = at(24.0);

        // The stack, top to bottom, all worked out from the one number.
        let header_top = at(16.0);
        let hairline_top = at(62.0);
        let stage_top = hairline_top + at(20.0);
        let stage_bottom = stage_top + at(PREVIEW_UNITS);

        // The two rows the look is said in: the style chips, the
        // colour well — a label at the left, the control at the right.
        let style_top = stage_bottom + at(14.0);
        let style_h = at(34.0);
        let color_top = style_top + style_h + at(10.0);
        let color_h = at(38.0);
        let hairline_mid = color_top + color_h + at(10.0);
        let redactions_top = hairline_mid + at(10.0);
        let lanes_top = redactions_top + at(20.0);
        let strip_top = lanes_top + (lanes as i32 * at(LANE_STEP_UNITS)) + at(2.0);
        let transport_top = strip_top + at(STRIP_UNITS) + at(10.0);
        let hairline_bottom = transport_top + at(30.0) + at(12.0);

        // The controls stand at the right of their rows; the labels
        // keep a room of their own at the left.
        let row_w = at(140.0);
        let well_w = at(62.0);
        let track_left = at(LANE_LEFT_UNITS);
        let track_right = whole - margin;

        Layout {
            file_name: RECT {
                left: margin,
                top: header_top,
                right: whole - margin,
                bottom: header_top + at(22.0),
            },
            detail: RECT {
                left: margin,
                top: header_top + at(24.0),
                right: whole - margin,
                bottom: header_top + at(40.0),
            },
            hairline_top: RECT {
                left: 0,
                top: hairline_top,
                right: whole,
                bottom: hairline_top + 1,
            },
            stage: RECT {
                left: margin,
                top: stage_top,
                right: whole - margin,
                bottom: stage_bottom,
            },
            style_label: RECT {
                left: margin,
                top: style_top,
                right: margin + at(64.0),
                bottom: style_top + style_h,
            },
            solid: RECT {
                left: whole - margin - row_w + at(3.0),
                top: style_top + at(4.0),
                right: whole - margin - row_w + at(3.0) + at(64.0),
                bottom: style_top + at(4.0) + at(26.0),
            },
            blur: RECT {
                left: whole - margin - row_w + at(73.0),
                top: style_top + at(4.0),
                right: whole - margin - row_w + at(73.0) + at(64.0),
                bottom: style_top + at(4.0) + at(26.0),
            },
            color_label: RECT {
                left: margin,
                top: color_top,
                right: margin + at(64.0),
                bottom: color_top + color_h,
            },
            well: RECT {
                left: whole - margin - well_w,
                top: color_top,
                right: whole - margin,
                bottom: color_top + color_h,
            },
            hairline_mid: RECT {
                left: 0,
                top: hairline_mid,
                right: whole,
                bottom: hairline_mid + 1,
            },
            redactions_label: RECT {
                left: margin,
                top: redactions_top,
                right: margin + at(120.0),
                bottom: redactions_top + at(16.0),
            },
            lanes: RECT {
                left: margin,
                top: lanes_top,
                right: track_right,
                bottom: lanes_top + (lanes as i32 * at(LANE_STEP_UNITS)) - at(6.0),
            },
            strip: RECT {
                left: track_left,
                top: strip_top,
                right: track_right,
                bottom: strip_top + at(STRIP_UNITS),
            },
            skip_back: RECT {
                left: track_left,
                top: transport_top,
                right: track_left + at(30.0),
                bottom: transport_top + at(30.0),
            },
            play: RECT {
                left: track_left + at(34.0),
                top: transport_top,
                right: track_left + at(64.0),
                bottom: transport_top + at(30.0),
            },
            skip_forward: RECT {
                left: track_left + at(68.0),
                top: transport_top,
                right: track_left + at(98.0),
                bottom: transport_top + at(30.0),
            },
            time_text: RECT {
                left: whole - margin - at(216.0),
                top: transport_top,
                right: whole - margin,
                bottom: transport_top + at(30.0),
            },
            hairline_bottom: RECT {
                left: 0,
                top: hairline_bottom,
                right: whole,
                bottom: hairline_bottom + 1,
            },
            // The footer is measured from the bottom of the window, so
            // a window of an unexpected height still puts the button
            // where a person reaches it.
            run: RECT {
                left: whole - margin - at(168.0),
                top: side - at(50.0),
                right: whole - margin,
                bottom: side - at(16.0),
            },
        }
    }
}

/// Where a row's track stands, in window pixels: the whole said for
/// the window the rows stand in, at the display's stretch.
fn lane_row(whole: i32, scale: f32, index: usize) -> RECT {
    let at = |value: f32| (value * scale) as i32;
    let top = at(LANES_TOP_UNITS) + (index as i32 * at(LANE_STEP_UNITS));
    RECT {
        left: at(LANE_LEFT_UNITS),
        right: whole - at(24.0),
        top,
        bottom: top + at(LANE_HEIGHT_UNITS),
    }
}

/// Which row the press landed on, if any. The air between the rows is
/// nobody's: a press there is left alone.
fn which_lane(whole: i32, scale: f32, lanes: usize, x: i32, y: i32) -> Option<usize> {
    (0..lanes).find(|&index| {
        let row = lane_row(whole, scale, index);
        x >= row.left && x <= row.right && y >= row.top && y <= row.bottom
    })
}

/// Where the strip of moments stands, in window pixels, said the way
/// the layout says it.
fn strip_rect(whole: i32, scale: f32, lanes: usize) -> RECT {
    let at = |value: f32| (value * scale) as i32;
    let top = at(LANES_TOP_UNITS) + (lanes as i32 * at(LANE_STEP_UNITS)) + at(2.0);
    RECT {
        left: at(LANE_LEFT_UNITS),
        right: whole - at(24.0),
        top,
        bottom: top + at(STRIP_UNITS),
    }
}

/// How many window pixels one pixel of the media takes: the smaller
/// of the two squeezes. A media with nothing to say takes nothing
/// away from itself.
fn fit_of(stage_w: f64, stage_h: f64, media_w: f64, media_h: f64) -> f64 {
    if stage_w <= 0.0 || stage_h <= 0.0 || media_w <= 0.0 || media_h <= 0.0 {
        return 1.0;
    }
    (stage_w / media_w).min(stage_h / media_h)
}

/// The box the media stands in, said from the layout's stage.
fn media_standing_box(layout: &Layout) -> (u32, u32) {
    (
        (layout.stage.right - layout.stage.left).max(1) as u32,
        (layout.stage.bottom - layout.stage.top).max(1) as u32,
    )
}

/// A cover pulled in a tenth from every edge: what the window opens
/// holding, so the first thing a person sees is what they will drag.
fn inset_cover(media_w: f64, media_h: f64) -> Box4 {
    (
        media_w * INSET,
        media_h * INSET,
        media_w * (1.0 - 2.0 * INSET),
        media_h * (1.0 - 2.0 * INSET),
    )
}

/// Reads a window RECT as a box of four numbers.
fn box4_of(rect: RECT) -> Box4 {
    (
        rect.left as f64,
        rect.top as f64,
        (rect.right - rect.left) as f64,
        (rect.bottom - rect.top) as f64,
    )
}

/// Whether a point stands within a box, with a little grace around it.
fn under(box4: Box4, grace: f64, point: (f64, f64)) -> bool {
    point.0 >= box4.0 - grace
        && point.0 <= box4.0 + box4.2 + grace
        && point.1 >= box4.1 - grace
        && point.1 <= box4.1 + box4.3 + grace
}

/// What a window pixel said as a pixel of the media.
fn to_media(point: (f64, f64), media_box: Box4, fit: f64) -> (f64, f64) {
    if fit <= 0.0 {
        return point;
    }
    ((point.0 - media_box.0) / fit, (point.1 - media_box.1) / fit)
}

/// What a pixel of the media said as a window pixel, as a box: where
/// the cover stands over the stage.
fn cut_in_window(cut: Box4, media_box: Box4, fit: f64) -> Box4 {
    (
        media_box.0 + cut.0 * fit,
        media_box.1 + cut.1 * fit,
        cut.2 * fit,
        cut.3 * fit,
    )
}

/// The four corners of a cover, in order: 0 the top-left, 1 the
/// top-right, 2 the bottom-right, 3 the bottom-left.
fn corners_of(cut: Box4) -> [(f64, f64); 4] {
    [
        (cut.0, cut.1),
        (cut.0 + cut.2, cut.1),
        (cut.0 + cut.2, cut.1 + cut.3),
        (cut.0, cut.1 + cut.3),
    ]
}

/// A rectangle between two places, whichever way the pointer went.
fn standardized(from: (f64, f64), to: (f64, f64)) -> Box4 {
    (
        from.0.min(to.0),
        from.1.min(to.1),
        (from.0 - to.0).abs(),
        (from.1 - to.1).abs(),
    )
}

/// A rectangle that runs off the media is pulled back to it, and what
/// is left is never without size.
fn clamped_to_media(cut: Box4, media_w: f64, media_h: f64) -> Box4 {
    let (x, y, w, h) = cut;
    let x = x.clamp(0.0, (media_w - 1.0).max(0.0));
    let y = y.clamp(0.0, (media_h - 1.0).max(0.0));
    let w = w.clamp(0.0, media_w - x).max(1.0);
    let h = h.clamp(0.0, media_h - y).max(1.0);
    (x, y, w, h)
}

/// The point of the cover a drag holds still: the corner across from
/// a corner's handle, the middle of the side across from a pill.
fn anchor_of(grab: Spot, cut: Box4) -> (f64, f64) {
    let corners = corners_of(cut);
    match grab {
        Spot::TopLeft => corners[2],
        Spot::TopRight => corners[3],
        Spot::BottomRight => corners[0],
        Spot::BottomLeft => corners[1],
        Spot::Top => (cut.0 + cut.2 / 2.0, cut.1 + cut.3),
        Spot::Right => (cut.0, cut.1 + cut.3 / 2.0),
        Spot::Bottom => (cut.0 + cut.2 / 2.0, cut.1),
        Spot::Left => (cut.0 + cut.2, cut.1 + cut.3 / 2.0),
    }
}

/// What a press in the stage took. The chosen cover is asked first —
/// its corners, then the pills on its edges, then its face. Any other
/// cover answers in the order they were drawn, the newest first. A
/// press into nothing begins a drawing.
fn picked_at(point: (f64, f64), rects: &[RedactBox], selected: Option<usize>, fit: f64) -> Pick {
    if let Some(index) = selected {
        if let Some(one) = rects.get(index) {
            let cut = one.cut;
            // The corners' handles, nearly the same reach the crop's
            // hands ask for.
            for (position, corner) in corners_of(cut).iter().enumerate() {
                if (point.0 - corner.0).abs() <= CORNER_REACH
                    && (point.1 - corner.1).abs() <= CORNER_REACH
                {
                    return Pick::Corner(match position {
                        1 => Spot::TopRight,
                        2 => Spot::BottomRight,
                        3 => Spot::BottomLeft,
                        _ => Spot::TopLeft,
                    });
                }
            }
            // The pills on the edges stand only on a cover big enough
            // to draw them; a press that deep is a carry-along there.
            let w = cut.2 * fit;
            let h = cut.3 * fit;
            if w > 44.0 && h > 22.0 {
                let along_x = point.0 > cut.0 + cut.2 * 0.25
                    && point.0 < cut.0 + cut.2 * 0.75;
                let along_y = point.1 > cut.1 + cut.3 * 0.25
                    && point.1 < cut.1 + cut.3 * 0.75;
                if along_x && (point.1 - cut.1).abs() <= EDGE_REACH {
                    return Pick::Corner(Spot::Top);
                }
                if along_x && (point.1 - (cut.1 + cut.3)).abs() <= EDGE_REACH {
                    return Pick::Corner(Spot::Bottom);
                }
                if along_y && (point.0 - cut.0).abs() <= EDGE_REACH {
                    return Pick::Corner(Spot::Left);
                }
                if along_y && (point.0 - (cut.0 + cut.2)).abs() <= EDGE_REACH {
                    return Pick::Corner(Spot::Right);
                }
            }
            // The face of the cover, away from its own borders.
            if point.0 > cut.0 + MOVE_INSET
                && point.0 < cut.0 + cut.2 - MOVE_INSET
                && point.1 > cut.1 + MOVE_INSET
                && point.1 < cut.1 + cut.3 - MOVE_INSET
            {
                return Pick::Track(index);
            }
        }
    }
    // Any other cover, newest first: its face takes the press and
    // comes along chosen.
    for index in (0..rects.len()).rev() {
        if Some(index) == selected {
            continue;
        }
        let cut = rects[index].cut;
        if point.0 > cut.0 + MOVE_INSET
            && point.0 < cut.0 + cut.2 - MOVE_INSET
            && point.1 > cut.1 + MOVE_INSET
            && point.1 < cut.1 + cut.3 - MOVE_INSET
        {
            return Pick::Track(index);
        }
    }
    Pick::Create
}

/// A corner dragged with the one across standing still; a pill on an
/// edge pulls one side only.
fn resized(
    grab: Spot,
    anchored: Box4,
    point: (f64, f64),
    media_w: f64,
    media_h: f64,
) -> Box4 {
    if !grab.is_edge() {
        let anchor = anchor_of(grab, anchored);
        let cut = standardized(anchor, point);
        let cut = (cut.0, cut.1, cut.2.max(1.0), cut.3.max(1.0));
        return clamped_to_media(cut, media_w, media_h);
    }
    let cut = match grab {
        Spot::Top => {
            let bottom = anchored.1 + anchored.3;
            let top = point.1.min(bottom - 1.0);
            (anchored.0, top, anchored.2, bottom - top)
        }
        Spot::Bottom => {
            let bottom = point.1.max(anchored.1 + 1.0);
            (anchored.0, anchored.1, anchored.2, bottom - anchored.1)
        }
        Spot::Left => {
            let right = anchored.0 + anchored.2;
            let left = point.0.min(right - 1.0);
            (left, anchored.1, right - left, anchored.3)
        }
        Spot::Right => {
            let right = point.0.max(anchored.0 + 1.0);
            (anchored.0, anchored.1, right - anchored.0, anchored.3)
        }
        _ => anchored,
    };
    clamped_to_media(cut, media_w, media_h)
}

/// A cover drawn between where the press took and where the pointer
/// stands, held inside the media.
fn drawn(start: (f64, f64), point: (f64, f64), media_w: f64, media_h: f64) -> Box4 {
    let cut = standardized(start, point);
    clamped_to_media(
        (cut.0, cut.1, cut.2.max(1.0), cut.3.max(1.0)),
        media_w,
        media_h,
    )
}

/// A cover carried along a drag, clamped so it never leaves the media.
fn placed(
    original: Box4,
    from: (f64, f64),
    to: (f64, f64),
    media_w: f64,
    media_h: f64,
) -> Box4 {
    let along_x = (to.0 - from.0).clamp(-original.0, (media_w - original.2 - original.0).max(0.0));
    let along_y = (to.1 - from.1).clamp(-original.1, (media_h - original.3 - original.1).max(0.0));
    (original.0 + along_x, original.1 + along_y, original.2, original.3)
}

/// Whether a drawn cover is too small to mean anything. A smaller
/// drag is dropped when the pointer lets go.
fn is_too_small(cut: Box4) -> bool {
    cut.2 < MIN_COVER || cut.3 < MIN_COVER
}

/// What a press on a row's track takes hold of. The reach of a cap is
/// said in pixels of the window, the way the person sees.
fn lane_pick(window: &Option<(f64, f64)>, press: f64, reach: f64) -> LanePick {
    match window {
        None => LanePick::EmptyTrack,
        Some((start, end)) => {
            let start = *start;
            let end = *end;
            if (press - start).abs() <= reach {
                LanePick::StartCap
            } else if (press - end).abs() <= reach {
                LanePick::EndCap
            } else if press > start && press < end {
                LanePick::Inside
            } else {
                LanePick::Apart
            }
        }
    }
}

/// A stretch drawn between the second a press took and the one the
/// pointer stands at, whichever way the drag went.
fn stretched_window(from: f64, to: f64) -> (f64, f64) {
    (from.min(to), from.max(to))
}

/// A stretch carried along a drag, held inside the video.
fn moved_window(
    original: (f64, f64),
    grab: f64,
    now: f64,
    duration: f64,
) -> (f64, f64) {
    let span = original.1 - original.0;
    let start = (now - grab).clamp(0.0, (duration - span).max(0.0));
    (start, start + span)
}

/// A stretch with one of its caps pulled, held inside the video and
/// never poorer than the least a stretch may be.
fn edged_window(original: (f64, f64), start: bool, now: f64, duration: f64) -> (f64, f64) {
    let (start_at, end_at) = original;
    if start {
        let begin = now.clamp(0.0, (end_at - MIN_WINDOW).max(0.0));
        (begin, end_at)
    } else {
        let finish = now.clamp((start_at + MIN_WINDOW).min(duration), duration);
        (start_at, finish)
    }
}

/// Whether a stretch of a row is too small to mean anything. A
/// smaller drag is dropped when the pointer lets go.
fn window_too_small(window: (f64, f64)) -> bool {
    window.1 - window.0 < MIN_WINDOW
}

/// The stretches the backend asks them: seconds said in thousandths
/// of a second, and no stretch said at all when there was none.
fn windows_of(window: Option<(f64, f64)>) -> (Option<u64>, Option<u64>) {
    window.map(|(start, end)| {
        (
            Some((start * 1000.0).round().max(0.0) as u64),
            Some((end * 1000.0).round().max(0.0) as u64),
        )
    })
    .unwrap_or((None, None))
}

/// A share of the media said in thousandths, as the backend carries
/// covers: half of the whole is five hundred.
fn mille(value: f64, whole: f64) -> u32 {
    if whole <= 0.0 {
        return 0;
    }
    ((value / whole * 1000.0).round() as i64).clamp(0, 1000) as u32
}

/// The covers as the backend asks them: thousandths everywhere, said
/// from the pixels of the media they were drawn on, each with its own
/// stretch of the video in thousandths of a second.
fn areas_of(rects: &[RedactBox], media_w: f64, media_h: f64) -> Vec<RedactArea> {
    rects
        .iter()
        .map(|one| {
            let (start_ms, end_ms) = windows_of(one.window);
            RedactArea {
                x_mille: mille(one.cut.0, media_w),
                y_mille: mille(one.cut.1, media_h),
                w_mille: mille(one.cut.2, media_w),
                h_mille: mille(one.cut.3, media_h),
                start_ms,
                end_ms,
            }
        })
        .collect()
}

/// The settings the card's own state says: the look, the colour, the
/// covers and their stretches.
fn settings_from(
    style: usize,
    color: usize,
    rects: &[RedactBox],
    media_w: f64,
    media_h: f64,
) -> ToolSettings {
    ToolSettings {
        redact_areas: areas_of(rects, media_w, media_h),
        redact_style: Some(if style == 0 {
            RedactStyle::Solid
        } else {
            RedactStyle::Blur
        }),
        redact_color: Some(color_hex(color).to_string()),
        ..Default::default()
    }
}

/// The colour of the wheel at a stop, as the settings carry it.
fn color_hex(index: usize) -> &'static str {
    PALETTE[index % PALETTE.len()]
}

/// The parts of a hex colour, three of them.
fn palette_parts(index: usize) -> (u8, u8, u8) {
    let hex = color_hex(index);
    let at = |number: usize| {
        u8::from_str_radix(&hex[number..number + 2], 16).unwrap_or(0)
    };
    (at(0), at(2), at(4))
}

/// The colour as the card wears it.
fn color_of(index: usize) -> slint::Color {
    let (red, green, blue) = palette_parts(index);
    slint::Color::from_rgb_u8(red, green, blue)
}

/// Puts the media where it stands on the stage given it, and says the
/// card so.
fn stand_media(card: &mut RedactVideoCard, stage: Box4) {
    let panel = STATE.with(|slot| {
        let mut state = slot.borrow_mut();
        let fit = fit_of(stage.2, stage.3, state.media_w, state.media_h);
        let w = state.media_w * fit;
        let h = state.media_h * fit;
        let left = stage.0 + (stage.2 - w) / 2.0;
        let top = stage.1 + (stage.3 - h) / 2.0;
        state.fit = fit;
        state.media_box = (left, top, w, h);
        Panel {
            x: left as f32,
            y: top as f32,
            width: w as f32,
            height: h as f32,
        }
    });
    card.set_media_box(panel);
}

/// Puts the covers where they stand over the media, and says the card
/// so, with the look they take and which one the person left chosen.
fn stand_covers(host: &mut Host) {
    let (covers, chosen, solid, color) = STATE.with(|slot| {
        let state = slot.borrow();
        let covers: Vec<RectData> = state
            .rects
            .iter()
            .map(|one| {
                let (x, y, w, h) = cut_in_window(one.cut, state.media_box, state.fit);
                RectData {
                    x: x as f32,
                    y: y as f32,
                    width: w as f32,
                    height: h as f32,
                }
            })
            .collect();
        (covers, state.selected, state.style == 0, state.color)
    });
    host.card.set_rects(ModelRc::new(VecModel::from(covers)));
    host.card
        .set_selected_rect(chosen.map(|index| index as i32).unwrap_or(-1));
    host.card.set_solid_style(solid);
    host.card.set_well_color(color_of(color));
    host.software.request_redraw();
}

/// Puts the rows where they stand under the words of the section, one
/// for every cover, each carrying its own stretch — or the saying
/// that asks for one to be dragged on.
fn stand_lanes(host: &mut Host) {
    let scale = scale_factor(host.window);
    let whole = host.width as i32;
    let lanes: Vec<LaneData> = STATE.with(|slot| {
        let state = slot.borrow();
        state
            .rects
            .iter()
            .enumerate()
            .map(|(index, one)| {
                let row = lane_row(whole, scale, index);
                let (start, end, filled) = match one.window {
                    Some((from, to)) if state.duration > 0.0 => {
                        (fraction(from, state.duration), fraction(to, state.duration), true)
                    }
                    _ => (0.0, 0.0, false),
                };
                LaneData {
                    index: index as i32,
                    number: (index + 1) as i32,
                    x: row.left as f32,
                    y: row.top as f32,
                    width: (row.right - row.left) as f32,
                    height: (row.bottom - row.top) as f32,
                    start: start as f32,
                    end: end as f32,
                    filled,
                }
            })
            .collect()
    });
    host.card.set_lanes(ModelRc::new(VecModel::from(lanes)));
    host.software.request_redraw();
}

/// Puts the state's covers and rows on the card and asks for a paint.
/// When a cover has come or gone, the window is re-fit for the rows
/// it now has — the telling of the system is kept away from the host,
/// because the size's own messages would find the host still held.
unsafe fn stand_all(window: HWND) {
    let re_fit = HOST.with(|slot| {
        let mut borrowed = slot.borrow_mut();
        let Some(host) = borrowed.as_mut() else {
            return false;
        };
        let lanes = lane_count();
        let re_fit = lanes != host.last_lanes;
        host.last_lanes = lanes;
        stand_covers(host);
        stand_lanes(host);
        re_fit
    });
    if re_fit {
        let scale = scale_factor(window);
        let (outer_w, outer_h) = outer_size(scale, lane_count());
        let _ = unsafe {
            SetWindowPos(
                window,
                None,
                0,
                0,
                outer_w,
                outer_h,
                SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOMOVE,
            )
        };
    }
    let _ = unsafe { InvalidateRect(Some(window), None, true) };
}

/// Says where the video stands, what the time is, and whether it is
/// playing.
fn refresh_times() {
    let (frac, said, playing) = STATE.with(|slot| {
        let state = slot.borrow();
        (
            fraction(state.playhead, state.duration),
            format!("{} / {}", timecode(state.playhead), timecode(state.duration)),
            state.playing,
        )
    });
    HOST.with(|slot| {
        if let Some(host) = slot.borrow_mut().as_mut() {
            host.card.set_playhead_fraction(frac);
            host.card.set_time_text(said.into());
            host.card.set_playing(playing);
        }
    });
}

/// The room the frame stands in, said the way the layout says it.
fn stage_pixels() -> (u32, u32) {
    HOST.with(|slot| {
        slot.borrow().as_ref().map(|host| {
            let scale = scale_factor(host.window);
            let layout = Layout::of(
                host.width as i32,
                host.height as i32,
                scale,
                lane_count(),
            );
            media_standing_box(&layout)
        })
    })
    .unwrap_or((372, 236))
}

/// Asks the video for the frame the playhead stands on, but only when
/// it has truly moved on: asking for a frame of a video is a seek and
/// a taking, and a drag of the playhead need not be shown nearer than
/// a person can see.
fn refresh_frame(force: bool) {
    STATE.with(|slot| {
        let mut state = slot.borrow_mut();
        let wanted = state.playhead;
        if !force && (wanted - state.frame_at).abs() < FRAME_STEP {
            return;
        }
        let Some(frame) = state
            .ffmpeg
            .as_ref()
            .and_then(|tools| frame_at(tools, &state.input, wanted))
        else {
            return;
        };
        state.frame_at = wanted;
        let (box_w, box_h) = stage_pixels();
        let small = frame.thumbnail(box_w.max(1), box_h.max(1));
        let rgba = small.to_rgba8();
        let buffer = SharedPixelBuffer::clone_from_slice(rgba.as_raw(), small.width(), small.height());
        HOST.with(|slot| {
            if let Some(host) = slot.borrow_mut().as_mut() {
                host.card.set_frame(slint::Image::from_rgba8(buffer));
                host.card.set_has_frame(true);
                host.software.request_redraw();
            }
        });
    });
}

/// Tells the card the keyboard has arrived at a stop.
unsafe fn apply_focus(window: HWND, index: i32) {
    FOCUS_INDEX.with(|slot| slot.set(index));
    HOST.with(|slot| {
        let mut borrowed = slot.borrow_mut();
        if let Some(host) = borrowed.as_mut() {
            if host.window == window {
                host.card.set_focus_stop(index);
                host.software.request_redraw();
            }
        }
    });
    unsafe {
        let _ = InvalidateRect(Some(window), None, true);
    }
}

/// Moves the keyboard to the stop after — or before — the one it is
/// on. The stops go round: the two style chips, the colour well, play,
/// and save.
unsafe fn cycle_focus(window: HWND, forward: bool) {
    let now = FOCUS_INDEX.with(|slot| slot.get());
    let next = if forward { now + 1 } else { now - 1 };
    let stop = if next > 5 {
        1
    } else if next < 1 {
        5
    } else {
        next
    };
    unsafe { apply_focus(window, stop) };
}

/// Takes the look the chip says: zero a box of one colour, one a
/// smear.
unsafe fn choose_style(window: HWND, chip: usize) {
    STATE.with(|slot| slot.borrow_mut().style = chip.min(1));
    unsafe { stand_all(window) };
}

/// Turns the colour well to the next colour of the wheel.
unsafe fn choose_color(window: HWND) {
    STATE.with(|slot| {
        let mut state = slot.borrow_mut();
        state.color = (state.color + 1) % PALETTE.len();
    });
    unsafe { stand_all(window) };
}

/// Takes the chosen cover away, and its row with it. What stands
/// under it answers: the newest of what is left becomes the chosen
/// one.
unsafe fn remove_selected(window: HWND) {
    STATE.with(|slot| {
        let mut state = slot.borrow_mut();
        if let Some(index) = state.selected.take() {
            if index < state.rects.len() {
                state.rects.remove(index);
            }
        }
    });
    unsafe { stand_all(window) };
}

/// Says what the window was asked, and puts it away. The covers are
/// said in thousandths of the media's own size and their stretches in
/// thousandths of a second; every picture of the drag takes the same
/// covering at the same relative places. A cover that was given no
/// stretch stands for the whole video.
unsafe fn run(window: HWND) {
    STATE.with(|slot| {
        let state = slot.borrow();
        if state.dragging != Drag::Nothing
            || state.lane_drag != LaneDrag::Nothing
            || state.rects.is_empty()
        {
            return;
        }
        let settings = settings_from(
            state.style,
            state.color,
            &state.rects,
            state.media_w,
            state.media_h,
        );
        OPEN_FOR.with(|slot| {
            let mut open_for = slot.borrow_mut();
            if let Some((operation, inputs)) = open_for.take() {
                send(ToolEvent::Run {
                    operation,
                    inputs,
                    settings,
                });
            }
        });
    });
    unsafe {
        let _ = ShowWindow(window, SW_HIDE);
    }
}

/// Says no to what the window was open for, and puts it away.
unsafe fn cancel(window: HWND) {
    send(ToolEvent::Cancelled);
    unsafe {
        let _ = ShowWindow(window, SW_HIDE);
    }
}

/// Says what the window was asked, to whoever is listening.
fn send(event: ToolEvent) {
    EVENTS.with(|slot| {
        if let Some(sender) = slot.borrow().as_ref() {
            let _ = sender.send(event);
        }
    });
}

/// Plays, or stops playing. The playing stands on a timer of its own,
/// carrying the playhead on beat after beat.
unsafe fn toggle_play(window: HWND) {
    let playing = STATE.with(|slot| {
        let mut state = slot.borrow_mut();
        if state.duration <= 0.0 {
            return false;
        }
        if state.playing {
            state.playing = false;
            false
        } else {
            state.playing = true;
            // Playing from the end begins again at the start.
            if state.playhead >= state.duration - f64::EPSILON {
                state.playhead = 0.0;
                state.frame_at = f64::NEG_INFINITY;
            }
            true
        }
    });
    let card_window = window_handle();
    if !card_window.0.is_null() {
        if playing {
            let _ = unsafe { SetTimer(Some(card_window), PLAY_TIMER_ID, 80, None) };
        } else {
            let _ = unsafe { KillTimer(Some(card_window), PLAY_TIMER_ID) };
        }
    }
    refresh_times();
    unsafe {
        let _ = InvalidateRect(Some(window), None, true);
    }
}

/// Brings the playhead to the start or the end of the video, and
/// asks the video for the frame there.
unsafe fn jump(window: HWND, forward: bool) {
    STATE.with(|slot| {
        let mut state = slot.borrow_mut();
        state.playhead = if forward { state.duration } else { 0.0 };
        state.frame_at = f64::NEG_INFINITY;
    });
    refresh_frame(true);
    refresh_times();
    unsafe {
        let _ = InvalidateRect(Some(window), None, true);
    }
}

/// The shape the cursor wears where the press would land on a cover:
/// a handle shapes, a cover slides, empty space draws.
fn cursor_for(pick: Pick) -> PCWSTR {
    match pick {
        Pick::Corner(Spot::TopLeft | Spot::BottomRight) => IDC_SIZENWSE,
        Pick::Corner(Spot::TopRight | Spot::BottomLeft) => IDC_SIZENESW,
        Pick::Corner(Spot::Top | Spot::Bottom) => IDC_SIZENS,
        Pick::Corner(Spot::Left | Spot::Right) => IDC_SIZEWE,
        Pick::Track(_) => IDC_SIZEALL,
        Pick::Create => IDC_CROSS,
    }
}

/// The shape the cursor wears where the press would land on a row: a
/// cap is pulled sideways, a stretch slides, an empty track draws.
fn lane_cursor_for(pick: LanePick) -> PCWSTR {
    match pick {
        LanePick::StartCap | LanePick::EndCap => IDC_SIZEWE,
        LanePick::Inside => IDC_SIZEALL,
        LanePick::EmptyTrack => IDC_CROSS,
        LanePick::Apart => IDC_ARROW,
    }
}

/// How wide a row's track is, in window pixels.
fn track_width(whole: i32, scale: f32) -> f64 {
    let at = |value: f32| (value * scale) as i32;
    ((whole - at(24.0)) as f64 - at(LANE_LEFT_UNITS) as f64).max(1.0)
}

/// What second of the video a window pixel of a row's track stands
/// at.
fn track_secs(x: i32, whole: i32, scale: f32, duration: f64) -> f64 {
    let at = |value: f32| (value * scale) as i32;
    let frac = ((x as f64 - at(LANE_LEFT_UNITS) as f64) / track_width(whole, scale)).clamp(0.0, 1.0);
    frac * duration
}

/// A press on a row: it chooses its cover, and takes a stretch on —
/// or pulls at the stretch already there.
unsafe fn lane_pressed(window: HWND, index: usize, x: i32, whole: i32, scale: f32) {
    let width = track_width(whole, scale);
    let press_secs = track_secs(x, whole, scale, 1.0);
    STATE.with(|slot| {
        let mut state = slot.borrow_mut();
        if state.duration <= MIN_WINDOW || state.lane_drag != LaneDrag::Nothing {
            return;
        }
        state.selected = Some(index);
        let press = press_secs * state.duration;
        let reach = CAP_REACH / width;
        let picked = state.rects.get(index).map(|one| one.window);
        let Some(window_at) = picked else {
            return;
        };
        match lane_pick(&window_at, press, reach) {
            LanePick::EmptyTrack => {
                state.lane_drag = LaneDrag::New { lane: index, from: press };
                let slot_value = state.rects.get_mut(index);
                if let Some(one) = slot_value {
                    one.window = Some((press, press));
                }
            }
            LanePick::StartCap => {
                state.lane_drag = LaneDrag::Edge { lane: index, start: true };
            }
            LanePick::EndCap => {
                state.lane_drag = LaneDrag::Edge { lane: index, start: false };
            }
            LanePick::Inside => {
                if let Some((start, end)) = window_at {
                    state.lane_drag = LaneDrag::Move {
                        lane: index,
                        grab: press - start,
                        span: end - start,
                    };
                }
            }
            LanePick::Apart => {
                // A press away from a stretch on a track that has one
                // is left alone: a person's slip is not taken for a
                // wish to redraw.
            }
        }
    });
    unsafe {
        let _ = SetCapture(window);
    }
    unsafe {
        stand_all(window);
    }
}

/// A press on the strip: the playhead is brought to where it stands.
unsafe fn seek_pressed(window: HWND, x: i32, whole: i32, scale: f32) {
    STATE.with(|slot| {
        let mut state = slot.borrow_mut();
        if state.duration <= 0.0 || state.lane_drag != LaneDrag::Nothing {
            return;
        }
        state.playhead = track_secs(x, whole, scale, state.duration);
        state.lane_drag = LaneDrag::Seek;
    });
    unsafe {
        let _ = SetCapture(window);
    }
    refresh_frame(false);
    refresh_times();
    unsafe {
        stand_all(window);
    }
}

/// A press on the stage: the same business the picture's own window
/// carries — a cover taken whole, a handle held, or a fresh one
/// drawn.
unsafe fn stage_pressed(window: HWND, point: (f64, f64)) {
    STATE.with(|slot| {
        let mut state = slot.borrow_mut();
        if state.dragging != Drag::Nothing {
            return;
        }
        match picked_at(point, &state.rects, state.selected, state.fit) {
            Pick::Create => {
                // A cover begins empty and grows while it is drawn;
                // nothing stands chosen while it is.
                let index = state.rects.len();
                state.rects.push(RedactBox {
                    cut: (point.0, point.1, 0.0, 0.0),
                    window: None,
                });
                state.selected = None;
                state.dragging = Drag::Drawing {
                    press: point,
                    index,
                };
            }
            Pick::Track(index) => {
                state.selected = Some(index);
                state.anchored = state.rects[index].cut;
                state.dragging = Drag::Moving {
                    index,
                    press: point,
                };
            }
            Pick::Corner(spot) => {
                let index = state.selected.unwrap_or(0);
                if index < state.rects.len() {
                    state.grab = spot;
                    state.anchored = state.rects[index].cut;
                    state.dragging = Drag::Resizing { index };
                }
            }
        }
    });
    unsafe {
        let _ = SetCapture(window);
    }
}

/// A move carries a row drag along: a stretch is drawn out, slid, or
/// pulled at one cap; the playhead follows the pointer on the strip.
fn lane_moved(x: i32, whole: i32, scale: f32) {
    STATE.with(|slot| {
        let mut state = slot.borrow_mut();
        match state.lane_drag {
            LaneDrag::Nothing => {}
            LaneDrag::Seek => {
                state.playhead = track_secs(x, whole, scale, state.duration);
            }
            LaneDrag::New { lane, from } => {
                let to = track_secs(x, whole, scale, state.duration);
                let slot_value = state.rects.get_mut(lane);
                if let Some(one) = slot_value {
                    one.window = Some(stretched_window(from, to));
                }
            }
            LaneDrag::Move { lane, grab, span } => {
                let now = track_secs(x, whole, scale, state.duration);
                // The span is all the original says to the carrier.
                let moved = moved_window((0.0, span), grab, now, state.duration);
                let slot_value = state.rects.get_mut(lane);
                if let Some(one) = slot_value {
                    one.window = Some(moved);
                }
            }
            LaneDrag::Edge { lane, start } => {
                let now = track_secs(x, whole, scale, state.duration);
                let sitting = state.rects.get(lane).and_then(|one| one.window);
                if let Some(window_at) = sitting {
                    let edged = edged_window(window_at, start, now, state.duration);
                    let slot_value = state.rects.get_mut(lane);
                    if let Some(one) = slot_value {
                        one.window = Some(edged);
                    }
                }
            }
        }
    });
}

/// A move carries a stage drag along, the way the picture's own
/// window does.
fn stage_moved(point: (f64, f64)) {
    STATE.with(|slot| {
        let mut state = slot.borrow_mut();
        let dragging = state.dragging;
        match dragging {
            Drag::Nothing => {}
            Drag::Moving { index, press } => {
                let cut = placed(state.anchored, press, point, state.media_w, state.media_h);
                let slot_value = state.rects.get_mut(index);
                if let Some(one) = slot_value {
                    one.cut = cut;
                }
            }
            Drag::Resizing { index } => {
                let cut = resized(state.grab, state.anchored, point, state.media_w, state.media_h);
                let slot_value = state.rects.get_mut(index);
                if let Some(one) = slot_value {
                    one.cut = cut;
                }
            }
            Drag::Drawing { press, index } => {
                let cut = drawn(press, point, state.media_w, state.media_h);
                let slot_value = state.rects.get_mut(index);
                if let Some(one) = slot_value {
                    one.cut = cut;
                }
            }
        }
    });
}

/// Converts what the pointer did on the window into what the covers
/// and the stretches should hold. The press is read here, where the
/// geometry is the same one the card was drawn by; a move carries the
/// drag along.
fn touched(window: HWND, x: i32, y: i32, pressed: bool) {
    let mine = HOST.with(|slot| {
        let borrowed = slot.borrow();
        borrowed.as_ref().map(|host| host.window == window)
    });
    if mine != Some(true) {
        return;
    }

    // How wide the card's own grid stands, and at what stretch.
    let (whole, scale, lanes) = HOST.with(|slot| {
        let borrowed = slot.borrow();
        let Some(host) = borrowed.as_ref() else {
            return (420, 1.0, 1);
        };
        (
            host.width as i32,
            scale_factor(host.window),
            lane_count(),
        )
    });
    let row = which_lane(whole, scale, lanes, x, y);
    let strip = strip_rect(whole, scale, lanes);
    let on_strip = x >= strip.left && x <= strip.right && y >= strip.top && y <= strip.bottom;

    // Where the pointer stands, said in the media's own pixels.
    let (point, on_media) = STATE.with(|slot| {
        let state = slot.borrow();
        (
            to_media((x as f64, y as f64), state.media_box, state.fit),
            under(state.media_box, GRACE, (x as f64, y as f64)),
        )
    });

    if pressed {
        // A row: the stretch of it is the business at hand.
        if let Some(index) = row {
            unsafe { lane_pressed(window, index, x, whole, scale) };
        } else if on_strip {
            unsafe { seek_pressed(window, x, whole, scale) };
        } else if on_media {
            unsafe { stage_pressed(window, point) };
        }
        unsafe {
            stateless_refresh(window);
        }
        return;
    }

    // A move carries a drag along, or only aims the cursor.
    let lane_busy = STATE.with(|slot| slot.borrow().lane_drag) != LaneDrag::Nothing;
    let stage_busy = STATE.with(|slot| slot.borrow().dragging) != Drag::Nothing;
    if lane_busy {
        lane_moved(x, whole, scale);
        unsafe { stateless_refresh(window) };
        if STATE.with(|slot| slot.borrow().lane_drag) == LaneDrag::Seek {
            refresh_frame(false);
            refresh_times();
        }
        unsafe {
            let _ = InvalidateRect(Some(window), None, true);
        }
    } else if stage_busy {
        stage_moved(point);
        unsafe { stand_all(window) };
    }

    aim_pointer(on_media, row, point, x, whole, scale);

    // The card still sees the pointer, for the hover of its buttons.
    let event = WindowEvent::PointerMoved {
        position: LogicalPosition::new(x as f32, y as f32),
    };
    let told = HOST.with(|slot| {
        let borrowed = slot.borrow();
        borrowed
            .as_ref()
            .map(|host| (host.software.clone(), host.window))
    });
    if let Some((software, own)) = told {
        if own == window {
            software.window().dispatch_event(event);
        }
    }
}

/// Says the time and where the video stands, without the host being
/// taken — a press on the strip or the transport happens while a
/// drawing may still be holding it.
unsafe fn stateless_refresh(window: HWND) {
    refresh_times();
    let _ = unsafe { InvalidateRect(Some(window), None, true) };
}

/// Puts the cursor in the shape the pointer's own news asks for: over
/// the media a cover's business, over a row a stretch's business, and
/// over the card's own buttons the arrow answers for itself.
fn aim_pointer(
    on_media: bool,
    row: Option<usize>,
    point: (f64, f64),
    x: i32,
    whole: i32,
    scale: f32,
) {
    let pick = if on_media {
        STATE.with(|slot| {
            let state = slot.borrow();
            match state.dragging {
                Drag::Nothing => Some(picked_at(point, &state.rects, state.selected, state.fit)),
                Drag::Moving { .. } => Some(Pick::Track(0)),
                Drag::Resizing { .. } => Some(Pick::Corner(state.grab)),
                Drag::Drawing { .. } => Some(Pick::Create),
            }
        })
    } else {
        None
    };
    if let Some(pick) = pick {
        unsafe {
            let cursor = LoadCursorW(None, cursor_for(pick)).unwrap_or_default();
            let _ = SetCursor(Some(cursor));
        }
        return;
    }
    if let Some(index) = row {
        let width = track_width(whole, scale);
        let bouncing = STATE.with(|slot| {
            let state = slot.borrow();
            let press = track_secs(x, whole, scale, 1.0) * state.duration;
            match state.lane_drag {
                LaneDrag::Nothing => state.rects.get(index).map(|one| {
                    lane_pick(&one.window, press, CAP_REACH / width)
                }),
                LaneDrag::Edge { .. } => Some(LanePick::StartCap),
                LaneDrag::New { .. } => Some(LanePick::EmptyTrack),
                LaneDrag::Move { .. } => Some(LanePick::Inside),
                LaneDrag::Seek => None,
            }
        });
        if let Some(pick) = bouncing {
            unsafe {
                let cursor = LoadCursorW(None, lane_cursor_for(pick)).unwrap_or_default();
                let _ = SetCursor(Some(cursor));
            }
            return;
        }
    }
}

/// Lets a drag go: a drawing too small to mean anything is dropped, a
/// stretch smaller than its least is dropped with it, a drawn cover
/// stands chosen, and everything else keeps what it drew.
unsafe fn released(window: HWND, x: i32, y: i32) {
    let lane_was = STATE.with(|slot| {
        let mut state = slot.borrow_mut();
        let lane_was = state.lane_drag;
        state.lane_drag = LaneDrag::Nothing;
        lane_was
    });
    if lane_was != LaneDrag::Nothing {
        STATE.with(|slot| {
            let mut state = slot.borrow_mut();
            match lane_was {
                LaneDrag::New { lane, .. }
                | LaneDrag::Move { lane, .. }
                | LaneDrag::Edge { lane, .. } => {
                    let sitting = state.rects.get(lane).and_then(|one| one.window);
                    if let Some(stretch) = sitting {
                        if window_too_small(stretch) && lane < state.rects.len() {
                            state.rects[lane].window = None;
                        }
                    }
                }
                _ => {}
            }
        });
    } else {
        STATE.with(|slot| {
            let mut state = slot.borrow_mut();
            let dragging = state.dragging;
            state.dragging = Drag::Nothing;
            match dragging {
                Drag::Drawing { index, .. } => {
                    let cut = state.rects.get(index).map(|one| one.cut);
                    match cut {
                        Some(cut) if is_too_small(cut) => {
                            state.rects.remove(index);
                            state.selected = None;
                        }
                        Some(_) => {
                            state.selected = Some(index);
                        }
                        None => {}
                    }
                }
                _ => {}
            }
        });
    }
    unsafe { stand_all(window) };

    // The card still sees the pointer: the button under it hears of
    // the letting go too.
    let told = HOST.with(|slot| {
        let borrowed = slot.borrow();
        borrowed
            .as_ref()
            .map(|host| (host.software.clone(), host.window))
    });
    if let Some((software, own)) = told {
        if own == window {
            software.window().dispatch_event(WindowEvent::PointerReleased {
                position: LogicalPosition::new(x as f32, y as f32),
                button: PointerEventButton::Left,
            });
        }
    }
}

/// The window's own messages, read and answered here, the way the
/// wheel's window answers its own.
unsafe extern "system" fn window_procedure(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    unsafe {
        match message {
            WM_PAINT => {
                let mut paint = Default::default();
                let _ = BeginPaint(window, &mut paint);
                draw_window(window);
                let _ = EndPaint(window, &paint);
                LRESULT(0)
            }
            WM_ERASEBKGND => LRESULT(1),
            WM_LBUTTONDOWN => {
                let _ = SetCapture(window);
                touched(window, x_of(lparam), y_of(lparam), true);
                LRESULT(0)
            }
            WM_LBUTTONUP => {
                let _ = ReleaseCapture();
                released(window, x_of(lparam), y_of(lparam));
                LRESULT(0)
            }
            WM_MOUSEMOVE => {
                touched(window, x_of(lparam), y_of(lparam), false);
                LRESULT(0)
            }
            WM_KEYDOWN => {
                let key = wparam.0 as u16;
                if key == VK_TAB.0 {
                    let forward =
                        (GetKeyState(VK_SHIFT.0 as i32) as u16 & 0x8000) == 0;
                    cycle_focus(window, forward);
                } else if key == VK_RETURN.0 {
                    run(window);
                } else if key == VK_ESCAPE.0 {
                    cancel(window);
                } else if key == VK_DELETE.0 || key == VK_BACK.0 {
                    remove_selected(window);
                }
                LRESULT(0)
            }
            WM_TIMER => {
                if wparam.0 as usize == FADE_TIMER_ID {
                    let alpha = FADE_ALPHA.with(|slot| slot.get().saturating_add(51));
                    FADE_ALPHA.with(|slot| slot.set(alpha));
                    let _ = SetLayeredWindowAttributes(
                        window,
                        COLORREF(0),
                        alpha,
                        LWA_ALPHA,
                    );
                    if alpha == 255 {
                        let _ = KillTimer(Some(window), FADE_TIMER_ID);
                    }
                } else if wparam.0 as usize == STRIP_TIMER_ID {
                    // The strip's moments come one by one from the
                    // worker elsewhere; when all of them have, the
                    // asking stops.
                    match strip_tick() {
                        Some(piece) => {
                            let _ = KillTimer(Some(window), STRIP_TIMER_ID);
                            wear_strip(piece);
                        }
                        None => {}
                    }
                } else if wparam.0 as usize == PLAY_TIMER_ID {
                    STATE.with(|slot| {
                        let mut state = slot.borrow_mut();
                        if state.playing {
                            state.playhead = (state.playhead + PLAY_STEP).min(state.duration);
                            if state.playhead >= state.duration {
                                state.playing = false;
                            }
                        }
                    });
                    let ran_on = STATE.with(|slot| slot.borrow().playing);
                    if !ran_on {
                        let _ = KillTimer(Some(window), PLAY_TIMER_ID);
                    }
                    refresh_frame(false);
                    refresh_times();
                    let _ = InvalidateRect(Some(window), None, true);
                }
                LRESULT(0)
            }
            WM_SIZE => {
                HOST.with(|slot| {
                    let mut borrowed = slot.borrow_mut();
                    if let Some(host) = borrowed.as_mut() {
                        if host.window == window {
                            host.resize();
                        }
                    }
                });
                LRESULT(0)
            }
            WM_DPICHANGED => {
                // The window is put where the system suggests, at the
                // size it suggests, and the buffer is remade for the
                // new stretch.
                if lparam.0 != 0 {
                    let rect = &*(lparam.0 as *const RECT);
                    let _ = SetWindowPos(
                        window,
                        None,
                        rect.left,
                        rect.top,
                        rect.right - rect.left,
                        rect.bottom - rect.top,
                        SWP_NOZORDER | SWP_NOACTIVATE,
                    );
                }
                HOST.with(|slot| {
                    let mut borrowed = slot.borrow_mut();
                    if let Some(host) = borrowed.as_mut() {
                        if host.window == window {
                            host.resize();
                            host.draw();
                        }
                    }
                });
                LRESULT(0)
            }
            WM_CLOSE => {
                cancel(window);
                LRESULT(0)
            }
            WM_DESTROY => {
                let _ = KillTimer(Some(window), FADE_TIMER_ID);
                let _ = KillTimer(Some(window), STRIP_TIMER_ID);
                let _ = KillTimer(Some(window), PLAY_TIMER_ID);
                DefWindowProcW(window, message, wparam, lparam)
            }
            _ => DefWindowProcW(window, message, wparam, lparam),
        }
    }
}

/// The sideways of a window message's own point.
fn x_of(lparam: LPARAM) -> i32 {
    (lparam.0 & 0xFFFF) as u16 as i16 as i32
}

/// The downwards of a window message's own point.
fn y_of(lparam: LPARAM) -> i32 {
    ((lparam.0 >> 16) & 0xFFFF) as u16 as i16 as i32
}

#[cfg(test)]
fn get_fade_done() -> bool {
    FADE_ALPHA.with(|slot| slot.get() == 255)
}

#[cfg(test)]
fn get_strip_worn() -> bool {
    STRIP_WORN.get()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One cover said where it stands, without a stretch of its own —
    /// what most of the tests ask for.
    fn box_at(cut: Box4) -> RedactBox {
        RedactBox {
            cut,
            window: None,
        }
    }

    #[test]
    fn the_height_of_a_cover_window_is_its_bands_and_its_stage() {
        // A window with a row for the one cover it opens holding.
        assert_eq!(Layout::height(1), 644.0);
        // One row more or less moves the bands under the Redactions by
        // one step of a row.
        assert_eq!(Layout::height(3), 708.0);
        assert_eq!(Layout::height(0), 612.0);
    }

    #[test]
    fn the_layout_stacks_the_body_top_to_bottom() {
        let layout = Layout::of(420, 644, 1.0, 1);

        // The stage stands between the two hairlines, with a room
        // under the header's own one.
        assert_eq!(layout.hairline_top.top, 62);
        assert_eq!(layout.stage.top, 82);
        assert_eq!(layout.stage.left, 24);
        assert_eq!(layout.stage.right, 396);
        assert_eq!(layout.stage.bottom, 318);

        // The two rows of the look, one after another below the stage.
        assert_eq!(layout.solid.top, 336);
        assert_eq!(layout.solid.bottom, 362);
        assert_eq!(layout.well.top, 376);
        assert_eq!(layout.hairline_mid.top, 424);

        // The words of the Redactions, and the one row of its list.
        assert_eq!(layout.redactions_label.top, 434);
        assert_eq!(layout.lanes.top, 454);
        assert_eq!(layout.lanes.right, 396);
        assert_eq!(layout.lanes.bottom, 454 + 32 - 6);

        // The strip under the rows, and the transport under that.
        assert_eq!(layout.strip.top, 488);
        assert_eq!(layout.strip.bottom, 528);
        assert_eq!(layout.skip_back.left, 44);
        assert_eq!(layout.play.right - layout.play.left, 30);
        assert_eq!(layout.skip_forward.left, 112);
        assert_eq!(layout.time_text.right, 396);
        assert_eq!(layout.hairline_bottom.top, 580);

        // The footer: measured from the bottom, the making of the copy
        // at the right.
        assert_eq!(layout.run.left, 228);
        assert_eq!(layout.run.right - layout.run.left, 168);
        assert_eq!(layout.run.top, 594);
        assert_eq!(layout.run.bottom, 628);
    }

    #[test]
    fn a_row_more_or_less_moves_everything_under_it() {
        // With three covers the rows take three steps, and the strip
        // stands that much lower.
        let one = Layout::of(420, 644, 1.0, 1);
        let three = Layout::of(420, 708, 1.0, 3);
        assert_eq!(three.strip.top - one.strip.top, 64);
        assert_eq!(three.hairline_bottom.top - one.hairline_bottom.top, 64);
        // The stage and the rows of the look stand where they were.
        assert_eq!(three.stage.top, one.stage.top);
        assert_eq!(three.well.top, one.well.top);
    }

    #[test]
    fn a_row_says_its_own_track_whatever_the_window_says() {
        // The rows' places, said for hit-testing, match what the
        // layout lays out: the first row's track under the words.
        let row = lane_row(420, 1.0, 0);
        assert_eq!(row.left, 44);
        assert_eq!(row.right, 396);
        assert_eq!(row.top, 454);
        assert_eq!(row.bottom - row.top, 26);
        // The second row stands one step lower.
        let second = lane_row(420, 1.0, 1);
        assert_eq!(second.top, 454 + 32);
        // The air between the rows belongs to nobody.
        assert_eq!(which_lane(420, 1.0, 2, 100, 482), None);
        assert_eq!(which_lane(420, 1.0, 2, 100, 470), Some(0));
        assert_eq!(which_lane(420, 1.0, 2, 100, 502), Some(1));
        // The strip under the rows, for a press to seek on.
        let strip = strip_rect(420, 1.0, 1);
        assert_eq!(strip.top, 488);
        assert_eq!(strip.bottom, 528);
        assert_eq!(strip.left, 44);
    }

    #[test]
    fn the_media_stands_as_large_as_the_stage_allows_and_no_larger() {
        // A landscape video leads by its height here.
        assert_eq!(fit_of(372.0, 236.0, 320.0, 240.0), 236.0 / 240.0);
        // A portrait one leads by its width here: the shorter squeeze.
        assert_eq!(fit_of(372.0, 236.0, 720.0, 400.0), 372.0 / 720.0);
        // Nothing to say: nothing changes.
        assert_eq!(fit_of(0.0, 0.0, 100.0, 100.0), 1.0);
    }

    #[test]
    fn a_cover_starts_a_tenth_in_from_every_edge() {
        assert_eq!(inset_cover(720.0, 1280.0), (72.0, 128.0, 576.0, 1024.0));
        assert_eq!(inset_cover(1600.0, 900.0), (160.0, 90.0, 1280.0, 720.0));
    }

    #[test]
    fn a_press_takes_a_handle_a_pill_or_the_face_of_a_cover() {
        let rects = [box_at((0.0, 0.0, 100.0, 100.0))];
        // The four corners, whichever of them the press is near.
        assert_eq!(
            picked_at((3.0, 3.0), &rects, Some(0), 1.0),
            Pick::Corner(Spot::TopLeft)
        );
        assert_eq!(
            picked_at((97.0, 3.0), &rects, Some(0), 1.0),
            Pick::Corner(Spot::TopRight)
        );
        assert_eq!(
            picked_at((97.0, 97.0), &rects, Some(0), 1.0),
            Pick::Corner(Spot::BottomRight)
        );
        assert_eq!(
            picked_at((3.0, 97.0), &rects, Some(0), 1.0),
            Pick::Corner(Spot::BottomLeft)
        );
        // The pills on the edges, at the middle of each side, on a
        // cover big enough to wear them.
        assert_eq!(picked_at((50.0, 3.0), &rects, Some(0), 1.0), Pick::Corner(Spot::Top));
        assert_eq!(picked_at((50.0, 97.0), &rects, Some(0), 1.0), Pick::Corner(Spot::Bottom));
        assert_eq!(picked_at((3.0, 50.0), &rects, Some(0), 1.0), Pick::Corner(Spot::Left));
        assert_eq!(picked_at((97.0, 50.0), &rects, Some(0), 1.0), Pick::Corner(Spot::Right));
        // Deep inside: the whole cover comes along.
        assert_eq!(picked_at((50.0, 50.0), &rects, Some(0), 1.0), Pick::Track(0));
        // Wholly away from it: a new cover is drawn.
        assert_eq!(picked_at((150.0, 50.0), &rects, Some(0), 1.0), Pick::Create);

        // A cover too small to wear the pills takes no pill press: its
        // face answers away from the corners, and a press near a bare
        // edge is a new drawing.
        let small = [box_at((0.0, 0.0, 30.0, 30.0))];
        assert_eq!(picked_at((15.0, 15.0), &small, Some(0), 1.0), Pick::Track(0));
        assert_eq!(picked_at((15.0, 3.0), &small, Some(0), 1.0), Pick::Create);

        // Nothing standing at all: whatever is pressed, a new cover is
        // drawn.
        assert_eq!(picked_at((50.0, 50.0), &[], None, 1.0), Pick::Create);
    }

    #[test]
    fn a_corner_drag_holds_the_one_across() {
        // A corner taken with the opposite one standing still.
        assert_eq!(
            resized(Spot::TopLeft, (100.0, 100.0, 50.0, 50.0), (40.0, 40.0), 1000.0, 1000.0),
            (40.0, 40.0, 110.0, 110.0)
        );
        assert_eq!(
            resized(Spot::BottomRight, (100.0, 100.0, 50.0, 50.0), (200.0, 200.0), 1000.0, 1000.0),
            (100.0, 100.0, 100.0, 100.0)
        );
        assert_eq!(
            resized(Spot::TopRight, (100.0, 100.0, 50.0, 50.0), (200.0, 40.0), 1000.0, 1000.0),
            (100.0, 40.0, 100.0, 110.0)
        );
        assert_eq!(
            resized(Spot::BottomLeft, (100.0, 100.0, 50.0, 50.0), (40.0, 200.0), 1000.0, 1000.0),
            (40.0, 100.0, 110.0, 100.0)
        );
    }

    #[test]
    fn an_edge_pill_pulls_one_side_only() {
        let anchored = (100.0, 100.0, 200.0, 100.0);
        assert_eq!(
            resized(Spot::Top, anchored, (150.0, 50.0), 1000.0, 1000.0),
            (100.0, 50.0, 200.0, 150.0)
        );
        assert_eq!(
            resized(Spot::Bottom, anchored, (150.0, 250.0), 1000.0, 1000.0),
            (100.0, 100.0, 200.0, 150.0)
        );
        assert_eq!(
            resized(Spot::Left, anchored, (50.0, 150.0), 1000.0, 1000.0),
            (50.0, 100.0, 250.0, 100.0)
        );
        assert_eq!(
            resized(Spot::Right, anchored, (350.0, 150.0), 1000.0, 1000.0),
            (100.0, 100.0, 250.0, 100.0)
        );
    }

    #[test]
    fn a_cover_drawn_between_two_places_stands_there() {
        assert_eq!(
            drawn((100.0, 100.0), (40.0, 160.0), 1000.0, 1000.0),
            (40.0, 100.0, 60.0, 60.0)
        );
        assert_eq!(
            drawn((100.0, 100.0), (180.0, 40.0), 1000.0, 1000.0),
            (100.0, 40.0, 80.0, 60.0)
        );
        // A drag past the media is pulled back to it.
        assert_eq!(
            drawn((100.0, 100.0), (1200.0, 50.0), 1600.0, 1200.0),
            (100.0, 50.0, 1100.0, 50.0)
        );
    }

    #[test]
    fn a_cover_carried_off_the_media_stays_on_it() {
        assert_eq!(
            placed((0.0, 0.0, 32.0, 32.0), (0.0, 0.0), (500.0, 0.0), 160.0, 120.0),
            (128.0, 0.0, 32.0, 32.0)
        );
        assert_eq!(
            placed((0.0, 0.0, 32.0, 32.0), (0.0, 0.0), (-5.0, -9.0), 160.0, 120.0),
            (0.0, 0.0, 32.0, 32.0)
        );
        assert_eq!(
            placed((64.0, 44.0, 32.0, 32.0), (64.0, 44.0), (200.0, 150.0), 160.0, 120.0),
            (128.0, 88.0, 32.0, 32.0)
        );
    }

    #[test]
    fn a_remark_of_size_drops_a_misdrawn_cover() {
        assert!(is_too_small((0.0, 0.0, 7.0, 100.0)));
        assert!(is_too_small((0.0, 0.0, 100.0, 7.0)));
        assert!(!is_too_small((0.0, 0.0, 8.0, 8.0)));
        assert!(!is_too_small((0.0, 0.0, 8.0, 100.0)));
    }

    #[test]
    fn a_press_on_a_track_takes_a_cap_a_slide_or_a_fresh_stretch() {
        let some = Some((3.0, 7.0));
        // A track with nothing on it: whatever is pressed, a stretch is
        // drawn.
        assert_eq!(lane_pick(&None, 2.0, 0.2), LanePick::EmptyTrack);
        // The cap at each end takes a press beside it.
        assert_eq!(lane_pick(&some, 3.1, 0.2), LanePick::StartCap);
        assert_eq!(lane_pick(&some, 6.9, 0.2), LanePick::EndCap);
        // Between the caps: the whole stretch comes along.
        assert_eq!(lane_pick(&some, 5.0, 0.2), LanePick::Inside);
        // Away from it on a track that has one: nothing taken.
        assert_eq!(lane_pick(&some, 9.0, 0.2), LanePick::Apart);
    }

    #[test]
    fn a_stretch_drawn_between_two_places_stands_there() {
        assert_eq!(stretched_window(4.0, 2.0), (2.0, 4.0));
        assert_eq!(stretched_window(1.0, 9.0), (1.0, 9.0));
    }

    #[test]
    fn a_stretch_carried_along_stays_in_the_video() {
        // A carry-along past the end stops at it, keeping its span.
        assert_eq!(moved_window((3.0, 7.0), 1.0, 20.0, 10.0), (6.0, 10.0));
        // A carry-along past the start stops at 0.
        assert_eq!(moved_window((3.0, 7.0), 0.0, -2.0, 10.0), (0.0, 4.0));
        assert_eq!(moved_window((3.0, 7.0), 2.0, 5.0, 10.0), (3.0, 7.0));
    }

    #[test]
    fn a_cap_pulled_holds_the_other_away_from_the_least() {
        // The start cap: the end keeps its word.
        assert_eq!(edged_window((3.0, 7.0), true, 1.0, 10.0), (1.0, 7.0));
        // A start cap pulled past the least a stretch may be stops
        // there.
        assert_eq!(
            edged_window((3.0, 7.0), true, 9.5, 10.0).0,
            6.75
        );
        // The end cap: the start keeps its word, even past the video's
        // end.
        assert_eq!(edged_window((3.0, 7.0), false, 9.9, 10.0), (3.0, 9.9));
        assert_eq!(edged_window((3.0, 7.0), false, 2.0, 10.0).1, 3.25);
    }

    #[test]
    fn a_stretch_smaller_than_its_least_drops() {
        assert!(window_too_small((3.0, 3.2)));
        assert!(!window_too_small((3.0, 3.5)));
        assert!(!window_too_small((3.0, 4.0)));
    }

    #[test]
    fn a_stretch_is_said_in_thousandths_of_a_second() {
        assert_eq!(windows_of(Some((0.25, 1.0))), (Some(250), Some(1000)));
        assert_eq!(windows_of(None), (None, None));
        // Rounding, half away from zero.
        assert_eq!(windows_of(Some((0.1255, 2.1234))), (Some(126), Some(2123)));
        // A stretch before the video is kept at the start.
        assert_eq!(windows_of(Some((-0.5, 2.0))), (Some(0), Some(2000)));
    }

    #[test]
    fn a_cover_is_said_in_thousandths_of_the_media() {
        let covered = RedactBox {
            cut: (0.0, 0.0, 800.0, 600.0),
            window: Some((0.0, 2.0)),
        };
        assert_eq!(
            areas_of(&[covered], 1600.0, 1200.0),
            vec![RedactArea {
                x_mille: 0,
                y_mille: 0,
                w_mille: 500,
                h_mille: 500,
                start_ms: Some(0),
                end_ms: Some(2000),
            }]
        );
        // A cover with no stretch stands for the whole video.
        assert_eq!(
            areas_of(&[box_at((8.0, 8.0, 792.0, 592.0))], 1600.0, 1200.0)[0].start_ms,
            None
        );
        // A share past the end is kept at the end, not past it.
        assert_eq!(mille(2000.0, 100.0), 1000);
        assert_eq!(mille(-1.0, 100.0), 0);
    }

    #[test]
    fn the_look_says_what_the_file_is_covered_by() {
        // Solid, and the well's own black.
        let solid = settings_from(
            0,
            0,
            &[box_at((0.0, 0.0, 1600.0, 1200.0))],
            1600.0,
            1200.0,
        );
        assert_eq!(solid.redact_style, Some(RedactStyle::Solid));
        assert_eq!(solid.redact_color, Some(String::from("000000")));
        assert_eq!(solid.redact_areas[0].w_mille, 1000);
        assert_eq!(solid.redact_areas[0].h_mille, 1000);
        assert_eq!(solid.redact_areas[0].start_ms, None);
        // A smear, then, and the third colour of the wheel, over a
        // stretch of its own.
        let stretch = RedactBox {
            cut: (8.0, 8.0, 792.0, 592.0),
            window: Some((1.5, 2.25)),
        };
        let blur = settings_from(1, 2, &[stretch], 1600.0, 1200.0);
        assert_eq!(blur.redact_style, Some(RedactStyle::Blur));
        assert_eq!(blur.redact_color, Some(String::from("808080")));
        assert_eq!(blur.redact_areas[0].start_ms, Some(1500));
        assert_eq!(blur.redact_areas[0].end_ms, Some(2250));
    }

    #[test]
    fn the_well_turns_round_the_wheel() {
        assert_eq!(color_hex(0), "000000");
        assert_eq!(color_hex(2), "808080");
        assert_eq!(color_hex(5), "3B82F6");
        // The wheel turns round: past the last colour, the first.
        assert_eq!(color_hex(6), "000000");
        assert_eq!(palette_parts(1), (255, 255, 255));
        assert_eq!(palette_parts(4), (0x2E, 0xCC, 0x71));
    }
}

#[cfg(test)]
mod show_tests {
    use super::*;
    use crate::platform::windows::windowhost::screenshot::make_test_video;
    use crate::platform::windows::windowhost::screenshot::capture;
    use windows::Win32::System::SystemServices::MK_LBUTTON;
    use windows::Win32::UI::WindowsAndMessaging::{
        DispatchMessageW, MSG, PeekMessageW, PM_REMOVE, SendMessageW, TranslateMessage,
    };

    /// Lays every message of the window down for a while, the way the
    /// coming-in of a window asks for.
    fn pump(rounds: u32) {
        for _ in 0..rounds {
            unsafe {
                let mut message = MSG::default();
                while PeekMessageW(&mut message, None, 0, 0, PM_REMOVE).as_bool() {
                    let _ = TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    /// A window message's own point, packed the way the system packs
    /// it: the two halves of the number.
    fn pack(x: i32, y: i32) -> LPARAM {
        LPARAM(((y as u16 as isize) << 16) | (x as u16 as isize))
    }

    #[test]
    fn showing_and_pumping_the_redact_video_card_does_not_panic() {
        crate::ui::wheel::install_platform();
        let (sender, _receiver) = std::sync::mpsc::channel();
        let mut window = RedactVideoWindow::new(sender).expect("the window was made");
        window.show_for(
            "tools.redact_video",
            "Redact video",
            "Cover the parts a person does not show",
            &[PathBuf::from("not-there.mp4")],
        );
        pump(40);
        window.hide();
    }

    #[test]
    fn a_style_chip_answers_the_pointer_without_the_host_being_taken() {
        crate::ui::wheel::install_platform();
        let (sender, _receiver) = std::sync::mpsc::channel();
        let mut window = RedactVideoWindow::new(sender).expect("the window was made");
        window.show_for(
            "tools.redact_video",
            "Redact video",
            "Cover the parts a person does not show",
            &[PathBuf::from("not-there.mp4")],
        );
        pump(10);
        // Where the Blur chip stands, said in the card's own units.
        let told = HOST.with(|slot| {
            let borrowed = slot.borrow();
            let Some(host) = borrowed.as_ref() else {
                return None;
            };
            let scale = scale_factor(host.window);
            let layout = Layout::of(host.width as i32, host.height as i32, scale, lane_count());
            let middle = (layout.blur.left + layout.blur.right) as f32 / 2.0 / scale;
            let middle_y = (layout.blur.top + layout.blur.bottom) as f32 / 2.0 / scale;
            Some((host.software.clone(), middle, middle_y))
        });
        let Some((software, x, y)) = told else {
            panic!("the card was not made");
        };
        // A press and a let-go on the chip, the way a person takes the
        // mouse.
        for event in [
            WindowEvent::PointerPressed {
                position: LogicalPosition::new(x, y),
                button: PointerEventButton::Left,
            },
            WindowEvent::PointerReleased {
                position: LogicalPosition::new(x, y),
                button: PointerEventButton::Left,
            },
        ] {
            software.window().dispatch_event(event);
            pump(1);
        }
        // The look was taken: a smear.
        let taken = STATE.with(|slot| slot.borrow().style);
        assert_eq!(taken, 1);
        window.hide();
    }

    #[test]
    fn a_cover_drawn_through_the_window_messages_stands_in_the_state() {
        crate::ui::wheel::install_platform();
        let (sender, _receiver) = std::sync::mpsc::channel();
        let mut window = RedactVideoWindow::new(sender).expect("the window was made");
        window.show_for(
            "tools.redact_video",
            "Redact video",
            "Cover the parts a person does not show",
            &[PathBuf::from("not-there.mp4")],
        );
        pump(10);
        // Where the seeded cover's own top-left hand stands, in window
        // pixels — on a fallback frame this is the only hand far
        // enough from its brothers for a press to take it — and a
        // place beyond it to drag the hand to, along the window's own
        // road.
        let said = STATE.with(|slot| {
            let state = slot.borrow();
            let at_media = |point: (f64, f64)| -> (i32, i32) {
                (
                    (state.media_box.0 + point.0 * state.fit) as i32,
                    (state.media_box.1 + point.1 * state.fit) as i32,
                )
            };
            let corners = corners_of(state.rects[0].cut);
            let was = state.rects[0].cut;
            Some((at_media(corners[0]), was))
        });
        let Some((press, was)) = said else {
            panic!("the cover never stood");
        };
        // A press on the top-left hand, a carry beyond it, and a
        // let-go — the window's own road.
        unsafe {
            let _ = SendMessageW(
                window.window,
                WM_LBUTTONDOWN,
                Some(WPARAM(MK_LBUTTON.0 as usize)),
                Some(pack(press.0, press.1)),
            );
            let _ = SendMessageW(
                window.window,
                WM_MOUSEMOVE,
                Some(WPARAM(0)),
                Some(pack(24, 82)),
            );
            let _ = SendMessageW(
                window.window,
                WM_LBUTTONUP,
                Some(WPARAM(0)),
                Some(pack(24, 82)),
            );
        }
        pump(5);
        let (count, cut, chosen) = STATE.with(|slot| {
            let state = slot.borrow();
            (
                state.rects.len(),
                state.rects.get(0).map(|one| one.cut),
                state.selected,
            )
        });
        assert_eq!(count, 1, "the seeded cover stayed the only one");
        assert_eq!(chosen, Some(0), "the chosen cover stayed chosen");
        let Some(cut) = cut else {
            panic!("the cover never stood");
        };
        assert!(
            cut.0 < was.0 && cut.1 < was.1 && cut.3 > was.3,
            "the top-left hand was dragged out: {:?} from {:?}",
            cut,
            was
        );
        window.hide();
    }

    /// Opens the cover window on a real video, lets it come in and
    /// draw, and captures what a person would see.
    #[test]
    #[ignore = "needs the conversion program; writes a screenshot"]
    fn capture_the_redact_video_window() {
        crate::ui::wheel::install_platform();
        let folder = std::env::temp_dir().join("open-tangerine-capture");
        std::fs::create_dir_all(&folder).expect("the capture folder was made");
        let video = make_test_video(&folder);
        let (sender, _receiver) = std::sync::mpsc::channel();
        let mut window = RedactVideoWindow::new(sender).expect("the window was made");
        window.show_for(
            "tools.redact_video",
            "Redact video",
            "Cover the parts a person does not show",
            &[video],
        );
        // Long enough for the fade to finish, the first frame to land
        // and the strip's own moments to be asked for one after
        // another: the strip is a run of seeks in the background, and
        // the shot wants it there. The window's own bell serves the
        // strip while the messages are pumped; the shot waits only for
        // a strip worn, and gives the whole thing up after a while so
        // a window without the conversion program still takes its
        // picture.
        for round in 0..2400 {
            unsafe {
                let mut message = MSG::default();
                while PeekMessageW(&mut message, None, 0, 0, PM_REMOVE).as_bool() {
                    let _ = TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
            if get_strip_worn() && get_fade_done() && round > 400 {
                break;
            }
        }
        let shots = std::path::Path::new("target").join("shots");
        std::fs::create_dir_all(&shots).expect("the shots folder was made");
        // The buffer itself, before any window machinery touches it:
        // if the footer is here, the drawing is right and the capture
        // lies; if not, the renderer is the one clipping.
        HOST.with(|slot| {
            let borrowed = slot.borrow();
            if let Some(host) = borrowed.as_ref() {
                let mut rgba = Vec::with_capacity(host.buffer.len() * 4);
                for pixel in &host.buffer {
                    let (_, r, g, b) = pixel.parts();
                    rgba.extend_from_slice(&[r, g, b, 255]);
                }
                let _ = image::save_buffer(
                    shots.join("redact-video-buffer.png"),
                    &rgba,
                    host.width,
                    host.height,
                    image::ExtendedColorType::Rgba8,
                );
            }
        });
        capture(window.window, &shots.join("redact-video-window.png"));
        window.hide();
        let _ = std::fs::remove_dir_all(&folder);
    }
}





