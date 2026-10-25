//! The window a video is trimmed in.
//!
//! The trim does not run the moment it is picked: it opens a window with
//! the video's own timeline in it — the frame at the playhead, the track
//! with the chosen stretch framed in orange and a white pill at each end,
//! the moments said in the face where every digit takes the same room,
//! and the transport as bare glyphs. The person moves the handles with
//! the pointer or the keyboard and says go or no.
//!
//! The card is drawn by the same renderer that draws the wheel, and the
//! skeleton it wears is the shell's — the same header, hairlines and
//! footer every tool window wears. The timeline is only drawn by the
//! card: the pressing, dragging and stepping are read by the window
//! itself, which holds the same geometry the card was given, the way the
//! wheel works. Each frame the playhead stands on is asked of the
//! conversion program with a fast seek, the moment it is asked for.
//!
//! The window is sized once, from what it holds: its width is fixed, its
//! height follows the shape of the video, and it cannot be stretched —
//! the way the reference product's windows are. When the display under
//! it changes its stretch, the window is made again at the new size.
//!
//! What the window is asked comes back as a message, the same message
//! the plain tool window sends: the window never runs anything itself.

use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc::Sender;

use slint::platform::software_renderer::MinimalSoftwareWindow;
use slint::platform::{PointerEventButton, WindowAdapter, WindowEvent};
use slint::{LogicalPosition, PhysicalSize, SharedPixelBuffer};
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{BeginPaint, CreateSolidBrush, InvalidateRect};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::AdjustWindowRectExForDpi;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, ReleaseCapture, SetCapture, SetFocus, VK_DELETE, VK_DOWN, VK_ESCAPE, VK_LEFT,
    VK_RETURN, VK_RIGHT, VK_SHIFT, VK_SPACE, VK_TAB, VK_UP,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, GetClientRect, GetWindowRect, KillTimer, LoadCursorW,
    RegisterClassW, SetForegroundWindow, SetLayeredWindowAttributes, SetTimer, SetWindowPos,
    SetWindowTextW, ShowWindow, SW_HIDE, SW_SHOW, SWP_NOACTIVATE, SWP_NOZORDER, CW_USEDEFAULT,
    IDC_ARROW, LWA_ALPHA, WNDCLASSW, WM_CLOSE, WM_DESTROY, WM_DPICHANGED, WM_ERASEBKGND,
    WM_KEYDOWN, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_PAINT, WM_SIZE, WM_TIMER,
    WS_EX_LAYERED,
};

use crate::media::frames::{duration_of, frame_at, pillar_of};
use crate::media::locator;
use crate::platform::windows::toolwindow::ToolEvent;
use crate::platform::windows::windowhost::{
    fixed_style, fraction, scale_factor, screen_scale, timecode, to_wide,
};
use crate::radial::execution::ToolSettings;
use crate::ui::wheel::{window_for_next_component, Bgra8, Panel, TrimCard};

/// The timer the window comes in softly on.
const FADE_TIMER_ID: usize = 52;

/// The timer the strip is waited on with.
const STRIP_TIMER_ID: usize = 54;

/// The card's width, in the drawing's own units.
const CARD_W: i32 = 420;
/// The least and the most a preview may be tall, so a landscape video
/// keeps its shape and a portrait one does not tower.
const PREVIEW_MIN: f32 = 190.0;
const PREVIEW_MAX: f32 = 300.0;

/// The strip the range is worn on: tall enough that the video's own
/// moments read inside it, and cut into pillars ~16 units wide.
const STRIP_UNITS: f32 = 72.0;
const PILLAR_UNITS: f32 = 16.0;

/// The least a trim may keep, in seconds. Less than this and the video
/// is not being trimmed, it is being cut to a blink.
const MIN_SPAN: f64 = 0.5;

/// How close a press must be to take an end cap instead of the playhead,
/// in pixels of the drawing. A cap is 28 wide, so half of it plus a
/// little grace.
const HANDLE_REACH: i32 = 16;

/// How close a press must be to take a split marker instead of the
/// playhead, in pixels of the drawing. A marker is 2 wide, but a needle
/// nobody can take is no good.
const MARKER_REACH: i32 = 8;

/// The split's own rules, which the original's source carries: a point
/// stands inside the video but not against its edge, no two points stand
/// nearer than this, the count of equal parts turns between 2 and 50,
/// and the Delete key takes a point no farther than this from the
/// playhead.
const POINT_MARGIN: f64 = 0.1;
const MIN_POINT_GAP: f64 = 0.05;
const PARTS_MIN: i32 = 2;
const PARTS_MAX: i32 = 50;
const DELETE_REACH: f64 = 0.5;

/// How far the playhead must move before a new frame is asked of the
/// conversion program, in seconds. A seek costs a moment; the frame does
/// not need to follow the playhead any closer than a person can see.
const FRAME_STEP: f64 = 0.25;

/// What the pointer is holding.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Drag {
    Nothing,
    Start,
    End,
    Playhead,
}

/// What the window is open for, and where the video stands in it. A trim
/// holds the two ends of a stretch; a split holds the points the video is
/// cut at and the count of its equal parts.
struct TrimState {
    duration: f64,
    start: f64,
    end: f64,
    playhead: f64,
    dragging: Drag,
    /// Where the frame showing was taken, so it is only asked for again
    /// when the playhead has truly moved.
    frame_at: f64,
    /// How tall the preview wants to be, in the drawing's own units,
    /// from the shape of the video.
    preview_units: f32,
    ffmpeg: Option<locator::Tools>,
    input: PathBuf,
    /// Whether this window is asked for a split, not a trim: the strip is
    /// worn with markers instead of a dragged range, and the words of the
    /// rows say cuts, not ends.
    split: bool,
    /// Whether this is an audio trim: waveform strip without preview frame,
    /// silence toggle, and play selection button.
    audio: bool,
    /// Whether the automatic silence trimming checkbox is checked.
    trim_silence: bool,
    /// The leading and trailing silence detected by analysis, if any.
    silence: Option<(f64, f64)>,
    /// The points a cut stands at, in seconds, in order. A trim carries
    /// none of these.
    points: Vec<f64>,
    /// The count the equal row says, between its own least and most.
    parts: i32,
    /// What the equal saying said, when the last stand of points came
    /// from the equal row: "each about 0:01.13". A cut added by hand or
    /// taken away leaves, and the saying is gone.
    equal_saying: Option<String>,
}

impl Default for TrimState {
    fn default() -> Self {
        Self {
            duration: 0.0,
            start: 0.0,
            end: 0.0,
            playhead: 0.0,
            dragging: Drag::Nothing,
            frame_at: f64::NEG_INFINITY,
            preview_units: 236.0,
            ffmpeg: None,
            input: PathBuf::new(),
            split: false,
            audio: false,
            trim_silence: false,
            silence: None,
            points: Vec::new(),
            parts: 2,
            equal_saying: None,
        }
    }
}

thread_local! {
    static EVENTS: std::cell::RefCell<Option<Sender<ToolEvent>>> =
        const { std::cell::RefCell::new(None) };
    static OPEN_FOR: std::cell::RefCell<Option<(String, Vec<PathBuf>)>> =
        const { std::cell::RefCell::new(None) };
    static STATE: std::cell::RefCell<TrimState> =
        std::cell::RefCell::new(TrimState::default());
    static HOST: std::cell::RefCell<Option<Host>> = const { std::cell::RefCell::new(None) };
    // The handles the card's own callbacks need, kept apart from the host
    // so a callback fired in the middle of a drawing never has to borrow
    // what is already borrowed. A handle of zero means "not there".
    static WINDOW: std::cell::Cell<isize> = const { std::cell::Cell::new(0) };
    // Where the keyboard is. A trim: 0 the start cap, 1 the end cap,
    // 2 run. A split: 0 the playhead, 1 add-split, 2 set, 3 run.
    static FOCUS_INDEX: std::cell::Cell<i32> = const { std::cell::Cell::new(0) };
    // How far along the soft coming-in is.
    static FADE_ALPHA: std::cell::Cell<u8> = const { std::cell::Cell::new(255) };
    // The strip a background thread is building arrives here: the window
    // is drawn at once, and the strip arrives as it is worked out, one
    // picture at the end. Opening a new trim replaces it, and a strip
    // finished for the last trim can land nowhere.
    static STRIP_INBOX: std::cell::RefCell<Option<std::sync::mpsc::Receiver<StripPiece>>> =
        const { std::cell::RefCell::new(None) };
    // Whether a strip has been worn: the judgement the capture check
    // waits on, answered without touching the strip's own inbox.
    static STRIP_WORN: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Whether a strip has been worn, without touching what still may come.
fn get_strip_worn() -> bool {
    STRIP_WORN.with(|slot| slot.get())
}

/// One finished strip, as plain bytes: pixels, then how wide and how
/// tall, plus optional silence range for audio. Slint's pictures do not
/// travel between threads, so the thread sends what a picture is made
/// of and the window makes it here.
type StripPiece = (Vec<u8>, u32, u32, Option<(f64, f64)>);

/// Asks the strip for the media in the background: its moments evenly
/// spread over its length. For audio, extracts PCM and renders a waveform
/// strip; for video, middle pillars of frames standing side by side.
fn ask_for_strip(
    tools: locator::Tools,
    input: PathBuf,
    duration: f64,
    width: u32,
    height: u32,
    is_audio: bool,
) {
    let (sender, receiver) = std::sync::mpsc::channel::<StripPiece>();
    std::thread::spawn(move || {
        if is_audio {
            let (samples, rate) =
                crate::media::audio::pcm_samples(&tools, &input).unwrap_or_default();
            let buckets = ((width as f32 / 4.0).round() as usize).clamp(50, 200);
            let peaks = crate::media::audio::waveform_peaks(&samples, buckets);
            let strip = crate::media::audio::render_waveform_strip(&peaks, width, height);
            let (wide, tall) = (strip.width(), strip.height());
            let silence = crate::media::audio::detect_silence(&samples, rate, 0.01);
            let _ = sender.send((strip.into_raw(), wide, tall, silence));
        } else {
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
            let _ = sender.send((strip.into_raw(), wide, tall, None));
        }
    });
    STRIP_INBOX.with(|slot| {
        *slot.borrow_mut() = Some(receiver);
    });
    STRIP_WORN.with(|slot| slot.set(false));
}

/// What the strip thread has been up to since last asked.
enum StripTick {
    /// A finished strip has arrived.
    Arrived(StripPiece),
    /// Still being worked out.
    Working,
    /// Gone, or never there.
    Gone,
}

/// Takes one turn of the strip's arriving, without waiting.
fn strip_tick() -> StripTick {
    STRIP_INBOX.with(|slot| {
        let mut buried = slot.borrow_mut();
        let Some(receiver) = buried.as_ref() else {
            return StripTick::Gone;
        };
        match receiver.try_recv() {
            Ok(piece) => StripTick::Arrived(piece),
            Err(std::sync::mpsc::TryRecvError::Empty) => StripTick::Working,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                *buried = None;
                StripTick::Gone
            }
        }
    })
}

/// Puts a finished strip on the card.
unsafe fn wear_strip(piece: StripPiece) {
    let (pixels, width, height, silence) = piece;
    if width == 0 || height == 0 {
        return;
    }
    let buffer = SharedPixelBuffer::clone_from_slice(&pixels, width, height);
    let image = slint::Image::from_rgba8(buffer);
    let mut updated_range = false;
    let mut start_frac = 0.0;
    let mut end_frac = 1.0;
    STATE.with(|slot| {
        let mut state = slot.borrow_mut();
        state.silence = silence;
        if state.audio && state.trim_silence {
            if let Some((lead, trail)) = silence {
                state.start = lead.clamp(0.0, state.duration);
                state.end = trail.clamp(state.start + MIN_SPAN, state.duration);
                start_frac = fraction(state.start, state.duration);
                end_frac = fraction(state.end, state.duration);
                updated_range = true;
            }
        }
    });
    HOST.with(|slot| {
        if let Some(host) = slot.borrow_mut().as_mut() {
            host.card.set_filmstrip(image);
            if updated_range {
                host.card.set_start_fraction(start_frac);
                host.card.set_end_fraction(end_frac);
            }
            host.software.request_redraw();
        }
    });
    // The strip is drawn by the window's own paint, and the paint comes
    // only when the window is told the card has changed under it.
    let window = window_handle();
    if !window.0.is_null() {
        if updated_range {
            refresh_times();
        }
        let _ = unsafe { InvalidateRect(Some(window), None, true) };
    }
    STRIP_WORN.with(|slot| slot.set(true));
}

/// The window handle, if there is one.
fn window_handle() -> HWND {
    HWND(WINDOW.get() as *mut _)
}

/// The window, its card, and the buffer the card is drawn into.
struct Host {
    window: HWND,
    card: TrimCard,
    software: Rc<MinimalSoftwareWindow>,
    buffer: Vec<Bgra8>,
    width: u32,
    height: u32,
}

/// The window a trim is asked in.
pub struct TimelineWindow {
    window: HWND,
}

// The window belongs to the thread that made it and is used by no other.
unsafe impl Send for TimelineWindow {}

/// How wide a window of this card must be made so that what a person
/// uses of it is the size the drawing asked for, at this stretch.
fn outer_size(scale: f32) -> (i32, i32) {
    let client_w = (CARD_W as f32 * scale) as i32;
    let content = if is_audio() {
        Layout::AUDIO_CONTENT_UNITS
    } else if is_split() {
        Layout::SPLIT_CONTENT_UNITS
    } else {
        Layout::CONTENT_UNITS
    };
    let client_h = ((content + client_preview_units() + Layout::FOOTER_UNITS) as f32 * scale)
        as i32;
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

/// The preview height the card is drawn with, in the drawing's own units.
fn client_preview_units() -> f32 {
    STATE.with(|slot| slot.borrow().preview_units)
}

/// Whether the window is open for an audio trim: no video preview frame,
/// waveform strip, and silence / play options.
fn is_audio() -> bool {
    STATE.with(|slot| slot.borrow().audio)
}

/// Whether the window is asked for a split and not a trim: the layout,
/// the hits and the words of the card all follow it.
fn is_split() -> bool {
    STATE.with(|slot| slot.borrow().split)
}

impl TimelineWindow {
    /// Makes the window, hidden until a trim is picked.
    pub fn new(events: Sender<ToolEvent>) -> Result<TimelineWindow, String> {
        EVENTS.with(|slot| *slot.borrow_mut() = Some(events));

        // A window is put aside for the card before the card is made, so
        // the two find each other.
        let software = window_for_next_component();
        let card = TrimCard::new().map_err(|error| error.to_string())?;

        let instance = unsafe { GetModuleHandleW(None).map_err(|error| error.to_string())? };
        let class_name = w!("OpenTangerineTimeline");
        // The card's own white is what fills it while it comes in, so a
        // dark theme's idea of "window colour" never shows through. The
        // true size is set when a trim is asked for, from the video's
        // own shape.
        let scale = screen_scale();
        let (outer_w, outer_h) = outer_size(scale);
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

        // The corners of the window are round on a system that rounds its
        // windows; on one that does not, they stay as they are.
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

        // The buttons of the card say what they were pressed for, the way
        // the wheel's choices do. They read the handle on its own, not the
        // host they are drawn by, because a button can be pressed while a
        // drawing is still holding the host.
        {
            card.on_run_chosen(move || {
                let window = window_handle();
                if !window.0.is_null() {
                    unsafe { run(window) };
                }
            });
            card.on_cancel_chosen(move || {
                let window = window_handle();
                if !window.0.is_null() {
                    unsafe { cancel(window) };
                }
            });
            card.on_step_backward(move || {
                let window = window_handle();
                if !window.0.is_null() {
                    unsafe { step(window, -1.0) };
                }
            });
            card.on_step_forward(move || {
                let window = window_handle();
                if !window.0.is_null() {
                    unsafe { step(window, 1.0) };
                }
            });
            card.on_add_split_chosen(move || {
                let window = window_handle();
                if !window.0.is_null() {
                    unsafe { add_split(window) };
                }
            });
            card.on_set_equal_chosen(move || {
                let window = window_handle();
                if !window.0.is_null() {
                    unsafe { set_equal(window) };
                }
            });
            card.on_stepper_up(move || {
                let window = window_handle();
                if !window.0.is_null() {
                    unsafe { turn_parts(window, 1) };
                }
            });
            card.on_stepper_down(move || {
                let window = window_handle();
                if !window.0.is_null() {
                    unsafe { turn_parts(window, -1) };
                }
            });
            card.on_silence_toggled(move || {
                let window = window_handle();
                if !window.0.is_null() {
                    unsafe { toggle_silence(window) };
                }
            });
            card.on_play_selection_chosen(move || {
                let window = window_handle();
                if !window.0.is_null() {
                    unsafe { play_selection(window) };
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
        };
        host.resize();
        HOST.with(|slot| *slot.borrow_mut() = Some(host));
        WINDOW.with(|slot| slot.set(window.0 as isize));

        Ok(TimelineWindow { window })
    }

    /// Shows the window for one trim on the given files.
    ///
    /// The first video is the one asked about: how long it is, what its
    /// first moment looks like, and from that, how tall the window wants
    /// to be. All of it is asked before the window is shown, so what
    /// appears is already the video's own card at its own shape.
    pub fn show_for(&mut self, operation: &str, label: &str, detail: &str, inputs: &[PathBuf]) {
        OPEN_FOR.with(|slot| {
            *slot.borrow_mut() = Some((operation.to_string(), inputs.to_vec()));
        });
        let split = operation == "tools.split";
        let is_audio = inputs
            .first()
            .and_then(|p| p.extension())
            .and_then(|e| e.to_str())
            .map(crate::radial::selection::FileKind::from_extension)
            == Some(crate::radial::selection::FileKind::Audio);

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
        let ffmpeg = locator::find();
        let duration = ffmpeg
            .as_ref()
            .and_then(|tools| duration_of(tools, &input))
            .unwrap_or(0.0);
        // The frame at the very beginning answers two questions at once:
        // what the card shows first, and how tall the preview wants to
        // be. An audio file has no frames (preview_units = 0). A video
        // whose shape cannot be read is shown as a wide one.
        let beginning = if is_audio {
            None
        } else {
            ffmpeg
                .as_ref()
                .and_then(|tools| frame_at(tools, &input, 0.0))
        };
        let preview_units = if is_audio {
            0.0
        } else {
            let aspect = beginning
                .as_ref()
                .map(|frame| frame.width() as f32 / frame.height().max(1) as f32)
                .unwrap_or(16.0 / 9.0);
            (CARD_W as f32 / aspect).clamp(PREVIEW_MIN, PREVIEW_MAX)
        };
        STATE.with(|slot| {
            *slot.borrow_mut() = TrimState {
                duration,
                start: 0.0,
                end: duration,
                playhead: 0.0,
                dragging: Drag::Nothing,
                frame_at: 0.0,
                preview_units,
                ffmpeg: ffmpeg.clone(),
                input: input.clone(),
                split,
                audio: is_audio,
                trim_silence: false,
                silence: None,
                points: Vec::new(),
                parts: PARTS_MIN,
                equal_saying: None,
            };
        });
        // The strip of the video's own moments is asked for apart: it is
        // many seeks, and the window must not wait on them. It arrives
        // with the minutes, one picture, and is worn as soon as it is
        // there. The card opens on the plain well.
        if ffmpeg.is_some() && duration > 0.0 {
            ask_for_strip(
                ffmpeg.clone().unwrap(),
                input.clone(),
                duration,
                (CARD_W - 48).max(1) as u32,
                STRIP_UNITS as u32,
                is_audio,
            );
        }

        // Everything the card needs is set first, while the host is held
        // only for that and nothing is shown yet. Where the video stands
        // is told after the host is let go, because telling it draws a
        // frame, and drawing must never find the card still held.
        let card_window = HOST.with(|slot| {
            let mut borrowed = slot.borrow_mut();
            let Some(host) = borrowed.as_mut() else {
                return HWND::default();
            };
            host.card.set_file_name(said.into());
            host.card.set_detail(detail.into());
            host.card.set_split_mode(split);
            host.card.set_audio_mode(is_audio);
            host.card
                .set_run_label(if split { "Split Video" } else if is_audio { "Apply" } else { "Trim" }.into());
            host.card.set_trim_silence(false);
            host.lay_out();
            // The frame that was asked for before sizing is put on now,
            // at the size of its own box.
            if let Some(frame) = beginning {
                let scale = scale_factor(host.window);
                let layout = Layout::of(host.width as i32, host.height as i32, scale);
                let box_w = (layout.preview.right - layout.preview.left).max(1) as u32;
                let box_h = (layout.preview.bottom - layout.preview.top).max(1) as u32;
                let small = frame.thumbnail(box_w, box_h);
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

        // And only then is anything shown: the messages that come of
        // showing a window can arrive at once, and the one that draws the
        // card must not find the card still held.
        if card_window.0.is_null() {
            return;
        }
        refresh_times();
        unsafe {
            // The window is made the size the video's shape wants, where
            // it already stands.
            let mut stands = RECT::default();
            let _ = GetWindowRect(card_window, &mut stands);
            let scale = scale_factor(card_window);
            let (outer_w, outer_h) = outer_size(scale);
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
            // The window comes in softly, from nothing to whole in a few
            // beats of the timer.
            FADE_ALPHA.with(|slot| slot.set(0));
            let _ = SetLayeredWindowAttributes(card_window, COLORREF(0), 0, LWA_ALPHA);
            let _ = SetForegroundWindow(card_window);
            ShowWindow(card_window, SW_SHOW);
            let _ = SetTimer(Some(card_window), FADE_TIMER_ID, 16, None);
            // The strip is being worked out elsewhere: the window asks it
            // every few beats whether some of it has come.
            let _ = SetTimer(Some(card_window), STRIP_TIMER_ID, 40, None);
            // The keyboard starts on the first stop: a trim's start cap,
            // a split's playhead.
            FOCUS_INDEX.with(|slot| slot.set(0));
            apply_focus(card_window, 0);
        }
        // The window was resized while it was hidden, and a hidden
        // window may not hear of it: the size is said again now that it
        // is shown, so the card's own bounds agree with the window's.
        // Nothing of the system is called inside the borrow, so a
        // message that arrives cannot find the card held.
        HOST.with(|slot| {
            let mut borrowed = slot.borrow_mut();
            if let Some(host) = borrowed.as_mut() {
                let (width, height) = (host.width, host.height);
                host.software.set_size(PhysicalSize::new(width, height));
                host.software.request_redraw();
            }
        });
    }

    /// Hides the window.
    pub fn hide(&self) {
        unsafe {
            ShowWindow(self.window, SW_HIDE);
        }
    }
}

impl Host {
    /// Sizes the buffer to the window and tells the card.
    fn resize(&mut self) {
        let mut rect = RECT::default();
        let _ = unsafe { GetClientRect(self.window, &mut rect) };
        let width = rect.right.max(1) as u32;
        let height = rect.bottom.max(1) as u32;
        if width != self.width || height != self.height {
            self.width = width;
            self.height = height;
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

    /// Tells the card where everything is. The same numbers are read by
    /// the window when the timeline is pressed, so what is drawn and
    /// what is taken can never disagree.
    fn lay_out(&mut self) -> Layout {
        let scale = scale_factor(self.window);
        let layout = Layout::of(self.width as i32, self.height as i32, scale);
        let panel = |rect: RECT| Panel {
            x: rect.left as f32,
            y: rect.top as f32,
            width: (rect.right - rect.left) as f32,
            height: (rect.bottom - rect.top) as f32,
        };
        self.card.set_ui_scale(scale);
        self.card.set_header_box(panel(layout.file_name));
        self.card.set_detail_box(panel(layout.detail));
        self.card.set_hairline_top(panel(layout.hairline_top));
        self.card.set_hairline_bottom(panel(layout.hairline_bottom));
        self.card.set_preview_box(panel(layout.preview));
        self.card.set_caption_box(panel(layout.caption));
        self.card.set_timeline_box(panel(layout.timeline));
        self.card.set_times_box(panel(layout.times));
        self.card.set_step_back_box(panel(layout.step_back));
        self.card.set_step_forward_box(panel(layout.step_forward));
        self.card.set_transport_box(panel(layout.transport));
        self.card.set_helper_box(panel(layout.helper));
        self.card.set_cancel_box(panel(layout.cancel));
        self.card.set_run_box(panel(layout.run));
        self.card.set_points_box(panel(layout.points));
        self.card.set_add_split_box(panel(layout.add_split));
        self.card.set_equal_label_box(panel(layout.equal_label));
        self.card.set_stepper_box(panel(layout.stepper));
        self.card.set_step_up_box(panel(layout.step_up));
        self.card.set_step_down_box(panel(layout.step_down));
        self.card.set_set_box(panel(layout.set));
        self.card.set_silence_box(panel(layout.silence));
        self.card.set_play_selection_box(panel(layout.play_selection));
        self.card.set_audio_mode(is_audio());
        self.card.set_trim_silence(STATE.with(|s| s.borrow().trim_silence));
        layout
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

    /// Tells the card where the pointer is and what it did, and draws
    /// what came of it.
    fn pointer(&mut self, event: WindowEvent) {
        self.software.window().dispatch_event(event);
        self.software.request_redraw();
        self.draw();
    }
}

/// Where every part of the window is, in pixels of the screen.
///
/// The card is a stack: a header, a hairline, the picture at the height
/// its shape wants, a line of small words, the track, the moments, the
/// transport, a hairline, and the footer. The Rust side works the stack
/// out once and both sides read it. A split card says cut points and the
/// equal row instead of the moments, and its stack answers for it.
#[derive(Clone, Copy, Debug, Default)]
struct Layout {
    file_name: RECT,
    detail: RECT,
    hairline_top: RECT,
    hairline_bottom: RECT,
    preview: RECT,
    caption: RECT,
    timeline: RECT,
    times: RECT,
    step_back: RECT,
    step_forward: RECT,
    transport: RECT,
    helper: RECT,
    cancel: RECT,
    run: RECT,
    // The split rows: the count of the cuts, the pill that puts one at
    // the playhead, and the equal row's label, stepper and Set pill.
    points: RECT,
    add_split: RECT,
    equal_label: RECT,
    stepper: RECT,
    step_up: RECT,
    step_down: RECT,
    set: RECT,
    // Audio trim options: silence checkbox and play selection button.
    silence: RECT,
    play_selection: RECT,
}

impl Layout {
    /// Everything of the stack except the picture and the footer, in the
    /// drawing's own units: header, hairline, the gap and the caption
    /// and the strip and the moments and the transport, and the closing
    /// hairline.
    const CONTENT_UNITS: f32 = 282.0;
    /// The same for a split card: its points row stands taller than the
    /// trim caption, it has no moments row, and after its transport the
    /// equal row takes the room.
    const SPLIT_CONTENT_UNITS: f32 = 308.0;
    /// The same for an audio trim card: no video preview frame, waveform
    /// strip, moments row, and silence / play options row.
    const AUDIO_CONTENT_UNITS: f32 = 272.0;
    /// The footer: the hairline's gap to the buttons, the buttons, and
    /// the air under them.
    const FOOTER_UNITS: f32 = 64.0;

    /// How tall a window of this preview wants to be, in the drawing's
    /// own units. This is the one number the window's size and the
    /// drawing's stack are both worked out from.
    fn height_for(preview_units: f32) -> f32 {
        Self::CONTENT_UNITS + preview_units + Self::FOOTER_UNITS
    }

    /// How tall a split window of this preview wants to be.
    fn height_for_split(preview_units: f32) -> f32 {
        Self::SPLIT_CONTENT_UNITS + preview_units + Self::FOOTER_UNITS
    }

    fn of(whole: i32, side: i32, scale: f32) -> Layout {
        let at = |value: f32| (value * scale) as i32;
        let margin = at(24.0);
        let preview_units = client_preview_units();
        let split = is_split();
        let audio = is_audio();

        // The stack, top to bottom, all worked out from the one number.
        let header_top = at(16.0);
        let hairline_top = at(62.0);
        let preview_top = hairline_top + if audio { 0 } else { at(20.0) };
        let preview_bottom = preview_top + at(preview_units);

        // A trim: the caption, then the moments row, and the closing
        // hairline two units nearer the footer than a rounded count
        // would put it — the way the trim card was measured in. A split:
        // a taller points row with its pill, no moments row, and after
        // the transport the equal row, and the hairline under it.
        let caption_top = if audio {
            hairline_top + at(16.0)
        } else {
            preview_bottom + at(16.0)
        };
        let timeline_top = if split {
            caption_top + at(30.0) + at(12.0)
        } else {
            caption_top + at(16.0) + at(12.0)
        };
        let (times_top, transport_top) = if split || audio {
            let after_strip = timeline_top + at(STRIP_UNITS) + at(10.0);
            (after_strip, after_strip)
        } else {
            let times = timeline_top + at(STRIP_UNITS) + at(10.0);
            (times, times + at(18.0) + at(10.0))
        };
        let transport_row = at(28.0);
        let equal_top = transport_top + transport_row + at(10.0);
        let options_top = times_top + at(18.0) + at(14.0);
        let hairline_bottom = if audio {
            options_top + at(32.0) + at(18.0)
        } else if split {
            equal_top + at(30.0) + at(18.0)
        } else {
            transport_top + at(30.0) + at(18.0)
        };

        // The equal row is laid to the right: the count, the two little
        // wings of its stepper, and the pill that lives where the cuts
        // are told to stand. The label keeps what is left of them.
        let set_right = whole - margin;
        let set_left = set_right - at(56.0);
        let stepper_left = set_left - at(8.0) - at(24.0) - at(6.0) - at(44.0);

        let mut layout = Layout {
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
            preview: RECT {
                left: margin,
                top: preview_top,
                right: whole - margin,
                bottom: preview_bottom,
            },
            caption: RECT {
                left: margin,
                top: caption_top,
                right: whole - margin,
                bottom: caption_top + if split { at(30.0) } else { at(16.0) },
            },
            timeline: RECT {
                left: margin,
                top: timeline_top,
                right: whole - margin,
                bottom: timeline_top + at(STRIP_UNITS),
            },
            times: RECT {
                left: margin,
                top: times_top,
                right: whole - margin,
                bottom: times_top + at(18.0),
            },
            step_back: RECT {
                left: margin,
                top: transport_top,
                right: margin + at(28.0),
                bottom: transport_top + at(28.0),
            },
            step_forward: RECT {
                left: margin + at(28.0) + at(4.0),
                top: transport_top,
                right: margin + at(28.0) + at(4.0) + at(28.0),
                bottom: transport_top + at(28.0),
            },
            transport: RECT {
                left: margin + at(28.0) + at(4.0) + at(28.0) + at(16.0),
                top: transport_top,
                right: whole - margin,
                bottom: transport_top + at(28.0),
            },
            hairline_bottom: RECT {
                left: 0,
                top: hairline_bottom,
                right: whole,
                bottom: hairline_bottom + 1,
            },
            // The footer is measured from the bottom of the window, so a
            // window of an unexpected height still puts the buttons where
            // a person reaches them.
            helper: RECT {
                left: margin,
                top: side - at(16.0) - at(34.0),
                right: whole - margin - at(84.0) - at(16.0),
                bottom: side - at(16.0),
            },
            cancel: RECT {
                left: whole - margin - at(84.0) - at(16.0) - at(76.0),
                top: side - at(16.0) - at(34.0),
                right: whole - margin - at(84.0) - at(16.0),
                bottom: side - at(16.0),
            },
            run: RECT {
                left: whole - margin - at(84.0),
                top: side - at(16.0) - at(34.0),
                right: whole - margin,
                bottom: side - at(16.0),
            },
            points: RECT::default(),
            add_split: RECT::default(),
            equal_label: RECT::default(),
            stepper: RECT::default(),
            step_up: RECT::default(),
            step_down: RECT::default(),
            set: RECT::default(),
            silence: RECT::default(),
            play_selection: RECT::default(),
        };

        if split {
            // The points row: the count on the left, the pill at the
            // right hand of it, standing where the caption stands.
            let pill_left = whole - margin - at(110.0);
            layout.points = RECT {
                left: margin,
                top: caption_top,
                right: pill_left - at(10.0),
                bottom: caption_top + at(30.0),
            };
            layout.add_split = RECT {
                left: pill_left,
                top: caption_top,
                right: whole - margin,
                bottom: caption_top + at(30.0),
            };
            // And the equal row after the transport.
            layout.equal_label = RECT {
                left: margin,
                top: equal_top,
                right: stepper_left - at(16.0),
                bottom: equal_top + at(30.0),
            };
            layout.stepper = RECT {
                left: stepper_left,
                top: equal_top,
                right: stepper_left + at(44.0),
                bottom: equal_top + at(30.0),
            };
            let wings_left = stepper_left + at(44.0) + at(6.0);
            layout.step_up = RECT {
                left: wings_left,
                top: equal_top,
                right: wings_left + at(24.0),
                bottom: equal_top + at(15.0),
            };
            layout.step_down = RECT {
                left: wings_left,
                top: equal_top + at(15.0),
                right: wings_left + at(24.0),
                bottom: equal_top + at(30.0),
            };
            let set_right = whole - margin;
            layout.set = RECT {
                left: set_right - at(56.0),
                top: equal_top,
                right: set_right,
                bottom: equal_top + at(30.0),
            };
        }

        if audio {
            let options_bottom = options_top + at(32.0);
            let play_right = whole - margin;
            let play_left = play_right - at(170.0);
            layout.silence = RECT {
                left: margin,
                top: options_top,
                right: margin + at(220.0),
                bottom: options_bottom,
            };
            layout.play_selection = RECT {
                left: play_left,
                top: options_top,
                right: play_right,
                bottom: options_bottom,
            };
        }

        layout
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

/// Puts the keyboard on one stop of the card: a cap or the go button. A
/// split's stops say themselves: the playhead keeps no ring, and the two
/// pills and the go button do.
unsafe fn apply_focus(window: HWND, index: i32) {
    // The focus is the window's own everywhere: the caps are drawn, not
    // made of the system's stuff, and the keyboard is read here.
    unsafe { let _ = SetFocus(Some(window)); }
    let split = is_split();
    let audio = is_audio();
    HOST.with(|slot| {
        let mut borrowed = slot.borrow_mut();
        if let Some(host) = borrowed.as_mut() {
            if audio {
                // 0 start cap, 1 end cap, 2 silence checkbox, 3 play selection, 4 run ("Apply")
                host.card
                    .set_handle_focus(if index == 0 || index == 1 { index + 1 } else { 0 });
                host.card
                    .set_audio_focus(if index == 2 { 1 } else if index == 3 { 2 } else { 0 });
                host.card
                    .set_button_focus(if index == 4 { 2 } else { 0 });
            } else if split {
                // 0 the playhead (no ring), 1 add-split, 2 set, 3 run.
                host.card.set_handle_focus(0);
                host.card
                    .set_button_focus(if index == 3 { 2 } else { 0 });
                host.card
                    .set_pill_focus(if index == 1 { 1 } else { if index == 2 { 2 } else { 0 } });
            } else {
                host.card
                    .set_handle_focus(if index == 0 || index == 1 { index + 1 } else { 0 });
                host.card.set_button_focus(if index == 2 { 2 } else { 0 });
            }
            host.software.request_redraw();
        }
    });
    let _ = unsafe { InvalidateRect(Some(window), None, true) };
}

/// Walks the keyboard to the next stop of the card, or the one before:
/// a trim walks the start cap, the end cap, and go; a split walks the
/// playhead, the two pills, and go.
unsafe fn cycle_focus(window: HWND, forward: bool) {
    let stops: &[i32] = if is_audio() {
        &[0, 1, 2, 3, 4]
    } else if is_split() {
        &[0, 1, 2, 3]
    } else {
        &[0, 1, 2]
    };
    let current = FOCUS_INDEX.with(|slot| slot.get());
    let at = stops
        .iter()
        .position(|&stop| stop == current)
        .unwrap_or(0);
    let next = if forward {
        (at + 1) % stops.len()
    } else {
        (at + stops.len() - 1) % stops.len()
    };
    let index = stops[next];
    FOCUS_INDEX.with(|slot| slot.set(index));
    unsafe { apply_focus(window, index) };
}

/// Tells the card where the video begins and ends, and where the
/// playhead stands, and what all of it says in words. A trim card says
/// its stretch; a split card says how many cuts stand and wears them.
fn refresh_times() {
    let split = is_split();
    HOST.with(|slot| {
        let mut borrowed = slot.borrow_mut();
        if let Some(host) = borrowed.as_mut() {
            STATE.with(|state| {
                let state = state.borrow();
                host.card.set_start_fraction(fraction(state.start, state.duration));
                host.card.set_end_fraction(fraction(state.end, state.duration));
                host.card
                    .set_playhead_fraction(fraction(state.playhead, state.duration));
                host.card
                    .set_selected_text(format!("{} selected", timecode(state.end - state.start)).into());
                host.card.set_start_text(timecode(state.start).into());
                host.card.set_end_text(timecode(state.end).into());
                host.card.set_playhead_text(
                    format!("{} / {}", timecode(state.playhead), timecode(state.duration)).into(),
                );
                if split {
                    let mut said = match state.points.len() {
                        0 => "0 split points".to_string(),
                        1 => "1 split point".to_string(),
                        n => format!("{n} split points"),
                    };
                    if let Some(about) = state.equal_saying.as_deref() {
                        said.push_str(&format!(" \u{b7} each about {about}"));
                    }
                    host.card.set_points_text(said.into());
                    host.card.set_parts_text(state.parts.to_string().into());
                    host.card.set_run_enabled(!state.points.is_empty());
                    let markers: Vec<f32> = state
                        .points
                        .iter()
                        .map(|&point| fraction(point, state.duration))
                        .collect();
                    host.card.set_markers(
                        std::rc::Rc::new(slint::VecModel::from(markers)).into(),
                    );
                }
            });
            host.software.request_redraw();
        }
    });
}

/// Draws the frame the playhead stands on, if it has moved far enough
/// for a new one to be worth a seek.
fn refresh_frame(force: bool) {
    if is_audio() {
        return;
    }
    let wanted = STATE.with(|slot| slot.borrow().playhead);
    let ready = force
        || STATE.with(|slot| (wanted - slot.borrow().frame_at).abs() >= FRAME_STEP);
    if !ready {
        return;
    }
    let picture = STATE.with(|slot| {
        let state = slot.borrow();
        state
            .ffmpeg
            .as_ref()
            .and_then(|tools| frame_at(tools, &state.input, state.playhead))
            .map(|picture| {
                let (box_w, box_h) = HOST.with(|slot| {
                    let borrowed = slot.borrow();
                    let Some(host) = borrowed.as_ref() else {
                        return (412u32, 236u32);
                    };
                    let scale = scale_factor(host.window);
                    let layout =
                        Layout::of(host.width as i32, host.height as i32, scale);
                    (
                        (layout.preview.right - layout.preview.left).max(1) as u32,
                        (layout.preview.bottom - layout.preview.top).max(1) as u32,
                    )
                });
                let small = picture.thumbnail(box_w, box_h);
                let rgba = small.to_rgba8();
                let buffer = SharedPixelBuffer::clone_from_slice(
                    rgba.as_raw(),
                    small.width(),
                    small.height(),
                );
                slint::Image::from_rgba8(buffer)
            })
    });
    STATE.with(|slot| slot.borrow_mut().frame_at = wanted);
    if let Some(picture) = picture {
        HOST.with(|slot| {
            let mut borrowed = slot.borrow_mut();
            if let Some(host) = borrowed.as_mut() {
                host.card.set_frame(picture);
                host.card.set_has_frame(true);
                host.software.request_redraw();
            }
        });
    }
}

/// Takes what the pointer pressed on the timeline, and moves what it
/// took to where the pointer is.
fn timeline_touched(window: HWND, x: i32, y: i32, pressed: bool) {
    let scale = scale_factor(window);
    let (width, height) = HOST.with(|slot| {
        let borrowed = slot.borrow();
        borrowed
            .as_ref()
            .map(|host| (host.width as i32, host.height as i32))
            .unwrap_or((0, 0))
    });
    if width == 0 {
        return;
    }
    let layout = Layout::of(width, height, scale);
    let bar = layout.timeline;
    // The timeline is taken generously: a little above and below the bar
    // still counts, because the handles stand taller than it.
    let at = |value: f32| (value * scale) as i32;
    let near = y >= bar.top - at(8.0)
        && y <= bar.bottom + at(8.0)
        && x >= bar.left
        && x <= bar.right;
    if !near {
        return;
    }
    let along = |moment: f64| {
        let duration = STATE.with(|slot| slot.borrow().duration);
        bar.left + (fraction(moment, duration) * (bar.right - bar.left) as f32) as i32
    };

    if pressed {
        STATE.with(|slot| {
            let mut state = slot.borrow_mut();
            if state.duration <= 0.0 {
                return;
            }
            if state.split {
                // A split takes no ends: a press on a marker stands the
                // playhead on the cut it took, anywhere else takes the
                // playhead itself.
                let on_marker = state
                    .points
                    .iter()
                    .any(|&point| (x - along(point)).abs() <= at(MARKER_REACH as f32));
                state.dragging = if on_marker {
                    Drag::Nothing
                } else {
                    Drag::Playhead
                };
            } else {
                let take_start = (x - along(state.start)).abs() <= at(HANDLE_REACH as f32);
                let take_end = (x - along(state.end)).abs() <= at(HANDLE_REACH as f32);
                state.dragging = if take_start && !take_end {
                    Drag::Start
                } else if take_end {
                    Drag::End
                } else {
                    Drag::Playhead
                };
            }
        });
        unsafe { let _ = SetCapture(window); };
    }

    let moved = STATE.with(|slot| {
        let mut state = slot.borrow_mut();
        if state.dragging == Drag::Nothing || state.duration <= 0.0 {
            return false;
        }
        let share = ((x - bar.left) as f64 / (bar.right - bar.left) as f64).clamp(0.0, 1.0);
        let moment = share * state.duration;
        match state.dragging {
            Drag::Start => state.start = moment.min(state.end - MIN_SPAN).max(0.0),
            Drag::End => state.end = moment.max(state.start + MIN_SPAN).min(state.duration),
            Drag::Playhead => state.playhead = moment,
            Drag::Nothing => {}
        }
        true
    });
    if moved {
        let scrubbing = STATE.with(|slot| slot.borrow().dragging) == Drag::Playhead;
        if scrubbing {
            refresh_frame(false);
        }
        refresh_times();
        let _ = unsafe { InvalidateRect(Some(window), None, true) };
    }
    // A press that took a marker stands the playhead on the cut it took:
    // the pointer need not hold on to anything to say where to go.
    if pressed {
        let duration = STATE.with(|slot| slot.borrow().duration);
        let moment = ((x - bar.left) as f64 / (bar.right - bar.left) as f64).clamp(0.0, 1.0)
            * duration;
        let reach = MARKER_REACH as f64 * duration / (bar.right - bar.left) as f64;
        let took = STATE.with(|slot| {
            slot.borrow()
                .points
                .iter()
                .copied()
                .min_by(|a, b| {
                    let da = (*a - moment).abs();
                    let db = (*b - moment).abs();
                    da.partial_cmp(&db).expect("the moments are not nothing")
                })
                .filter(|&point| (point - moment).abs() <= reach)
        });
        if let Some(point) = took {
            STATE.with(|slot| slot.borrow_mut().playhead = point);
            refresh_frame(false);
            refresh_times();
            let _ = unsafe { InvalidateRect(Some(window), None, true) };
        }
    }
}

/// Lets go of whatever the pointer was holding.
fn released(window: HWND) {
    STATE.with(|slot| slot.borrow_mut().dragging = Drag::Nothing);
    unsafe { let _ = ReleaseCapture(); };
    let _ = unsafe { InvalidateRect(Some(window), None, true) };
}

/// Steps the handle the keyboard is on, back or forward a second — or
/// five, when Shift is held. A split card has no handles: the playhead
/// moves.
unsafe fn step(window: HWND, direction: f64) {
    let shift = unsafe { GetKeyState(VK_SHIFT.0 as i32) < 0 };
    let focused = FOCUS_INDEX.with(|slot| slot.get());
    STATE.with(|slot| {
        let mut state = slot.borrow_mut();
        if state.duration <= 0.0 {
            return;
        }
        let step_unit = if state.audio { 0.1 } else { 1.0 };
        let by = direction * if shift { step_unit * 5.0 } else { step_unit };
        if state.split {
            state.playhead = (state.playhead + by).clamp(0.0, state.duration);
        } else {
            match focused {
                0 => state.start = (state.start + by).clamp(0.0, state.end - MIN_SPAN),
                1 => state.end = (state.end + by).clamp(state.start + MIN_SPAN, state.duration),
                _ => state.playhead = (state.playhead + by).clamp(0.0, state.duration),
            }
        }
    });
    refresh_frame(false);
    refresh_times();
    let _ = unsafe { InvalidateRect(Some(window), None, true) };
}

/// Puts a cut where the playhead stands, if the playhead stands where a
/// cut may go: inside the video, but not against either end, and not so
/// close to another cut that the two make no clip of their own.
unsafe fn add_split(window: HWND) {
    let made = STATE.with(|slot| {
        let mut state = slot.borrow_mut();
        let stand = state.playhead;
        let may = state.duration > 0.0
            && stand > POINT_MARGIN
            && stand < state.duration - POINT_MARGIN
            && !state
                .points
                .iter()
                .any(|&point| (point - stand).abs() < MIN_POINT_GAP);
        if may {
            state.points.push(stand);
            state.points.sort_by(|a, b| a.partial_cmp(b).expect("the moments are not nothing"));
            state.equal_saying = None;
        }
        may
    });
    if made {
        refresh_times();
        let _ = unsafe { InvalidateRect(Some(window), None, true) };
    }
}

/// Says where the equal parts cut: as many stands inside the video as
/// the count has gaps, spread evenly, the way the original works its
/// equal saying out.
unsafe fn set_equal(window: HWND) {
    STATE.with(|slot| {
        let mut state = slot.borrow_mut();
        if state.duration <= 0.0 {
            return;
        }
        let parts = state.parts;
        state.points = (1..parts)
            .map(|step| state.duration * step as f64 / parts as f64)
            .collect();
        state.equal_saying = Some(timecode(state.duration / parts as f64));
    });
    refresh_times();
    let _ = unsafe { InvalidateRect(Some(window), None, true) };
}

/// Turns the count of the equal parts between its least and most.
unsafe fn turn_parts(window: HWND, direction: i32) {
    let said = STATE.with(|slot| {
        let mut state = slot.borrow_mut();
        let next = (state.parts + direction).clamp(PARTS_MIN, PARTS_MAX);
        let turn = next != state.parts;
        state.parts = next;
        turn
    });
    if said {
        refresh_times();
        let _ = unsafe { InvalidateRect(Some(window), None, true) };
    }
}

/// Toggles the automatic silence trimming and snaps the range.
unsafe fn toggle_silence(window: HWND) {
    let (new_state, duration) = STATE.with(|slot| {
        let mut state = slot.borrow_mut();
        state.trim_silence = !state.trim_silence;
        if state.trim_silence {
            if let Some((lead, trail)) = state.silence {
                state.start = lead.clamp(0.0, state.duration);
                state.end = trail.clamp(state.start + MIN_SPAN, state.duration);
            }
        } else {
            state.start = 0.0;
            state.end = state.duration;
        }
        (state.trim_silence, state.duration)
    });
    HOST.with(|slot| {
        if let Some(host) = slot.borrow_mut().as_mut() {
            host.card.set_trim_silence(new_state);
            STATE.with(|slot| {
                let state = slot.borrow();
                host.card.set_start_fraction(fraction(state.start, duration));
                host.card.set_end_fraction(fraction(state.end, duration));
            });
            host.software.request_redraw();
        }
    });
    refresh_times();
    let _ = unsafe { InvalidateRect(Some(window), None, true) };
}

/// Preview playback affordance for the selected range.
unsafe fn play_selection(_window: HWND) {
    // Affordance for media playback
}

/// Takes away the cut nearest the playhead, when one stands near enough.
unsafe fn take_point_near(window: HWND) {
    let took = STATE.with(|slot| {
        let mut state = slot.borrow_mut();
        let stand = state.playhead;
        let share = state
            .points
            .iter()
            .copied()
            .enumerate()
            .min_by(|(_, a), (_, b)| {
                let da = (*a - stand).abs();
                let db = (*b - stand).abs();
                da.partial_cmp(&db).expect("the moments are not nothing")
            });
        match share {
            Some((index, point)) if (point - stand).abs() <= DELETE_REACH => {
                state.points.remove(index);
                state.equal_saying = None;
                true
            }
            _ => false,
        }
    });
    if took {
        refresh_times();
        let _ = unsafe { InvalidateRect(Some(window), None, true) };
    }
}

unsafe extern "system" fn window_procedure(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_PAINT => {
            let mut paint = windows::Win32::Graphics::Gdi::PAINTSTRUCT::default();
            let dc = unsafe { BeginPaint(window, &mut paint) };
            HOST.with(|slot| {
                let mut borrowed = slot.borrow_mut();
                if let Some(host) = borrowed.as_mut() {
                    host.resize();
                    host.draw();
                    let _ = dc;
                }
            });
            let _ = unsafe { windows::Win32::Graphics::Gdi::EndPaint(window, &paint) };
            LRESULT(0)
        }
        WM_SIZE => {
            HOST.with(|slot| {
                let mut borrowed = slot.borrow_mut();
                if let Some(host) = borrowed.as_mut() {
                    host.resize();
                    host.software.request_redraw();
                }
            });
            let _ = unsafe { InvalidateRect(Some(window), None, true) };
            LRESULT(0)
        }
        WM_DPICHANGED => {
            // The display under the window has changed its stretch: the
            // window is made the size its content wants at the new one,
            // where the system suggests it stand.
            unsafe {
                let scale = scale_factor(window);
                let (outer_w, outer_h) = outer_size(scale);
                let mut stands = RECT::default();
                let _ = GetWindowRect(window, &mut stands);
                let _ = SetWindowPos(
                    window,
                    None,
                    stands.left,
                    stands.top,
                    outer_w,
                    outer_h,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                );
            }
            HOST.with(|slot| {
                let mut borrowed = slot.borrow_mut();
                if let Some(host) = borrowed.as_mut() {
                    host.resize();
                    host.software.request_redraw();
                }
            });
            let _ = unsafe { InvalidateRect(Some(window), None, true) };
            LRESULT(0)
        }
        WM_MOUSEMOVE | WM_LBUTTONDOWN | WM_LBUTTONUP => {
            let x = (lparam.0 & 0xffff) as i16 as i32;
            let y = ((lparam.0 >> 16) & 0xffff) as i16 as i32;
            // The timeline is read here, where the geometry is the same
            // one the card was drawn by.
            match message {
                WM_LBUTTONDOWN => timeline_touched(window, x, y, true),
                WM_MOUSEMOVE => {
                    let holding = STATE.with(|slot| slot.borrow().dragging);
                    if holding != Drag::Nothing {
                        timeline_touched(window, x, y, false);
                    }
                }
                _ => released(window),
            }
            // The card still sees the pointer, for the hover of its
            // buttons.
            let position = LogicalPosition::new(x as f32, y as f32);
            let event = match message {
                WM_LBUTTONDOWN => WindowEvent::PointerPressed {
                    position,
                    button: PointerEventButton::Left,
                },
                WM_LBUTTONUP => WindowEvent::PointerReleased {
                    position,
                    button: PointerEventButton::Left,
                },
                _ => WindowEvent::PointerMoved { position },
            };
            HOST.with(|slot| {
                let mut borrowed = slot.borrow_mut();
                if let Some(host) = borrowed.as_mut() {
                    host.pointer(event);
                }
            });
            LRESULT(0)
        }
        WM_KEYDOWN => {
            let key = wparam.0 as u16;
            let shift = unsafe { GetKeyState(VK_SHIFT.0 as i32) < 0 };
            let cycle = key == VK_TAB.0 || key == VK_DOWN.0 || key == VK_UP.0;
            if cycle {
                unsafe { cycle_focus(window, key != VK_UP.0 && (key != VK_TAB.0 || !shift)) };
                LRESULT(0)
            } else if key == VK_LEFT.0 {
                unsafe { step(window, -1.0) };
                LRESULT(0)
            } else if key == VK_RIGHT.0 {
                unsafe { step(window, 1.0) };
                LRESULT(0)
            } else if key == VK_RETURN.0 || key == VK_SPACE.0 {
                // An audio or split card's keyboard: Space triggers the focused control.
                // Enter always says go.
                if is_audio() && key == VK_SPACE.0 {
                    let stop = FOCUS_INDEX.with(|slot| slot.get());
                    match stop {
                        2 => unsafe { toggle_silence(window) },
                        3 => unsafe { play_selection(window) },
                        4 => unsafe { run(window) },
                        _ => {}
                    }
                } else if is_split() && key == VK_SPACE.0 {
                    let stop = FOCUS_INDEX.with(|slot| slot.get());
                    match stop {
                        0 | 1 => unsafe { add_split(window) },
                        2 => unsafe { set_equal(window) },
                        _ => unsafe { run(window) },
                    }
                } else {
                    unsafe { run(window) };
                }
                LRESULT(0)
            } else if key == VK_ESCAPE.0 {
                unsafe { cancel(window) };
                LRESULT(0)
            } else if key == VK_DELETE.0 && is_split() {
                unsafe { take_point_near(window) };
                LRESULT(0)
            } else {
                unsafe { DefWindowProcW(window, message, wparam, lparam) }
            }
        }
        WM_TIMER => {
            // The coming-in: a few beats from nothing to whole.
            if wparam.0 == FADE_TIMER_ID {
                let alpha = FADE_ALPHA.with(|slot| slot.get()).saturating_add(51);
                FADE_ALPHA.with(|slot| slot.set(alpha));
                unsafe {
                    let _ = SetLayeredWindowAttributes(window, COLORREF(0), alpha, LWA_ALPHA);
                    if alpha == 255 {
                        let _ = KillTimer(Some(window), FADE_TIMER_ID);
                    }
                }
            } else if wparam.0 == STRIP_TIMER_ID {
                // The strip is asked after; one that has arrived is put
                // on the card, which redraws the window with it. Nothing
                // left to come and the timer is let go of.
                let tick = strip_tick();
                if let StripTick::Arrived(piece) = &tick {
                    unsafe { wear_strip(piece.clone()) };
                }
                if matches!(tick, StripTick::Gone) {
                    let _ = unsafe { KillTimer(Some(window), STRIP_TIMER_ID) };
                }
            }
            LRESULT(0)
        }
        WM_ERASEBKGND => LRESULT(1),
        WM_CLOSE => {
            send(ToolEvent::Cancelled);
            unsafe {
                ShowWindow(window, SW_HIDE);
            }
            LRESULT(0)
        }
        WM_DESTROY => LRESULT(0),
        _ => unsafe { DefWindowProcW(window, message, wparam, lparam) },
    }
}

/// Runs what the window was open for, with the range the handles hold —
/// or, when the window is asked for a split, with the cuts standing.
unsafe fn run(window: HWND) {
    let (split, empty_points) = STATE.with(|slot| {
        let state = slot.borrow();
        (
            state.split,
            state.split && state.points.is_empty(),
        )
    });
    // A split with no cut yet has nothing to say: the go button is
    // already retired, and the key says nothing either.
    if split && empty_points {
        return;
    }
    let settings = STATE.with(|slot| {
        let state = slot.borrow();
        if state.points.is_empty() {
            ToolSettings {
                start_ms: Some((state.start * 1000.0).round() as u64),
                end_ms: Some((state.end * 1000.0).round() as u64),
                ..Default::default()
            }
        } else {
            ToolSettings {
                points_ms: state
                    .points
                    .iter()
                    .map(|point| (point * 1000.0).round() as u64)
                    .collect(),
                ..Default::default()
            }
        }
    });
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
    unsafe { ShowWindow(window, SW_HIDE) };
}

/// Says no to what the window was open for, and puts it away.
unsafe fn cancel(window: HWND) {
    send(ToolEvent::Cancelled);
    unsafe { ShowWindow(window, SW_HIDE) };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_moment_is_said_in_minutes_and_hundredths() {
        assert_eq!(timecode(0.0), "0:00.00");
        assert_eq!(timecode(83.456), "1:23.46");
        assert_eq!(timecode(4.0), "0:04.00");
        assert_eq!(timecode(600.0), "10:00.00");
    }

    #[test]
    fn a_share_of_the_whole_is_kept_between_the_ends() {
        assert_eq!(fraction(5.0, 10.0), 0.5);
        assert_eq!(fraction(-1.0, 10.0), 0.0);
        assert_eq!(fraction(20.0, 10.0), 1.0);
        // No duration known: everything stands at the beginning.
        assert_eq!(fraction(5.0, 0.0), 0.0);
    }

    #[test]
    fn the_height_of_a_window_is_its_content_and_its_preview() {
        assert_eq!(Layout::height_for(236.0), 582.0);
        // A portrait video is held at the most a preview may be.
        assert_eq!(Layout::height_for(300.0), 646.0);
    }

    #[test]
    fn the_layout_stacks_the_body_top_to_bottom() {
        STATE.with(|slot| slot.borrow_mut().preview_units = 236.0);
        let whole = 420;
        let side = 582;
        let layout = Layout::of(whole, side, 1.0);
        // The picture keeps the margins the reference gives its stages.
        assert_eq!(layout.preview.left, 24);
        assert_eq!(layout.preview.right, whole - 24);
        // The strip is as tall as the reference's is, in units.
        assert_eq!(layout.timeline.bottom - layout.timeline.top,
                   (STRIP_UNITS * 1.0) as i32);
        // The picture keeps the margins the reference gives its stages.
        assert_eq!(layout.preview.left, 24);
        assert_eq!(layout.preview.right, whole - 24);
        // The stack: header, hairline, picture, words, strip, moments,
        // transport, hairline, footer.
        assert!(layout.hairline_top.bottom <= layout.preview.top);
        assert!(layout.preview.bottom <= layout.caption.top);
        assert!(layout.caption.bottom <= layout.timeline.top);
        assert!(layout.timeline.bottom <= layout.times.top);
        assert!(layout.times.bottom <= layout.step_back.top);
        assert_eq!(layout.step_back.top, layout.step_forward.top);
        assert!(layout.transport.bottom + 10 <= layout.hairline_bottom.top);
        // The go button stands inside the window, at the bottom right.
        assert!(layout.run.right <= whole);
        assert!(layout.run.bottom <= side);
        // The helper says its piece without running into the go.
        assert!(layout.helper.right <= layout.run.left);
    }

    #[test]
    fn a_taller_preview_makes_a_taller_window_and_the_same_widths() {
        STATE.with(|slot| slot.borrow_mut().preview_units = 300.0);
        let narrow = Layout::of(420, 646, 1.0);
        STATE.with(|slot| slot.borrow_mut().preview_units = 236.0);
        let wide = Layout::of(420, 582, 1.0);
        // The picture takes all the height the shape asked for; nothing
        // else in the stack moves by a pixel.
        assert_eq!(narrow.preview.bottom - narrow.preview.top, 300);
        assert_eq!(wide.preview.bottom - wide.preview.top, 236);
        assert_eq!(narrow.run.left, wide.run.left);
        assert_eq!(narrow.timeline.top - narrow.timeline.bottom,
                   wide.timeline.top - wide.timeline.bottom);
    }

    #[test]
    fn the_height_of_a_split_window_is_its_own_content_and_its_preview() {
        assert_eq!(Layout::height_for_split(236.0), 608.0);
        assert_eq!(Layout::height_for_split(300.0), 672.0);
    }

    #[test]
    fn a_split_layout_stands_the_rows_the_card_says() {
        STATE.with(|slot| {
            slot.borrow_mut().preview_units = 236.0;
            slot.borrow_mut().split = true;
        });
        let whole = 420;
        let side = 608;
        let layout = Layout::of(whole, side, 1.0);
        // The points row is two stops: the count on the left, the pill at
        // the right, on the same line.
        assert_eq!(layout.points.top, layout.add_split.top);
        assert!(layout.points.right + 10 <= layout.add_split.left, "the count does not run into the pill");
        assert_eq!(layout.add_split.right, whole - 24);
        // The strip stands where the trim card wears it, as tall as ever.
        assert_eq!(
            layout.timeline.bottom - layout.timeline.top,
            (STRIP_UNITS * 1.0) as i32
        );
        // The moments row is not a split card's: the transport follows
        // the strip, and the equal row after it, both on one line.
        assert_eq!(layout.step_back.top, layout.step_forward.top);
        assert!(layout.step_forward.bottom + 10 <= layout.equal_label.top,
                "the equal row stands under the transport");
        assert_eq!(layout.equal_label.top, layout.stepper.top);
        assert_eq!(layout.stepper.top, layout.set.top);
        assert!(layout.equal_label.right < layout.stepper.left);
        assert!(layout.step_down.right < layout.set.left,
                "the Set pill stands at the right, clear of the wings");
        assert_eq!(layout.set.right, whole - 24);
        // The stack still closes: hairline above and below, the footer
        // inside the window.
        assert!(layout.timeline.bottom < layout.hairline_bottom.top);
        assert!(layout.run.right <= whole && layout.run.bottom <= side);
        STATE.with(|slot| slot.borrow_mut().split = false);
    }

    #[test]
    fn an_audio_trim_layout_collapses_preview_and_positions_options() {
        STATE.with(|slot| {
            let mut s = slot.borrow_mut();
            s.preview_units = 0.0;
            s.split = false;
            s.audio = true;
        });
        assert_eq!(Layout::AUDIO_CONTENT_UNITS + Layout::FOOTER_UNITS, 336.0);
        let whole = 420;
        let side = 336;
        let layout = Layout::of(whole, side, 1.0);
        // Preview height is zero
        assert_eq!(layout.preview.bottom - layout.preview.top, 0);
        // Caption stands directly below hairline
        assert_eq!(layout.caption.top, layout.hairline_top.top + 16);
        // Timeline stands under caption
        assert_eq!(layout.timeline.top, layout.caption.bottom + 12);
        assert_eq!(
            layout.timeline.bottom - layout.timeline.top,
            STRIP_UNITS as i32
        );
        // Times row under timeline
        assert_eq!(layout.times.top, layout.timeline.bottom + 10);
        // Silence checkbox on the left
        assert_eq!(layout.silence.left, 24);
        assert_eq!(layout.silence.top, layout.times.bottom + 14);
        // Play selection on the right
        assert_eq!(layout.play_selection.right, whole - 24);
        assert_eq!(layout.play_selection.top, layout.times.bottom + 14);
        // Hairline bottom and footer
        assert!(layout.silence.bottom <= layout.hairline_bottom.top);
        assert!(layout.run.right <= whole && layout.run.bottom <= side);
        STATE.with(|slot| slot.borrow_mut().audio = false);
    }
}

#[cfg(test)]
mod show_tests {
    use super::*;
    use crate::platform::windows::windowhost::screenshot::{capture, make_test_video};
    use windows::Win32::UI::WindowsAndMessaging::{
        DispatchMessageW, MSG, PM_REMOVE, PeekMessageW, TranslateMessage,
    };

    /// Showing the card and pumping the messages that come of it must not
    /// panic, even when there is nothing to ask the length of: the
    /// program's own loop does exactly this when a trim is picked.
    #[test]
    fn showing_and_pumping_the_card_does_not_panic() {
        crate::ui::wheel::install_platform();
        let (sender, _receiver) = std::sync::mpsc::channel();
        let mut window = TimelineWindow::new(sender).expect("the window was made");
        window.show_for(
            "tools.trim",
            "Trim",
            "Cut the beginning or the end",
            &[PathBuf::from("not-there.mp4")],
        );
        for _ in 0..40 {
            unsafe {
                let mut message = MSG::default();
                while PeekMessageW(&mut message, None, 0, 0, PM_REMOVE).as_bool() {
                    let _ = TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        window.hide();
    }

    /// Opens the trim window on a real video, lets it come in and draw,
    /// and captures what a person would see.
    #[test]
    #[ignore = "needs the conversion program; writes a screenshot"]
    fn capture_the_trim_window() {
        crate::ui::wheel::install_platform();
        let folder = std::env::temp_dir().join("open-tangerine-capture");
        std::fs::create_dir_all(&folder).expect("the capture folder was made");
        let video = make_test_video(&folder);
        let (sender, _receiver) = std::sync::mpsc::channel();
        let mut window = TimelineWindow::new(sender).expect("the window was made");
        window.show_for("tools.trim", "Trim", "Cut the beginning or the end", &[video]);
        // Long enough for the fade to finish, the first frame to land and
        // the strip's own moments to be asked for one after another: the
        // strip is a run of seeks in the background, and the shot wants
        // it there. The window's own bell serves the strip while the
        // messages are pumped; the shot waits only for a strip worn, and
        // gives the whole thing up after a while so a window without the
        // conversion program still takes its picture.
        for round in 0..2400 {
            unsafe {
                let mut message = MSG::default();
                while PeekMessageW(&mut message, None, 0, 0, PM_REMOVE).as_bool() {
                    let _ = TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
            if get_strip_worn() && round > 400 {
                break;
            }
        }
        let shots = std::path::Path::new("target").join("shots");
        std::fs::create_dir_all(&shots).expect("the shots folder was made");
        // The buffer itself, before any window machinery touches it: if
        // the footer is here, the drawing is right and the capture lies;
        // if not, the renderer is the one clipping.
        HOST.with(|slot| {
            let borrowed = slot.borrow();
            if let Some(host) = borrowed.as_ref() {
                let mut rgba = Vec::with_capacity(host.buffer.len() * 4);
                for pixel in &host.buffer {
                    let (_, r, g, b) = pixel.parts();
                    rgba.extend_from_slice(&[r, g, b, 255]);
                }
                let _ = image::save_buffer(
                    shots.join("trim-buffer.png"),
                    &rgba,
                    host.width,
                    host.height,
                    image::ExtendedColorType::Rgba8,
                );
            }
        });
        capture(window.window, &shots.join("trim-window.png"));
        window.hide();
        let _ = std::fs::remove_dir_all(&folder);
    }

    /// Showing the split card and pumping its messages must not panic
    /// either: the same loop the program runs when a split is picked.
    #[test]
    fn showing_and_pumping_the_split_card_does_not_panic() {
        crate::ui::wheel::install_platform();
        let (sender, _receiver) = std::sync::mpsc::channel();
        let mut window = TimelineWindow::new(sender).expect("the window was made");
        window.show_for(
            "tools.split",
            "Split",
            "Cut the video into clips",
            &[PathBuf::from("not-there.mp4")],
        );
        for _ in 0..40 {
            unsafe {
                let mut message = MSG::default();
                while PeekMessageW(&mut message, None, 0, 0, PM_REMOVE).as_bool() {
                    let _ = TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        window.hide();
    }

    /// Opens the split window on a real video, stands the cuts on it the
    /// way a person's hands would, and captures what they would see.
    #[test]
    #[ignore = "needs the conversion program; writes a screenshot"]
    fn capture_the_split_window() {
        crate::ui::wheel::install_platform();
        let folder = std::env::temp_dir().join("open-tangerine-capture");
        std::fs::create_dir_all(&folder).expect("the capture folder was made");
        let video = make_test_video(&folder);
        let (sender, _receiver) = std::sync::mpsc::channel();
        let mut window = TimelineWindow::new(sender).expect("the window was made");
        window.show_for("tools.split", "Split", "Cut the video into clips", &[video]);
        // The strip is a run of seeks behind the window, and the shot
        // wants it there; the pump below serves its timer.
        for round in 0..2400 {
            unsafe {
                let mut message = MSG::default();
                while PeekMessageW(&mut message, None, 0, 0, PM_REMOVE).as_bool() {
                    let _ = TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
            if get_strip_worn() && round > 400 {
                break;
            }
        }
        // The hands: one cut put where the playhead stands, a scrub to
        // where the next goes, and the equal row said for four parts —
        // which puts three markers on the strip and says how long every
        // clip lasts about.
        unsafe { add_split(window.window) };
        STATE.with(|slot| slot.borrow_mut().playhead = 1.5);
        STATE.with(|slot| slot.borrow_mut().parts = 4);
        unsafe { set_equal(window.window) };
        for _ in 0..60 {
            unsafe {
                let mut message = MSG::default();
                while PeekMessageW(&mut message, None, 0, 0, PM_REMOVE).as_bool() {
                    let _ = TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let shots = std::path::Path::new("target").join("shots");
        std::fs::create_dir_all(&shots).expect("the shots folder was made");
        HOST.with(|slot| {
            let borrowed = slot.borrow();
            if let Some(host) = borrowed.as_ref() {
                let mut rgba = Vec::with_capacity(host.buffer.len() * 4);
                for pixel in &host.buffer {
                    let (_, r, g, b) = pixel.parts();
                    rgba.extend_from_slice(&[r, g, b, 255]);
                }
                let _ = image::save_buffer(
                    shots.join("split-buffer.png"),
                    &rgba,
                    host.width,
                    host.height,
                    image::ExtendedColorType::Rgba8,
                );
            }
        });
        capture(window.window, &shots.join("split-window.png"));
        window.hide();
        let _ = std::fs::remove_dir_all(&folder);
    }

    /// Makes a 2-second WAV file with two bursts of sound matching trimAudio.webp.
    fn make_test_audio(folder: &std::path::Path) -> PathBuf {
        let rate = 8000u32;
        let duration = 2.0f32;
        let sample_count = (duration * rate as f32) as u32;
        let data_len = sample_count * 2;
        let mut wav = Vec::with_capacity(44 + data_len as usize);
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&(36 + data_len).to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes()); // PCM
        wav.extend_from_slice(&1u16.to_le_bytes()); // Mono
        wav.extend_from_slice(&rate.to_le_bytes());
        wav.extend_from_slice(&(rate * 2).to_le_bytes());
        wav.extend_from_slice(&2u16.to_le_bytes());
        wav.extend_from_slice(&16u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&data_len.to_le_bytes());
        for i in 0..sample_count {
            let t = i as f32 / rate as f32;
            // Two bursts: 0.35s..0.85s and 1.15s..1.65s (matching trimAudio.webp)
            let active = (0.35..0.85).contains(&t) || (1.15..1.65).contains(&t);
            let sample: i16 = if active {
                if (i / 8) % 2 == 0 { 24000 } else { -24000 }
            } else {
                0
            };
            wav.extend_from_slice(&sample.to_le_bytes());
        }
        let path = folder.join("Voice with pauses.wav");
        std::fs::write(&path, &wav).expect("test audio written");
        path
    }

    /// Showing the audio trim card and pumping its messages must not panic.
    #[test]
    fn showing_and_pumping_the_audio_trim_card_does_not_panic() {
        crate::ui::wheel::install_platform();
        let (sender, _receiver) = std::sync::mpsc::channel();
        let mut window = TimelineWindow::new(sender).expect("the window was made");
        window.show_for(
            "tools.trim",
            "Trim",
            "Keeps the part you choose, trims silences.",
            &[PathBuf::from("not-there.wav")],
        );
        for _ in 0..40 {
            unsafe {
                let mut message = MSG::default();
                while PeekMessageW(&mut message, None, 0, 0, PM_REMOVE).as_bool() {
                    let _ = TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        window.hide();
    }

    /// Opens the trim window on a real audio file, lets waveform render,
    /// and captures target/shots/trim-audio-window.png.
    #[test]
    #[ignore = "needs the conversion program; writes a screenshot"]
    fn capture_the_trim_audio_window() {
        crate::ui::wheel::install_platform();
        let folder = std::env::temp_dir().join("open-tangerine-capture-audio");
        std::fs::create_dir_all(&folder).expect("the capture folder was made");
        let audio = make_test_audio(&folder);
        let (sender, _receiver) = std::sync::mpsc::channel();
        let mut window = TimelineWindow::new(sender).expect("the window was made");
        window.show_for(
            "tools.trim",
            "Trim",
            "Keeps the part you choose, trims silences.",
            &[audio],
        );
        // Wait for waveform strip to be built in background
        for round in 0..2400 {
            unsafe {
                let mut message = MSG::default();
                while PeekMessageW(&mut message, None, 0, 0, PM_REMOVE).as_bool() {
                    let _ = TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
            if get_strip_worn() && round > 400 {
                break;
            }
        }
        let shots = std::path::Path::new("target").join("shots");
        std::fs::create_dir_all(&shots).expect("the shots folder was made");
        HOST.with(|slot| {
            let borrowed = slot.borrow();
            if let Some(host) = borrowed.as_ref() {
                let mut rgba = Vec::with_capacity(host.buffer.len() * 4);
                for pixel in &host.buffer {
                    let (_, r, g, b) = pixel.parts();
                    rgba.extend_from_slice(&[r, g, b, 255]);
                }
                let _ = image::save_buffer(
                    shots.join("trim-audio-buffer.png"),
                    &rgba,
                    host.width,
                    host.height,
                    image::ExtendedColorType::Rgba8,
                );
            }
        });
        capture(window.window, &shots.join("trim-audio-window.png"));
        window.hide();
        let _ = std::fs::remove_dir_all(&folder);
    }
}
