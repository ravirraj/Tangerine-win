//! The window a sound is covered in.
//!
//! The covering does not happen the moment it is picked: it opens a
//! window with the sound laid out the way the reference's bleep window
//! stands — the count of the bleeps at the top, a Bleep row where every
//! covered stretch stands on one track, pulled whole or at one of its
//! caps, the Audio row wearing the sound's own peaks with the playhead
//! in it, the times under the rows, the words that tell how a bleep is
//! made, and a Preview row that hears the sound plain and hears it with
//! the tone laid over the stretches. The footer holds only the making
//! of the bleeped copy, and it is refused while no stretch stands.
//!
//! The stretches are kept whole in the sound's own seconds. On the save
//! they are said in thousandths of a second, the tone is asked for at
//! the reference's own pitch, and the sound is taken over stretch by
//! stretch — the same taking the saved copy is made by.
//!
//! The card is drawn by the same renderer that draws the wheel, and the
//! skeleton it wears is the shell's. The card only draws: the pressing,
//! the dragging and the seeking are read by the window itself, which
//! holds the same geometry the card was given — the way the crop, the
//! trim and the video's own cover do.
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
use windows::Win32::Graphics::Gdi::{InvalidateRect, BeginPaint, CreateSolidBrush, EndPaint};
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
    CW_USEDEFAULT, IDC_ARROW, IDC_CROSS, IDC_SIZEALL, IDC_SIZEWE, LWA_ALPHA, WNDCLASSW,
    WM_CLOSE, WM_DESTROY, WM_DPICHANGED, WM_ERASEBKGND, WM_KEYDOWN, WM_LBUTTONDOWN,
    WM_LBUTTONUP, WM_MOUSEMOVE, WM_PAINT, WM_SIZE, WM_TIMER, WS_EX_LAYERED,
};
use windows::Win32::Media::Audio::{PlaySoundW, SND_ASYNC, SND_MEMORY};

use crate::media::audio::{
    decode_full_pcm, encode_wav_bytes, render_waveform_strip, waveform_peaks, PcmAudio,
};
use crate::media::frames::duration_of;
use crate::media::locator;
use crate::platform::windows::toolwindow::ToolEvent;
use crate::platform::windows::windowhost::{
    fixed_style, fraction, scale_factor, screen_scale, timecode, to_wide,
};
use crate::radial::execution::{BleepRange, ToolSettings};
use crate::radial::tools::bleep_over;
use crate::ui::wheel::{window_for_next_component, Bgra8, LaneData, Panel, RedactAudioCard};

/// The timer the window comes in softly on.
const FADE_TIMER_ID: usize = 52;
/// The timer the window asks the peaks' own worker by, whether some of
/// them have come.
const STRIP_TIMER_ID: usize = 54;
/// The timer that carries a preview on, beat after beat.
const PLAY_TIMER_ID: usize = 56;
/// How far a preview moves on in one beat of its timer, in seconds.
const PLAY_STEP: f64 = 0.08;

/// The width of the card, in the drawing's own units.
const CARD_W: i32 = 420;
/// How tall the row of peaks is, in the drawing's own units.
const WAVE_UNITS: f32 = 40.0;
/// One bar of the peaks wants to stand about this wide, in the
/// drawing's own units, before another of them is asked of the sound.
const PILLAR_UNITS: f32 = 4.0;

/// How far from the cap at the end of a stretch a press still takes
/// the cap, in pixels of the window.
const CAP_REACH: f64 = 8.0;
/// The least a stretch may be, in seconds of the sound, before a
/// drawn one is dropped.
const MIN_WINDOW: f64 = 0.25;

/// The rows' own places, in the drawing's own units: where the Bleep
/// track and the row of peaks begin at the left, how tall each row is,
/// and where each stands under the last.
const TRACK_LEFT_UNITS: f32 = 68.0;
const BLEEP_TOP_UNITS: f32 = 112.0;
const BLEEP_HEIGHT_UNITS: f32 = 26.0;
const AUDIO_TOP_UNITS: f32 = 150.0;
const AUDIO_HEIGHT_UNITS: f32 = 40.0;

/// How high the tone sings, in hertz: the reference's own default, what
/// the settings fall back to when nothing was asked of the pitch.
const BLEEP_FREQUENCY: u32 = 1000;

/// The ways the pointer can be busy on the track: nothing taken, a
/// fresh stretch being drawn between the press and now, a whole
/// stretch being carried along, one of its caps being pulled, or the
/// playhead being brought to where the pointer stands.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
enum BleepDrag {
    #[default]
    Nothing,
    /// A fresh stretch: at what second of the sound the press took.
    New { from: f64 },
    /// A whole stretch comes along: which one, at what second the
    /// press took beside its start, and how much the stretch spans.
    Move { index: usize, grab: f64, span: f64 },
    /// One cap of a stretch is pulled: which one, and whether it is
    /// the one at the start.
    Edge { index: usize, start: bool },
    /// The playhead is brought to the pointer, on the peaks.
    Seek,
}

/// What a press on the track lands itself on: a fresh stretch to draw,
/// a cap to pull, or a stretch to carry along. An empty track with no
/// stretch under the press is always a fresh one, so the helper only
/// speaks when it found one of the stretches standing there.
#[derive(Clone, Copy, Debug, PartialEq)]
enum BleepPick {
    Edge { index: usize, start: bool },
    Move { index: usize, grab: f64, span: f64 },
}

/// What the window remembers between the frames: the sound's own
/// samples and rate, where the playhead stands, the covered stretches,
/// which one stands chosen, and what the pointer is doing.
#[derive(Debug, Default)]
struct BleepState {
    /// The sound's own samples, once the peaks' worker has answered.
    pcm: Option<PcmAudio>,
    /// How long the sound runs, in seconds, and where its playhead
    /// stands; whether a preview is playing.
    duration: f64,
    playhead: f64,
    playing: bool,
    /// The covered stretches, in the sound's own seconds.
    ranges: Vec<(f64, f64)>,
    /// Which stretch the pointer's last choice left chosen.
    selected: Option<usize>,
    /// What the pointer is busy with.
    drag: BleepDrag,
    /// The programs and the file the sound comes of.
    ffmpeg: Option<locator::Tools>,
    input: PathBuf,
}

thread_local! {
    static HOST: std::cell::RefCell<Option<Host>> = std::cell::RefCell::new(None);
    static WINDOW: std::cell::Cell<isize> = const { std::cell::Cell::new(0) };
    static STATE: std::cell::RefCell<BleepState> =
        std::cell::RefCell::new(BleepState::default());
    static OPEN_FOR: std::cell::RefCell<Option<(String, Vec<PathBuf>)>> =
        const { std::cell::RefCell::new(None) };
    static EVENTS: std::cell::RefCell<Option<Sender<ToolEvent>>> =
        const { std::cell::RefCell::new(None) };
    /// The keyboard's stop: 0 nothing, 1 the original, 2 the bleeped
    /// preview, 3 save.
    static FOCUS_INDEX: std::cell::Cell<i32> = const { std::cell::Cell::new(0) };
    static FADE_ALPHA: std::cell::Cell<u8> = const { std::cell::Cell::new(255) };
    /// The peaks are worked out elsewhere; what comes of the work is
    /// buried here for the window to collect.
    static STRIP_INBOX:
        std::cell::RefCell<Option<std::sync::mpsc::Receiver<StripPiece>>> =
        const { std::cell::RefCell::new(None) };
    static STRIP_WORN: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// The bytes of the last preview offered to the system's own
    /// player: it reads them while they play, so they stay put here
    /// until the next preview takes their place.
    static WAV_BYTES: std::cell::RefCell<Vec<u8>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// One finished wearing: the peaks' pixels laid one colour after
/// another and how wide and tall they should be said, and — when the
/// sound decoded along the way — the sound itself for the previews.
type StripPiece = (Vec<u8>, u32, u32, Option<PcmAudio>);

/// The window handle, if there is one.
fn window_handle() -> HWND {
    HWND(WINDOW.get() as *mut _)
}

/// The window, its card, and the buffer the card is drawn into.
struct Host {
    window: HWND,
    card: RedactAudioCard,
    software: Rc<MinimalSoftwareWindow>,
    buffer: Vec<Bgra8>,
    width: u32,
    height: u32,
}

/// The window a sound is covered in.
pub struct RedactAudioWindow {
    window: HWND,
}

// The window belongs to the thread that made it and is us
unsafe impl Send for RedactAudioWindow {}

impl RedactAudioWindow {
    /// Makes the window, hidden until a bleep is asked for.
    pub fn new(events: Sender<ToolEvent>) -> Result<RedactAudioWindow, String> {
        EVENTS.with(|slot| *slot.borrow_mut() = Some(events));

        // A window is put aside for the card before the card is made, so
        // the two find each other.
        let software = window_for_next_component();
        let card = RedactAudioCard::new().map_err(|error| error.to_string())?;

        let instance = unsafe { GetModuleHandleW(None).map_err(|error| error.to_string())? };
        let class_name = w!("OpenTangerineRedactAudio");
        // The card's own white is what fills it while it comes in, so a
        // dark theme's idea of "window colour" never shows through.
        let scale = screen_scale();
        let (outer_w, outer_h) = outer_size(scale);
        let window = unsafe {
            let owner = crate::platform::windows::windowhost::root_owner_window();
            let class = WNDCLASSW {
                lpfnWndProc: Some(window_procedure),
                hInstance: instance.into(),
                lpszClassName: class_name,
                hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
                hIcon: crate::platform::windows::windowhost::get_app_icon(),
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
                if owner.0.is_null() { None } else { Some(owner) },
                None,
                Some(instance.into()),
                None,
            )
        }
        .map_err(|error| error.to_string())?;
        crate::platform::windows::windowhost::prepare_tool_window(window);

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
        // pressed while a drag is still holding the host.
        {
            card.on_run_chosen(move || {
                let window = window_handle();
                if !window.0.is_null() {
                    unsafe { run(window) };
                }
            });
            card.on_original_chosen(move || {
                let window = window_handle();
                if !window.0.is_null() {
                    unsafe { preview(window, false) };
                }
            });
            card.on_with_bleeps_chosen(move || {
                let window = window_handle();
                if !window.0.is_null() {
                    unsafe { preview(window, true) };
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

        Ok(RedactAudioWindow { window })
    }

    /// Asks the window to hide itself.
    pub fn hide(&mut self) {
        unsafe {
            let _ = ShowWindow(self.window, SW_HIDE);
        }
    }

    /// How long the sound runs is asked of the conversion program
    /// before the window is shown, so what appears is already the
    /// sound's own card at its own length. The sound's own samples are
    /// asked of a worker elsewhere; they arrive on the peaks.
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

        // A bleep works on a sound and nothing else: when what was
        // dragged is not one, the window does not open at all.
        if crate::media::probe::kind_of(&extension) != Some(crate::media::probe::Kind::Audio) {
            OPEN_FOR.with(|slot| *slot.borrow_mut() = None);
            return;
        }

        let ffmpeg = locator::find();
        let duration = ffmpeg
            .as_ref()
            .and_then(|tools| duration_of(tools, &input))
            .unwrap_or(0.0);

        STATE.with(|slot| {
            *slot.borrow_mut() = BleepState {
                pcm: None,
                duration,
                playhead: 0.0,
                playing: false,
                ranges: Vec::new(),
                selected: None,
                drag: BleepDrag::Nothing,
                ffmpeg: ffmpeg.clone(),
                input: input.clone(),
            }
        });

        // Everything the card needs is set first, while the host is
        // held only for that and nothing is shown yet. The timelines
        // and the count are said with the host let go, because their
        // saying takes the host again for its own telling.
        let card_window = HOST.with(|slot| {
            let mut borrowed = slot.borrow_mut();
            let Some(host) = borrowed.as_mut() else {
                return HWND::default();
            };
            host.card.set_file_name(said.into());
            host.card.set_detail(detail.into());
            host.card.set_has_wave(false);
            host.card.set_wave_image(slint::Image::default());
            host.lay_out();
            host.software.request_redraw();
            host.window
        });
        refresh_times();

        if card_window.0.is_null() {
            return;
        }

        // The peaks are worked out elsewhere: the window asks it every
        // few beats whether some of them have come.
        STATE.with(|slot| {
            let state = slot.borrow();
            if let Some(tools) = state.ffmpeg.as_ref() {
                let edges = wave_edges(card_window);
                ask_for_wave(tools.clone(), state.input.clone(), edges.0, edges.1);
                unsafe {
                    let _ = SetTimer(Some(card_window), STRIP_TIMER_ID, 40, None);
                }
            }
        });

        unsafe {
            // The window is made the size the card wants, where it
            // already stands.
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
            // The window comes in softly, from nothing to whole in a
            // few beats of the timer.
            FADE_ALPHA.with(|slot| slot.set(0));
            let _ = SetLayeredWindowAttributes(card_window, COLORREF(0), 0, LWA_ALPHA);
            let _ = SetForegroundWindow(card_window);
            let _ = ShowWindow(card_window, SW_SHOW);
            crate::platform::windows::windowhost::hide_from_taskbar(card_window);
            let _ = SetTimer(Some(card_window), FADE_TIMER_ID, 16, None);
            // The keyboard starts with nothing under it: the pills are
            // a Tab away, and the track takes the pointer first.
            FOCUS_INDEX.with(|slot| slot.set(0));
        }
        draw_window(card_window);
    }
}

/// Where the peaks are being worn, in window pixels of the window the
/// peaks are being worked for: how wide and tall they will be worn as.
fn wave_edges(window: HWND) -> (u32, u32) {
    HOST.with(|slot| {
        slot.borrow().as_ref().map(|host| {
            if host.window != window {
                return (328, 40);
            }
            let scale = scale_factor(host.window);
            (
                (Layout::track_width_units() * scale) as u32,
                (WAVE_UNITS * scale) as u32,
            )
        })
    })
    .unwrap_or((328, 40))
}

/// Asks the sound's own peaks elsewhere: the sound decoded whole, the
/// peaks laid out over the row's own width, and the picture sent back
/// with the sound itself, so the previews never decode twice.
fn ask_for_wave(tools: locator::Tools, input: PathBuf, width: u32, height: u32) {
    let (sender, receiver) = std::sync::mpsc::channel::<StripPiece>();
    std::thread::spawn(move || {
        let wave_w = width.max(1);
        let wave_h = height.max(1);
        let Some(pcm) = decode_full_pcm(&tools, &input) else {
            let _ = sender.send((Vec::new(), 0, 0, None));
            return;
        };
        let buckets = ((wave_w as f32 / PILLAR_UNITS) as usize).clamp(16, 600);
        let peaks = waveform_peaks(&pcm.samples, buckets);
        let strip = render_waveform_strip(&peaks, wave_w, wave_h);
        let (wide, tall) = (strip.width(), strip.height());
        let _ = sender.send((strip.into_raw(), wide, tall, Some(pcm)));
    });
    STRIP_INBOX.with(|slot| {
        *slot.borrow_mut() = Some(receiver);
    });
    STRIP_WORN.with(|slot| slot.set(false));
}

/// Takes one turn of the peaks' arriving, without waiting.
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

/// Puts a finished wearing on the card — the peaks, and the sound
/// itself for the previews — and asks the window to paint it.
unsafe fn wear_wave(piece: StripPiece) {
    let (pixels, width, height, pcm) = piece;
    if let Some(pcm) = pcm {
        STATE.with(|slot| {
            let mut state = slot.borrow_mut();
            state.pcm = Some(pcm);
            // The decoded sound knows its own length more exactly than
            // the probe did.
            if let Some(said) = said_of(&state) {
                state.duration = said;
            }
        });
        refresh_times();
    }
    if width == 0 || height == 0 || pixels.is_empty() {
        let window = window_handle();
        if !window.0.is_null() {
            let _ = unsafe { InvalidateRect(Some(window), None, true) };
        }
        STRIP_WORN.with(|slot| slot.set(true));
        return;
    }
    let buffer = SharedPixelBuffer::clone_from_slice(&pixels, width, height);
    let image = slint::Image::from_rgba8(buffer);
    HOST.with(|slot| {
        if let Some(host) = slot.borrow_mut().as_mut() {
            host.card.set_wave_image(image);
            host.card.set_has_wave(true);
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
/// uses of it is the size the drawing asked for, at this stretch.
fn outer_size(scale: f32) -> (i32, i32) {
    let client_w = (CARD_W as f32 * scale) as i32;
    let client_h = (Layout::HEIGHT * scale) as i32;
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

    /// Says every box the card asks for. The same numbers are read by
    /// the window when the track is pressed, so what is drawn and what
    /// is taken can never disagree.
    fn lay_out(&mut self) {
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
        self.card.set_hairline_top_box(panel(layout.hairline_top));
        self.card.set_hairline_mid_box(panel(layout.hairline_mid));
        self.card.set_hairline_bottom_box(panel(layout.hairline_bottom));
        self.card.set_bleeps_label_box(panel(layout.bleeps_label));
        self.card.set_bleeps_said_box(panel(layout.bleeps_said));
        self.card.set_bleep_label_box(panel(layout.bleep_label));
        self.card.set_bleep_track_box(panel(layout.bleep_track));
        self.card.set_audio_label_box(panel(layout.audio_label));
        self.card.set_audio_track_box(panel(layout.audio_track));
        self.card.set_time_left_box(panel(layout.time_left));
        self.card.set_time_right_box(panel(layout.time_right));
        self.card.set_helper_box(panel(layout.helper));
        self.card.set_preview_label_box(panel(layout.preview_label));
        self.card.set_original_box(panel(layout.original));
        self.card.set_with_bleeps_box(panel(layout.with_bleeps));
        self.card.set_run_box(panel(layout.run));
        stand_bleeps(self);
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
/// The card is a stack: a header, a hairline, the count of the bleeps,
/// the Bleep row and the Audio row, the times, the words of the making,
/// an opening for the Preview, the two pills, a closing hairline, and
/// the footer. The Rust side works the stack out once and both sides
/// read it.
#[derive(Clone, Copy, Debug, Default)]
struct Layout {
    file_name: RECT,
    detail: RECT,
    hairline_top: RECT,
    bleeps_label: RECT,
    bleeps_said: RECT,
    bleep_label: RECT,
    bleep_track: RECT,
    audio_label: RECT,
    audio_track: RECT,
    time_left: RECT,
    time_right: RECT,
    helper: RECT,
    hairline_mid: RECT,
    preview_label: RECT,
    original: RECT,
    with_bleeps: RECT,
    hairline_bottom: RECT,
    run: RECT,
}

impl Layout {
    /// Everything of the card, in the drawing's own units: the
    /// header's band, the body, and the footer.
    const HEADER_UNITS: f32 = 62.0;
    const CONTENT_UNITS: f32 = 250.0;
    const FOOTER_UNITS: f32 = 64.0;
    const HEIGHT: f32 = Self::HEADER_UNITS + Self::CONTENT_UNITS + Self::FOOTER_UNITS;

    /// How wide the tracks run, in the drawing's own units: from the
    /// left where the rows' controls begin to the right margin.
    fn track_width_units() -> f32 {
        396.0 - TRACK_LEFT_UNITS
    }

    fn of(whole: i32, side: i32, scale: f32) -> Layout {
        let at = |value: f32| (value * scale) as i32;
        let margin = at(24.0);

        // The stack, top to bottom, all worked out from the one number.
        let header_top = at(16.0);
        let hairline_top = at(62.0);
        let bleeps_top = hairline_top + at(22.0);
        let bleep_row_top = at(BLEEP_TOP_UNITS);
        let audio_row_top = at(AUDIO_TOP_UNITS);
        let times_top = audio_row_top + at(AUDIO_HEIGHT_UNITS) + at(8.0);
        let helper_top = times_top + at(16.0) + at(6.0);
        let hairline_mid = helper_top + at(32.0) + at(10.0);
        let preview_top = hairline_mid + at(10.0);
        let hairline_bottom = preview_top + at(30.0) + at(10.0);

        // The tracks run from the rows' own left to the right margin;
        // the controls of the preview stand at its right end.
        let track_left = at(TRACK_LEFT_UNITS);
        let track_right = whole - margin;
        let original_w = at(64.0);
        let bleeped_w = at(114.0);

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
            bleeps_label: RECT {
                left: margin,
                top: bleeps_top,
                right: margin + at(56.0),
                bottom: bleeps_top + at(18.0),
            },
            bleeps_said: RECT {
                left: margin + at(60.0),
                top: bleeps_top,
                right: whole - margin,
                bottom: bleeps_top + at(18.0),
            },
            bleep_label: RECT {
                left: margin,
                top: bleep_row_top,
                right: track_left - at(4.0),
                bottom: bleep_row_top + at(BLEEP_HEIGHT_UNITS),
            },
            bleep_track: RECT {
                left: track_left,
                top: bleep_row_top,
                right: track_right,
                bottom: bleep_row_top + at(BLEEP_HEIGHT_UNITS),
            },
            audio_label: RECT {
                left: margin,
                top: audio_row_top,
                right: track_left - at(4.0),
                bottom: audio_row_top + at(AUDIO_HEIGHT_UNITS),
            },
            audio_track: RECT {
                left: track_left,
                top: audio_row_top,
                right: track_right,
                bottom: audio_row_top + at(AUDIO_HEIGHT_UNITS),
            },
            time_left: RECT {
                left: track_left,
                top: times_top,
                right: track_left + at(124.0),
                bottom: times_top + at(16.0),
            },
            time_right: RECT {
                left: track_right - at(124.0),
                top: times_top,
                right: track_right,
                bottom: times_top + at(16.0),
            },
            helper: RECT {
                left: margin,
                top: helper_top,
                right: whole - margin,
                bottom: helper_top + at(32.0),
            },
            hairline_mid: RECT {
                left: 0,
                top: hairline_mid,
                right: whole,
                bottom: hairline_mid + 1,
            },
            preview_label: RECT {
                left: margin,
                top: preview_top,
                right: margin + at(64.0),
                bottom: preview_top + at(30.0),
            },
            original: RECT {
                left: track_right - bleeped_w - at(4.0) - original_w,
                top: preview_top,
                right: track_right - bleeped_w - at(4.0),
                bottom: preview_top + at(30.0),
            },
            with_bleeps: RECT {
                left: track_right - bleeped_w,
                top: preview_top,
                right: track_right,
                bottom: preview_top + at(30.0),
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

/// Where the Bleep track stands, in window pixels, said the way the
/// layout says it.
fn bleep_track(whole: i32, scale: f32) -> RECT {
    let at = |value: f32| (value * scale) as i32;
    RECT {
        left: at(TRACK_LEFT_UNITS),
        top: at(BLEEP_TOP_UNITS),
        right: whole - at(24.0),
        bottom: at(BLEEP_TOP_UNITS) + at(BLEEP_HEIGHT_UNITS),
    }
}

/// Where the row of peaks stands, in window pixels, said the way the
/// layout says it.
fn audio_track(whole: i32, scale: f32) -> RECT {
    let at = |value: f32| (value * scale) as i32;
    RECT {
        left: at(TRACK_LEFT_UNITS),
        top: at(AUDIO_TOP_UNITS),
        right: whole - at(24.0),
        bottom: at(AUDIO_TOP_UNITS) + at(AUDIO_HEIGHT_UNITS),
    }
}

/// How wide a track is, in window pixels.
fn track_width(whole: i32, scale: f32) -> f64 {
    let at = |value: f32| (value * scale) as i32;
    ((whole - at(24.0)) as f64 - at(TRACK_LEFT_UNITS) as f64).max(1.0)
}

/// What second of the sound a window pixel of a track stands at.
fn track_secs(x: i32, whole: i32, scale: f32, duration: f64) -> f64 {
    let at = |value: f32| (value * scale) as i32;
    let frac = ((x as f64 - at(TRACK_LEFT_UNITS) as f64) / track_width(whole, scale))
        .clamp(0.0, 1.0);
    frac * duration
}

/// Whether a press lands in a track's own rect, with a little grace,
/// and alone: a press between the rows is nobody's.
fn in_track(rect: RECT, x: i32, y: i32) -> bool {
    x >= rect.left && x <= rect.right && y >= rect.top && y <= rect.bottom
}

/// A stretch drawn between the second a press took and the one the
/// pointer stands at, whichever way the drag went.
fn stretched_window(from: f64, to: f64) -> (f64, f64) {
    (from.min(to), from.max(to))
}

/// A stretch carried along a drag, held inside the sound.
fn moved_window(original: (f64, f64), grab: f64, now: f64, duration: f64) -> (f64, f64) {
    let span = original.1 - original.0;
    let start = (now - grab).clamp(0.0, (duration - span).max(0.0));
    (start, start + span)
}

/// A stretch with one of its caps pulled, held inside the sound and
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

/// Whether a stretch is too small to mean anything. A smaller drag is
/// dropped when the pointer lets go.
fn window_too_small(window: (f64, f64)) -> bool {
    window.1 - window.0 < MIN_WINDOW
}

/// What a press on the track took hold of, said over every stretch
/// standing: the chosen one is asked first, then the others in the
/// order they were drawn — the cap at an end, then the carrying-along.
/// A press away from them all is a fresh stretch to draw.
fn pick_bleep(
    ranges: &[(f64, f64)],
    selected: Option<usize>,
    press: f64,
    reach: f64,
) -> Option<BleepPick> {
    let mut order: Vec<usize> = Vec::with_capacity(ranges.len());
    if let Some(index) = selected {
        if index < ranges.len() {
            order.push(index);
        }
    }
    for index in 0..ranges.len() {
        if Some(index) != selected {
            order.push(index);
        }
    }
    for index in order {
        let (start, end) = ranges[index];
        if (press - start).abs() <= reach {
            return Some(BleepPick::Edge { index, start: true });
        }
        if (press - end).abs() <= reach {
            return Some(BleepPick::Edge { index, start: false });
        }
        if press > start && press < end {
            return Some(BleepPick::Move {
                index,
                grab: press - start,
                span: end - start,
            });
        }
    }
    // Nothing under the press: the caller draws a fresh stretch there.
    None
}

/// The stretches the settings ask them: seconds said in milliseconds,
/// the tone at its own pitch unless nothing was said of it.
fn settings_from(ranges: &[(f64, f64)]) -> ToolSettings {
    ToolSettings {
        bleep_ranges: ranges
            .iter()
            .map(|(start, end)| BleepRange {
                start_ms: ((*start * 1000.0).round().max(0.0)) as u64,
                end_ms: ((*end * 1000.0).round().max(0.0)) as u64,
            })
            .collect(),
        ..Default::default()
    }
}

/// How long the sound the window holds runs, said by its own samples.
fn said_of(state: &BleepState) -> Option<f64> {
    let pcm = state.pcm.as_ref()?;
    let stride = (pcm.channels.max(1) as usize).max(1);
    if pcm.sample_rate == 0 {
        return None;
    }
    Some((pcm.samples.len() / stride) as f64 / pcm.sample_rate as f64)
}

/// Puts the stretches where they stand on the track, and says the card
/// so, with which one the person left chosen.
fn stand_bleeps(host: &mut Host) {
    let scale = scale_factor(host.window);
    let whole = host.width as i32;
    let (bleeps, chosen) = STATE.with(|slot| {
        let state = slot.borrow();
        let row = bleep_track(whole, scale);
        let said: Vec<LaneData> = state
            .ranges
            .iter()
            .enumerate()
            .map(|(index, (start, end))| {
                let (start, end) = if state.duration > 0.0 {
                    (
                        fraction(*start, state.duration),
                        fraction(*end, state.duration),
                    )
                } else {
                    (0.0, 0.0)
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
                    filled: true,
                }
            })
            .collect();
        (said, state.selected)
    });
    host.card.set_bleeps(ModelRc::new(VecModel::from(bleeps)));
    host.card
        .set_selected_bleep(chosen.map(|index| index as i32).unwrap_or(-1));
    host.software.request_redraw();
}

/// Puts the state's stretches on the card and asks for a paint.
unsafe fn stand_all(window: HWND) {
    HOST.with(|slot| {
        let mut borrowed = slot.borrow_mut();
        if let Some(host) = borrowed.as_mut() {
            stand_bleeps(host);
        }
    });
    let _ = unsafe { InvalidateRect(Some(window), None, true) };
}

/// Says where the playhead stands, what the times are, the count of
/// the bleeps, and whether a preview plays.
fn refresh_times() {
    let (frac, left, right, said, count) = STATE.with(|slot| {
        let state = slot.borrow();
        (
            fraction(state.playhead, state.duration),
            format!("{}", timecode(state.playhead)),
            format!("{}", timecode(state.duration)),
            format!("{}", state.ranges.len()),
            state.ranges.len() as i32,
        )
    });
    HOST.with(|slot| {
        if let Some(host) = slot.borrow_mut().as_mut() {
            host.card.set_playhead_fraction(frac);
            host.card.set_time_left(left.into());
            host.card.set_time_right(right.into());
            host.card.set_bleeps_said(said.into());
            host.card.set_bleeps_count(count);
        }
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
/// on. The stops go round: the original, the bleeped preview, save.
unsafe fn cycle_focus(window: HWND, forward: bool) {
    let now = FOCUS_INDEX.with(|slot| slot.get());
    let next = if forward { now + 1 } else { now - 1 };
    let stop = if next > 3 {
        1
    } else if next < 1 {
        3
    } else {
        next
    };
    unsafe { apply_focus(window, stop) };
}

/// Takes the chosen stretch away, and what was under it answers.
unsafe fn remove_selected(window: HWND) {
    STATE.with(|slot| {
        let mut state = slot.borrow_mut();
        if let Some(index) = state.selected.take() {
            if index < state.ranges.len() {
                state.ranges.remove(index);
            }
        }
    });
    refresh_times();
    unsafe { stand_all(window) };
}

/// Makes the bytes of a WAV out of the sound the window holds — plain
/// when none was asked, with the tone laid over every stretch when it
/// was — and leaves them resting for the system's own player.
fn wav_bytes_for(bleeped: bool) -> Option<Vec<u8>> {
    STATE.with(|slot| {
        let state = slot.borrow();
        let pcm = state.pcm.as_ref()?;
        let samples = if bleeped {
            // The preview hears what the saved copy will: the real
            // taking, said the way the save says it.
            let ranges = settings_from(&state.ranges)
                .bleep_ranges
                .iter()
                .map(|one| {
                    (
                        one.start_ms as f64 / 1000.0,
                        one.end_ms as f64 / 1000.0,
                    )
                })
                .collect::<Vec<(f64, f64)>>();
            let mut samples = pcm.samples.clone();
            let stride = (pcm.channels.max(1) as usize).max(1);
            let frames = samples.len() / stride;
            for (start, end) in &ranges {
                let start_frame =
                    ((*start * pcm.sample_rate as f64).round() as usize).min(frames);
                let end_frame =
                    ((*end * pcm.sample_rate as f64).round() as usize).clamp(start_frame, frames);
                bleep_over(
                    &mut samples,
                    pcm.channels,
                    start_frame,
                    end_frame,
                    pcm.sample_rate,
                    BLEEP_FREQUENCY,
                );
            }
            samples
        } else {
            pcm.samples.clone()
        };
        Some(encode_wav_bytes(
            &samples,
            pcm.sample_rate,
            pcm.channels,
        ))
    })
}

/// Hears the sound — plain, or with the tone laid over the stretches —
/// through the system's own player, and carries the playhead along on
/// a timer of its own.
unsafe fn preview(window: HWND, bleeped: bool) {
    let Some(bytes) = wav_bytes_for(bleeped) else {
        // Nothing decoded yet: the previews say nothing, and say it
        // quietly.
        return;
    };
    let playing = STATE.with(|slot| {
        let mut state = slot.borrow_mut();
        if state.duration <= 0.0 {
            return false;
        }
        // What the sound plays from the top.
        state.playhead = 0.0;
        state.playing = true;
        true
    });
    if !playing {
        return;
    }
    WAV_BYTES.with(|slot| *slot.borrow_mut() = bytes);
    let card_window = window_handle();
    if !card_window.0.is_null() {
        let rested = WAV_BYTES.with(|slot| {
            let borrowed = slot.borrow();
            borrowed.as_ptr() as *const u16
        });
        let _ = unsafe {
            PlaySoundW(
                PCWSTR(rested),
                None,
                SND_MEMORY | SND_ASYNC,
            )
        };
        let _ = unsafe { SetTimer(Some(card_window), PLAY_TIMER_ID, 80, None) };
    }
    refresh_times();
    unsafe {
        let _ = InvalidateRect(Some(window), None, true);
    }
}

/// Says what the window was asked, and puts it away. The stretches are
/// said in thousandths of a second, the way the sound's own making
/// asks it.
unsafe fn run(window: HWND) {
    STATE.with(|slot| {
        let state = slot.borrow();
        if state.drag != BleepDrag::Nothing || state.ranges.is_empty() {
            return;
        }
        let settings = settings_from(&state.ranges);
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

/// The shape the cursor wears where the press would land on the track:
/// a cap is pulled sideways, a stretch slides, empty space draws.
fn cursor_for_track(
    ranges: &[(f64, f64)],
    selected: Option<usize>,
    press: f64,
    reach: f64,
) -> PCWSTR {
    match pick_bleep(ranges, selected, press, reach) {
        Some(BleepPick::Edge { .. }) => IDC_SIZEWE,
        Some(BleepPick::Move { .. }) => IDC_SIZEALL,
        None => IDC_CROSS,
    }
}

/// A press on the Bleep track: it chooses what it took — a cap of some
/// stretch, a whole stretch, or nowhere — and a fresh stretch begins.
unsafe fn track_pressed(window: HWND, x: f64, whole: i32, scale: f32) {
    let width = track_width(whole, scale);
    let press_secs = (x / width.max(1.0)).clamp(0.0, 1.0);
    STATE.with(|slot| {
        let mut state = slot.borrow_mut();
        if state.duration <= MIN_WINDOW || state.drag != BleepDrag::Nothing {
            return;
        }
        let press = press_secs * state.duration;
        let reach = CAP_REACH / width;
        match pick_bleep(&state.ranges, state.selected, press, reach) {
            Some(BleepPick::Edge { index, start }) => {
                state.selected = Some(index);
                state.drag = BleepDrag::Edge { index, start };
            }
            Some(BleepPick::Move { index, grab, span }) => {
                state.selected = Some(index);
                state.drag = BleepDrag::Move { index, grab, span };
            }
            None => {
                // A fresh stretch begins empty and grows while it is
                // drawn; nothing stands chosen while it is.
                state.selected = None;
                state.ranges.push((press, press));
                state.drag = BleepDrag::New { from: press };
            }
        }
    });
    unsafe {
        let _ = SetCapture(window);
    }
}

/// A press on the peaks: the playhead is brought to where it stands.
unsafe fn seek_pressed(window: HWND, x: f64, whole: i32, scale: f32) {
    STATE.with(|slot| {
        let mut state = slot.borrow_mut();
        if state.duration <= 0.0 || state.drag != BleepDrag::Nothing {
            return;
        }
        state.playhead = (x / track_width(whole, scale).max(1.0)).clamp(0.0, 1.0)
            * state.duration;
        state.drag = BleepDrag::Seek;
    });
    unsafe {
        let _ = SetCapture(window);
    }
    refresh_times();
    unsafe {
        stand_all(window);
    }
}

/// A move carries a drag along: a stretch is drawn out, slid, or
/// pulled at one cap; the playhead follows the pointer on the peaks.
fn drag_moved(x: f64, whole: i32, scale: f32) {
    let width = track_width(whole, scale);
    STATE.with(|slot| {
        let mut state = slot.borrow_mut();
        let frac = (x / width).clamp(0.0, 1.0);
        match state.drag {
            BleepDrag::Nothing => {}
            BleepDrag::Seek => {
                state.playhead = frac * state.duration;
            }
            BleepDrag::New { from } => {
                let to = frac * state.duration;
                let last = state.ranges.len() - 1;
                state.ranges[last] = stretched_window(from, to);
            }
            BleepDrag::Move { index, grab, span } => {
                let now = frac * state.duration;
                let original = (0.0, span);
                let moved = moved_window(original, grab, now, state.duration);
                if index < state.ranges.len() {
                    state.ranges[index] = moved;
                }
            }
            BleepDrag::Edge { index, start } => {
                let now = frac * state.duration;
                if index < state.ranges.len() {
                    let sitting = state.ranges[index];
                    state.ranges[index] = edged_window(sitting, start, now, state.duration);
                }
            }
        }
    });
}

/// Converts what the pointer did on the window into what the stretches
/// and the playhead should hold. The press is read here, where the
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
    let (whole, scale) = HOST.with(|slot| {
        let borrowed = slot.borrow();
        let Some(host) = borrowed.as_ref() else {
            return (420, 1.0);
        };
        (host.width as i32, scale_factor(host.window))
    });
    let bleeps = bleep_track(whole, scale);
    let wave = audio_track(whole, scale);

    if pressed {
        if in_track(bleeps, x, y) {
            unsafe { track_pressed(window, (x - bleeps.left) as f64, whole, scale) };
        } else if in_track(wave, x, y) {
            unsafe { seek_pressed(window, (x - wave.left) as f64, whole, scale) };
        }
        return;
    }

    // A move carries a drag along, or only aims the cursor.
    let busy = STATE.with(|slot| slot.borrow().drag) != BleepDrag::Nothing;
    if busy {
        drag_moved((x - bleeps.left) as f64, whole, scale);
        refresh_times();
        unsafe { stand_all(window) };
        if STATE.with(|slot| slot.borrow().drag) == BleepDrag::Seek {
            refresh_times();
        }
    } else {
        aim_pointer(x, y, whole, scale, bleeps, wave);
    }

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

/// Puts the cursor in the shape the pointer's own news asks for: over
/// the track a stretch's business, over the rest the arrow answers
/// for itself.
fn aim_pointer(x: i32, y: i32, whole: i32, scale: f32, bleeps: RECT, wave: RECT) {
    let aimed = STATE.with(|slot| {
        let state = slot.borrow();
        if in_track(bleeps, x, y) && state.duration > MIN_WINDOW {
            let press = ((x - bleeps.left) as f64 / track_width(whole, scale))
                .clamp(0.0, 1.0)
                * state.duration;
            Some(cursor_for_track(
                &state.ranges,
                state.selected,
                press,
                CAP_REACH / track_width(whole, scale),
            ))
        } else if in_track(wave, x, y) {
            Some(IDC_ARROW)
        } else {
            None
        }
    });
    if let Some(shape) = aimed {
        unsafe {
            let cursor = LoadCursorW(None, shape).unwrap_or_default();
            let _ = SetCursor(Some(cursor));
        }
    }
}

/// Lets a drag go: a fresh stretch smaller than its least is dropped,
/// and everything else keeps what it drew.
unsafe fn released(window: HWND, x: i32, y: i32) {
    let was = STATE.with(|slot| {
        let mut state = slot.borrow_mut();
        let was = state.drag;
        state.drag = BleepDrag::Nothing;
        was
    });
    if was != BleepDrag::Nothing {
        STATE.with(|slot| {
            let mut state = slot.borrow_mut();
            if let BleepDrag::New { .. } = was {
                // A drawing that never grew past its least is dropped.
                let Some(last) = state.ranges.last().copied() else {
                    return;
                };
                if window_too_small(last) {
                    state.ranges.pop();
                    state.selected = None;
                } else {
                    state.selected = Some(state.ranges.len() - 1);
                }
            }
        });
        refresh_times();
        unsafe { stand_all(window) };
    }

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
                    // The peaks come one by one from the worker
                    // elsewhere; when all of them have, the asking
                    // stops.
                    match strip_tick() {
                        Some(piece) => {
                            let _ = KillTimer(Some(window), STRIP_TIMER_ID);
                            wear_wave(piece);
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

    #[test]
    fn the_height_of_a_bleep_window_is_its_bands() {
        assert_eq!(Layout::HEIGHT, 376.0);
        assert_eq!(Layout::HEADER_UNITS + Layout::CONTENT_UNITS + Layout::FOOTER_UNITS, 376.0);
    }

    #[test]
    fn the_layout_stacks_the_body_top_to_bottom() {
        let layout = Layout::of(420, 376, 1.0);

        // The stage of the rows stands between the two hairlines, with
        // a room under the header's own one.
        assert_eq!(layout.hairline_top.top, 62);

        // The count of the bleeps, then the two rows of them.
        assert_eq!(layout.bleeps_label.top, 84);
        assert_eq!(layout.bleep_track.left, 68);
        assert_eq!(layout.bleep_track.right, 396);
        assert_eq!(layout.bleep_track.top, 112);
        assert_eq!(layout.bleep_track.bottom, 138);
        assert_eq!(layout.audio_track.top, 150);
        assert_eq!(layout.audio_track.bottom, 190);
        assert_eq!(layout.time_left.top, 198);
        assert_eq!(layout.time_right.left, 272);
        assert_eq!(layout.helper.top, 220);
        assert_eq!(layout.hairline_mid.top, 262);

        // The preview row, the two pills at its right.
        assert_eq!(layout.original.left, 214);
        assert_eq!(layout.original.right, 278);
        assert_eq!(layout.with_bleeps.left, 282);
        assert_eq!(layout.with_bleeps.right, 396);
        assert_eq!(layout.hairline_bottom.top, 312);

        // The footer: measured from the bottom, the making of the copy
        // at the right.
        assert_eq!(layout.run.left, 228);
        assert_eq!(layout.run.right - layout.run.left, 168);
        assert_eq!(layout.run.top, 326);
        assert_eq!(layout.run.bottom, 360);
    }

    #[test]
    fn the_tracks_answer_where_a_press_lands() {
        let bleeps = bleep_track(420, 1.0);
        assert_eq!(bleeps.left, 68);
        assert_eq!(bleeps.top, 112);
        assert_eq!(bleeps.bottom - bleeps.top, 26);
        let wave = audio_track(420, 1.0);
        assert_eq!(wave.top, 150);
        assert_eq!(wave.bottom - wave.top, 40);
        // In and out: the air between the rows is nobody's.
        assert!(in_track(bleeps, 100, 130));
        assert!(!in_track(bleeps, 100, 145));
        assert!(in_track(wave, 300, 170));
        assert!(!in_track(wave, 60, 170));
    }

    #[test]
    fn a_pixel_of_the_track_says_its_own_second() {
        // The left edge is the start of the sound; the right edge its
        // end; between them, the share of it.
        assert_eq!(track_secs(68, 420, 1.0, 2.0), 0.0);
        let half = track_secs(232, 420, 1.0, 2.0);
        assert!((half - 1.0).abs() < 1e-9);
        assert_eq!(track_secs(500, 420, 1.0, 2.0), 2.0);
        // Nothing to say: no second is pressed.
        assert_eq!(track_secs(232, 420, 1.0, 0.0), 0.0);
    }

    #[test]
    fn a_press_on_a_track_takes_a_cap_a_slide_or_a_fresh_stretch() {
        let ranges = [(3.0, 7.0)];
        // The cap at each end takes a press beside it.
        assert_eq!(
            pick_bleep(&ranges, Some(0), 3.1, 0.2),
            Some(BleepPick::Edge { index: 0, start: true })
        );
        assert_eq!(
            pick_bleep(&ranges, Some(0), 6.9, 0.2),
            Some(BleepPick::Edge { index: 0, start: false })
        );
        // Between the caps: the whole stretch comes along.
        assert_eq!(
            pick_bleep(&ranges, Some(0), 5.0, 0.2),
            Some(BleepPick::Move {
                index: 0,
                grab: 2.0,
                span: 4.0
            })
        );
        // Away from it on a track that has one: a fresh stretch is
        // drawn.
        assert_eq!(pick_bleep(&ranges, Some(0), 9.0, 0.2), None);
        // A track with nothing on it: whatever is pressed, a stretch
        // is drawn.
        assert_eq!(pick_bleep(&[], None, 2.0, 0.2), None);
    }

    #[test]
    fn the_chosen_stretch_answers_first() {
        // Two stretches sharing the track; the second chosen. A press
        // in their shared middle carries the chosen one along.
        let ranges = [(2.0, 6.0), (4.0, 8.0)];
        assert_eq!(
            pick_bleep(&ranges, Some(1), 5.0, 0.5),
            Some(BleepPick::Move {
                index: 1,
                grab: 1.0,
                span: 4.0
            })
        );
        // The same press with nothing chosen takes the first drawn.
        assert_eq!(
            pick_bleep(&ranges, None, 5.0, 0.5),
            Some(BleepPick::Move {
                index: 0,
                grab: 3.0,
                span: 4.0
            })
        );
    }

    #[test]
    fn a_stretch_drawn_between_two_places_stands_there() {
        assert_eq!(stretched_window(4.0, 2.0), (2.0, 4.0));
        assert_eq!(stretched_window(1.0, 9.0), (1.0, 9.0));
    }

    #[test]
    fn a_stretch_carried_along_stays_in_the_sound() {
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
        assert_eq!(edged_window((3.0, 7.0), true, 9.5, 10.0).0, 6.75);
        // The end cap: the start keeps its word, even past the sound's
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
    fn a_stretch_is_said_in_milliseconds() {
        let settings = settings_from(&[(0.25, 1.0), (1.5, 2.25)]);
        assert_eq!(
            settings.bleep_ranges,
            vec![
                BleepRange {
                    start_ms: 250,
                    end_ms: 1000
                },
                BleepRange {
                    start_ms: 1500,
                    end_ms: 2250
                },
            ]
        );
        // Rounding, half away from zero; a stretch before the sound is
        // kept at its start.
        let rounded = settings_from(&[(-0.5, 2.1234)]);
        assert_eq!(rounded.bleep_ranges[0].start_ms, 0);
        assert_eq!(rounded.bleep_ranges[0].end_ms, 2123);
        // The tone sings at the reference's own pitch.
        assert_eq!(settings.bleep_frequency_hz(), 1000);
    }

    #[test]
    fn the_sound_says_its_own_length() {
        // Half a minute of one channel, said by eight thousandths of a
        // second apiece.
        let state = BleepState {
            pcm: Some(PcmAudio {
                samples: vec![0; 240_000],
                sample_rate: 8000,
                channels: 1,
            }),
            ..Default::default()
        };
        assert_eq!(said_of(&state), Some(30.0));
        // Two channels share every frame: the same length is said by
        // twice the samples' say.
        let wide = BleepState {
            pcm: Some(PcmAudio {
                samples: vec![0; 480_000],
                sample_rate: 8000,
                channels: 2,
            }),
            ..Default::default()
        };
        assert_eq!(said_of(&wide), Some(30.0));
        // Nothing decoded yet.
        assert_eq!(said_of(&BleepState::default()), None);
    }
}

#[cfg(test)]
mod show_tests {
    use super::*;
    use crate::platform::windows::windowhost::screenshot::capture;
    use windows::Win32::System::SystemServices::MK_LBUTTON;
    use windows::Win32::UI::Input::KeyboardAndMouse::VK_RETURN;
    use windows::Win32::UI::WindowsAndMessaging::{
        DispatchMessageW, MSG, PeekMessageW, PM_REMOVE, SendMessageW, TranslateMessage,
    };

    /// Makes a 2-second WAV file with two bursts of sound, so the
    /// capture has real peaks to wear and a real length to say.
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
            // Two bursts: 0.35s..0.85s and 1.15s..1.65s.
            let active = (0.35..0.85).contains(&t) || (1.15..1.65).contains(&t);
            let sample: i16 = if active {
                if (i / 8) % 2 == 0 {
                    24000
                } else {
                    -24000
                }
            } else {
                0
            };
            wav.extend_from_slice(&sample.to_le_bytes());
        }
        let path = folder.join("Voice with pauses.wav");
        std::fs::write(&path, &wav).expect("test audio written");
        path
    }

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
    fn showing_and_pumping_the_redact_audio_card_does_not_panic() {
        crate::ui::wheel::install_platform();
        let (sender, _receiver) = std::sync::mpsc::channel();
        let mut window = RedactAudioWindow::new(sender).expect("the window was made");
        window.show_for(
            "tools.bleep",
            "Bleep audio",
            "Cover private parts of sounds",
            &[PathBuf::from("not-there.wav")],
        );
        pump(40);
        window.hide();
    }

    #[test]
    fn a_pill_answers_the_pointer_without_the_host_being_taken() {
        crate::ui::wheel::install_platform();
        let (sender, _receiver) = std::sync::mpsc::channel();
        let mut window = RedactAudioWindow::new(sender).expect("the window was made");
        window.show_for(
            "tools.bleep",
            "Bleep audio",
            "Cover private parts of sounds",
            &[PathBuf::from("not-there.wav")],
        );
        pump(10);
        // Where the original pill stands, said in the card's own units.
        let told = HOST.with(|slot| {
            let borrowed = slot.borrow();
            let Some(host) = borrowed.as_ref() else {
                return None;
            };
            let scale = scale_factor(host.window);
            let layout = Layout::of(host.width as i32, host.height as i32, scale);
            let middle = (layout.original.left + layout.original.right) as f32 / 2.0 / scale;
            let middle_y = (layout.original.top + layout.original.bottom) as f32 / 2.0 / scale;
            Some((host.software.clone(), middle, middle_y))
        });
        let Some((software, x, y)) = told else {
            panic!("the card was not made");
        };
        // A press and a let-go on the pill, the way a person takes the
        // mouse. Nothing decoded yet: the preview says nothing, and
        // says it quietly, and the state keeps its own word.
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
        let playing = STATE.with(|slot| slot.borrow().playing);
        assert!(!playing, "a preview without a sound stays quiet");
        window.hide();
    }

    /// Opens the bleep window on a real sound, lets it come in and
    /// draw, and captures what a person would see; then drag a
    /// stretch along the track and save, and the window's own message
    /// says what the copy would be made by.
    #[test]
    #[ignore = "needs the conversion program; writes a screenshot and plays a sound"]
    fn capture_the_redact_audio_window() {
        crate::ui::wheel::install_platform();
        let folder = std::env::temp_dir().join("open-tangerine-capture");
        std::fs::create_dir_all(&folder).expect("the capture folder was made");
        let sound = make_test_audio(&folder);
        let (sender, receiver) = std::sync::mpsc::channel();
        let mut window = RedactAudioWindow::new(sender).expect("the window was made");
        window.show_for(
            "tools.bleep",
            "Bleep audio",
            "Cover private parts of sounds",
            &[sound],
        );
        // Long enough for the fade to finish and the peaks to be worn:
        // the sound is decoded whole elsewhere, and the shot wants it
        // there. The window's own bell serves the peaks while the
        // messages are pumped; the shot waits only for a wearing, and
        // gives the whole thing up after a while so a window without
        // the conversion program still takes its picture.
        for round in 0..2400 {
            unsafe {
                let mut message = MSG::default();
                while PeekMessageW(&mut message, None, 0, 0, PM_REMOVE).as_bool() {
                    let _ = TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
            if get_strip_worn() && get_fade_done() && round > 60 {
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
                    shots.join("redact-audio-buffer.png"),
                    &rgba,
                    host.width,
                    host.height,
                    image::ExtendedColorType::Rgba8,
                );
            }
        });
        capture(window.window, &shots.join("redact-audio-window.png"));

        // A stretch dragged along the track through the window's own
        // road: pressed a tenth in, carried to two fifths, let go.
        unsafe {
            let _ = SendMessageW(
                window.window,
                WM_LBUTTONDOWN,
                Some(WPARAM(MK_LBUTTON.0 as usize)),
                Some(pack(100, 125)),
            );
            let _ = SendMessageW(window.window, WM_MOUSEMOVE, Some(WPARAM(0)), Some(pack(200, 125)));
            let _ = SendMessageW(window.window, WM_LBUTTONUP, Some(WPARAM(0)), Some(pack(200, 125)));
        }
        pump(3);
        // Enter says save: the copy's own making was asked, and it was
        // asked with the stretch in it.
        unsafe {
            let _ = SendMessageW(
                window.window,
                WM_KEYDOWN,
                Some(WPARAM(VK_RETURN.0 as usize)),
                Some(LPARAM(0)),
            );
        }
        pump(2);
        let asked = receiver
            .try_recv()
            .expect("the save was asked for through the window's own road");
        let ToolEvent::Run { settings, .. } = asked else {
            panic!("the save was asked for");
        };
        assert_eq!(
            settings.bleep_ranges,
            vec![BleepRange {
                start_ms: 195,
                end_ms: 805,
            }],
            "the stretch was said in milliseconds of the sound"
        );
        // The tone sings at the reference's own pitch.
        assert_eq!(settings.bleep_frequency_hz(), 1000);
        window.hide();
        let _ = std::fs::remove_dir_all(&folder);
    }
}





