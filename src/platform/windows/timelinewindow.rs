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
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::Sender;

use slint::platform::software_renderer::MinimalSoftwareWindow;
use slint::platform::{PointerEventButton, WindowAdapter, WindowEvent};
use slint::{LogicalPosition, PhysicalSize, SharedPixelBuffer};
use std::ffi::OsString;
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{BeginPaint, CreateSolidBrush, InvalidateRect};
use windows::Win32::Media::Multimedia::mciSendStringW;
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
use crate::media::runner::{ProcessRunner, SystemRunner};
use crate::platform::windows::toolwindow::ToolEvent;
use crate::platform::windows::windowhost::{
    fixed_style, fraction, scale_factor, screen_scale, timecode, to_wide,
};
use crate::radial::execution::ToolSettings;
use crate::ui::wheel::{window_for_next_component, Bgra8, Panel, ShotData, TrimCard};

/// The timer the window comes in softly on.
const FADE_TIMER_ID: usize = 52;

/// The timer the strip is waited on with.
const STRIP_TIMER_ID: usize = 54;

/// The timer the play of the preview is walked by: a step of the video
/// is shown once every tick of it, and it is let go of when the walk
/// ends or the person takes the playhead back.
const PLAY_TIMER_ID: usize = 55;

/// How much video a step of the play is, and how often it is taken: a
/// quarter of a second of the video every quarter of a second, the pace
/// of the video itself. Every step is a seek of the conversion program,
/// so the steps are no smaller than a seek is worth.
const PLAY_TICK_MS: u32 = 250;
const PLAY_STEP: f64 = 0.25;

/// The card's width, in the drawing's own units.
const CARD_W: i32 = 420;
/// The least a preview may be tall, so a landscape video
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

/// What a chip of a taken frame is high and how far two stand apart, in
/// the drawing's own units. The card draws its chips with the same
/// measure — two in a row, and the rows under each other — so the two
/// must agree.
const SHOT_CHIP_TOP_UNITS: f32 = 84.0;
const SHOT_CHIP_GAP_UNITS: f32 = 8.0;

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

/// A frame the card has taken: the moment it stands at, and the chip's
/// own little picture of it. The full picture is not kept; the run
/// takes the moments to the handler, which asks for the frames again
/// where they stand.
struct Shot {
    at: f64,
    picture: slint::Image,
}

/// What the window is open for, and where the video stands in it. A trim
/// holds the two ends of a stretch; a split holds the points the video is
/// cut at and the count of its equal parts; a snapshots card holds the
/// frames that were taken.
struct TrimState {
    duration: f64,
    start: f64,
    end: f64,
    playhead: f64,
    dragging: Drag,
    /// Whether the preview is being played right now: the play's own
    /// bell walks the playhead a step every tick, until the card's out
    /// point says no more or the person takes the playhead back.
    playing: bool,
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
    /// Whether this window is asked for snapshots, not a trim: the strip
    /// is scrubbed for moments to take, and what was taken stands where
    /// an equal row would stand.
    shots: bool,
    /// The frames taken, in the order they were taken. The chips row
    /// wears them, and the run says their moments to the handler.
    shots_taken: Vec<Shot>,
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
    /// The name the sound of an audio play is spoken under, in the
    /// media control interface. None says nothing is being played.
    mci_alias: Option<String>,
    /// The cut-out wav an audio play speaks from. It is taken back when
    /// the play is put down.
    play_sound: Option<PathBuf>,
}

impl Default for TrimState {
    fn default() -> Self {
        Self {
            duration: 0.0,
            start: 0.0,
            end: 0.0,
            playhead: 0.0,
            dragging: Drag::Nothing,
            playing: false,
            frame_at: f64::NEG_INFINITY,
            preview_units: 236.0,
            ffmpeg: None,
            input: PathBuf::new(),
            split: false,
            shots: false,
            shots_taken: Vec::new(),
            audio: false,
            trim_silence: false,
            silence: None,
            points: Vec::new(),
            parts: 2,
            equal_saying: None,
            mci_alias: None,
            play_sound: None,
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
    // A cancellation token for the background strip worker.
    static STRIP_CANCEL: std::cell::RefCell<Option<std::sync::Arc<std::sync::atomic::AtomicBool>>> =
        const { std::cell::RefCell::new(None) };
    // Whether a strip has been worn: the judgement the capture check
    // waits on, answered without touching the strip's own inbox.
    static STRIP_WORN: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Cancels any running strip generation worker.
fn cancel_strip_worker() {
    STRIP_CANCEL.with(|slot| {
        if let Some(cancel) = slot.borrow().as_ref() {
            cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        }
    });
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
    cancel_strip_worker();
    let cancel_token = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let cancel_thread = cancel_token.clone();
    STRIP_CANCEL.with(|slot| {
        *slot.borrow_mut() = Some(cancel_token);
    });

    let (sender, receiver) = std::sync::mpsc::channel::<StripPiece>();
    std::thread::spawn(move || {
        if is_audio {
            let (samples, rate) =
                crate::media::audio::pcm_samples(&tools, &input).unwrap_or_default();
            if cancel_thread.load(std::sync::atomic::Ordering::Relaxed) {
                return;
            }
            let buckets = ((width as f32 / 4.0).round() as usize).clamp(50, 200);
            let peaks = crate::media::audio::waveform_peaks(&samples, buckets);
            let strip = crate::media::audio::render_waveform_strip(&peaks, width, height);
            let (wide, tall) = (strip.width(), strip.height());
            let silence = crate::media::audio::detect_silence(&samples, rate, 0.01);
            if !cancel_thread.load(std::sync::atomic::Ordering::Relaxed) {
                let _ = sender.send((strip.into_raw(), wide, tall, silence));
            }
        } else {
            let tiles = ((width as f32 / PILLAR_UNITS).round() as u32).clamp(6, 30);
            let tile_w = (width / tiles).max(1);
            let tile_h = height.max(1);
            let step = (duration / tiles as f64).max(0.05);
            let mut strip = image::RgbaImage::new(tile_w * tiles, tile_h);
            for index in 0..tiles {
                if cancel_thread.load(std::sync::atomic::Ordering::Relaxed) {
                    return;
                }
                let moment = ((index as f64 + 0.5) * step).min(duration);
                let Some(frame) = frame_at(&tools, &input, moment) else {
                    continue;
                };
                let pillar = pillar_of(&frame, tile_w, tile_h);
                image::imageops::overlay(&mut strip, &pillar, (index * tile_w) as i64, 0);
            }
            if !cancel_thread.load(std::sync::atomic::Ordering::Relaxed) {
                let (wide, tall) = (strip.width(), strip.height());
                let _ = sender.send((strip.into_raw(), wide, tall, None));
            }
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
    } else if is_shots() {
        Layout::SHOTS_BASE_CONTENT_UNITS + chips_units()
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

/// Whether the window is asked for snapshots and not a trim: the strip
/// is scrubbed and the frames taken stand in the chips row, where an
/// equal row would stand.
fn is_shots() -> bool {
    STATE.with(|slot| slot.borrow().shots)
}

/// How much room the rows of taken frames ask for, in the drawing's own
/// units: every row carries two chips and ends with the air between the
/// rows, and nothing is asked when no frame has been taken.
fn chips_units() -> f32 {
    let taken = STATE.with(|slot| slot.borrow().shots_taken.len());
    if taken == 0 {
        0.0
    } else {
        let rows = ((taken + 1) / 2) as f32;
        rows * (SHOT_CHIP_TOP_UNITS + SHOT_CHIP_GAP_UNITS) - SHOT_CHIP_GAP_UNITS
    }
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
                    unsafe {
                        if is_shots() {
                            add_shot(window);
                        } else {
                            add_split(window);
                        }
                    }
                }
            });
            card.on_remove_shot_chosen(move |index| {
                let window = window_handle();
                if !window.0.is_null() {
                    unsafe { remove_shot_at(window, index) };
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
        let shots = operation == "tools.snapshots";
        let is_audio = inputs
            .first()
            .and_then(|p| p.extension())
            .and_then(|e| e.to_str())
            .map(crate::radial::selection::FileKind::from_extension)
            == Some(crate::radial::selection::FileKind::Audio);

        // The window is titled as the reference designs title their
        // cards: a trim says what it trims, a split and a snapshots
        // card say the film they work on.
        let label = if operation == "tools.trim" {
            if is_audio { "Trim Audio" } else { "Trim Video" }
        } else if operation == "tools.split" {
            "Split Video"
        } else if operation == "tools.snapshots" {
            "Video Snapshots"
        } else {
            label
        };

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
                playing: false,
                frame_at: 0.0,
                preview_units,
                ffmpeg: ffmpeg.clone(),
                input: input.clone(),
                split,
                shots,
                audio: is_audio,
                trim_silence: false,
                silence: None,
                points: Vec::new(),
                parts: PARTS_MIN,
                equal_saying: None,
                shots_taken: Vec::new(),
                mci_alias: None,
                play_sound: None,
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
            host.card.set_shots_mode(shots);
            host.card.set_audio_mode(is_audio);
            host.card.set_run_label(
                if split {
                    "Split Video"
                } else if is_audio {
                    "Apply"
                } else if shots {
                    "Save Frames"
                } else {
                    "Trim"
                }
                .into(),
            );
            host.card.set_points_text("0 taken".into());
            host.card.set_shots(
                std::rc::Rc::new(slint::VecModel::from(Vec::<ShotData>::new())).into(),
            );
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
            let _ = ShowWindow(card_window, SW_SHOW);
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
            let _ = ShowWindow(self.window, SW_HIDE);
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
        self.card.set_shots_box(panel(layout.shots));
        self.card.set_silence_box(panel(layout.silence));
        self.card.set_play_selection_box(panel(layout.play_selection));
        self.card.set_audio_mode(is_audio());
        self.card.set_split_mode(is_split());
        self.card.set_shots_mode(is_shots());
        self.card.set_trim_silence(STATE.with(|s| s.borrow().trim_silence));
        layout
    }

    /// Draws the card if it changed, and puts what was drawn on the
    /// window.
    fn draw(&mut self) {
        if self.width == 0 || self.height == 0 {
            return;
        }
        let needed = (self.width * self.height) as usize;
        let mut buffer = std::mem::take(&mut self.buffer);
        if buffer.len() != needed {
            buffer = vec![Bgra8::default(); needed];
        }
        let stride = self.width as usize;
        self.software
            .draw_if_needed(|renderer| {
                renderer.render(&mut buffer, stride);
            });
        self.buffer = buffer;
        unsafe { self.put_on_window() };
    }

    /// Puts the pixels of the last drawing on the window.
    unsafe fn put_on_window(&self) {
        crate::platform::windows::windowhost::blit_card_buffer(
            self.window,
            &self.buffer,
            self.width,
            self.height,
        );
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
    // The row of the frames taken, where an equal row would stand.
    shots: RECT,
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
    /// The same for a snapshots card: its points row stands where a
    /// split's does, and after the transport the chips row takes the
    /// equal row's room — its own height when frames stand, nothing
    /// when none are taken yet.
    const SHOTS_BASE_CONTENT_UNITS: f32 = 260.0;
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

    /// How tall a snapshots window of this preview and this much room
    /// for chips wants to be.
    fn height_for_shots(preview_units: f32, chips_units: f32) -> f32 {
        Self::SHOTS_BASE_CONTENT_UNITS + chips_units + preview_units + Self::FOOTER_UNITS
    }

    fn of(whole: i32, side: i32, scale: f32) -> Layout {
        let at = |value: f32| (value * scale) as i32;
        let margin = at(24.0);
        let preview_units = client_preview_units();
        let split = is_split();
        let audio = is_audio();
        let shots = is_shots();

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
        let timeline_top = if split || shots {
            caption_top + at(30.0) + at(12.0)
        } else {
            caption_top + at(16.0) + at(12.0)
        };
        let (times_top, transport_top) = if split || shots || audio {
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
        } else if shots {
            // The chips row stands where an equal row would: as tall as
            // the frames taken ask for, and the air under it.
            equal_top + at(chips_units()) + at(18.0)
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
                bottom: caption_top + if split || shots { at(30.0) } else { at(16.0) },
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
            shots: RECT::default(),
            silence: RECT::default(),
            play_selection: RECT::default(),
        };

        if split || shots {
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
        }

        if split {
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

        if shots {
            // The chips row after the transport: the whole width, as
            // tall as the frames taken ask for.
            layout.shots = RECT {
                left: margin,
                top: equal_top,
                right: whole - margin,
                bottom: equal_top + at(chips_units()),
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
    let shots = is_shots();
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
            } else if shots {
                // 0 the playhead (no ring), 1 the taking pill, 2 run.
                host.card.set_handle_focus(0);
                host.card
                    .set_button_focus(if index == 2 { 2 } else { 0 });
                host.card
                    .set_pill_focus(if index == 1 { 1 } else { 0 });
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
/// playhead, the two pills, and go; a snapshots card walks the playhead,
/// the taking pill, and go.
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
/// its stretch; a split card says how many cuts stand and wears them; a
/// snapshots card says how many frames stand and wears their chips.
fn refresh_times() {
    let split = is_split();
    let shots = is_shots();
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
                if shots {
                    let said = match state.shots_taken.len() {
                        0 => "0 taken".to_string(),
                        1 => "1 taken".to_string(),
                        n => format!("{n} taken"),
                    };
                    host.card.set_points_text(said.into());
                    host.card.set_run_enabled(!state.shots_taken.is_empty());
                    // The go pill says what the run does, and says it for
                    // the count: one frame or many, the way the mockup's
                    // footer carries it.
                    host.card.set_run_label(match state.shots_taken.len() {
                        0 => "Save Frames".to_string(),
                        1 => "Save 1 Frame".to_string(),
                        n => format!("Save {n} Frames"),
                    }.into());
                    let chips: Vec<ShotData> = state
                        .shots_taken
                        .iter()
                        .map(|shot| ShotData {
                            picture: shot.picture.clone(),
                            time: timecode(shot.at).into(),
                        })
                        .collect();
                    host.card
                        .set_shots(std::rc::Rc::new(slint::VecModel::from(chips)).into());
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
    let along = |moment: f64, duration: f64| {
        bar.left + (fraction(moment, duration) * (bar.right - bar.left) as f32) as i32
    };
    // The length of the whole is read once, before anything of the
    // state is held: the closures below stand inside the held card,
    // and a borrow that asks again there would answer with a panic
    // that lets the window fall with everything else.
    let duration = STATE.with(|slot| slot.borrow().duration);

    if pressed {
        STATE.with(|slot| {
            let mut state = slot.borrow_mut();
            if state.duration <= 0.0 {
                return;
            }
            if state.split || state.shots {
                // A split takes no ends: a press on a marker stands the
                // playhead on the cut it took, anywhere else takes the
                // playhead itself. A snapshots card wears the same
                // straightness: its strip is only scrubbed.
                let on_marker = state
                    .points
                    .iter()
                    .any(|&point| (x - along(point, duration)).abs() <= at(MARKER_REACH as f32));
                state.dragging = if on_marker {
                    Drag::Nothing
                } else {
                    Drag::Playhead
                };
            } else {
                let take_start = (x - along(state.start, duration)).abs() <= at(HANDLE_REACH as f32);
                let take_end = (x - along(state.end, duration)).abs() <= at(HANDLE_REACH as f32);
                state.dragging = if take_start && !take_end {
                    Drag::Start
                } else if take_end {
                    Drag::End
                } else {
                    Drag::Playhead
                };
            }
        });
        // Whatever the press took, the person is taking the pointer's own
        // turn at where things stand: a play that was walking stands down.
        unsafe { halt_play(window) };
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
            Drag::Start => {
                state.start = moment.min(state.end - MIN_SPAN).max(0.0);
                // The preview follows the cap being dragged: what the
                // playhead stands on is what the card would take, so the
                // person can see the cut they are making.
                state.playhead = state.start;
            }
            Drag::End => {
                state.end = moment.max(state.start + MIN_SPAN).min(state.duration);
                state.playhead = state.end;
            }
            Drag::Playhead => state.playhead = moment,
            Drag::Nothing => {}
        }
        true
    });
    if moved {
        let scrubbing = STATE.with(|slot| slot.borrow().dragging) != Drag::Nothing;
        if scrubbing {
            refresh_frame(false);
        }
        refresh_times();
        let _ = unsafe { InvalidateRect(Some(window), None, true) };
    }
    // A press that took a marker stands the playhead on the cut it took:
    // the pointer need not hold on to anything to say where to go.
    if pressed {
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
/// Plays the preview from where the playhead stands, a step every tick
/// of the play's bell, until the card's out point; playing again stops
/// the walk where it stands. A walk that stands at or past the out point
/// starts over at the card's in point, so there is something to show.
unsafe fn toggle_play(window: HWND) {
    let (started, halted) = STATE.with(|slot| {
        let mut state = slot.borrow_mut();
        if state.playing {
            state.playing = false;
            return (false, true);
        }
        if state.duration <= 0.0 {
            return (false, false);
        }
        if state.playhead >= state.end - 0.001 {
            state.playhead = state.start.max(0.0);
        }
        state.playing = true;
        (true, false)
    });
    if started {
        unsafe { let _ = SetTimer(Some(window), PLAY_TIMER_ID, PLAY_TICK_MS, None); };
    }
    if halted {
        unsafe { let _ = KillTimer(Some(window), PLAY_TIMER_ID); };
    }
    unsafe { let _ = InvalidateRect(Some(window), None, true); };
}

/// Stops the walk of the preview where it stands, if one is walking.
/// Everything that takes the playhead back from the play - a press on
/// the bar, the go, the no - says this first. The sound of an audio
/// card's play goes with the walk.
unsafe fn halt_play(window: HWND) {
    let was = STATE.with(|slot| {
        let was = slot.borrow().playing;
        slot.borrow_mut().playing = false;
        was
    });
    if was {
        unsafe { let _ = KillTimer(Some(window), PLAY_TIMER_ID); };
    }
    close_play_sound();
}

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
        if state.split || state.shots {
            state.playhead = (state.playhead + by).clamp(0.0, state.duration);
        } else {
            match focused {
                0 => {
                    state.start = (state.start + by).clamp(0.0, state.end - MIN_SPAN);
                    // The steps show what they step: the playhead stands
                    // on the cap being moved, the way a drag of it does.
                    state.playhead = state.start;
                }
                1 => {
                    state.end = (state.end + by).clamp(state.start + MIN_SPAN, state.duration);
                    state.playhead = state.end;
                }
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
/// Says one sentence to the media control interface, or says the code
/// it answered with.
///
/// The playing of a sound is spoken in sentences: an open, a play, a
/// stop, a close. An answer of zero means it was taken; any other
/// answer is the interface saying no.
fn mci_say(sentence: &str) -> Result<(), u32> {
    let mut words: Vec<u16> = sentence.encode_utf16().collect();
    words.push(0);
    let code = unsafe {
        mciSendStringW(
            PCWSTR::from_raw(words.as_ptr()),
            None,
            None,
        )
    };
    if code == 0 {
        Ok(())
    } else {
        Err(code)
    }
}

/// Asks the media control interface a question, and brings its answer
/// back. None says the interface said no, or nothing.
#[cfg(test)]
fn mci_ask(sentence: &str) -> Option<String> {
    let mut asked: Vec<u16> = sentence.encode_utf16().collect();
    asked.push(0);
    // The answer is asked of the interface into a held-out row of
    // words; an answer begins with zero code and ends with a nothing
    // word.
    let mut answer = [0u16; 160];
    let code = unsafe {
        mciSendStringW(
            PCWSTR::from_raw(asked.as_ptr()),
            Some(&mut answer),
            None,
        )
    };
    if code != 0 {
        return None;
    }
    let spoken = answer
        .iter()
        .copied()
        .take_while(|word| *word != 0)
        .collect::<Vec<u16>>();
    String::from_utf16(&spoken).ok()
}

/// A name of the sound's own in the media control interface: one play's
/// own, and only that play's. The names are counted, because the
/// interface hears every window's plays the same.
static PLAY_VOICES: AtomicU64 = AtomicU64::new(0);

/// Takes the playing sound back: the interface is told to stop and put
/// the name down, and the cut the sound spoke from is taken away.
///
/// Doing nothing when nothing was played is part of the work: every
/// putting-down of a walk says this, whichever kind of card it walked.
fn close_play_sound() {
    let (alias, cut) = STATE.with(|slot| {
        let mut state = slot.borrow_mut();
        (
            state.mci_alias.take(),
            state.play_sound.take(),
        )
    });
    if let Some(alias) = alias {
        let _ = mci_say(&format!("stop {alias}"));
        let _ = mci_say(&format!("close {alias}"));
    }
    if let Some(cut) = cut {
        let _ = std::fs::remove_file(&cut);
    }
}

/// Plays the card's chosen stretch of a sound, and walks the playhead
/// with the sound.
///
/// Saying play again puts the play down, the sound with it. The stretch
/// is cut out of the file into a wav of the play's own first (so every
/// kind of sound a card can stand can be heard), and the media control
/// interface speaks the wav; the walk — the same walk the frames of a
/// video take — carries the minutes row along while the sound stands,
/// and puts itself down at the stretch's out end.
///
/// The cut is real work the window waits on: a moment, the way the
/// card's own coming in was.
unsafe fn play_selection(window: HWND) {
    // Saying play again is saying stop: the play is put down, the
    // sound with it.
    let was = STATE.with(|slot| slot.borrow().playing);
    if was {
        unsafe { halt_play(window) };
        return;
    }

    // Only a card of sound has a play to say, and only when it has a
    // stretch said of it and the conversion program to cut with.
    let job = STATE.with(|slot| {
        let state = slot.borrow();
        if !state.audio
            || state.ffmpeg.is_none()
            || state.input.as_os_str().is_empty()
            || state.duration <= 0.0
            || state.end <= state.start
        {
            return None;
        }
        Some((
            state.ffmpeg.clone().expect("the tools stood"),
            state.input.clone(),
            state.start,
            state.end,
        ))
    });
    let Some((tools, input, from, to)) = job else {
        // No stretch to play: the card stands still, and says nothing.
        return;
    };

    // The stretch is cut into a wav of the play's own, near nothing the
    // user owns: a work file of the temp folder, named with this
    // program's id and the play's count, so two plays of two windows
    // can never pour into one another's bucket. A play's cut is taken
    // back when its play is put down.
    let voice = PLAY_VOICES.fetch_add(1, Ordering::Relaxed);
    let cut = std::env::temp_dir().join(format!(
        "tangerine-trim-play-{}.{}.wav",
        std::process::id(),
        voice
    ));
    let alias = format!("tangerine_play_{}.{}", std::process::id(), voice);
    let span_ms = ((to - from) * 1000.0).round().max(1.0) as i64;
    let from_s = format!("{:.3}", from.max(0.0));
    let span_s = format!("{:.3}", (to - from).max(0.001));
    let input_name = input.to_string_lossy().into_owned();
    let cut_name = cut.to_string_lossy().into_owned();
    let arguments = [
        "-hide_banner",
        "-nostdin",
        "-y",
        "-ss",
        &from_s,
        "-i",
        &input_name,
        "-t",
        &span_s,
        "-vn",
        "-c:a",
        "pcm_s16le",
        &cut_name,
    ]
    .iter()
    .map(|word| OsString::from(*word))
    .collect::<Vec<OsString>>();
    let made = SystemRunner.run(&tools.ffmpeg, &arguments, None);
    match made {
        Ok(answer) if answer.succeeded() => {}
        Ok(answer) => {
            crate::diagnostics::log(
                crate::diagnostics::Level::Warn,
                "action",
                &format!(
                    "the stretch could not be cut to be played: the program said {}",
                    answer.status.map(|code| code.to_string()).unwrap_or_else(|| "no answer".into())
                ),
            );
            let _ = std::fs::remove_file(&cut);
            return;
        }
        Err(reason) => {
            crate::diagnostics::log(
                crate::diagnostics::Level::Warn,
                "action",
                &format!("the stretch could not be cut to be played: {reason}"),
            );
            let _ = std::fs::remove_file(&cut);
            return;
        }
    }

    // The sound is spoken under the play's own name. Where no wave
    // device stands to hear it, the say says so, and the walk still
    // runs: the play of a card is the preview of it, and the minutes
    // row is what it carries.
    STATE.with(|slot| {
        let mut state = slot.borrow_mut();
        state.mci_alias = Some(alias.clone());
        state.play_sound = Some(cut.clone());
    });
    let open = mci_say(&format!(
        "open \"{}\" type waveaudio alias {alias}",
        cut.display()
    ))
    .and_then(|_| mci_say(&format!("play {alias} from 0 to {span_ms}")));
    if let Err(code) = open {
        crate::diagnostics::log(
            crate::diagnostics::Level::Warn,
            "action",
            &format!("the play could not be heard: mci said {code}"),
        );
        close_play_sound();
    }

    // The walk of it: stood at the in point, walking, the same bell the
    // frames walk by.
    STATE.with(|slot| {
        let mut state = slot.borrow_mut();
        state.playhead = from.max(0.0);
        state.playing = true;
    });
    refresh_frame(false);
    refresh_times();
    unsafe { let _ = SetTimer(Some(window), PLAY_TIMER_ID, PLAY_TICK_MS, None); };
    unsafe { let _ = InvalidateRect(Some(window), None, true); };
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

/// Takes a frame of the video where the playhead stands, to stand in the
/// chips row. The chip wears a small picture of the frame; the running
/// of the card later says only where the frame stands, and the handler
/// asks for them again where they belong.
unsafe fn add_shot(window: HWND) {
    let made = STATE.with(|slot| {
        let mut state = slot.borrow_mut();
        if state.duration <= 0.0 {
            return false;
        }
        let at = state.playhead;
        // The frame is asked for at the moment it stands at. The card is
        // not held here; the picture arrives whole or nothing says so.
        let Some(took) = state
            .ffmpeg
            .as_ref()
            .and_then(|tools| frame_at(tools, &state.input, at))
        else {
            return false;
        };
        let small = took.thumbnail(160, 60);
        let rgba = small.to_rgba8();
        let buffer =
            SharedPixelBuffer::clone_from_slice(rgba.as_raw(), small.width(), small.height());
        state.shots_taken.push(Shot {
            at,
            picture: slint::Image::from_rgba8(buffer),
        });
        true
    });
    if made {
        unsafe { refit(window) };
        refresh_times();
        let _ = unsafe { InvalidateRect(Some(window), None, true) };
    }
}

/// Takes away the taken frame nearest the playhead, when one stands
/// near enough — the Delete key's way of undoing a taking.
unsafe fn take_shot_near(window: HWND) {
    let took = STATE.with(|slot| {
        let mut state = slot.borrow_mut();
        let stand = state.playhead;
        let share = state
            .shots_taken
            .iter()
            .enumerate()
            .min_by(|(_, a), (_, b)| {
                let da = (a.at - stand).abs();
                let db = (b.at - stand).abs();
                da.partial_cmp(&db).expect("the moments are not nothing")
            });
        match share {
            Some((index, shot)) if (shot.at - stand).abs() <= DELETE_REACH => {
                state.shots_taken.remove(index);
                true
            }
            _ => false,
        }
    });
    if took {
        unsafe { refit(window) };
        refresh_times();
        let _ = unsafe { InvalidateRect(Some(window), None, true) };
    }
}

/// Takes away the chip the person pressed its cross on.
unsafe fn remove_shot_at(window: HWND, index: i32) {
    let took = STATE.with(|slot| {
        let index = index as usize;
        let mut state = slot.borrow_mut();
        if index < state.shots_taken.len() {
            state.shots_taken.remove(index);
            true
        } else {
            false
        }
    });
    if took {
        unsafe { refit(window) };
        refresh_times();
        let _ = unsafe { InvalidateRect(Some(window), None, true) };
    }
}

/// Makes the window the size its card wants again: a frame taken makes
/// the card taller, the same way a redact card grows with its rows. The
/// window stands where it stood.
unsafe fn refit(window: HWND) {
    let scale = scale_factor(window);
    let (outer_w, outer_h) = outer_size(scale);
    let mut stands = RECT::default();
    let _ = unsafe { GetWindowRect(window, &mut stands) };
    let _ = unsafe {
        SetWindowPos(
            window,
            None,
            stands.left,
            stands.top,
            outer_w,
            outer_h,
            SWP_NOZORDER | SWP_NOACTIVATE,
        )
    };
    HOST.with(|slot| {
        let mut borrowed = slot.borrow_mut();
        if let Some(host) = borrowed.as_mut() {
            host.resize();
            host.software.request_redraw();
        }
    });
    let _ = unsafe { InvalidateRect(Some(window), None, true) };
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
            // buttons. The telling is not done while the host is held:
            // a button under the pointer answers the moment the card
            // hears of it, and its answer steps the card or flips a rail
            // of its own, which reads the host again — and asking for a
            // borrow that is already held falls with everything else.
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
            let told = HOST.with(|slot| {
                let borrowed = slot.borrow();
                borrowed.as_ref().map(|host| (host.software.clone(), host.window))
            });
            let Some((software, told_window)) = told else {
                return LRESULT(0);
            };
            software.window().dispatch_event(event);
            software.request_redraw();
            draw_window(told_window);
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
                } else if is_shots() && key == VK_SPACE.0 {
                    let stop = FOCUS_INDEX.with(|slot| slot.get());
                    match stop {
                        0 | 1 => unsafe { add_shot(window) },
                        _ => unsafe { run(window) },
                    }
                } else if is_split() && key == VK_SPACE.0 {
                    let stop = FOCUS_INDEX.with(|slot| slot.get());
                    match stop {
                        0 | 1 => unsafe { add_split(window) },
                        2 => unsafe { set_equal(window) },
                        _ => unsafe { run(window) },
                    }
                } else if key == VK_RETURN.0 {
                    // Enter says go on every card, as it always did.
                    unsafe { run(window) };
                } else {
                    // A trim of a video: Space plays the preview where it
                    // stands, the way a player does, and playing again
                    // stops the walk. The go of the card is said with
                    // Enter alone.
                    unsafe { toggle_play(window) };
                }
                LRESULT(0)
            } else if key == VK_ESCAPE.0 {
                unsafe { cancel(window) };
                LRESULT(0)
            } else if key == VK_DELETE.0 && (is_split() || is_shots()) {
                if is_shots() {
                    unsafe { take_shot_near(window) };
                } else {
                    unsafe { take_point_near(window) };
                }
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
            } else if wparam.0 == PLAY_TIMER_ID {
                // A step of the walk: the playhead is stood a step on, and
                // the frame of it comes with the times said of it. The
                // walk ends at the card's out point; the bell is let go of
                // and the playhead stays where the walk got to.
                let walking = STATE.with(|slot| {
                    let mut state = slot.borrow_mut();
                    if !state.playing || state.duration <= 0.0 {
                        state.playing = false;
                        return false;
                    }
                    state.playhead = (state.playhead + PLAY_STEP).min(state.end);
                    state.playing = state.playhead < state.end;
                    true
                });
                if walking {
                    refresh_frame(false);
                    refresh_times();
                }
                if !STATE.with(|slot| slot.borrow().playing) {
                    // The walk is done; the sound it walked by goes with
                    // it — the say of an audio play is taken back here
                    // when it walked to its own end.
                    unsafe { let _ = KillTimer(Some(window), PLAY_TIMER_ID); };
                    close_play_sound();
                }
                unsafe { let _ = InvalidateRect(Some(window), None, true); };
            }
            LRESULT(0)
        }
        WM_ERASEBKGND => LRESULT(1),
        WM_CLOSE => {
            cancel_strip_worker();
            send(ToolEvent::Cancelled);
            unsafe {
                let _ = ShowWindow(window, SW_HIDE);
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
    // The person said go: a play that was walking stands down.
    unsafe { halt_play(window) };
    cancel_strip_worker();
    let (split, shots, empty_points) = STATE.with(|slot| {
        let state = slot.borrow();
        let empty = if state.shots {
            state.shots_taken.is_empty()
        } else {
            state.points.is_empty()
        };
        (state.split, state.shots, empty)
    });
    // A split with no cut yet has nothing to say: the go button is
    // already retired, and the key says nothing either. So does a run
    // with no frame taken.
    if (split || shots) && empty_points {
        return;
    }
    let settings = STATE.with(|slot| {
        let state = slot.borrow();
        if split {
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
        } else {
            ToolSettings::default()
        }
    });
    // The run of a snapshots card carries the moments the frames stand
    // at, from the first to the last; the handler numbers the pictures
    // in the video's own order.
    let settings = if shots {
        let mut moments: Vec<u64> = STATE.with(|slot| {
            slot.borrow()
                .shots_taken
                .iter()
                .map(|shot| (shot.at * 1000.0).round() as u64)
                .collect()
        });
        moments.sort_unstable();
        ToolSettings {
            snapshots_ms: moments,
            ..Default::default()
        }
    } else {
        settings
    };
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
    unsafe { let _ = ShowWindow(window, SW_HIDE); };
}

/// Says no to what the window was open for, and puts it away.
unsafe fn cancel(window: HWND) {
    unsafe { halt_play(window) };
    cancel_strip_worker();
    send(ToolEvent::Cancelled);
    unsafe { let _ = ShowWindow(window, SW_HIDE); };
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

    /// A chip's picture of one pixel, for the tests of the chips row.
    fn blank_shot_picture() -> slint::Image {
        let buffer = SharedPixelBuffer::clone_from_slice(&[0u8, 0, 255, 255], 1, 1);
        slint::Image::from_rgba8(buffer)
    }

    #[test]
    fn the_height_of_a_shots_window_is_its_content_and_its_chips() {
        // Nothing taken yet: the chips row takes no room.
        assert_eq!(Layout::height_for_shots(236.0, 0.0), 560.0);
        // One row of two chips: its own height, once.
        assert_eq!(Layout::height_for_shots(236.0, 92.0), 652.0);
    }

    #[test]
    fn a_shots_layout_stands_the_rows_the_card_says() {
        STATE.with(|slot| {
            let mut s = slot.borrow_mut();
            s.preview_units = 236.0;
            s.shots = true;
            s.shots_taken = vec![
                Shot { at: 0.5, picture: blank_shot_picture() },
                Shot { at: 1.5, picture: blank_shot_picture() },
                Shot { at: 2.5, picture: blank_shot_picture() },
            ];
        });
        let whole = 420;
        let side = 260.0 + 176.0 + 236.0 + 64.0;
        let layout = Layout::of(whole, side as i32, 1.0);
        // The points row is two stops, the same line the split's says.
        assert_eq!(layout.points.top, layout.add_split.top);
        assert!(layout.points.right + 10 <= layout.add_split.left);
        assert_eq!(layout.add_split.right, whole - 24);
        // The strip stands where the split card wears it, and the chips
        // row stands where the equal row would: under the transport, the
        // whole width, as tall as three taken frames ask (two rows).
        assert_eq!(layout.step_forward.bottom + 10, layout.shots.top);
        assert_eq!(layout.shots.left, 24);
        assert_eq!(layout.shots.right, whole - 24);
        assert_eq!(layout.shots.bottom - layout.shots.top, 176);
        // The hairline under the chips, and the run in the window.
        assert_eq!(layout.shots.bottom + 18, layout.hairline_bottom.top);
        assert!(layout.run.right <= whole && layout.run.bottom <= side as i32);
        // The equal row's own places stand empty: nothing is laid there.
        assert_eq!(layout.equal_label, RECT::default());
        assert_eq!(layout.set, RECT::default());
        STATE.with(|slot| {
            let mut s = slot.borrow_mut();
            s.shots = false;
            s.shots_taken.clear();
        });
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

    /// Saying play on an audio card begins the walk of the selection:
    /// the playhead stands where the stretch's in end stands, the walk
    /// is said as walking, and the stretch is cut to a sound of the
    /// play's own to be heard. Saying play again puts the play down, and
    /// the cut the sound spoke from is taken back — nothing of the play
    /// is left in the temp folder.
    #[test]
    fn saying_play_on_an_audio_card_begins_a_walk_of_the_selection() {
        crate::ui::wheel::install_platform();
        let folder = std::env::temp_dir().join("open-tangerine-play-selection");
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).expect("the play folder was made");
        let audio = make_test_audio(&folder);
        let (sender, _receiver) = std::sync::mpsc::channel();
        let mut window = TimelineWindow::new(sender).expect("the window was made");
        // The card's own saying: a sound of two seconds, a stretch said
        // from 0.4 to 1.2, and the conversion program found where it
        // stands. The stretch is said on the state the way a real open
        // of the card would have said it.
        STATE.with(|slot| {
            let mut state = slot.borrow_mut();
            state.audio = true;
            state.duration = 2.0;
            state.start = 0.4;
            state.end = 1.2;
            state.input = audio.clone();
            state.ffmpeg = locator::find();
        });

        unsafe { play_selection(window.window) };
        STATE.with(|slot| {
            let state = slot.borrow();
            assert!(state.playing, "the walk is walking");
            assert!(
                (state.playhead - 0.4).abs() < 0.001,
                "the playhead stood at the stretch's in end, said {}",
                state.playhead
            );
            let cut = state
                .play_sound
                .clone()
                .expect("the cut was said while the play stands");
            assert!(cut.is_file(), "the cut stood to be heard");
        });

        // Saying play again puts the play down.
        unsafe { play_selection(window.window) };
        STATE.with(|slot| {
            let state = slot.borrow();
            assert!(!state.playing, "the walk is put down");
            assert!(state.mci_alias.is_none(), "the sound's name is taken back");
            assert!(state.play_sound.is_none(), "the cut is taken back from the saying");
        });
        let leftovers: Vec<PathBuf> = std::fs::read_dir(&folder)
            .expect("the folder is there")
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path())
            .filter(|path| path.extension().map(|ext| ext == "wav").unwrap_or(false))
            .collect();
        assert_eq!(
            leftovers.len(),
            1,
            "only the test's own wav stands, yet {} too",
            leftovers.last().map(|p| p.display().to_string()).unwrap_or_default()
        );
        window.hide();
        let _ = std::fs::remove_dir_all(&folder);
    }

    /// Saying play on an audio card is heard: the sound stands playing
    /// by the interface's own word, the walk carries the playhead
    /// along, and putting the play down takes the sound and its cut
    /// back with it.
    #[test]
    #[ignore = "needs the conversion program; plays real audio"]
    fn a_play_of_the_selection_is_heard_and_walking() {
        crate::ui::wheel::install_platform();
        let folder = std::env::temp_dir().join("open-tangerine-play-heard");
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).expect("the play folder was made");
        let audio = make_test_audio(&folder);
        let (sender, _receiver) = std::sync::mpsc::channel();
        let mut window = TimelineWindow::new(sender).expect("the window was made");
        STATE.with(|slot| {
            let mut state = slot.borrow_mut();
            state.audio = true;
            state.duration = 2.0;
            state.start = 0.4;
            state.end = 1.2;
            state.input = audio.clone();
            state.ffmpeg = locator::find();
        });

        unsafe { play_selection(window.window) };
        // The walk of it: stood at the in point, and a tick or two of
        // the bell carries it on.
        let said = STATE.with(|slot| {
            let state = slot.borrow();
            assert!(state.playing, "the walk is walking");
            (state.mci_alias.clone().expect("the sound's name is said"),
             state.play_sound.clone().expect("the cut is said"),
             state.playhead)
        });
        let (alias, cut, stood) = said;
        // The interface's own word for it: the sound stands playing.
        let mode = mci_ask(&format!("status {alias} mode"));
        assert_eq!(
            mode.as_deref(),
            Some("playing"),
            "the sound said playing, heard {:?}",
            mode
        );
        assert!(
            (stood - 0.4).abs() < 0.001,
            "the playhead stood at the in point, said {}",
            stood
        );

        // The bell walks the playhead on: the window's own bell is
        // served while the messages are pumped.
        for _ in 0..25 {
            unsafe {
                let mut message = MSG::default();
                while PeekMessageW(&mut message, None, 0, 0, PM_REMOVE).as_bool() {
                    let _ = TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        STATE.with(|slot| {
            let state = slot.borrow();
            assert!(
                state.playhead > 0.4 && state.playhead <= 1.2,
                "the playhead walked on within the stretch, said {}",
                state.playhead
            );
            assert!(state.playing, "the walk stands within its stretch");
        });

        // Saying play again puts the sound down for good: it no longer
        // says playing, and its cut is gone.
        unsafe { play_selection(window.window) };
        let mode_after = mci_ask(&format!("status {alias} mode"));
        assert_ne!(
            mode_after.as_deref(),
            Some("playing"),
            "the sound was put down"
        );
        assert!(!cut.is_file(), "the cut went with the play");
        assert!(
            STATE.with(|slot| !slot.borrow().playing),
            "the walk was put down"
        );
        window.hide();
        let _ = std::fs::remove_dir_all(&folder);
    }

    /// Touches the audio trim card where a person's finger and mouse
    /// land — the middle of the card, the strip itself, the transport —
    /// and the window stays whole through every one of them.
    #[test]
    #[ignore = "needs the conversion program; opens a window"]
    fn a_touch_on_the_audio_trim_card_leaves_the_window_up() {
        use windows::Win32::UI::WindowsAndMessaging::{IsWindowVisible, SendMessageW};

        crate::ui::wheel::install_platform();
        let folder = std::env::temp_dir().join("open-tangerine-press-audio");
        std::fs::create_dir_all(&folder).expect("the press folder was made");
        let audio = make_test_audio(&folder);
        let (sender, _receiver) = std::sync::mpsc::channel();
        let mut window = TimelineWindow::new(sender).expect("the window was made");
        window.show_for(
            "tools.trim",
            "Trim",
            "Keeps the part you choose, trims silences.",
            &[audio],
        );
        // The fade takes a few beats: the window is whole before the
        // touch, the way a person finds it.
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
        unsafe {
            assert!(
                IsWindowVisible(window.window).as_bool(),
                "the trim window is up before the touch"
            )
        };

        // Where the touch lands: the middle of the card, the strip
        // itself, the transport above the footer, and the header.
        let spots = {
            let mut rect = RECT::default();
            unsafe {
                let _ = GetClientRect(window.window, &mut rect);
            }
            let mid_x = (rect.left + rect.right) / 2;
            let layout = Layout::of(rect.right, rect.bottom, 1.0);
            let strip_y = (layout.timeline.top + layout.timeline.bottom) / 2;
            [(mid_x, (rect.top + rect.bottom) / 2), (mid_x, strip_y)]
        };
        let at = |spot: (i32, i32)| {
            LPARAM(((spot.1 as isize & 0xFFFF) << 16) | (spot.0 as isize & 0xFFFF))
        };
        for spot in spots {
            unsafe {
                let _ = SendMessageW(
                    window.window,
                    WM_LBUTTONDOWN,
                    Some(WPARAM(
                        windows::Win32::System::SystemServices::MK_LBUTTON.0 as usize,
                    )),
                    Some(at(spot)),
                );
                let _ = SendMessageW(window.window, WM_LBUTTONUP, Some(WPARAM(0)), Some(at(spot)));
            }
            for _ in 0..8 {
                unsafe {
                    let mut message = MSG::default();
                    while PeekMessageW(&mut message, None, 0, 0, PM_REMOVE).as_bool() {
                        let _ = TranslateMessage(&message);
                        DispatchMessageW(&message);
                    }
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            unsafe {
                assert!(
                    IsWindowVisible(window.window).as_bool(),
                    "the trim window stays up through a touch at ({}, {})",
                    spot.0,
                    spot.1
                )
            };
        }
        window.hide();
        let _ = std::fs::remove_dir_all(&folder);
    }

    /// Presses the speaking controls of the audio trim card — the silence
    /// rail, the play-selection button, and both of the transport's step
    /// chips. Every one of them answers by reading the host again, so
    /// telling the card about the press while the host is still held
    /// takes the whole window down. The card is heard of only after the
    /// host has been let go.
    #[test]
    #[ignore = "needs the conversion program; opens a window"]
    fn pressing_the_speaking_controls_of_the_card_leaves_the_window_up() {
        use windows::Win32::UI::WindowsAndMessaging::{IsWindowVisible, SendMessageW};

        crate::ui::wheel::install_platform();
        let folder = std::env::temp_dir().join("open-tangerine-press-speaking");
        std::fs::create_dir_all(&folder).expect("the press folder was made");
        let audio = make_test_audio(&folder);
        let (sender, _receiver) = std::sync::mpsc::channel();
        let mut window = TimelineWindow::new(sender).expect("the window was made");
        window.show_for(
            "tools.trim",
            "Trim",
            "Keeps the part you choose, trims silences.",
            &[audio],
        );
        // The fade takes a few beats: the window is whole before the
        // press, the way a person finds it.
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
        unsafe {
            assert!(
                IsWindowVisible(window.window).as_bool(),
                "the trim window is up before the press"
            )
        };

        // Where the presses land: each a control whose answer reads the
        // host on its own.
        let spots = {
            let mut rect = RECT::default();
            unsafe {
                let _ = GetClientRect(window.window, &mut rect);
            }
            let layout = Layout::of(rect.right, rect.bottom, 1.0);
            let middle = |box_field: &RECT| {
                (
                    (box_field.left + box_field.right) / 2,
                    (box_field.top + box_field.bottom) / 2,
                )
            };
            [
                middle(&layout.silence),
                middle(&layout.play_selection),
                middle(&layout.step_back),
                middle(&layout.step_forward),
            ]
        };
        let at = |spot: (i32, i32)| {
            LPARAM(((spot.1 as isize & 0xFFFF) << 16) | (spot.0 as isize & 0xFFFF))
        };
        for spot in spots {
            unsafe {
                let _ = SendMessageW(
                    window.window,
                    WM_LBUTTONDOWN,
                    Some(WPARAM(
                        windows::Win32::System::SystemServices::MK_LBUTTON.0 as usize,
                    )),
                    Some(at(spot)),
                );
                let _ = SendMessageW(window.window, WM_LBUTTONUP, Some(WPARAM(0)), Some(at(spot)));
            }
            for _ in 0..8 {
                unsafe {
                    let mut message = MSG::default();
                    while PeekMessageW(&mut message, None, 0, 0, PM_REMOVE).as_bool() {
                        let _ = TranslateMessage(&message);
                        DispatchMessageW(&message);
                    }
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            unsafe {
                assert!(
                    IsWindowVisible(window.window).as_bool(),
                    "the trim window stays up through a press at ({}, {})",
                    spot.0,
                    spot.1
                )
            };
        }
        window.hide();
        let _ = std::fs::remove_dir_all(&folder);
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

    /// Opens the snapshots window on a real video, takes two frames the
    /// way the card takes them, says what the run would carry, and
    /// captures target/shots/snapshots-window.png.
    #[test]
    #[ignore = "needs the conversion program; writes a screenshot"]
    fn capture_the_snapshots_window() {
        crate::ui::wheel::install_platform();
        let folder = std::env::temp_dir().join("open-tangerine-capture-snapshots");
        std::fs::create_dir_all(&folder).expect("the capture folder was made");
        let video = make_test_video(&folder);
        let expected = video.clone();
        let (sender, receiver) = std::sync::mpsc::channel();
        let mut window = TimelineWindow::new(sender).expect("the window was made");
        window.show_for(
            "tools.snapshots",
            "Snapshots",
            "Saves frames of the video as pictures.",
            &[video],
        );
        // Wait for the strip to be built in the background, the way the
        // trim waits for its own.
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
        // The first frame is taken where the playhead stands: the video's
        // own beginning. Then the playhead is stood at a second and a
        // half, and the frame there is taken too.
        unsafe { add_shot(window.window) };
        STATE.with(|slot| slot.borrow_mut().playhead = 1.5);
        refresh_frame(true);
        refresh_times();
        unsafe { add_shot(window.window) };
        // The card wears two chips and says two taken.
        let taken = STATE.with(|slot| slot.borrow().shots_taken.len());
        assert_eq!(taken, 2, "two frames were taken");
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
                    shots.join("snapshots-buffer.png"),
                    &rgba,
                    host.width,
                    host.height,
                    image::ExtendedColorType::Rgba8,
                );
            }
        });
        capture(window.window, &shots.join("snapshots-window.png"));
        // The run says the moments the frames stand at, from the first to
        // the last, whatever order they were taken in.
        unsafe { run(window.window) };
        let event = receiver
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("the run was said");
        let crate::platform::windows::toolwindow::ToolEvent::Run {
            operation,
            inputs,
            settings,
        } = event
        else {
            panic!("the run was not a run");
        };
        assert_eq!(operation, "tools.snapshots");
        assert_eq!(inputs, vec![expected]);
        assert_eq!(settings.snapshots_ms, vec![0, 1_500]);
        window.hide();
        let _ = std::fs::remove_dir_all(&folder);
    }

    /// Lets the window's own beats run: a few rounds over the queue with
    /// a small sleep each, so whatever timers were ringing get their
    /// say, the way the other tests pump their windows.
    fn pump_breaths(rounds: usize) {
        for _ in 0..rounds {
            unsafe {
                let mut message = MSG::default();
                while PeekMessageW(&mut message, None, 0, 0, PM_REMOVE).as_bool() {
                    let _ = TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(8));
        }
    }

    /// Space on a trim card plays the preview and says nothing to the
    /// handler; Enter says go, as it alone ever did. With no video the
    /// play has nothing to walk on and keeps still - but the go must not
    /// happen on Space either way.
    #[test]
    fn space_on_a_trim_card_says_nothing_and_enter_says_go() {
        use windows::Win32::UI::WindowsAndMessaging::SendMessageW;

        crate::ui::wheel::install_platform();
        let (sender, receiver) = std::sync::mpsc::channel();
        let mut window = TimelineWindow::new(sender).expect("the window was made");
        window.show_for(
            "tools.trim",
            "Trim",
            "Cut the beginning or the end",
            &[PathBuf::from("not-there.mp4")],
        );
        pump_breaths(40);
        STATE.with(|slot| {
            let mut state = slot.borrow_mut();
            state.split = false;
            state.shots = false;
            state.audio = false;
            state.shots_taken.clear();
        });
        let card = window.window;
        // Space: no run is said, and no walk is begun either, with no
        // video on the card.
        unsafe {
            let _ = SendMessageW(
                card,
                WM_KEYDOWN,
                Some(WPARAM(VK_SPACE.0 as usize)),
                Some(LPARAM(0)),
            );
        }
        assert!(
            receiver.try_recv().is_err(),
            "the space key said something to the handler"
        );
        assert!(
            !STATE.with(|slot| slot.borrow().playing),
            "the space key began a walk with no video"
        );
        // Enter says go, as it always did.
        unsafe {
            let _ = SendMessageW(
                card,
                WM_KEYDOWN,
                Some(WPARAM(VK_RETURN.0 as usize)),
                Some(LPARAM(0)),
            );
        }
        match receiver.recv_timeout(std::time::Duration::from_secs(5)) {
            Ok(crate::platform::windows::toolwindow::ToolEvent::Run {
                operation, ..
            }) => assert_eq!(operation, "tools.trim", "the run was the trim's"),
            other => panic!("the enter key did not say the run: {other:?}"),
        }
        window.hide();
    }

    /// Dragging a cap of the trim stands the playhead on the cap being
    /// dragged: the preview follows what the card would take, whether it
    /// is the in cap or the out one.
    #[test]
    fn dragging_a_cap_of_the_trim_stands_the_playhead_on_it() {
        crate::ui::wheel::install_platform();
        let (sender, _receiver) = std::sync::mpsc::channel();
        let mut window = TimelineWindow::new(sender).expect("the window was made");
        window.show_for(
            "tools.trim",
            "Trim",
            "Cut the beginning or the end",
            &[PathBuf::from("not-there.mp4")],
        );
        pump_breaths(40);
        STATE.with(|slot| {
            let mut state = slot.borrow_mut();
            state.split = false;
            state.shots = false;
            state.audio = false;
            state.shots_taken.clear();
            state.preview_units = 236.0;
            state.duration = 10.0;
            state.start = 2.0;
            state.end = 8.0;
        });
        let card = window.window;
        let (width, height) = HOST.with(|slot| {
            slot.borrow()
                .as_ref()
                .map(|host| (host.width as i32, host.height as i32))
                .expect("the card was laid out")
        });
        let bar = Layout::of(width, height, scale_factor(card)).timeline;
        let along = |moment: f64| {
            bar.left + (fraction(moment, 10.0) * (bar.right - bar.left) as f32) as i32
        };
        // A press on the in cap, then a slide of it a little way on: the
        // playhead follows it.
        timeline_touched(card, along(2.0), bar.top, true);
        timeline_touched(card, along(4.0), bar.top, false);
        STATE.with(|slot| {
            let state = slot.borrow();
            assert!(
                (state.start - 4.0).abs() < 0.05,
                "the in cap slid near 4.0, reads {}",
                state.start
            );
            assert_eq!(
                state.playhead, state.start,
                "the playhead did not follow the in cap"
            );
        });
        // The out cap goes the same way.
        timeline_touched(card, along(8.0), bar.top, true);
        timeline_touched(card, along(6.0), bar.top, false);
        STATE.with(|slot| {
            let state = slot.borrow();
            assert!(
                (state.end - 6.0).abs() < 0.05,
                "the out cap slid near 6.0, reads {}",
                state.end
            );
            assert_eq!(
                state.playhead, state.end,
                "the playhead did not follow the out cap"
            );
        });
        released(card);
        window.hide();
    }

    /// Plays the preview of a real trim card with Space: the walk takes
    /// the playhead a step with every tick, and nothing is said to the
    /// handler; playing again stops the walk where it stands; Enter
    /// alone says go.
    #[test]
    #[ignore = "needs the conversion program; opens a window"]
    fn space_plays_the_preview_of_a_real_trim_card() {
        use windows::Win32::UI::WindowsAndMessaging::SendMessageW;

        crate::ui::wheel::install_platform();
        let folder = std::env::temp_dir().join(format!(
            "open-tangerine-space-play-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&folder).expect("the play folder was made");
        let video = make_test_video(&folder);
        let (sender, receiver) = std::sync::mpsc::channel();
        let mut window = TimelineWindow::new(sender).expect("the window was made");
        window.show_for(
            "tools.trim",
            "Trim",
            "Cut the beginning or the end",
            &[video],
        );
        // The card comes in: the length of the video must be known
        // before anything of the play can be asked of it.
        for _ in 0..400 {
            pump_breaths(1);
            if STATE.with(|slot| slot.borrow().duration > 0.0) {
                break;
            }
        }
        assert!(
            STATE.with(|slot| slot.borrow().duration > 0.0),
            "the length of the video was never known"
        );
        let card = window.window;
        let stand_now = || STATE.with(|slot| slot.borrow().playhead);
        // Space: the walk begins, and nothing is said to the handler.
        unsafe {
            let _ = SendMessageW(
                card,
                WM_KEYDOWN,
                Some(WPARAM(VK_SPACE.0 as usize)),
                Some(LPARAM(0)),
            );
        }
        assert!(STATE.with(|slot| slot.borrow().playing), "the play began");
        assert!(
            receiver.try_recv().is_err(),
            "the space key said something to the handler"
        );
        let before = stand_now();
        pump_breaths(45);
        let after = stand_now();
        assert!(
            STATE.with(|slot| slot.borrow().playing),
            "the walk was still on"
        );
        assert!(
            after > before,
            "the walk took the playhead on, stood at {} then {}",
            before,
            after
        );
        assert!(
            receiver.try_recv().is_err(),
            "playing said something to the handler"
        );
        // Space again: the walk stops where it stands.
        unsafe {
            let _ = SendMessageW(
                card,
                WM_KEYDOWN,
                Some(WPARAM(VK_SPACE.0 as usize)),
                Some(LPARAM(0)),
            );
        }
        assert!(
            !STATE.with(|slot| slot.borrow().playing),
            "playing again stopped the walk"
        );
        let stood = stand_now();
        pump_breaths(12);
        assert_eq!(
            stand_now(),
            stood,
            "the stopped walk stood still"
        );
        // Enter alone says go.
        unsafe {
            let _ = SendMessageW(
                card,
                WM_KEYDOWN,
                Some(WPARAM(VK_RETURN.0 as usize)),
                Some(LPARAM(0)),
            );
        }
        match receiver.recv_timeout(std::time::Duration::from_secs(5)) {
            Ok(crate::platform::windows::toolwindow::ToolEvent::Run {
                operation, ..
            }) => assert_eq!(operation, "tools.trim", "the run was the trim's"),
            other => panic!("the enter key did not say the run: {other:?}"),
        }
        window.hide();
        let _ = std::fs::remove_dir_all(&folder);
    }
}
