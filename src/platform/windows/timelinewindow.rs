//! The window a video is trimmed in.
//!
//! The trim does not run the moment it is picked: it opens a window with
//! the video's own timeline in it — the frame at the playhead, the bar
//! with the two handles that hold where the video begins and ends, the
//! time codes, and the step buttons. The person moves the handles with
//! the pointer or the keyboard, watches the frame follow the playhead,
//! and says go or no.
//!
//! The card is drawn by the same renderer that draws the wheel, and the
//! skeleton it wears is the shell's — the same header, hairlines and
//! footer every tool window wears. The timeline is only drawn by the
//! card: the pressing, dragging and stepping are read by the window
//! itself, which holds the same geometry the card was given, the way the
//! wheel works. Each frame the playhead stands on is asked of the
//! conversion program with a fast seek, the moment it is asked for.
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
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, ReleaseCapture, SetCapture, SetFocus, VK_DOWN, VK_ESCAPE, VK_LEFT, VK_RETURN,
    VK_RIGHT, VK_SHIFT, VK_SPACE, VK_TAB, VK_UP,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, GetClientRect, KillTimer, LoadCursorW, RegisterClassW,
    SetForegroundWindow, SetLayeredWindowAttributes, SetTimer,
    SetWindowTextW, ShowWindow, SW_HIDE, SW_SHOW, CW_USEDEFAULT, IDC_ARROW, LWA_ALPHA,
    MINMAXINFO, WNDCLASSW, WM_CLOSE, WM_DESTROY, WM_ERASEBKGND, WM_GETMINMAXINFO, WM_KEYDOWN,
    WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_PAINT, WM_SIZE, WM_TIMER, WS_EX_LAYERED,
    WS_MAXIMIZEBOX, WS_OVERLAPPEDWINDOW,
};

use crate::media::frames::{duration_of, frame_at};
use crate::media::locator;
use crate::platform::windows::toolwindow::ToolEvent;
use crate::radial::execution::ToolSettings;
use crate::ui::wheel::{window_for_next_component, Bgra8, Panel, TrimCard};

/// The timer the window comes in softly on.
const FADE_TIMER_ID: usize = 52;

/// The card, in the drawing's own units, before the display stretches it.
const CARD_W: i32 = 460;
const CARD_H: i32 = 560;
/// How small the card may be made.
const MIN_W: i32 = 420;
const MIN_H: i32 = 460;

/// The least a trim may keep, in seconds. Less than this and the video
/// is not being trimmed, it is being cut to a blink.
const MIN_SPAN: f64 = 0.5;

/// How close a press must be to take a handle instead of the playhead,
/// in pixels of the drawing.
const HANDLE_REACH: i32 = 14;

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

/// What the window is open for, and where the video stands in it.
struct TrimState {
    duration: f64,
    start: f64,
    end: f64,
    playhead: f64,
    dragging: Drag,
    /// Where the frame showing was taken, so it is only asked for again
    /// when the playhead has truly moved.
    frame_at: f64,
    ffmpeg: Option<locator::Tools>,
    input: PathBuf,
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
            ffmpeg: None,
            input: PathBuf::new(),
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
    // Where the keyboard is: 0 the start handle, 1 the end handle,
    // 2 cancel, 3 run.
    static FOCUS_INDEX: std::cell::Cell<i32> = const { std::cell::Cell::new(0) };
    // How far along the soft coming-in is.
    static FADE_ALPHA: std::cell::Cell<u8> = const { std::cell::Cell::new(255) };
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
        // The card is made at the size the display stretches it to, and
        // the card's own white is what fills it while it comes in.
        let scale = screen_scale();
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
                WS_OVERLAPPEDWINDOW & !WS_MAXIMIZEBOX,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                (CARD_W as f32 * scale) as i32,
                (CARD_H as f32 * scale) as i32,
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
    /// The first video is the one asked about: how long it is, and what
    /// its first moment looks like. Both are asked before the window is
    /// shown, so what appears is already the video's own card.
    pub fn show_for(&mut self, operation: &str, label: &str, detail: &str, inputs: &[PathBuf]) {
        OPEN_FOR.with(|slot| {
            *slot.borrow_mut() = Some((operation.to_string(), inputs.to_vec()));
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
        let ffmpeg = locator::find();
        let duration = ffmpeg
            .as_ref()
            .and_then(|tools| duration_of(tools, &input))
            .unwrap_or(0.0);
        STATE.with(|slot| {
            *slot.borrow_mut() = TrimState {
                duration,
                start: 0.0,
                end: duration,
                playhead: 0.0,
                dragging: Drag::Nothing,
                frame_at: f64::NEG_INFINITY,
                ffmpeg: ffmpeg.clone(),
                input: input.clone(),
            };
        });

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
            host.lay_out();
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
        refresh_frame(true);
        unsafe {
            let _ = SetWindowTextW(card_window, PCWSTR::from_raw(to_wide(label).as_ptr()));
            let _ = InvalidateRect(Some(card_window), None, true);
            // The window comes in softly, from nothing to whole in a few
            // beats of the timer.
            FADE_ALPHA.with(|slot| slot.set(0));
            let _ = SetLayeredWindowAttributes(card_window, COLORREF(0), 0, LWA_ALPHA);
            let _ = SetForegroundWindow(card_window);
            ShowWindow(card_window, SW_SHOW);
            let _ = SetTimer(Some(card_window), FADE_TIMER_ID, 16, None);
            // The keyboard starts on the handle that holds where the
            // video begins.
            FOCUS_INDEX.with(|slot| slot.set(0));
            apply_focus(card_window, 0);
        }
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
        self.card.set_frame_box(panel(layout.frame));
        self.card.set_timeline_box(panel(layout.timeline));
        self.card.set_codes_box(panel(layout.codes));
        self.card.set_step_back_box(panel(layout.step_back));
        self.card.set_step_forward_box(panel(layout.step_forward));
        self.card.set_hint_box(panel(layout.hint));
        self.card.set_helper_box(panel(layout.helper));
        self.card.set_cancel_box(panel(layout.cancel));
        self.card.set_run_box(panel(layout.run));
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
#[derive(Clone, Copy, Debug, Default)]
struct Layout {
    file_name: RECT,
    detail: RECT,
    hairline_top: RECT,
    hairline_bottom: RECT,
    frame: RECT,
    timeline: RECT,
    codes: RECT,
    step_back: RECT,
    step_forward: RECT,
    hint: RECT,
    helper: RECT,
    cancel: RECT,
    run: RECT,
}

impl Layout {
    fn of(whole: i32, side: i32, scale: f32) -> Layout {
        let at = |value: i32| (value as f32 * scale) as i32;
        let margin = at(24);

        let mut layout = Layout {
            file_name: RECT {
                left: margin,
                top: at(18),
                right: whole - margin,
                bottom: at(18) + at(22),
            },
            detail: RECT {
                left: margin,
                top: at(42),
                right: whole - margin,
                bottom: at(42) + at(16),
            },
            hairline_top: RECT {
                left: 0,
                top: at(64),
                right: whole,
                bottom: at(64) + 1,
            },
            hairline_bottom: RECT {
                left: 0,
                top: side - at(72),
                right: whole,
                bottom: side - at(72) + 1,
            },
            frame: RECT::default(),
            timeline: RECT::default(),
            codes: RECT::default(),
            step_back: RECT::default(),
            step_forward: RECT::default(),
            hint: RECT::default(),
            helper: RECT::default(),
            cancel: RECT::default(),
            run: RECT::default(),
        };

        // The footer: no and go on the right, the shortcuts on the left.
        let footer_top = side - at(72);
        let button_top = footer_top + at(16);
        let button_bottom = button_top + at(40);
        layout.run = RECT {
            left: whole - margin - at(120),
            top: button_top,
            right: whole - margin,
            bottom: button_bottom,
        };
        layout.cancel = RECT {
            left: whole - margin - at(120) - at(12) - at(96),
            top: button_top,
            right: whole - margin - at(120) - at(12),
            bottom: button_bottom,
        };
        layout.helper = RECT {
            left: margin,
            top: button_top,
            right: layout.cancel.left - at(16),
            bottom: button_bottom,
        };

        // The body, top to bottom: the frame, the timeline, the time
        // codes, the step buttons with the hint beside them.
        let timeline_top = at(384);
        layout.timeline = RECT {
            left: margin,
            top: timeline_top,
            right: whole - margin,
            bottom: timeline_top + at(32),
        };
        layout.codes = RECT {
            left: margin,
            top: at(424),
            right: whole - margin,
            bottom: at(424) + at(18),
        };
        layout.step_back = RECT {
            left: margin,
            top: at(450),
            right: margin + at(72),
            bottom: at(450) + at(28),
        };
        layout.step_forward = RECT {
            left: margin + at(72) + at(8),
            top: at(450),
            right: margin + at(72) + at(8) + at(72),
            bottom: at(450) + at(28),
        };
        layout.hint = RECT {
            left: margin + at(72) + at(8) + at(72) + at(16),
            top: at(450),
            right: whole - margin,
            bottom: at(450) + at(28),
        };
        layout.frame = RECT {
            left: margin,
            top: layout.hairline_top.bottom + at(14),
            right: whole - margin,
            bottom: timeline_top - at(12),
        };
        layout
    }
}

/// A share of the whole, from a moment of it.
fn fraction(moment: f64, duration: f64) -> f32 {
    if duration > 0.0 {
        (moment / duration).clamp(0.0, 1.0) as f32
    } else {
        0.0
    }
}

/// How a moment is said: minutes, seconds, and a tenth of a second.
fn timecode(moment: f64) -> String {
    let moment = moment.max(0.0);
    let minutes = (moment / 60.0) as u64;
    let seconds = moment - (minutes as f64) * 60.0;
    format!("{minutes}:{seconds:04.1}")
}

/// Says what the window was asked, to whoever is listening.
fn send(event: ToolEvent) {
    EVENTS.with(|slot| {
        if let Some(sender) = slot.borrow().as_ref() {
            let _ = sender.send(event);
        }
    });
}

/// Puts the keyboard on one stop of the card: a handle or a button.
unsafe fn apply_focus(window: HWND, index: i32) {
    // The focus is the window's own everywhere: the handles are drawn,
    // not made of the system's stuff, and the keyboard is read here.
    unsafe { let _ = SetFocus(Some(window)); }
    HOST.with(|slot| {
        let mut borrowed = slot.borrow_mut();
        if let Some(host) = borrowed.as_mut() {
            host.card
                .set_handle_focus(if index == 0 || index == 1 { index + 1 } else { 0 });
            host.card
                .set_button_focus(if index >= 2 { index - 1 } else { 0 });
            host.software.request_redraw();
        }
    });
    let _ = unsafe { InvalidateRect(Some(window), None, true) };
}

/// Walks the keyboard to the next stop of the card, or the one before.
unsafe fn cycle_focus(window: HWND, forward: bool) {
    let stops: [i32; 4] = [0, 1, 2, 3];
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
/// playhead stands.
fn refresh_times() {
    HOST.with(|slot| {
        let mut borrowed = slot.borrow_mut();
        if let Some(host) = borrowed.as_mut() {
            STATE.with(|state| {
                let state = state.borrow();
                host.card.set_start_fraction(fraction(state.start, state.duration));
                host.card.set_end_fraction(fraction(state.end, state.duration));
                host.card
                    .set_playhead_fraction(fraction(state.playhead, state.duration));
                host.card.set_start_text(timecode(state.start).into());
                host.card.set_playhead_text(timecode(state.playhead).into());
                host.card.set_end_text(timecode(state.end).into());
            });
            host.software.request_redraw();
        }
    });
}

/// Draws the frame the playhead stands on, if it has moved far enough
/// for a new one to be worth a seek.
fn refresh_frame(force: bool) {
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
                let small = picture.thumbnail(412, 290);
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
    let near = y >= bar.top - at_scale(8, scale)
        && y <= bar.bottom + at_scale(8, scale)
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
            let take_start = (x - along(state.start)).abs() <= at_scale(HANDLE_REACH, scale);
            let take_end = (x - along(state.end)).abs() <= at_scale(HANDLE_REACH, scale);
            state.dragging = if take_start && !take_end {
                Drag::Start
            } else if take_end {
                Drag::End
            } else {
                Drag::Playhead
            };
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
}

/// How many pixels a distance of the drawing is, on this display.
fn at_scale(value: i32, scale: f32) -> i32 {
    (value as f32 * scale) as i32
}

/// Lets go of whatever the pointer was holding.
fn released(window: HWND) {
    STATE.with(|slot| slot.borrow_mut().dragging = Drag::Nothing);
    unsafe { let _ = ReleaseCapture(); };
    let _ = unsafe { InvalidateRect(Some(window), None, true) };
}

/// Steps the handle the keyboard is on, back or forward a second — or
/// five, when Shift is held.
unsafe fn step(window: HWND, direction: f64) {
    let shift = unsafe { GetKeyState(VK_SHIFT.0 as i32) < 0 };
    let by = direction * if shift { 5.0 } else { 1.0 };
    let focused = FOCUS_INDEX.with(|slot| slot.get());
    STATE.with(|slot| {
        let mut state = slot.borrow_mut();
        if state.duration <= 0.0 {
            return;
        }
        match focused {
            0 => state.start = (state.start + by).clamp(0.0, state.end - MIN_SPAN),
            1 => state.end = (state.end + by).clamp(state.start + MIN_SPAN, state.duration),
            _ => state.playhead = (state.playhead + by).clamp(0.0, state.duration),
        }
    });
    refresh_frame(false);
    refresh_times();
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
                if FOCUS_INDEX.with(|slot| slot.get()) == 2 {
                    unsafe { cancel(window) };
                } else {
                    unsafe { run(window) };
                }
                LRESULT(0)
            } else if key == VK_ESCAPE.0 {
                unsafe { cancel(window) };
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
            }
            LRESULT(0)
        }
        WM_GETMINMAXINFO => {
            unsafe {
                let scale = scale_factor(window);
                let info = &mut *(lparam.0 as *mut MINMAXINFO);
                info.ptMinTrackSize.x = (MIN_W as f32 * scale) as i32;
                info.ptMinTrackSize.y = (MIN_H as f32 * scale) as i32;
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

/// Runs what the window was open for, with the range the handles hold.
unsafe fn run(window: HWND) {
    let settings = STATE.with(|slot| {
        let state = slot.borrow();
        if state.duration > 0.0 {
            ToolSettings {
                start_ms: Some((state.start * 1000.0).round() as u64),
                end_ms: Some((state.end * 1000.0).round() as u64),
                ..Default::default()
            }
        } else {
            ToolSettings::default()
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

/// How much the display stretches everything.
fn scale_factor(window: HWND) -> f32 {
    unsafe {
        let context = windows::Win32::Graphics::Gdi::GetDC(Some(window));
        let dots = windows::Win32::Graphics::Gdi::GetDeviceCaps(
            Some(context),
            windows::Win32::Graphics::Gdi::LOGPIXELSY,
        );
        let _ = windows::Win32::Graphics::Gdi::ReleaseDC(Some(window), context);
        dots as f32 / 96.0
    }
}

/// How much the display stretches everything, before any window is made
/// to ask it of.
fn screen_scale() -> f32 {
    unsafe {
        let context = windows::Win32::Graphics::Gdi::GetDC(None);
        let dots = windows::Win32::Graphics::Gdi::GetDeviceCaps(
            Some(context),
            windows::Win32::Graphics::Gdi::LOGPIXELSY,
        );
        let _ = windows::Win32::Graphics::Gdi::ReleaseDC(None, context);
        dots as f32 / 96.0
    }
}

/// Words in the wide letters the system reads.
fn to_wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain([0]).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_moment_is_said_in_minutes_and_tenths() {
        assert_eq!(timecode(0.0), "0:00.0");
        assert_eq!(timecode(4.06), "0:04.1");
        assert_eq!(timecode(65.34), "1:05.3");
        assert_eq!(timecode(600.0), "10:00.0");
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
    fn the_layout_stacks_the_body_top_to_bottom() {
        let layout = Layout::of(460, 560, 1.0);
        // The buttons sit inside the window, next to each other.
        assert!(layout.cancel.right <= layout.run.left);
        assert!(layout.run.bottom <= 560);
        // The frame, then the timeline, then the codes, then the steps.
        assert!(layout.frame.bottom <= layout.timeline.top);
        assert!(layout.timeline.bottom <= layout.codes.top);
        assert!(layout.codes.bottom <= layout.step_back.top);
        assert!(layout.step_back.bottom <= layout.hairline_bottom.top);
        assert_eq!(layout.step_back.top, layout.step_forward.top);
        // The hint says its piece beside the steps, not under them.
        assert!(layout.hint.left >= layout.step_forward.right);
        assert!(layout.helper.right <= layout.cancel.left);
    }
}

#[cfg(test)]
mod show_tests {
    use super::*;
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
}
