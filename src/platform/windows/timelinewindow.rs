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
use slint::{ComponentHandle, LogicalPosition, PhysicalSize, SharedPixelBuffer};
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{BeginPaint, CreateSolidBrush, InvalidateRect};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::AdjustWindowRectExForDpi;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, ReleaseCapture, SetCapture, SetFocus, VK_DOWN, VK_ESCAPE, VK_LEFT, VK_RETURN,
    VK_RIGHT, VK_SHIFT, VK_SPACE, VK_TAB, VK_UP,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, GetClientRect, GetWindowRect, KillTimer, LoadCursorW,
    RegisterClassW, SetForegroundWindow, SetLayeredWindowAttributes, SetTimer, SetWindowPos,
    SetWindowTextW, ShowWindow, SW_HIDE, SW_SHOW, SWP_NOACTIVATE, SWP_NOZORDER, CW_USEDEFAULT,
    IDC_ARROW, LWA_ALPHA, WNDCLASSW, WM_CLOSE, WM_DESTROY, WM_DPICHANGED, WM_ERASEBKGND,
    WM_KEYDOWN, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_PAINT, WM_SIZE, WM_TIMER,
    WS_EX_LAYERED, WS_MAXIMIZEBOX, WS_OVERLAPPEDWINDOW, WS_THICKFRAME,
};

use crate::media::frames::{duration_of, frame_at};
use crate::media::locator;
use crate::platform::windows::toolwindow::ToolEvent;
use crate::radial::execution::ToolSettings;
use crate::ui::wheel::{window_for_next_component, Bgra8, Panel, TrimCard};

/// The timer the window comes in softly on.
const FADE_TIMER_ID: usize = 52;

/// The card's width, in the drawing's own units.
const CARD_W: i32 = 420;
/// The least and the most a preview may be tall, so a landscape video
/// keeps its shape and a portrait one does not tower.
const PREVIEW_MIN: f32 = 190.0;
const PREVIEW_MAX: f32 = 300.0;

/// The least a trim may keep, in seconds. Less than this and the video
/// is not being trimmed, it is being cut to a blink.
const MIN_SPAN: f64 = 0.5;

/// How close a press must be to take an end cap instead of the playhead,
/// in pixels of the drawing. A cap is 28 wide, so half of it plus a
/// little grace.
const HANDLE_REACH: i32 = 16;

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
    /// How tall the preview wants to be, in the drawing's own units,
    /// from the shape of the video.
    preview_units: f32,
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
            preview_units: 236.0,
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

/// The style of a window that opens the size its content wants and stays
/// that way: a caption to carry it, no grip to stretch it.
fn fixed_style() -> windows::Win32::UI::WindowsAndMessaging::WINDOW_STYLE {
    WS_OVERLAPPEDWINDOW & !(WS_THICKFRAME | WS_MAXIMIZEBOX)
}

/// How wide a window of this card must be made so that what a person
/// uses of it is the size the drawing asked for, at this stretch.
fn outer_size(scale: f32) -> (i32, i32) {
    let client_w = (CARD_W as f32 * scale) as i32;
    let client_h =
        ((Layout::CONTENT_UNITS + client_preview_units() + Layout::FOOTER_UNITS) as f32 * scale)
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
        // be. A video whose shape cannot be read is shown as a wide one.
        let beginning = ffmpeg
            .as_ref()
            .and_then(|tools| frame_at(tools, &input, 0.0));
        let aspect = beginning
            .as_ref()
            .map(|frame| frame.width() as f32 / frame.height().max(1) as f32)
            .unwrap_or(16.0 / 9.0);
        let preview_units = (CARD_W as f32 / aspect).clamp(PREVIEW_MIN, PREVIEW_MAX);
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
            // The keyboard starts on the handle that holds where the
            // video begins.
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
        layout
    }

    /// Draws the card if it changed, and puts what was drawn on the
    /// window.
    fn draw(&mut self) {
        #[cfg(test)]
        eprintln!(
            "draw: timeline={:?} run={:?} hair={:?} frame={}",
            self.card.get_timeline_box(),
            self.card.get_run_box(),
            self.card.get_hairline_bottom(),
            self.card.get_has_frame(),
        );
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
/// out once and both sides read it.
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
}

impl Layout {
    /// Everything of the stack except the picture and the footer, in the
    /// drawing's own units: header, hairline, the gap and the caption
    /// and the strip and the moments and the transport, and the closing
    /// hairline.
    const CONTENT_UNITS: f32 = 243.0;
    /// The footer: the hairline's gap to the buttons, the buttons, and
    /// the air under them.
    const FOOTER_UNITS: f32 = 64.0;

    /// How tall a window of this preview wants to be, in the drawing's
    /// own units. This is the one number the window's size and the
    /// drawing's stack are both worked out from.
    fn height_for(preview_units: f32) -> f32 {
        Self::CONTENT_UNITS + preview_units + Self::FOOTER_UNITS
    }

    fn of(whole: i32, side: i32, scale: f32) -> Layout {
        let at = |value: f32| (value * scale) as i32;
        let margin = at(24.0);
        let preview_units = client_preview_units();

        // The stack, top to bottom, all worked out from the one number.
        let header_top = at(16.0);
        let hairline_top = at(62.0);
        let preview_top = hairline_top + at(20.0);
        let preview_bottom = preview_top + at(preview_units);
        let caption_top = preview_bottom + at(16.0);
        let timeline_top = caption_top + at(16.0) + at(12.0);
        let times_top = timeline_top + at(30.0) + at(10.0);
        let transport_top = times_top + at(18.0) + at(10.0);
        let hairline_bottom = transport_top + at(30.0) + at(18.0);

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
                bottom: caption_top + at(16.0),
            },
            timeline: RECT {
                left: margin,
                top: timeline_top,
                right: whole - margin,
                bottom: timeline_top + at(30.0),
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
        }
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

/// How a moment is said: minutes, seconds and the hundredths, in the
/// face where every digit takes the same room — the way the reference
/// says its times.
fn timecode(moment: f64) -> String {
    let moment = moment.max(0.0);
    let minutes = (moment / 60.0) as u64;
    let seconds = moment - (minutes as f64) * 60.0;
    format!("{minutes}:{seconds:05.2}")
}

/// Says what the window was asked, to whoever is listening.
fn send(event: ToolEvent) {
    EVENTS.with(|slot| {
        if let Some(sender) = slot.borrow().as_ref() {
            let _ = sender.send(event);
        }
    });
}

/// Puts the keyboard on one stop of the card: a cap or the go button.
unsafe fn apply_focus(window: HWND, index: i32) {
    // The focus is the window's own everywhere: the caps are drawn, not
    // made of the system's stuff, and the keyboard is read here.
    unsafe { let _ = SetFocus(Some(window)); }
    HOST.with(|slot| {
        let mut borrowed = slot.borrow_mut();
        if let Some(host) = borrowed.as_mut() {
            host.card
                .set_handle_focus(if index == 0 || index == 1 { index + 1 } else { 0 });
            host.card.set_button_focus(if index == 2 { 2 } else { 0 });
            host.software.request_redraw();
        }
    });
    let _ = unsafe { InvalidateRect(Some(window), None, true) };
}

/// Walks the keyboard to the next stop of the card, or the one before:
/// the start cap, the end cap, and go.
unsafe fn cycle_focus(window: HWND, forward: bool) {
    let stops: [i32; 3] = [0, 1, 2];
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
/// playhead stands, and what all of it says in words.
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
                host.card
                    .set_selected_text(format!("{} selected", timecode(state.end - state.start)).into());
                host.card.set_start_text(timecode(state.start).into());
                host.card.set_end_text(timecode(state.end).into());
                host.card.set_playhead_text(
                    format!("{} / {}", timecode(state.playhead), timecode(state.duration)).into(),
                );
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
            let take_start = (x - along(state.start)).abs() <= at(HANDLE_REACH as f32);
            let take_end = (x - along(state.end)).abs() <= at(HANDLE_REACH as f32);
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
                unsafe { run(window) };
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
        assert_eq!(Layout::height_for(236.0), 543.0);
        // A portrait video is held at the most a preview may be.
        assert_eq!(Layout::height_for(300.0), 607.0);
    }

    #[test]
    fn the_layout_stacks_the_body_top_to_bottom() {
        STATE.with(|slot| slot.borrow_mut().preview_units = 236.0);
        let whole = 420;
        let side = 543;
        let layout = Layout::of(whole, side, 1.0);
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
        let narrow = Layout::of(420, 607, 1.0);
        STATE.with(|slot| slot.borrow_mut().preview_units = 236.0);
        let wide = Layout::of(420, 543, 1.0);
        // The picture takes all the height the shape asked for; nothing
        // else in the stack moves by a pixel.
        assert_eq!(narrow.preview.bottom - narrow.preview.top, 300);
        assert_eq!(wide.preview.bottom - wide.preview.top, 236);
        assert_eq!(narrow.run.left, wide.run.left);
        assert_eq!(narrow.timeline.top - narrow.timeline.bottom,
                   wide.timeline.top - wide.timeline.bottom);
    }
}

#[cfg(test)]
mod show_tests {
    use super::*;
    use crate::media::runner::{ProcessRunner, SystemRunner};
    use windows::Win32::Graphics::Gdi::{
        CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDIBits,
        GetWindowDC, ReleaseDC, SelectObject, BITMAPINFO, BITMAPINFOHEADER,
        BI_RGB, DIB_RGB_COLORS,
    };
    use windows::Win32::Storage::Xps::{PrintWindow, PRINT_WINDOW_FLAGS};
    use windows::Win32::UI::WindowsAndMessaging::{
        DispatchMessageW, GetWindowRect, MSG, PM_REMOVE, PeekMessageW, TranslateMessage,
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

    /// Makes a small test video the way the trim test does, so the
    /// capture has a real frame and a real length to show.
    fn make_test_video(folder: &std::path::Path) -> PathBuf {
        use std::ffi::OsString;
        let ffmpeg = std::path::Path::new("vendor")
            .join("ffmpeg")
            .join("ffmpeg.exe");
        assert!(ffmpeg.is_file(), "the conversion program is not there");
        let raw = folder.join("capture.raw");
        std::fs::write(&raw, vec![255u8, 0, 0, 255].repeat(160 * 120 * 30))
            .expect("the pixels could be written");
        let video = folder.join("capture.mp4");
        let arguments: Vec<OsString> = [
            "-hide_banner",
            "-nostdin",
            "-y",
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgba",
            "-s",
            "160x120",
            "-r",
            "10",
            "-i",
        ]
        .iter()
        .map(|word| OsString::from(*word))
        .chain([
            raw.as_os_str().to_os_string(),
            OsString::from("-c:v"),
            OsString::from("mpeg4"),
            OsString::from("-pix_fmt"),
            OsString::from("yuv420p"),
            OsString::from("-f"),
            OsString::from("mov"),
            video.as_os_str().to_os_string(),
        ])
        .collect();
        let run = SystemRunner.run(&ffmpeg, &arguments, None).expect("the video was made");
        assert!(run.succeeded(), "the capture video was not made: {}", run.stderr);
        video
    }

    /// Puts what the window looks like into a picture, so the design can
    /// be checked without someone sitting at the screen.
    fn capture(window: HWND, path: &std::path::Path) {
        unsafe {
            let mut rect = RECT::default();
            let _ = GetWindowRect(window, &mut rect);
            let width = rect.right - rect.left;
            let height = rect.bottom - rect.top;
            let screen = GetWindowDC(Some(window));
            let memory = CreateCompatibleDC(Some(screen));
            let bitmap = CreateCompatibleBitmap(screen, width, height);
            let old = SelectObject(memory, bitmap.into());
            // PW_RENDERFULLCONTENT: the window is layered and rounded;
            // plain PrintWindow would miss what the system composes.
            const PW_RENDERFULLCONTENT: PRINT_WINDOW_FLAGS =
                PRINT_WINDOW_FLAGS(2);
            let _ = PrintWindow(window, memory, PW_RENDERFULLCONTENT);
            let mut info = BITMAPINFO::default();
            info.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
            info.bmiHeader.biWidth = width;
            info.bmiHeader.biHeight = -height;
            info.bmiHeader.biPlanes = 1;
            info.bmiHeader.biBitCount = 32;
            info.bmiHeader.biCompression = BI_RGB.0 as u32;
            let mut pixels = vec![0u8; (width.max(0) * height.max(0)) as usize * 4];
            let _ = GetDIBits(
                memory,
                bitmap,
                0,
                height as u32,
                Some(pixels.as_mut_ptr() as *mut core::ffi::c_void),
                &mut info,
                DIB_RGB_COLORS,
            );
            let _ = SelectObject(memory, old);
            let _ = DeleteObject(bitmap.into());
            let _ = DeleteDC(memory);
            let _ = ReleaseDC(Some(window), screen);
            // The bytes come back blue-first; the picture wants red-first.
            for pixel in pixels.chunks_exact_mut(4) {
                pixel.swap(0, 2);
            }
            image::save_buffer(
                path,
                &pixels,
                width as u32,
                height as u32,
                image::ExtendedColorType::Rgba8,
            )
            .expect("the capture was written");
        }
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
        // Long enough for the fade to finish and the first frame to land.
        for _ in 0..80 {
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
}
