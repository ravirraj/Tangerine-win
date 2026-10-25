//! The window a video or a picture is cropped in.
//!
//! The crop does not happen the moment it is picked: it opens a window
//! with the media's own picture in it — fitted to a stage, with the
//! rectangle that is kept standing over it in the orange of choosing,
//! a square handle at each of its four corners. Around the stage the
//! aspect presets are two rows of quiet chips, a line of small words
//! says what the rectangle keeps and how big the media is, a video
//! carries a scrub row — a slider, the two step glyphs, and where the
//! playhead stands said against the whole — and a picture carries the
//! Centre and Whole image buttons instead. The footer's Reset puts the
//! rectangle back where it started, and Escape says no.
//!
//! The card is drawn by the same renderer that draws the wheel, and the
//! skeleton it wears is the shell's. The card only draws: the pressing,
//! the corners, the dragging and the walking are read by the window
//! itself, which holds the same geometry the card was given — the way
//! the wheel works. The rectangle is kept whole in the media's own
//! pixels, so what the person sees and what the file is cut by can
//! never disagree; each frame a video's playhead stands on is asked of
//! the conversion program with a fast seek, the way the trim asks.
//!
//! The window is sized once, from what it holds: its width is fixed,
//! its height follows the shape of the media and the class of the card,
//! and it stays that way. When the display under it changes its
//! stretch, the window is made again at the new size.
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
    GetKeyState, ReleaseCapture, SetCapture, VK_DOWN, VK_ESCAPE, VK_LEFT, VK_RETURN, VK_RIGHT,
    VK_SHIFT, VK_SPACE, VK_TAB, VK_UP,
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
use crate::ui::wheel::{window_for_next_component, Bgra8, CropCard, Panel};

/// The timer the window comes in softly on.
const FADE_TIMER_ID: usize = 52;

/// The card's width, in the drawing's own units.
const CARD_W: i32 = 420;
/// The least and the most a stage may be tall, so a landscape media
/// keeps its shape and a portrait one does not tower.
const PREVIEW_MIN: f32 = 190.0;
const PREVIEW_MAX: f32 = 300.0;

/// How far the playhead must move before a new frame is asked of the
/// conversion program, in seconds. A seek costs a moment; the frame
/// does not need to follow the playhead any closer than a person can
/// see.
const FRAME_STEP: f64 = 0.25;

/// How close a press must stand to a corner to take the handle instead
/// of the space around it, in pixels of the drawing — the reach the
/// original's canvas gives its corners.
const CORNER_REACH: f64 = 12.0;

/// How deep a press must stand inside the rectangle before it is
/// understood as a wish to move it rather than a wish to draw a new
/// one, in pixels of the drawing.
const MOVE_INSET: f64 = 4.0;

/// The aspect presets, in the order the card offers them: the free
/// rectangle keeps nothing but what is drawn, and the original keeps
/// the media's own shape, whatever that turns out to be.
const PRESET_FREE: usize = 0;
const PRESET_ORIGINAL: usize = 1;

/// The least a dropped crop may keep, in pixels of the media. A
/// rectangle drawn smaller than this over a video is given up when the
/// pointer lets go — a crop of a video under it is nothing at all. A
/// picture is honest at a single pixel.
const MIN_CROP_VIDEO: f64 = 8.0;

/// What the pointer is holding. A corner is told by its place: 0 the
/// top-left, 1 the top-right, 2 the bottom-right, 3 the bottom-left.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Drag {
    Nothing,
    /// The rectangle is dragged by its middle.
    Track,
    /// A corner is dragged.
    Corner(usize),
    /// A new rectangle is being drawn.
    Create,
    /// The video's playhead is being scrubbed.
    Playhead,
}

/// A place, in pixels. Kept as its own two numbers because every sum of
/// the crop is worked out on places.
type Spot = (f64, f64);

/// The rectangle a crop keeps, in pixels of the media: where it starts,
/// and how far it runs.
type Cut = (f64, f64, f64, f64);

/// A box in the window's own pixels: where a part of the card stands.
type Box4 = (f64, f64, f64, f64);

/// What the window holds about the media it is cropped, and where the
/// pointer is inside it.
struct CropState {
    /// A crop of a video moves in time; a crop of a picture does not.
    video: bool,
    /// The media's own size, in pixels.
    media_w: f64,
    media_h: f64,
    /// Where the media sits on the stage, and how many window pixels
    /// one pixel of the media takes. The rectangle is said in media
    /// pixels and drawn in window pixels, and these two carry it over.
    media_box: Box4,
    fit: f64,
    /// The rectangle that is kept, in pixels of the media. Nothing
    /// stands there when a drawn rectangle was dropped as too small.
    cut: Option<Cut>,
    /// Which aspect preset is chosen, and where the pointer took its
    /// hold when a drag began, and the corner a resize holds still:
    /// the opposite of the one taken.
    preset: usize,
    grab: Spot,
    anchor: Spot,
    dragging: Drag,
    /// How tall the stage is, in the drawing's own units.
    preview_units: f32,
    /// Where the video stands, and which moment the shown frame is of.
    duration: f64,
    playhead: f64,
    frame_at: f64,
    ffmpeg: Option<locator::Tools>,
    input: PathBuf,
}

impl Default for CropState {
    fn default() -> Self {
        Self {
            video: false,
            media_w: 0.0,
            media_h: 0.0,
            media_box: (0.0, 0.0, 0.0, 0.0),
            fit: 1.0,
            cut: None,
            preset: PRESET_FREE,
            grab: (0.0, 0.0),
            anchor: (0.0, 0.0),
            dragging: Drag::Nothing,
            preview_units: 236.0,
            duration: 0.0,
            playhead: 0.0,
            frame_at: f64::NEG_INFINITY,
            ffmpeg: None,
            input: PathBuf::new(),
        }
    }
}

thread_local! {
    static HOST: std::cell::RefCell<Option<Host>> = std::cell::RefCell::new(None);
    static WINDOW: std::cell::Cell<isize> = const { std::cell::Cell::new(0) };
    static STATE: std::cell::RefCell<CropState> = std::cell::RefCell::new(CropState::default());
    static OPEN_FOR: std::cell::RefCell<Option<(String, Vec<PathBuf>)>> =
        const { std::cell::RefCell::new(None) };
    static EVENTS: std::cell::RefCell<Option<Sender<ToolEvent>>> =
        const { std::cell::RefCell::new(None) };
    /// The keyboard's stop: 0 nothing, 1 to 10 a chip, 11 the go button.
    static FOCUS_INDEX: std::cell::Cell<i32> = const { std::cell::Cell::new(0) };
    static FADE_ALPHA: std::cell::Cell<u8> = const { std::cell::Cell::new(255) };
}

/// The window handle, if there is one.
fn window_handle() -> HWND {
    HWND(WINDOW.get() as *mut _)
}

/// The window, its card, and the buffer the card is drawn into.
struct Host {
    window: HWND,
    card: CropCard,
    software: Rc<MinimalSoftwareWindow>,
    buffer: Vec<Bgra8>,
    width: u32,
    height: u32,
}

/// The window a crop is asked in.
pub struct CropWindow {
    window: HWND,
}

// The window belongs to the thread that made it and is us
unsafe impl Send for CropWindow {}

/// The style of a window that opens the size its content wants and stays
/// that way: a caption to carry it, no grip to stretch it.
fn fixed_style() -> windows::Win32::UI::WindowsAndMessaging::WINDOW_STYLE {
    WS_OVERLAPPEDWINDOW & !(WS_THICKFRAME | WS_MAXIMIZEBOX)
}

/// How tall the stage is, in the drawing's own units.
fn client_preview_units() -> f32 {
    STATE.with(|slot| slot.borrow().preview_units)
}

/// Whether the card is for a moving media: a video's card carries the
/// scrub row, a picture's the Centre and Whole image buttons.
fn is_video() -> bool {
    STATE.with(|slot| slot.borrow().video)
}

/// How wide a window of this card must be made so that what a person
/// uses of it is the size the drawing asked for, at this stretch.
fn outer_size(scale: f32) -> (i32, i32) {
    let client_w = (CARD_W as f32 * scale) as i32;
    let content = if is_video() {
        Layout::CONTENT_UNITS_VIDEO
    } else {
        Layout::CONTENT_UNITS_IMAGE
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

impl CropWindow {
    /// Makes the window, hidden until a crop is picked.
    pub fn new(events: Sender<ToolEvent>) -> Result<CropWindow, String> {
        EVENTS.with(|slot| *slot.borrow_mut() = Some(events));

        // A window is put aside for the card before the card is made, so
        // the two find each other.
        let software = window_for_next_component();
        let card = CropCard::new().map_err(|error| error.to_string())?;

        let instance = unsafe { GetModuleHandleW(None).map_err(|error| error.to_string())? };
        let class_name = w!("OpenTangerineCrop");
        // The card's own white is what fills it while it comes in, so a
        // dark theme's idea of "window colour" never shows through. The
        // true size is set when a crop is asked for, from the media's
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
            card.on_reset_chosen(move || {
                let window = window_handle();
                if !window.0.is_null() {
                    unsafe { reset_rect(window) };
                }
            });
            card.on_aspect_chosen(move |chip| {
                let window = window_handle();
                if !window.0.is_null() {
                    unsafe { choose_aspect(window, (chip.max(1) - 1) as usize) };
                }
            });
            card.on_centre_chosen(move || {
                let window = window_handle();
                if !window.0.is_null() {
                    unsafe { centre_rect(window) };
                }
            });
            card.on_whole_chosen(move || {
                let window = window_handle();
                if !window.0.is_null() {
                    unsafe { whole_rect(window) };
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

        Ok(CropWindow { window })
    }

    /// Asks the window to hide itself.
    pub fn hide(&mut self) {
        unsafe { ShowWindow(self.window, SW_HIDE) };
    }

    /// The first media is the one asked about: what it is made of, how
    /// big it is, and from that, how tall the window wants to be. All of
    /// it is asked before the window is shown, so what appears is
    /// already the media's own card at its own shape.
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

        // What the media is, said by the house the wheels already use.
        // A crop works on a video and a picture and nothing else: when
        // what was dragged is neither, the window does not open at all,
        // and no one is left staring at an empty well.
        let kind = match crate::media::probe::kind_of(&extension) {
            Some(crate::media::probe::Kind::Video) => {
                Some(true)
            }
            Some(crate::media::probe::Kind::Image) => Some(false),
            _ => {
                OPEN_FOR.with(|slot| *slot.borrow_mut() = None);
                return;
            }
        };
        let video = kind.expect("a kind was decided above");

        let ffmpeg = locator::find();
        // A video: the fold it opens on answers for its shape and it is
        // asked how long it runs. A picture: it is opened here, with the
        // program's own two hands, and its own size is what it says.
        let (media_w, media_h, duration, opened) = if video {
            let frame = ffmpeg
                .as_ref()
                .and_then(|tools| frame_at(tools, &input, 0.0));
            let (w, h) = frame
                .as_ref()
                .map(|frame| (frame.width(), frame.height()))
                .unwrap_or((16, 9));
            let duration = ffmpeg
                .as_ref()
                .and_then(|tools| duration_of(tools, &input))
                .unwrap_or(0.0);
            (w as f64, h as f64, duration, frame)
        } else {
            let opened = image::open(&input).ok();
            let (w, h) = opened
                .as_ref()
                .map(|one| (one.width(), one.height()))
                .unwrap_or((16, 9));
            (w as f64, h as f64, 0.0, opened)
        };
        let aspect = (media_w / media_h.max(1.0)).max(1.0 / 12.0);
        let preview_units = (CARD_W as f32 / aspect as f32).clamp(PREVIEW_MIN, PREVIEW_MAX);

        STATE.with(|slot| {
            let mut state = slot.borrow_mut();
            *state = CropState {
                video,
                media_w,
                media_h,
                cut: Some(inset_rect(media_w, media_h)),
                preset: PRESET_FREE,
                dragging: Drag::Nothing,
                preview_units,
                duration,
                playhead: 0.0,
                frame_at: 0.0,
                ffmpeg: ffmpeg.clone(),
                input: input.clone(),
                ..Default::default()
            }
        });

        // Everything the card needs is set first, while the host is held
        // only for that and nothing is shown yet. Where the media stands
        // is told after the host is let go, because telling it draws a
        // frame, and drawing must never find the card still held.
        let card_window = HOST.with(|slot| {
            let mut borrowed = slot.borrow_mut();
            let Some(host) = borrowed.as_mut() else {
                return HWND::default();
            };
            host.card.set_file_name(said.into());
            host.card.set_detail(detail.into());
            host.card.set_video_mode(video);
            host.card.set_run_label("Crop".into());
            host.card.set_selected_chip(1);
            host.lay_out();
            // The media itself, at the size of its own box: the fold the
            // video opens on, or the picture opened whole and made small
            // enough to fit where it is going.
            if let Some(one) = opened {
                let scale = scale_factor(host.window);
                let layout = Layout::of(host.width as i32, host.height as i32, scale);
                let (box_w, box_h) = media_standing_box(&layout);
                let small = one.thumbnail(box_w.max(1), box_h.max(1));
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
        refresh_words();
        unsafe {
            // The window is made the size the media's shape wants, where
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
            // The keyboard starts with nothing under it: the chips are
            // one Tab away, and the rectangle takes the pointer first.
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
    /// the window when the stage is pressed, so what is drawn and what
    /// is taken can never disagree.
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
        self.card.set_stage_box(panel(layout.stage));
        self.card.set_aspect_label_box(panel(layout.aspect_label));
        self.card.set_chip_1_box(panel(layout.chip_1));
        self.card.set_chip_2_box(panel(layout.chip_2));
        self.card.set_chip_3_box(panel(layout.chip_3));
        self.card.set_chip_4_box(panel(layout.chip_4));
        self.card.set_chip_5_box(panel(layout.chip_5));
        self.card.set_chip_6_box(panel(layout.chip_6));
        self.card.set_chip_7_box(panel(layout.chip_7));
        self.card.set_chip_8_box(panel(layout.chip_8));
        self.card.set_chip_9_box(panel(layout.chip_9));
        self.card.set_chip_10_box(panel(layout.chip_10));
        self.card.set_crop_area_box(panel(layout.crop_area));
        self.card.set_media_size_box(panel(layout.media_size));
        self.card.set_slider_box(panel(layout.slider));
        self.card.set_step_back_box(panel(layout.step_back));
        self.card.set_step_forward_box(panel(layout.step_forward));
        self.card.set_transport_box(panel(layout.transport));
        self.card.set_centre_box(panel(layout.centre));
        self.card.set_whole_box(panel(layout.whole));
        self.card.set_helper_box(panel(layout.helper));
        self.card.set_run_box(panel(layout.run));
        // Where the media and the rectangle stand is the one part that
        // listens to what is held, not only to where the window is.
        let stage = (
            layout.stage.left as f64,
            layout.stage.top as f64,
            (layout.stage.right - layout.stage.left) as f64,
            (layout.stage.bottom - layout.stage.top) as f64,
        );
        stand_media(&mut self.card, stage);
        stand_cut(&mut self.card);
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
/// The card is a stack: a header, a hairline, the media at the height
/// its shape wants, the label of the presets and their two rows of
/// chips, a line of small words, the scrub row of a video or the
/// picture's own two buttons, a hairline, and the footer. The Rust side
/// works the stack out once and both sides read it.
#[derive(Clone, Copy, Debug, Default)]
struct Layout {
    file_name: RECT,
    detail: RECT,
    hairline_top: RECT,
    hairline_bottom: RECT,
    stage: RECT,
    aspect_label: RECT,
    chip_1: RECT,
    chip_2: RECT,
    chip_3: RECT,
    chip_4: RECT,
    chip_5: RECT,
    chip_6: RECT,
    chip_7: RECT,
    chip_8: RECT,
    chip_9: RECT,
    chip_10: RECT,
    crop_area: RECT,
    media_size: RECT,
    slider: RECT,
    step_back: RECT,
    step_forward: RECT,
    transport: RECT,
    centre: RECT,
    whole: RECT,
    helper: RECT,
    run: RECT,
}

impl Layout {
    /// Everything of the stack except the stage and the footer, in the
    /// drawing's own units: header, hairline, the air under it, the
    /// presets, the words, the scrub row of a video, and the closing
    /// hairline.
    const CONTENT_UNITS_VIDEO: f32 = 278.0;
    /// The same for a picture: where the scrub row would stand, the
    /// Centre and Whole image buttons stand instead, a little shorter.
    const CONTENT_UNITS_IMAGE: f32 = 260.0;
    /// The footer: the hairline's gap to the buttons, the buttons, and
    /// the air under them.
    const FOOTER_UNITS: f32 = 64.0;

    /// How tall a video window of this stage wants to be.
    fn height_for_video(preview_units: f32) -> f32 {
        Self::CONTENT_UNITS_VIDEO + preview_units + Self::FOOTER_UNITS
    }

    /// How tall a picture window of this stage wants to be.
    fn height_for_image(preview_units: f32) -> f32 {
        Self::CONTENT_UNITS_IMAGE + preview_units + Self::FOOTER_UNITS
    }

    fn of(whole: i32, side: i32, scale: f32) -> Layout {
        let at = |value: f32| (value * scale) as i32;
        let margin = at(24.0);
        let preview_units = client_preview_units();
        let video = is_video();

        // The stack, top to bottom, all worked out from the one number.
        let header_top = at(16.0);
        let hairline_top = at(62.0);
        let stage_top = hairline_top + at(20.0);
        let stage_bottom = stage_top + at(preview_units);

        // The presets: their label, then two rows of five chips, then
        // the words that say what is kept — and then the card's own kind
        // takes over: a video's scrub row, or a picture's two buttons.
        let aspect_label_top = stage_bottom + at(12.0);
        let chips_top = aspect_label_top + at(22.0);
        let chips_second_top = chips_top + at(26.0) + at(6.0);
        let words_top = chips_second_top + at(26.0) + at(10.0);
        let slider_top = words_top + at(18.0) + at(8.0);
        let transport_top = slider_top + at(14.0) + at(8.0);
        let buttons_top = words_top + at(18.0) + at(10.0);
        let hairline_bottom = if video {
            transport_top + at(28.0) + at(18.0)
        } else {
            buttons_top + at(30.0) + at(18.0)
        };

        // The chips: five of a kind to a row, each 64 wide and standing
        // 8 apart, the second row under the first with 6 between.
        let chip_w = at(64.0);
        let chip_gap = at(8.0);
        let chip_left = |index: i32| margin + index * (chip_w + chip_gap);

        // The words: what is kept at the left, the media's size at the
        // right, the air between them left alone.
        let crop_area_w = at(200.0);
        let media_size_w = at(150.0);

        // The picture's own two buttons stand side by side under the
        // words.
        let centre_w = at(84.0);

        let (slider_top, transport_top, buttons_top) = if video {
            (slider_top, transport_top, 0)
        } else {
            (0, 0, buttons_top)
        };
        let video = if video { 1 } else { 0 };

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
            stage: RECT {
                left: margin,
                top: stage_top,
                right: whole - margin,
                bottom: stage_bottom,
            },
            aspect_label: RECT {
                left: margin,
                top: aspect_label_top,
                right: whole - margin,
                bottom: aspect_label_top + at(16.0),
            },
            crop_area: RECT {
                left: margin,
                top: words_top,
                right: margin + crop_area_w,
                bottom: words_top + at(18.0),
            },
            media_size: RECT {
                left: whole - margin - media_size_w,
                top: words_top,
                right: whole - margin,
                bottom: words_top + at(18.0),
            },
            slider: RECT {
                left: if video == 1 { margin } else { 0 },
                top: if video == 1 { slider_top } else { 0 },
                right: if video == 1 { whole - margin } else { 0 },
                bottom: if video == 1 { slider_top + at(14.0) } else { 0 },
            },
            step_back: RECT {
                left: if video == 1 { margin } else { 0 },
                top: if video == 1 { transport_top } else { 0 },
                right: if video == 1 { margin + at(28.0) } else { 0 },
                bottom: if video == 1 { transport_top + at(28.0) } else { 0 },
            },
            step_forward: RECT {
                left: if video == 1 { margin + at(28.0) + at(4.0) } else { 0 },
                top: if video == 1 { transport_top } else { 0 },
                right: if video == 1 {
                    margin + at(28.0) + at(4.0) + at(28.0)
                } else {
                    0
                },
                bottom: if video == 1 { transport_top + at(28.0) } else { 0 },
            },
            transport: RECT {
                left: if video == 1 {
                    margin + at(28.0) + at(4.0) + at(28.0) + at(16.0)
                } else {
                    0
                },
                top: if video == 1 { transport_top } else { 0 },
                right: if video == 1 { whole - margin } else { 0 },
                bottom: if video == 1 { transport_top + at(28.0) } else { 0 },
            },
            centre: RECT {
                left: if video == 1 { 0 } else { margin },
                top: if video == 1 { 0 } else { buttons_top },
                right: if video == 1 { 0 } else { margin + centre_w },
                bottom: if video == 1 { 0 } else { buttons_top + at(30.0) },
            },
            whole: RECT {
                left: if video == 1 { 0 } else { margin + centre_w + at(12.0) },
                top: if video == 1 { 0 } else { buttons_top },
                right: if video == 1 {
                    0
                } else {
                    margin + centre_w + at(12.0) + at(96.0)
                },
                bottom: if video == 1 { 0 } else { buttons_top + at(30.0) },
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
                right: margin + at(84.0),
                bottom: side - at(16.0),
            },
            run: RECT {
                left: whole - margin - at(84.0),
                top: side - at(16.0) - at(34.0),
                right: whole - margin,
                bottom: side - at(16.0),
            },
            ..Default::default()
        };

        for index in 0..5 {
            let left = chip_left(index);
            let first = RECT {
                left,
                right: left + chip_w,
                top: chips_top,
                bottom: chips_top + at(26.0),
            };
            let second = RECT {
                top: chips_second_top,
                bottom: chips_second_top + at(26.0),
                ..first
            };
            match index {
                0 => layout.chip_1 = first,
                1 => layout.chip_2 = first,
                2 => layout.chip_3 = first,
                3 => layout.chip_4 = first,
                _ => layout.chip_5 = first,
            }
            match index {
                0 => layout.chip_6 = second,
                1 => layout.chip_7 = second,
                2 => layout.chip_8 = second,
                3 => layout.chip_9 = second,
                _ => layout.chip_10 = second,
            }
        }

        layout
    }
}

/// Puts the media where it stands on the stage given it, and says the
/// card so.
fn stand_media(card: &mut CropCard, stage: Box4) {
    let stood = STATE.with(|slot| {
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
    card.set_media_box(stood);
}

/// Puts the rectangle where it stands over the media, and says the card
/// so. Both boxes are said in the window's own pixels, absolute, and
/// the card does the standing itself.
fn stand_cut(card: &mut CropCard) {
    let panel = STATE.with(|slot| {
        let state = slot.borrow();
        state
            .cut
            .map(|cut| cut_in_window(cut, state.media_box, state.fit))
    });
    let panel = panel.unwrap_or(Panel {
        x: 0.0,
        y: 0.0,
        width: 0.0,
        height: 0.0,
    });
    card.set_crop_box(panel);
}

/// The rectangle of the media's own pixels, standing in the window's
/// pixels: it follows where the media is, at the stretch the media
/// stands at.
fn cut_in_window(cut: Cut, media_box: Box4, fit: f64) -> Panel {
    let (x, y, w, h) = (cut.0 * fit, cut.1 * fit, cut.2 * fit, cut.3 * fit);
    Panel {
        x: (media_box.0 + x) as f32,
        y: (media_box.1 + y) as f32,
        width: w as f32,
        height: h as f32,
    }
}

/// How much one pixel of the media takes on the stage: the less of the
/// two squeezes, so the whole media is seen at once.
fn fit_of(stage_w: f64, stage_h: f64, media_w: f64, media_h: f64) -> f64 {
    if media_w <= 0.0 || media_h <= 0.0 || stage_w <= 0.0 || stage_h <= 0.0 {
        return 1.0;
    }
    (stage_w / media_w).min(stage_h / media_h)
}

/// The box the a thumbnail of the media wants to fill, from the layout:
/// where the media itself stands when the window is this size.
fn media_standing_box(layout: &Layout) -> (u32, u32) {
    let stage_w = (layout.stage.right - layout.stage.left) as f64;
    let stage_h = (layout.stage.bottom - layout.stage.top) as f64;
    STATE.with(|slot| {
        let state = slot.borrow();
        let fit = fit_of(stage_w, stage_h, state.media_w, state.media_h);
        let w = (state.media_w * fit).round().max(1.0) as u32;
        let h = (state.media_h * fit).round().max(1.0) as u32;
        (w, h)
    })
}

/// The rectangle a crop starts as: a tenth of the media held from every
/// edge, said in whole pixels — the start the original starts at.
fn inset_rect(media_w: f64, media_h: f64) -> Cut {
    let x = (media_w * 0.1).round().max(0.0);
    let y = (media_h * 0.1).round().max(0.0);
    let w = (media_w - x * 2.0).max(1.0);
    let h = (media_h - y * 2.0).max(1.0);
    (x, y, w, h)
}

/// The ratio a preset asks for: none for the free rectangle, the
/// media's own for the original, and its own two numbers for every
/// other.
fn resolved_ratio(preset: usize, media_w: f64, media_h: f64) -> Option<f64> {
    match preset {
        PRESET_FREE => None,
        PRESET_ORIGINAL => Some(media_w / media_h.max(1.0)),
        2 => Some(1.0),
        3 => Some(4.0 / 3.0),
        4 => Some(3.0 / 2.0),
        5 => Some(16.0 / 9.0),
        6 => Some(9.0 / 16.0),
        7 => Some(3.0 / 4.0),
        8 => Some(2.0 / 3.0),
        9 => Some(5.0 / 4.0),
        _ => None,
    }
}

/// The four corners of a rectangle, in order: 0 the top-left, 1 the
/// top-right, 2 the bottom-right, 3 the bottom-left.
fn corners_of(cut: Cut) -> [Spot; 4] {
    [
        (cut.0, cut.1),
        (cut.0 + cut.2, cut.1),
        (cut.0 + cut.2, cut.1 + cut.3),
        (cut.0, cut.1 + cut.3),
    ]
}

/// A rectangle between two places, whichever way the pointer went.
fn standardized(from: Spot, to: Spot) -> Cut {
    (
        from.0.min(to.0),
        from.1.min(to.1),
        (from.0 - to.0).abs(),
        (from.1 - to.1).abs(),
    )
}

/// A rectangle that runs off the media is pulled back to it, and what
/// is left is never without size.
fn clamped_to_media(cut: Cut, media_w: f64, media_h: f64) -> Option<Cut> {
    let (x, y, w, h) = cut;
    let x = x.clamp(0.0, (media_w - 1.0).max(0.0));
    let y = y.clamp(0.0, (media_h - 1.0).max(0.0));
    let w = w.clamp(0.0, media_w - x).max(1.0);
    let h = h.clamp(0.0, media_h - y).max(1.0);
    Some((x, y, w, h))
}

/// The preset's ratio laid on a rectangle: the height follows the
/// width, and a rectangle that would run past the bottom is held back
/// with its width remade — the way the original keeps its promise.
/// Whatever is left is said in whole pixels.
fn applied_aspect(cut: Cut, media_w: f64, media_h: f64, ratio: Option<f64>) -> Option<Cut> {
    let Some(ratio) = ratio else {
        return clamped_to_media(cut, media_w, media_h);
    };
    let (x, y, w) = (cut.0, cut.1, cut.2.max(1.0));
    let mut h = w / ratio;
    let mut w = w;
    if y + h > media_h {
        h = (media_h - y).max(1.0);
        w = h * ratio;
    }
    let whole = (
        x.round().max(0.0),
        y.round().max(0.0),
        w.round().max(1.0),
        h.round().max(1.0),
    );
    clamped_to_media(whole, media_w, media_h)
}

/// What a press inside the stage took. A corner whose handle the press
/// stands near is a corner's drag; a deep-enough press inside the
/// rectangle is a drag of the whole; anything else is the start of a
/// new one. The reach and the depth are said in pixels of the media.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Pick {
    Corner(usize),
    Track,
    Create,
}

fn picked_at(point: Spot, cut: Option<Cut>, reach: f64, inset: f64) -> Option<Pick> {
    let Some(cut) = cut else {
        return Some(Pick::Create);
    };
    let corners = corners_of(cut);
    for (index, corner) in corners.iter().enumerate() {
        let side_x = (point.0 - corner.0).abs();
        let side_y = (point.1 - corner.1).abs();
        if side_x <= reach && side_y <= reach {
            return Some(Pick::Corner(index));
        }
    }
    let inside = point.0 > cut.0 + inset
        && point.0 < cut.0 + cut.2 - inset
        && point.1 > cut.1 + inset
        && point.1 < cut.1 + cut.3 - inset;
    if inside {
        return Some(Pick::Track);
    }
    Some(Pick::Create)
}

/// A corner dragged with the one across standing still: the dragged
/// edge follows the pointer, and the preset's ratio is laid on where it
/// asks. A top corner's drag holds the bottom edge; a left corner's
/// drag holds the right edge — wherever the pointer went past, the
/// span is kept.
fn resized(corner: usize, anchor: Spot, point: Spot, ratio: Option<f64>, media_w: f64, media_h: f64) -> Option<Cut> {
    let mut w = (point.0 - anchor.0).abs().max(1.0);
    let mut h = (point.1 - anchor.1).abs().max(1.0);
    if let Some(ratio) = ratio {
        h = w / ratio;
    }
    // A top corner drags the top edge: the bottom stays. A bottom
    // corner drags the bottom edge: the top stays. A left corner drags
    // the left edge: the right stays, and the other way about.
    let top_drag = corner < 2;
    let left_drag = corner == 0 || corner == 3;
    let x = if left_drag {
        anchor.0 - w
    } else {
        anchor.0
    };
    let y = if top_drag { anchor.1 - h } else { anchor.1 };
    clamped_to_media((x, y, w, h), media_w, media_h)
}

/// A rectangle drawn between where the press took and where the pointer
/// stands, with the preset's ratio laid on where it asks: the height
/// follows the width, and the side the drawing began on holds.
fn drawn(start: Spot, point: Spot, ratio: Option<f64>, media_w: f64, media_h: f64) -> Option<Cut> {
    let mut cut = standardized(start, point);
    if let Some(ratio) = ratio {
        cut.3 = cut.2 / ratio;
        // The drawing runs from the side it began on: upward the top
        // edge climbs, downward the bottom edge falls.
        if point.1 < start.1 {
            cut.1 = start.1 - cut.3;
        }
    }
    clamped_to_media((cut.0, cut.1, cut.2.max(1.0), cut.3.max(1.0)), media_w, media_h)
}

/// Converts what the pointer did on the window into what the crop
/// should hold. The press is read here, where the geometry is the same
/// one the card was drawn by.
fn touched(window: HWND, x: i32, y: i32, pressed: bool) {
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
    let stage = (
        layout.stage.left as f64,
        layout.stage.top as f64,
        (layout.stage.right - layout.stage.left) as f64,
        (layout.stage.bottom - layout.stage.top) as f64,
    );
    let slider = (
        layout.slider.left as f64,
        layout.slider.top as f64,
        (layout.slider.right - layout.slider.left) as f64,
        (layout.slider.bottom - layout.slider.top) as f64,
    );
    let under = |box4: Box4, grace: f64| {
        x as f64 >= box4.0 - grace
            && x as f64 <= box4.0 + box4.2 + grace
            && y as f64 >= box4.1 - grace
            && y as f64 <= box4.1 + box4.3 + grace
    };
    let on_slider = is_video() && under(slider, 8.0 * scale as f64);
    let on_stage = under(stage, 0.0);

    if pressed {
        let took = STATE.with(|slot| {
            let mut state = slot.borrow_mut();
            if on_slider {
                state.dragging = Drag::Playhead;
                return true;
            }
            if !on_stage {
                return false;
            }
            // Where the press stands, in the media's own pixels.
            let point = to_media(&state, x as f64, y as f64);
            // The reach of the handles and the depth of the middle, in
            // pixels of the media, from their own in the drawing.
            let reach = CORNER_REACH * scale as f64 / state.fit.max(0.01);
            let inset = MOVE_INSET * scale as f64 / state.fit.max(0.01);
            match picked_at(point, state.cut, reach, inset) {
                Some(Pick::Corner(corner)) => {
                    let cut = state.cut.expect("the corner was of a rectangle");
                    state.grab = point;
                    state.anchor = opposite_corner(corner, cut);
                    state.dragging = Drag::Corner(corner);
                    true
                }
                Some(Pick::Track) => {
                    let cut = state.cut.expect("the middle was of a rectangle");
                    state.grab = (point.0 - cut.0, point.1 - cut.1);
                    state.dragging = Drag::Track;
                    true
                }
                Some(Pick::Create) => {
                    state.grab = point;
                    state.dragging = Drag::Create;
                    // The drawing starts as nothing: a rectangle of no
                    // size, if the pointer never moves.
                    state.cut = None;
                    true
                }
                None => false,
            }
        });
        if took {
            unsafe {
                let _ = SetCapture(window);
            }
        }
    }

    let moved = STATE.with(|slot| {
        let mut state = slot.borrow_mut();
        let dragging = state.dragging;
        match dragging {
            Drag::Nothing => false,
            Drag::Playhead => {
                let share = if slider.2 > 0.0 {
                    ((x as f64 - slider.0) / slider.2).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                state.playhead = share * state.duration;
                true
            }
            _ => {
                if !on_stage {
                    return false;
                }
                let point = to_media(&state, x as f64, y as f64);
                let ratio = resolved_ratio(state.preset, state.media_w, state.media_h);
                match dragging {
                    Drag::Track => {
                        let cut = state.cut.expect("a track holds a rectangle");
                        let said = (point.0 - state.grab.0, point.1 - state.grab.1);
                        state.cut = placed(cut, said.0, said.1, state.media_w, state.media_h);
                        true
                    }
                    Drag::Corner(corner) => {
                        state.cut = resized(
                            corner,
                            state.anchor,
                            point,
                            ratio,
                            state.media_w,
                            state.media_h,
                        );
                        true
                    }
                    Drag::Create => {
                        state.cut = drawn(
                            state.grab,
                            point,
                            ratio,
                            state.media_w,
                            state.media_h,
                        );
                        true
                    }
                    Drag::Nothing | Drag::Playhead => false,
                }
            }
        }
    });
    if moved {
        if STATE.with(|slot| slot.borrow().dragging) == Drag::Playhead && !pressed {
            refresh_frame(false);
        }
        refresh_words();
        let _ = unsafe { InvalidateRect(Some(window), None, true) };
    }
}

/// Lets go of whatever the pointer was holding. A rectangle drawn over
/// a video smaller than the least a crop may take is given up: a crop
/// of nothing is not a crop.
fn released(window: HWND) {
    let dropped = STATE.with(|slot| {
        let mut state = slot.borrow_mut();
        let was_drawing = matches!(state.dragging, Drag::Create);
        state.dragging = Drag::Nothing;
        if state.video && was_drawing && is_too_small(state.cut, true) {
            state.cut = None;
            return true;
        }
        false
    });
    if dropped {
        refresh_words();
    }
    unsafe { let _ = ReleaseCapture(); };
    let _ = unsafe { InvalidateRect(Some(window), None, true) };
}

/// Whether a rectangle is too small to be a crop at all.
fn is_too_small(cut: Option<Cut>, video: bool) -> bool {
    match cut {
        None => true,
        Some(cut) => video && (cut.2 < MIN_CROP_VIDEO || cut.3 < MIN_CROP_VIDEO),
    }
}

/// The corner across the one taken: where the rectangle is held still.
fn opposite_corner(corner: usize, cut: Cut) -> Spot {
    corners_of(cut)[(corner + 2) % 4]
}

/// Where a point of the window stands, in the media's own pixels.
fn to_media(state: &CropState, x: f64, y: f64) -> Spot {
    let fit = state.fit.max(0.01);
    (
        (x - state.media_box.0) / fit,
        (y - state.media_box.1) / fit,
    )
}

/// The rectangle placed where a drag of its middle wants it: it moves
/// as far as the pointer went and never past the media.
fn placed(cut: Cut, x: f64, y: f64, media_w: f64, media_h: f64) -> Option<Cut> {
    let x = x.clamp(0.0, (media_w - cut.2).max(0.0));
    let y = y.clamp(0.0, (media_h - cut.3).max(0.0));
    Some((x, y, cut.2, cut.3))
}

/// Says what the words of the card are now: what the rectangle keeps,
/// how big the media is, where the video stands, and whether the go
/// button may be used.
fn refresh_words() {
    let words = STATE.with(|slot| {
        let state = slot.borrow();
        let keep = match state.cut {
            Some(cut) => format!(
                "{} × {} at {}, {}",
                cut.2.round() as u32,
                cut.3.round() as u32,
                cut.0.round() as u32,
                cut.1.round() as u32
            ),
            None => "Nothing is kept yet".to_string(),
        };
        let media = if state.video {
            format!(
                "Video: {} × {}",
                state.media_w.round() as u32,
                state.media_h.round() as u32
            )
        } else {
            format!(
                "Original: {} × {}",
                state.media_w.round() as u32,
                state.media_h.round() as u32
            )
        };
        let ready = match state.cut {
            None => false,
            Some(cut) => {
                if state.video {
                    // What the crop program asks of a video: the widths
                    // it will be evens to may not run out to nothing.
                    let w = (cut.2.round() as u32) / 2 * 2;
                    let h = (cut.3.round() as u32) / 2 * 2;
                    w >= 2 && h >= 2
                } else {
                    cut.2 >= 1.0 && cut.3 >= 1.0
                }
            }
        };
        Words {
            keep,
            media,
            time: format!(
                "{} / {}",
                timecode(state.playhead),
                timecode(state.duration.max(0.0))
            ),
            share: fraction(state.playhead, state.duration),
            ready,
        }
    });
    HOST.with(|slot| {
        let mut borrowed = slot.borrow_mut();
        let Some(host) = borrowed.as_mut() else {
            return;
        };
        host.card.set_crop_caption(words.keep.clone().into());
        host.card.set_media_caption(words.media.clone().into());
        if is_video() {
            host.card.set_time_text(words.time.clone().into());
            host.card.set_scrub_fraction(words.share);
        }
        host.card.set_run_enabled(words.ready);
        stand_cut(&mut host.card);
        host.software.request_redraw();
    });
    let window = window_handle();
    if !window.0.is_null() {
        let _ = unsafe { InvalidateRect(Some(window), None, true) };
    }
}

/// What the card's words say, worked out in one read of the state.
#[derive(Clone, Debug, Default)]
struct Words {
    keep: String,
    media: String,
    time: String,
    share: f32,
    ready: bool,
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
                    let layout = Layout::of(host.width as i32, host.height as i32, scale);
                    media_standing_box(&layout)
                });
                let small = picture.thumbnail(box_w.max(1), box_h.max(1));
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

/// Puts the keyboard on one stop of the card: a chip or the go button.
unsafe fn apply_focus(window: HWND, index: i32) {
    FOCUS_INDEX.with(|slot| slot.set(index));
    let chip = if (1..=10).contains(&index) { index } else { 0 };
    let button = if index == 11 { 2 } else { 0 };
    HOST.with(|slot| {
        let mut borrowed = slot.borrow_mut();
        if let Some(host) = borrowed.as_mut() {
            host.card.set_chip_focus(chip);
            host.card.set_button_focus(button);
            host.software.request_redraw();
        }
    });
    let _ = unsafe { InvalidateRect(Some(window), None, true) };
}

/// Moves the keyboard to the stop after — or before — the one it is on.
/// The stops go round: the ten chips, and then go.
unsafe fn cycle_focus(window: HWND, forward: bool) {
    let now = FOCUS_INDEX.with(|slot| slot.get());
    let next = if forward {
        now + 1
    } else {
        now - 1
    };
    let stop = if next > 11 {
        1
    } else if next < 0 {
        0
    } else {
        next
    };
    unsafe { apply_focus(window, stop) };
}

/// Takes the preset whose chip was chosen, and lays its ratio on the
/// rectangle the way the original keeps its promise.
unsafe fn choose_aspect(window: HWND, preset: usize) {
    STATE.with(|slot| {
        let mut state = slot.borrow_mut();
        state.preset = preset;
        let ratio = resolved_ratio(preset, state.media_w, state.media_h);
        let stand = state.cut.unwrap_or_else(|| inset_rect(state.media_w, state.media_h));
        state.cut = applied_aspect(stand, state.media_w, state.media_h, ratio);
    });
    HOST.with(|slot| {
        let mut borrowed = slot.borrow_mut();
        if let Some(host) = borrowed.as_mut() {
            host.card.set_selected_chip((preset + 1) as i32);
        }
    });
    refresh_words();
    let _ = unsafe { InvalidateRect(Some(window), None, true) };
}

/// Puts the rectangle back where it started: a tenth of the media held
/// from every edge. The preset it wears stays as it is.
unsafe fn reset_rect(window: HWND) {
    STATE.with(|slot| {
        let mut state = slot.borrow_mut();
        state.cut = Some(inset_rect(state.media_w, state.media_h));
    });
    refresh_words();
    let _ = unsafe { InvalidateRect(Some(window), None, true) };
}

/// Moves the rectangle to the middle of the media, keeping its size.
unsafe fn centre_rect(window: HWND) {
    STATE.with(|slot| {
        let mut state = slot.borrow_mut();
        let Some(cut) = state.cut else {
            return;
        };
        let x = ((state.media_w - cut.2) / 2.0).max(0.0).round();
        let y = ((state.media_h - cut.3) / 2.0).max(0.0).round();
        state.cut = placed(cut, x, y, state.media_w, state.media_h);
    });
    refresh_words();
    let _ = unsafe { InvalidateRect(Some(window), None, true) };
}

/// Opens the rectangle to the whole of the media.
unsafe fn whole_rect(window: HWND) {
    STATE.with(|slot| {
        let mut state = slot.borrow_mut();
        state.cut = Some(clamped_to_media(
            (0.0, 0.0, state.media_w, state.media_h),
            state.media_w,
            state.media_h,
        )
        .unwrap_or((1.0, 1.0, 1.0, 1.0)));
    });
    refresh_words();
    let _ = unsafe { InvalidateRect(Some(window), None, true) };
}

/// Steps the playhead back or forward a second — or five, when Shift is
/// held. A picture stands still: there is nothing to step.
unsafe fn step(window: HWND, direction: f64) {
    let shift = unsafe { GetKeyState(VK_SHIFT.0 as i32) < 0 };
    let by = direction * if shift { 5.0 } else { 1.0 };
    STATE.with(|slot| {
        let mut state = slot.borrow_mut();
        if !state.video || state.duration <= 0.0 {
            return;
        }
        state.playhead = (state.playhead + by).clamp(0.0, state.duration);
    });
    refresh_frame(false);
    refresh_words();
    let _ = unsafe { InvalidateRect(Some(window), None, true) };
}

/// Runs what the window was open for, with the rectangle it keeps. The
/// numbers are said in whole pixels: where the rectangle starts
/// rounded, and how far it runs either even for a video or a single
/// pixel at the least — the rounding the original says its crop with,
/// and that the crop asks of it again.
unsafe fn run(window: HWND) {
    let nothing = STATE.with(|slot| slot.borrow().cut.is_none());
    if nothing {
        // A rectangle too small is already retired: the go button says
        // so, and the key says nothing.
        return;
    }
    let settings = STATE.with(|slot| {
        let state = slot.borrow();
        let Some(cut) = state.cut else {
            return ToolSettings::default();
        };
        let x = (cut.0.round().max(0.0) as u32).min(state.media_w.max(1.0) as u32 - 1);
        let y = (cut.1.round().max(0.0) as u32).min(state.media_h.max(1.0) as u32 - 1);
        let w = (cut.2.round().max(1.0) as u32).min(state.media_w.max(1.0) as u32 - x).max(1);
        let h = (cut.3.round().max(1.0) as u32).min(state.media_h.max(1.0) as u32 - y).max(1);
        if state.video {
            // The picture is made again at the even numbers its code
            // asks for; the crop program evens them again if it must.
            ToolSettings {
                crop_x: Some(x),
                crop_y: Some(y),
                crop_w: Some(w / 2 * 2),
                crop_h: Some(h / 2 * 2),
                ..Default::default()
            }
        } else {
            ToolSettings {
                crop_x: Some(x),
                crop_y: Some(y),
                crop_w: Some(w),
                crop_h: Some(h),
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

/// Says what the window was asked, to whoever is listening.
fn send(event: ToolEvent) {
    EVENTS.with(|slot| {
        if let Some(sender) = slot.borrow().as_ref() {
            let _ = sender.send(event);
        }
    });
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
            // The stage and the scrub row are read here, where the
            // geometry is the same one the card was drawn by.
            match message {
                WM_LBUTTONDOWN => touched(window, x, y, true),
                WM_MOUSEMOVE => {
                    let holding = STATE.with(|slot| {
                        slot.borrow().dragging != Drag::Nothing
                    });
                    if holding {
                        touched(window, x, y, false);
                    }
                }
                _ => released(window),
            }
            // The card still sees the pointer, for the hover of its
            // buttons. The telling is not done while the host is held:
            // a button under the pointer answers the moment the card
            // hears of it, and its answer reads the host on its own.
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
            } else if key == VK_LEFT.0 || key == VK_RIGHT.0 {
                // Along the chips the keyboard walks one stop to the
                // side, round the row's end; on go it says nothing.
                let now = FOCUS_INDEX.with(|slot| slot.get());
                if (1..=10).contains(&now) {
                    let next = if key == VK_LEFT.0 {
                        if now == 1 { 10 } else { now - 1 }
                    } else {
                        if now == 10 { 1 } else { now + 1 }
                    };
                    unsafe { apply_focus(window, next) };
                } else if now == 11 && (key == VK_LEFT.0) {
                    unsafe { apply_focus(window, 10) };
                } else if now == 11 {
                    unsafe { apply_focus(window, 1) };
                }
                LRESULT(0)
            } else if key == VK_RETURN.0 || key == VK_SPACE.0 {
                let stop = FOCUS_INDEX.with(|slot| slot.get());
                if (1..=10).contains(&stop) {
                    unsafe { choose_aspect(window, (stop - 1) as usize) };
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

/// Whether the coming-in has said its last word. The capture tests read
/// it to know when the window has stopped changing.
#[cfg(test)]
fn get_fade_done() -> bool {
    FADE_ALPHA.with(|slot| slot.get() == 255)
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
    fn the_height_of_a_video_window_is_its_content_and_its_preview() {
        assert_eq!(Layout::height_for_video(236.0), 578.0);
        // A portrait video is held at the most a stage may be.
        assert_eq!(Layout::height_for_video(300.0), 642.0);
    }

    #[test]
    fn the_height_of_a_picture_window_is_its_own_content_and_its_preview() {
        // Where a video's scrub row would stand, the picture's two
        // buttons stand instead, 18 units shorter.
        assert_eq!(Layout::height_for_image(236.0), 560.0);
        assert_eq!(Layout::height_for_image(300.0), 624.0);
    }

    #[test]
    fn the_layout_stacks_the_body_top_to_bottom() {
        STATE.with(|slot| {
            let mut state = slot.borrow_mut();
            state.preview_units = 236.0;
            state.video = true;
        });
        let whole = 420;
        let side = 578;
        let layout = Layout::of(whole, side, 1.0);
        // The stage keeps the margins the reference gives its stages.
        assert_eq!(layout.stage.left, 24);
        assert_eq!(layout.stage.right, whole - 24);
        // The stack: header, hairline, stage, the presets' label, the
        // two rows of chips, the words, the scrub row, hairline, footer.
        assert!(layout.hairline_top.bottom <= layout.stage.top);
        assert!(layout.stage.bottom <= layout.aspect_label.top);
        assert!(layout.aspect_label.bottom + 4 <= layout.chip_1.top);
        // Five chips to a row, 8 apart, the second row under the first.
        assert_eq!(layout.chip_1.left, 24);
        assert_eq!(layout.chip_2.left, 96);
        assert_eq!(layout.chip_3.left, 168);
        assert_eq!(layout.chip_4.left, 240);
        assert_eq!(layout.chip_5.left, 312);
        assert_eq!(
            layout.chip_6.top - layout.chip_1.bottom,
            6,
            "the rows stand 6 apart"
        );
        assert!(layout.chip_10.bottom + 8 <= layout.crop_area.top);
        assert!(layout.crop_area.bottom + 6 <= layout.slider.top);
        assert!(layout.slider.bottom + 6 <= layout.step_back.top);
        assert_eq!(layout.step_back.top, layout.step_forward.top);
        assert_eq!(layout.step_back.top, layout.transport.top);
        assert!(layout.transport.bottom + 10 <= layout.hairline_bottom.top);
        // The go button stands inside the window, at the bottom right.
        assert!(layout.run.right <= whole);
        assert!(layout.run.bottom <= side);
        assert!(layout.helper.right + 10 <= layout.run.left);
    }

    #[test]
    fn a_picture_lays_its_buttons_where_a_videos_scrub_row_stands() {
        STATE.with(|slot| {
            let mut state = slot.borrow_mut();
            state.preview_units = 236.0;
            state.video = false;
        });
        let whole = 420;
        let side = 560;
        let layout = Layout::of(whole, side, 1.0);
        // No scrub row on a picture: the slider and the transport stand
        // nowhere at all.
        assert_eq!(layout.slider.left, 0);
        assert_eq!(layout.slider.right, 0);
        assert_eq!(layout.step_back.bottom, 0);
        // The two buttons instead, side by side on one line.
        assert_eq!(layout.centre.top, layout.whole.top);
        assert_eq!(layout.centre.left, 24);
        assert!(layout.centre.right + 8 <= layout.whole.left);
        assert!(layout.whole.right <= whole - 24);
        // And the closing hairline stands 18 nearer the stage: the card
        // is just that much shorter.
        assert_eq!(
            layout.hairline_bottom.top - layout.whole.bottom,
            18,
            "the hairline follows the buttons"
        );
        STATE.with(|slot| slot.borrow_mut().video = true);
    }

    #[test]
    fn a_taller_stage_makes_a_taller_window_and_the_same_widths() {
        STATE.with(|slot| {
            let mut state = slot.borrow_mut();
            state.preview_units = 300.0;
            state.video = true;
        });
        let portrait = Layout::of(420, 642, 1.0);
        STATE.with(|slot| {
            let mut state = slot.borrow_mut();
            state.preview_units = 236.0;
            state.video = true;
        });
        let landscape = Layout::of(420, 578, 1.0);
        // The stage takes all the height the shape asked for; nothing
        // else in the stack moves by a pixel.
        assert_eq!(portrait.stage.bottom - portrait.stage.top, 300);
        assert_eq!(landscape.stage.bottom - landscape.stage.top, 236);
        assert_eq!(portrait.run.left, landscape.run.left);
        assert_eq!(portrait.stage.left, landscape.stage.left);
    }

    #[test]
    fn the_media_stands_as_large_as_the_stage_allows_and_no_larger() {
        // A landscape media of a landscape stage: the height leads.
        assert_eq!(fit_of(372.0, 236.0, 160.0, 120.0), 236.0 / 120.0);
        // A wide video against a portrait stage: the width leads.
        assert_eq!(fit_of(372.0, 236.0, 1920.0, 1080.0), 372.0 / 1920.0);
        // A small media fits no larger than the stage: it is not blown
        // up past what one of the two squeezes says.
        assert_eq!(fit_of(372.0, 236.0, 64.0, 48.0), 236.0 / 48.0);
        // Nothing to squeeze: nothing happens.
        assert_eq!(fit_of(0.0, 236.0, 160.0, 120.0), 1.0);
        assert_eq!(fit_of(372.0, 236.0, 0.0, 0.0), 1.0);
    }

    #[test]
    fn a_crop_starts_a_tenth_in_from_every_edge() {
        // The 720 × 1280 the reference's own example is worked on.
        assert_eq!(
            inset_rect(720.0, 1280.0),
            (72.0, 128.0, 576.0, 1024.0)
        );
        assert_eq!(
            inset_rect(1600.0, 900.0),
            (160.0, 90.0, 1280.0, 720.0)
        );
    }

    #[test]
    fn the_presets_say_the_ratios_the_card_wears() {
        let (wide, tall) = (1600.0, 900.0);
        assert_eq!(resolved_ratio(0, wide, tall), None);
        // The original keeps the media's own shape.
        assert_eq!(resolved_ratio(1, wide, tall), Some(1600.0 / 900.0));
        assert_eq!(resolved_ratio(2, wide, tall), Some(1.0));
        assert_eq!(resolved_ratio(3, wide, tall), Some(4.0 / 3.0));
        assert_eq!(resolved_ratio(4, wide, tall), Some(3.0 / 2.0));
        assert_eq!(resolved_ratio(5, wide, tall), Some(16.0 / 9.0));
        assert_eq!(resolved_ratio(6, wide, tall), Some(9.0 / 16.0));
        assert_eq!(resolved_ratio(7, wide, tall), Some(3.0 / 4.0));
        assert_eq!(resolved_ratio(8, wide, tall), Some(2.0 / 3.0));
        assert_eq!(resolved_ratio(9, wide, tall), Some(5.0 / 4.0));
    }

    #[test]
    fn the_ratio_is_laid_on_and_held_back_before_the_bottom() {
        let (wide, tall) = (1600.0, 900.0);
        // A 16:9 face on a rectangle that has room for it: only the
        // height is remade.
        let said = applied_aspect(
            (100.0, 50.0, 400.0, 300.0),
            wide,
            tall,
            Some(16.0 / 9.0),
        )
        .expect("the rectangle was kept");
        assert_eq!(said, (100.0, 50.0, 400.0, 225.0));
        // A 9:16 face on the whole media runs past the bottom: it is
        // held back with its width remade, in whole pixels.
        let said = applied_aspect((0.0, 0.0, 1600.0, 900.0), wide, tall, Some(9.0 / 16.0))
            .expect("the rectangle was kept");
        assert_eq!(said, (0.0, 0.0, 506.0, 900.0));
        // The free rectangle keeps what was drawn, pulled inside the
        // media.
        let said = applied_aspect((-10.0, -5.0, 300.0, 200.0), wide, tall, None)
            .expect("the rectangle was kept");
        assert_eq!(said, (0.0, 0.0, 300.0, 200.0));
    }

    #[test]
    fn a_press_takes_a_handle_a_middle_or_the_space_around() {
        let cut = Some((0.0, 0.0, 100.0, 100.0));
        let reach = 12.0;
        let inset = 4.0;
        // Near a corner: the handle, whichever corner it is.
        assert_eq!(picked_at((2.0, 2.0), cut, reach, inset), Some(Pick::Corner(0)));
        assert_eq!(
            picked_at((90.0, 10.0), cut, reach, inset),
            Some(Pick::Corner(1))
        );
        assert_eq!(
            picked_at((88.0, 88.0), cut, reach, inset),
            Some(Pick::Corner(2))
        );
        assert_eq!(
            picked_at((3.0, 104.0), cut, reach, inset),
            Some(Pick::Corner(3))
        );
        // Deep inside: the whole rectangle comes along.
        assert_eq!(picked_at((50.0, 50.0), cut, reach, inset), Some(Pick::Track));
        // Just at its edge, or outside it: a new rectangle is drawn.
        assert_eq!(
            picked_at((3.0, 50.0), cut, reach, inset),
            Some(Pick::Create)
        );
        assert_eq!(
            picked_at((150.0, 50.0), cut, reach, inset),
            Some(Pick::Create)
        );
        // Nothing drawn yet: whatever is pressed, a new one is drawn.
        assert_eq!(picked_at((50.0, 50.0), None, reach, inset), Some(Pick::Create));
    }

    #[test]
    fn a_corner_is_drags_with_the_one_across_standing_still() {
        let (wide, tall) = (640.0, 480.0);
        // The top-left handle, the bottom-right standing still.
        let said = resized(0, (100.0, 100.0), (40.0, 40.0), None, wide, tall)
            .expect("the rectangle was kept");
        assert_eq!(said, (40.0, 40.0, 60.0, 60.0));
        // With a square asked for, the edge away from the anchor is the
        // one the ratio remakes: the span faces the anchor either way.
        let said = resized(0, (100.0, 100.0), (40.0, 70.0), Some(1.0), wide, tall)
            .expect("the rectangle was kept");
        assert_eq!(said, (40.0, 40.0, 60.0, 60.0));
        // A wide ratio from the same catch: the height follows.
        let said = resized(0, (100.0, 100.0), (200.0, 200.0), Some(16.0 / 9.0), wide, tall)
            .expect("the rectangle was kept");
        assert_eq!(said.0, 0.0);
        assert_eq!(said.2, 100.0);
        assert_eq!(said.3, 100.0 / (16.0 / 9.0));
        // The bottom-right handle keeps the top-left where it was.
        let said = resized(2, (0.0, 0.0), (300.0, 150.0), None, wide, tall)
            .expect("the rectangle was kept");
        assert_eq!(said, (0.0, 0.0, 300.0, 150.0));
    }

    #[test]
    fn a_new_rectangle_runs_from_where_the_press_took() {
        let (wide, tall) = (640.0, 480.0);
        // Free: the rectangle is whatever the pointer went for.
        let said = drawn((100.0, 100.0), (40.0, 160.0), None, wide, tall)
            .expect("the rectangle was kept");
        assert_eq!(said, (40.0, 100.0, 60.0, 60.0));
        // Upward with a ratio: the drawing keeps the side it began on.
        let said = drawn((100.0, 100.0), (180.0, 40.0), Some(1.0), wide, tall)
            .expect("the rectangle was kept");
        assert_eq!(said, (100.0, 20.0, 80.0, 80.0));
        // A tall face drawn upward: the width leads, the height follows.
        let said = drawn((100.0, 100.0), (120.0, 20.0), Some(9.0 / 16.0), wide, tall)
            .expect("the rectangle was kept");
        assert_eq!(said.2, 20.0);
        assert_eq!(said.3, 20.0 / (9.0 / 16.0));
        assert!((said.1 - (100.0 - said.3)).abs() < 0.001);
    }

    #[test]
    fn a_drag_of_the_middle_never_carries_the_rectangle_off_the_media() {
        let said = placed((0.0, 0.0, 32.0, 32.0), 500.0, 0.0, 160.0, 120.0)
            .expect("the rectangle was kept");
        assert_eq!(said, (128.0, 0.0, 32.0, 32.0));
        let said = placed((0.0, 0.0, 32.0, 32.0), -5.0, -9.0, 160.0, 120.0)
            .expect("the rectangle was kept");
        assert_eq!(said, (0.0, 0.0, 32.0, 32.0));
    }

    #[test]
    fn a_remark_of_size_says_what_a_drop_may_not_keep() {
        // Nothing drawn: too small by definition, so the next drop of a
        // drawing on a video puts the rectangle away.
        assert!(is_too_small(None, true));
        // On a video, under the least a crop may take: given up.
        assert!(is_too_small(Some((0.0, 0.0, 7.0, 100.0)), true));
        assert!(is_too_small(Some((0.0, 0.0, 100.0, 7.0)), true));
        assert!(!is_too_small(Some((0.0, 0.0, 8.0, 100.0)), true));
        // On a picture, a single pixel is honest.
        assert!(!is_too_small(Some((0.0, 0.0, 1.0, 1.0)), false));
    }

    #[test]
    fn the_corners_stand_west_north_first() {
        let corners = corners_of((10.0, 20.0, 300.0, 400.0));
        assert_eq!(corners[0], (10.0, 20.0));
        assert_eq!(corners[1], (310.0, 20.0));
        assert_eq!(corners[2], (310.0, 420.0));
        assert_eq!(corners[3], (10.0, 420.0));
    }

    #[test]
    fn the_opposite_of_a_corner_is_the_one_it_is_held_by() {
        let cut = (10.0, 20.0, 300.0, 400.0);
        assert_eq!(opposite_corner(0, cut), (310.0, 420.0));
        assert_eq!(opposite_corner(2, cut), (10.0, 20.0));
        assert_eq!(opposite_corner(1, cut), (10.0, 420.0));
        assert_eq!(opposite_corner(3, cut), (310.0, 20.0));
    }
}

#[cfg(test)]
mod show_tests {
    use super::*;
    use crate::media::runner::{ProcessRunner, SystemRunner};
    use windows::Win32::Graphics::Gdi::{
        CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDIBits,
        GetWindowDC, ReleaseDC, SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB,
        DIB_RGB_COLORS,
    };
    use windows::Win32::Storage::Xps::{PrintWindow, PRINT_WINDOW_FLAGS};
    use windows::Win32::UI::WindowsAndMessaging::{
        DispatchMessageW, GetWindowRect, MSG, PeekMessageW, PM_REMOVE, TranslateMessage,
    };

    #[test]
    fn showing_and_pumping_the_video_card_does_not_panic() {
        crate::ui::wheel::install_platform();
        let (sender, _receiver) = std::sync::mpsc::channel();
        let mut window = CropWindow::new(sender).expect("the window was made");
        window.show_for(
            "tools.crop",
            "Crop",
            "Keep a rectangle of the video",
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

    #[test]
    fn showing_and_pumping_the_picture_card_does_not_panic() {
        crate::ui::wheel::install_platform();
        let (sender, _receiver) = std::sync::mpsc::channel();
        let mut window = CropWindow::new(sender).expect("the window was made");
        window.show_for(
            "tools.crop",
            "Crop",
            "Keep a rectangle of the picture",
            &[PathBuf::from("not-there.png")],
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

    /// The presets answer the pointer: a chip pressed through the
    /// card's own events lays its ratio on. This walks the same road the
    /// mouse does — the card told without the host held — so a button's
    /// answer may read the host on its own, the way the wheel's answers
    /// do.
    #[test]
    fn a_chip_answers_the_pointer_without_the_host_being_taken() {
        crate::ui::wheel::install_platform();
        let (sender, _receiver) = std::sync::mpsc::channel();
        let mut window = CropWindow::new(sender).expect("the window was made");
        window.show_for(
            "tools.crop",
            "Crop",
            "Keep a rectangle of the picture",
            &[PathBuf::from("not-there.png")],
        );
        for _ in 0..10 {
            unsafe {
                let mut message = MSG::default();
                while PeekMessageW(&mut message, None, 0, 0, PM_REMOVE).as_bool() {
                    let _ = TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
            }
        }
        // Where the Square chip stands, said in the card's own units.
        let told = HOST.with(|slot| {
            let borrowed = slot.borrow();
            let Some(host) = borrowed.as_ref() else {
                return None;
            };
            let scale = scale_factor(host.window);
            let layout = Layout::of(host.width as i32, host.height as i32, scale);
            let middle = (layout.chip_3.left + layout.chip_3.right) as f32 / 2.0 / scale;
            let middle_y = (layout.chip_3.top + layout.chip_3.bottom) as f32 / 2.0 / scale;
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
            unsafe {
                let mut message = MSG::default();
                while PeekMessageW(&mut message, None, 0, 0, PM_REMOVE).as_bool() {
                    let _ = TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
            }
            draw_window(window.window);
        }
        // The square was taken, the card wears it, and the rectangle is
        // square again.
        let (preset, said) = STATE.with(|slot| {
            let state = slot.borrow();
            (state.preset, state.cut)
        });
        assert_eq!(preset, 2, "the square chip was not taken");
        let cut = said.expect("the crop keeps a rectangle");
        assert_eq!(cut.2, cut.3, "the square keeps its face");
        let chip = HOST.with(|slot| {
            let borrowed = slot.borrow();
            borrowed
                .as_ref()
                .map(|host| host.card.get_selected_chip())
                .unwrap_or(0)
        });
        assert_eq!(chip, 3, "the chip wears the choice");
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

    /// Makes a small test picture, a blue-to-green ramp, so the capture
    /// has something of its own colour that the conversion is not asked
    /// for.
    fn make_test_image(folder: &std::path::Path) -> PathBuf {
        let mut picture = image::RgbaImage::new(240, 180);
        for (x, y, pixel) in picture.enumerate_pixels_mut() {
            *pixel = image::Rgba([
                (x * 255 / 240) as u8,
                (y * 255 / 180) as u8,
                128,
                255,
            ]);
        }
        let path = folder.join("capture.png");
        picture.save(&path).expect("the picture was written");
        path
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
            const PW_RENDERFULLCONTENT: PRINT_WINDOW_FLAGS = PRINT_WINDOW_FLAGS(2);
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

    /// Opens the crop window on a real video, lets it come in and draw,
    /// and captures what a person would see.
    #[test]
    #[ignore = "needs the conversion program; writes a screenshot"]
    fn capture_the_crop_video_window() {
        crate::ui::wheel::install_platform();
        let folder = std::env::temp_dir().join("open-tangerine-capture");
        std::fs::create_dir_all(&folder).expect("the capture folder was made");
        let video = make_test_video(&folder);
        let (sender, _receiver) = std::sync::mpsc::channel();
        let mut window = CropWindow::new(sender).expect("the window was made");
        window.show_for("tools.crop", "Crop", "Keep a rectangle of the video", &[video]);
        // Long enough for the fade to finish and the first fold to land:
        // the shot wants the rectangle standing on a real frame. The
        // shot waits only for the coming-in to be over, and gives the
        // whole thing up after a while so a window without the
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
            if get_fade_done() && round > 200 {
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
                    shots.join("crop-buffer.png"),
                    &rgba,
                    host.width,
                    host.height,
                    image::ExtendedColorType::Rgba8,
                );
            }
        });
        capture(window.window, &shots.join("crop-video-window.png"));
        window.hide();
        let _ = std::fs::remove_dir_all(&folder);
    }

    /// Opens the crop window on a real picture, lets it come in and
    /// draw, and captures what a person would see: the Centre and Whole
    /// image buttons wear the scrub row's place.
    #[test]
    #[ignore = "writes a screenshot"]
    fn capture_the_crop_image_window() {
        crate::ui::wheel::install_platform();
        let folder = std::env::temp_dir().join("open-tangerine-capture");
        std::fs::create_dir_all(&folder).expect("the capture folder was made");
        let picture = make_test_image(&folder);
        let (sender, _receiver) = std::sync::mpsc::channel();
        let mut window = CropWindow::new(sender).expect("the window was made");
        window.show_for(
            "tools.crop",
            "Crop",
            "Keep a rectangle of the picture",
            &[picture],
        );
        // Long enough for the fade to finish and the picture to be read.
        for round in 0..2400 {
            unsafe {
                let mut message = MSG::default();
                while PeekMessageW(&mut message, None, 0, 0, PM_REMOVE).as_bool() {
                    let _ = TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
            if get_fade_done() && round > 200 {
                break;
            }
        }
        let shots = std::path::Path::new("target").join("shots");
        std::fs::create_dir_all(&shots).expect("the shots folder was made");
        capture(window.window, &shots.join("crop-image-window.png"));
        window.hide();
        let _ = std::fs::remove_dir_all(&folder);
    }
}

