//! The window a picture is covered in.
//!
//! The covering does not happen the moment it is picked: it opens a
//! window with the picture in it — fitted to a dark stage, with the
//! rectangles of cover standing over it, a square hand at each corner
//! of the chosen one and a pill on each of its edges. Around the stage
//! the look is said in rows: which of the two coverings the boxes take,
//! what colour a solid box wears, and the detectors the reference
//! carries that this Windows port does not. The footer's Reset puts the
//! covers away, and the making of the covered copy is refused until
//! there is a cover to make.
//!
//! The card is drawn by the same renderer that draws the wheel, and the
//! skeleton it wears is the shell's. The card only draws: the pressing,
//! the corners, the edges and the dragging are read by the window
//! itself, which holds the same geometry the card was given — the way
//! the crop does. The rectangles are kept whole in the media's own
//! pixels, so what the person sees and what the file is covered by can
//! never disagree; on the save they are said in thousandths of the
//! media's own size, so one drawn rectangle can land on every picture
//! of a drag at the same relative place.
//!
//! The window is sized once and stays that way: its width is fixed, its
//! height follows the fixed class of the card.
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
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreateSolidBrush, EndPaint, InvalidateRect,
};
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
    IDC_SIZEWE, LWA_ALPHA, WNDCLASSW, WM_CLOSE, WM_DESTROY, WM_DPICHANGED, WM_ERASEBKGND,
    WM_KEYDOWN, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_PAINT, WM_SIZE, WM_TIMER,
    WS_EX_LAYERED,
};

use crate::platform::windows::toolwindow::ToolEvent;
use crate::platform::windows::windowhost::{fixed_style, scale_factor, screen_scale, to_wide};
use crate::radial::execution::{RedactArea, RedactStyle, ToolSettings};
use crate::ui::wheel::{window_for_next_component, Bgra8, Panel, RectData, RedactImageCard};

/// The timer the window comes in softly on.
const FADE_TIMER_ID: usize = 52;

/// The width of the card, in the drawing's own units. The other cards
/// of the tools carry the same width.
const CARD_W: i32 = 420;

/// How tall the stage is, in the drawing's own units. A cover-up is
/// always of a picture, and a picture leans landscape on the whole, so
/// the stage asks for no more than the crop's own default.
const PREVIEW_UNITS: f32 = 236.0;

/// How far from a corner a press still takes the handle, in pixels of
/// the media. The crop's corners ask the same.
const CORNER_REACH: f64 = 12.0;

/// How far from the middle of an edge its pill still takes the press,
/// in pixels of the media.
const EDGE_REACH: f64 = 6.0;

/// The least a cover may be, in pixels of the media along either side,
/// before a drawn one is dropped. A smaller drag was a miss, not a
/// message.
const MIN_COVER: f64 = 8.0;

/// How much a cover is pulled in from the media's edges when the
/// window arrives: a tenth of each, the crop's own first gesture.
const INSET: f64 = 0.1;

/// How deep inside a cover a press must stand to carry it along
/// instead of drawing past it: the same air the crop leaves at its own
/// borders.
const MOVE_INSET: f64 = 4.0;

/// How far off the stage or the media a press may land and still be
/// read as touching it, in window pixels.
const GRACE: f64 = 8.0;

/// The colours a solid box takes on, in order: the well cycles them.
/// Hex, as the settings carry it; the first is the well's own black.
const PALETTE: [&str; 6] = ["000000", "FFFFFF", "808080", "C0392B", "2ECC71", "3B82F6"];

/// A rectangle said in four numbers: where it stands, and how much it
/// covers.
pub type Box4 = (f64, f64, f64, f64);

/// Where a press took hold. The corners carry their numbers the way
/// the crop counts them — TL, TR, BR, BL — and the edges were given the
/// numbers after.
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
    /// Whether the spot is a pill on an edge rather than a handle on a
    /// corner: an edge drag holds one side and lets the other run.
    fn is_edge(self) -> bool {
        matches!(
            self,
            Spot::Top | Spot::Right | Spot::Bottom | Spot::Left
        )
    }
}

/// What a press lands on: a handle of the one standing chosen — a
/// corner's hand or a pill on one of its edges — the face of a cover
/// (chosen or not — a face always takes the cover and carries it
/// along), or the space where a new one is drawn.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Pick {
    Corner(Spot),
    Track(usize),
    Create,
}

/// The ways a drag is standing: nothing is taken, a cover is coming
/// along from where it was, a handle holds a cover, or a new cover is
/// being drawn.
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

/// What the window remembers between the frames: the media and where
/// it stands, the covers in the media's own pixels, which one stands
/// chosen, the look they take, and what the pointer is doing.
#[derive(Debug)]
struct RedactState {
    /// The media's own size, in pixels.
    media_w: f64,
    media_h: f64,
    /// Where the media sits on the stage, and how many window pixels
    /// one pixel of the media takes. A cover is said in media pixels
    /// and drawn in window pixels, and these two carry it over.
    media_box: Box4,
    fit: f64,
    /// The covers, each in pixels of the media.
    rects: Vec<Box4>,
    /// Which cover the pointer's last choice left chosen.
    selected: Option<usize>,
    /// The look the covers take: zero a solid box, one a smear.
    style: usize,
    /// Which colour of the wheel the solid boxes stand at.
    color: usize,
    dragging: Drag,
    /// The handle a resizing drag took. A drag begins with nothing.
    grab: Spot,
    /// The box a drag began from: a move counts the air from it, a
    /// resize holds its far side to it.
    anchored: Box4,
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
            grab: Spot::TopLeft,
            anchored: (0.0, 0.0, 0.0, 0.0),
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
    /// colour well, 4 Reset, 5 save.
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
    card: RedactImageCard,
    software: Rc<MinimalSoftwareWindow>,
    buffer: Vec<Bgra8>,
    width: u32,
    height: u32,
}

/// The window a picture is covered in.
pub struct RedactImageWindow {
    window: HWND,
}

// The window belongs to the thread that made it and is us
unsafe impl Send for RedactImageWindow {}

impl RedactImageWindow {
    /// Makes the window, hidden until a cover is asked for.
    pub fn new(events: Sender<ToolEvent>) -> Result<RedactImageWindow, String> {
        EVENTS.with(|slot| *slot.borrow_mut() = Some(events));

        // A window is put aside for the card before the card is made, so
        // the two find each other.
        let software = window_for_next_component();
        let card = RedactImageCard::new().map_err(|error| error.to_string())?;

        let instance = unsafe { GetModuleHandleW(None).map_err(|error| error.to_string())? };
        let class_name = w!("OpenTangerineRedactImage");
        // The card's own white is what fills it while it comes in, so a
        // dark theme's idea of "window colour" never shows through.
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
            card.on_reset_chosen(move || {
                let window = window_handle();
                if !window.0.is_null() {
                    unsafe { reset_cover(window) };
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

        Ok(RedactImageWindow { window })
    }

    /// Asks the window to hide itself.
    pub fn hide(&mut self) {
        unsafe {
            let _ = ShowWindow(self.window, SW_HIDE);
        }
    }

    /// The first picture of the drag is the one asked about: what it is
    /// made of, how big it is. All of it is asked before the window is
    /// shown, so what appears is already the picture's own card at its
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

        // A cover-up works on a picture and nothing else: when what was
        // dragged is not one, the window does not open at all, and no
        // one is left staring at an empty well.
        if crate::media::probe::kind_of(&extension) != Some(crate::media::probe::Kind::Image) {
            OPEN_FOR.with(|slot| *slot.borrow_mut() = None);
            return;
        }

        // A picture is opened here, with the house's own two hands, and
        // its own size is what it says. A drawing the picture crate does
        // not carry is rasterised first.
        let opened = if extension == "svg" {
            crate::media::svg::rasterise(&input)
                .ok()
                .and_then(|bytes| image::load_from_memory(&bytes).ok())
        } else {
            image::open(&input).ok()
        };
        let (media_w, media_h) = opened
            .as_ref()
            .map(|one| (one.width(), one.height()))
            .unwrap_or((16, 9));
        let (media_w, media_h) = (media_w as f64, media_h as f64);

        STATE.with(|slot| {
            let mut state = slot.borrow_mut();
            *state = RedactState {
                media_w,
                media_h,
                // One cover stands drawn as the window arrives, a tenth
                // in from every edge, and it stands chosen: the first
                // thing a person sees is what they will drag.
                rects: vec![inset_cover(media_w, media_h)],
                selected: Some(0),
                style: 0,
                color: 0,
                dragging: Drag::Nothing,
                grab: Spot::TopLeft,
                anchored: (0.0, 0.0, 0.0, 0.0),
                fit: 1.0,
                media_box: (0.0, 0.0, 0.0, 0.0),
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
            host.lay_out();
            // The picture itself, at the size of its own box: opened
            // whole and made small enough to fit where it is going.
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
            // The window comes in softly, from nothing to whole in a few
            // beats of the timer.
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

/// How wide a window of this card must be made so that what a person
/// uses of it is the size the drawing asked for, at this stretch.
fn outer_size(scale: f32) -> (i32, i32) {
    let client_w = (CARD_W as f32 * scale) as i32;
    let client_h = (Layout::height() * scale) as i32;
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

    /// Says every box the card asks for, and puts the media and the
    /// covers where they stand. The same numbers are read by the window
    /// when the stage is pressed, so what is drawn and what is taken
    /// can never disagree.
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
        self.card.set_hairline_bottom_box(panel(layout.hairline_bottom));
        self.card.set_stage_box(panel(layout.stage));
        self.card.set_style_label_box(panel(layout.style_label));
        self.card.set_style_box(panel(layout.style_box));
        self.card.set_solid_box(panel(layout.solid));
        self.card.set_blur_box(panel(layout.blur));
        self.card.set_color_label_box(panel(layout.color_label));
        self.card.set_well_box(panel(layout.well));
        self.card.set_detect_label_box(panel(layout.detect_label));
        self.card.set_faces_box(panel(layout.faces));
        self.card.set_text_box(panel(layout.text));
        self.card.set_note_box(panel(layout.note));
        self.card.set_reset_box(panel(layout.reset));
        self.card.set_run_box(panel(layout.run));
        stand_media(&mut self.card, box4_of(layout.stage));
        stand_covers(self);
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
/// The card is a stack: a header, a hairline, the stage, the three
/// rows the look is said in, a note about the detectors, a closing
/// hairline, and the footer. The Rust side works the stack out once
/// and both sides read it.
#[derive(Clone, Copy, Debug, Default)]
struct Layout {
    file_name: RECT,
    detail: RECT,
    hairline_top: RECT,
    hairline_bottom: RECT,
    stage: RECT,
    style_label: RECT,
    style_box: RECT,
    solid: RECT,
    blur: RECT,
    color_label: RECT,
    well: RECT,
    detect_label: RECT,
    faces: RECT,
    text: RECT,
    note: RECT,
    reset: RECT,
    run: RECT,
}

impl Layout {
    /// Everything of the stack except the header's own band and the
    /// footer, in the drawing's own units: the stage and the three
    /// rows and the note and the closing hairline.
    const CONTENT_UNITS: f32 = 434.0;
    /// The header's band: the file's name, its detail, and the
    /// hairline under them.
    const HEADER_UNITS: f32 = 62.0;
    /// The footer: the hairline's gap to the buttons, the buttons, and
    /// the air under them.
    const FOOTER_UNITS: f32 = 64.0;

    /// How tall a window of this card is.
    fn height() -> f32 {
        Self::HEADER_UNITS + Self::CONTENT_UNITS + Self::FOOTER_UNITS
    }

    fn of(whole: i32, side: i32, scale: f32) -> Layout {
        let at = |value: f32| (value * scale) as i32;
        let margin = at(24.0);

        // The stack, top to bottom, all worked out from the one number.
        let header_top = at(16.0);
        let hairline_top = at(62.0);
        let stage_top = hairline_top + at(20.0);
        let stage_bottom = stage_top + at(PREVIEW_UNITS);

        // The three rows the look is said in: the style chips, the
        // colour well, the detectors — a label at the left, the control
        // at the right, one line each.
        let style_top = stage_bottom + at(14.0);
        let style_h = at(34.0);
        let color_top = style_top + style_h + at(10.0);
        let color_h = at(38.0);
        let detect_top = color_top + color_h + at(10.0);
        let detect_h = at(34.0);
        let note_top = detect_top + detect_h + at(8.0);
        let note_h = at(16.0);
        let hairline_bottom = note_top + note_h + at(14.0);

        // The controls stand at the right of their rows; the labels
        // keep a room of their own at the left.
        let row_w = at(140.0);
        let well_w = at(62.0);
        let chip_w = at(84.0);

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
            style_box: RECT {
                left: whole - margin - row_w,
                top: style_top,
                right: whole - margin,
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
            detect_label: RECT {
                left: margin,
                top: detect_top,
                right: margin + at(64.0),
                bottom: detect_top + detect_h,
            },
            faces: RECT {
                left: whole - margin - chip_w - at(8.0) - chip_w,
                top: detect_top + at(4.0),
                right: whole - margin - chip_w - at(8.0),
                bottom: detect_top + at(4.0) + at(26.0),
            },
            text: RECT {
                left: whole - margin - chip_w,
                top: detect_top + at(4.0),
                right: whole - margin,
                bottom: detect_top + at(4.0) + at(26.0),
            },
            note: RECT {
                left: margin,
                top: note_top,
                right: whole - margin,
                bottom: note_top + note_h,
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
            reset: RECT {
                left: margin,
                top: side - at(50.0),
                right: margin + at(84.0),
                bottom: side - at(16.0),
            },
            run: RECT {
                left: whole - margin - at(168.0),
                top: side - at(50.0),
                right: whole - margin,
                bottom: side - at(16.0),
            },
        }
    }
}

/// How many window pixels one pixel of the media takes: the smaller of
/// the two squeezes. A media with nothing to say takes nothing away
/// from itself.
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

/// The point of the cover a drag holds still: the corner across from a
/// corner's handle, the middle of the side across from a pill.
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
fn picked_at(point: (f64, f64), rects: &[Box4], selected: Option<usize>, fit: f64) -> Pick {
    if let Some(index) = selected {
        if let Some(cut) = rects.get(index) {
            // The corners' handles, nearly the same reach the crop's
            // hands ask for.
            for (position, corner) in corners_of(*cut).iter().enumerate() {
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
        let cut = rects[index];
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

/// A corner dragged with the one across standing still.
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
        let cut = (
            cut.0,
            cut.1,
            cut.2.max(1.0),
            cut.3.max(1.0),
        );
        return clamped_to_media(cut, media_w, media_h);
    }
    // A pill on an edge pulls one side only: the cover keeps its own
    // place on the other two directions, so a smear never creeps
    // sideways that was not asked for.
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
    (
        original.0 + along_x,
        original.1 + along_y,
        original.2,
        original.3,
    )
}

/// Whether a drawn cover is too small to mean anything. A smaller drag
/// is dropped when the pointer lets go.
fn is_too_small(cut: Box4) -> bool {
    cut.2 < MIN_COVER || cut.3 < MIN_COVER
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
/// from the pixels of the media they were drawn on.
fn areas_of(rects: &[Box4], media_w: f64, media_h: f64) -> Vec<RedactArea> {
    rects
        .iter()
        .map(|cut| RedactArea {
            x_mille: mille(cut.0, media_w),
            y_mille: mille(cut.1, media_h),
            w_mille: mille(cut.2, media_w),
            h_mille: mille(cut.3, media_h),
            start_ms: None,
            end_ms: None,
        })
        .collect()
}

/// The settings the card's own state says: the look, the colour, the
/// covers. A photo has no time to be in, so no bounds are set.
fn settings_from(
    style: usize,
    color: usize,
    rects: &[Box4],
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
fn stand_media(card: &mut RedactImageCard, stage: Box4) {
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
/// Both boxes are said in the window's own pixels, absolute, and the
/// card does the standing itself.
fn stand_covers(host: &mut Host) {
    let (covers, chosen, solid, color) = STATE.with(|slot| {
        let state = slot.borrow();
        let covers: Vec<RectData> = state
            .rects
            .iter()
            .map(|cut| {
                let (x, y, w, h) = cut_in_window(*cut, state.media_box, state.fit);
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

/// Puts the state's covers on the card and asks for a paint.
fn stand_all(window: HWND) {
    HOST.with(|slot| {
        let mut borrowed = slot.borrow_mut();
        if let Some(host) = borrowed.as_mut() {
            if host.window == window {
                stand_covers(host);
            }
        }
    });
    unsafe {
        let _ = InvalidateRect(Some(window), None, true);
    }
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
/// on. The stops go round: the two style chips, the colour well,
/// Reset, and save.
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
    stand_all(window);
}

/// Turns the colour well to the next colour of the wheel.
unsafe fn choose_color(window: HWND) {
    STATE.with(|slot| {
        let mut state = slot.borrow_mut();
        state.color = (state.color + 1) % PALETTE.len();
    });
    stand_all(window);
}

/// Puts the covers away: the card is clean of them, the way the
/// reference's Reset clears its list.
unsafe fn reset_cover(window: HWND) {
    STATE.with(|slot| {
        let mut state = slot.borrow_mut();
        state.rects.clear();
        state.selected = None;
        state.dragging = Drag::Nothing;
    });
    stand_all(window);
}

/// Takes the chosen cover away, the way the reference's list lets a
/// person lift their mistakes off. What stands under it answers: the
/// newest of what is left becomes the chosen one.
unsafe fn remove_selected(window: HWND) {
    STATE.with(|slot| {
        let mut state = slot.borrow_mut();
        if let Some(index) = state.selected.take() {
            if index < state.rects.len() {
                state.rects.remove(index);
            }
        }
    });
    stand_all(window);
}

/// Says what the window was asked, and puts it away. The covers are
/// said in thousandths of the media's own size, the look in the
/// settings' own words; every picture of the drag takes the same
/// covering at the same relative places.
unsafe fn run(window: HWND) {
    STATE.with(|slot| {
        let state = slot.borrow();
        if state.dragging != Drag::Nothing || state.rects.is_empty() {
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

/// The shape the cursor wears where the press would land: a handle
/// shapes, a cover slides, empty space draws.
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

/// Puts the cursor in the shape the pointer's own news asks for; where
/// nothing under the media answers, the arrow answers for itself.
fn aim_cursor(pick: Option<Pick>) {
    let Some(pick) = pick else {
        return;
    };
    unsafe {
        let cursor = LoadCursorW(None, cursor_for(pick)).unwrap_or_default();
        let _ = SetCursor(Some(cursor));
    }
}

/// Converts what the pointer did on the window into what the covers
/// should hold. The press is read here, where the geometry is the same
/// one the card was drawn by; a move carries the drag along.
fn touched(window: HWND, x: i32, y: i32, pressed: bool) {
    let mine = HOST.with(|slot| {
        let borrowed = slot.borrow();
        borrowed.as_ref().map(|host| host.window == window)
    });
    if mine != Some(true) {
        return;
    }

    // Where the pointer stands, said in the media's own pixels.
    let (point, on_media) = STATE.with(|slot| {
        let state = slot.borrow();
        (
            to_media((x as f64, y as f64), state.media_box, state.fit),
            under(state.media_box, GRACE, (x as f64, y as f64)),
        )
    });

    if pressed {
        // A press begins things only on the media: a cover is taken
        // from the picture alone.
        if on_media {
            STATE.with(|slot| {
                let mut state = slot.borrow_mut();
                if state.dragging != Drag::Nothing {
                    return;
                }
                match picked_at(point, &state.rects, state.selected, state.fit) {
                    Pick::Create => {
                        // A cover begins empty and grows while it is
                        // drawn; nothing stands chosen while it is.
                        let index = state.rects.len();
                        state.rects.push((point.0, point.1, 0.0, 0.0));
                        state.selected = None;
                        state.dragging = Drag::Drawing {
                            press: point,
                            index,
                        };
                        unsafe {
                            let _ = SetCapture(window);
                        }
                    }
                    Pick::Track(index) => {
                        state.selected = Some(index);
                        state.anchored = state.rects[index];
                        state.dragging = Drag::Moving {
                            index,
                            press: point,
                        };
                        unsafe {
                            let _ = SetCapture(window);
                        }
                    }
                    Pick::Corner(spot) => {
                        let index = state.selected.unwrap_or(0);
                        if index < state.rects.len() {
                            state.grab = spot;
                            state.anchored = state.rects[index];
                            state.dragging = Drag::Resizing { index };
                            unsafe {
                                let _ = SetCapture(window);
                            }
                        }
                    }
                }
            });
        }
    } else {
        // A move carries a drag along, or only aims the cursor.
        STATE.with(|slot| {
            let mut state = slot.borrow_mut();
            let dragging = state.dragging;
            match dragging {
                Drag::Nothing => {}
                Drag::Moving { index, press } => {
                    let cut = placed(state.anchored, press, point, state.media_w, state.media_h);
                    if let Some(one) = state.rects.get_mut(index) {
                        *one = cut;
                    }
                }
                Drag::Resizing { index } => {
                    let cut = resized(
                        state.grab,
                        state.anchored,
                        point,
                        state.media_w,
                        state.media_h,
                    );
                    if let Some(one) = state.rects.get_mut(index) {
                        *one = cut;
                    }
                }
                Drag::Drawing { press, index } => {
                    let cut = drawn(press, point, state.media_w, state.media_h);
                    if let Some(one) = state.rects.get_mut(index) {
                        *one = cut;
                    }
                }
            }
        });
        let changed = STATE.with(|slot| slot.borrow().dragging != Drag::Nothing);
        if changed {
            stand_all(window);
        }
    }

    // The cursor follows what the press would take — but only when the
    // media is under the pointer; over the card's own buttons the
    // arrow answers for itself. While a drag stands, the hand in hand
    // says the shape.
    let aim = if on_media {
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
    aim_cursor(aim);

    // The card still sees the pointer, for the hover of its buttons.
    // The telling is not done while the host is held: a button under
    // the pointer answers the moment the card hears of it, and its
    // answer reads the host on its own.
    let event = if pressed {
        WindowEvent::PointerPressed {
            position: LogicalPosition::new(x as f32, y as f32),
            button: PointerEventButton::Left,
        }
    } else {
        WindowEvent::PointerMoved {
            position: LogicalPosition::new(x as f32, y as f32),
        }
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

/// Lets a drag go: a drawing too small to mean anything is dropped, a
/// drawn cover stands chosen, and everything else keeps what it drew.
unsafe fn released(window: HWND, x: i32, y: i32) {
    STATE.with(|slot| {
        let mut state = slot.borrow_mut();
        let dragging = state.dragging;
        state.dragging = Drag::Nothing;
        match dragging {
            Drag::Drawing { index, .. } => {
                let cut = state.rects.get(index).copied();
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
    stand_all(window);

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
mod tests {
    use super::*;

    #[test]
    fn the_height_of_a_cover_window_is_its_bands_and_its_stage() {
        assert_eq!(Layout::height(), 560.0);
    }

    #[test]
    fn the_layout_stacks_the_body_top_to_bottom() {
        let layout = Layout::of(420, 560, 1.0);

        // The stage stands between the two hairlines, with a room under
        // the header's own one.
        assert_eq!(layout.hairline_top.top, 62);
        assert_eq!(layout.stage.top, 82);
        assert_eq!(layout.stage.left, 24);
        assert_eq!(layout.stage.right, 396);
        assert_eq!(layout.stage.bottom, 318);

        // The three rows of the look, one after another below the stage.
        assert_eq!(layout.style_box.top, 332);
        assert_eq!(layout.style_box.bottom, 366);
        assert_eq!(layout.well.top, 376);
        assert_eq!(layout.text.top, 428);

        // The chips of the style row: two of them, a hand's width apart,
        // standing in the row's own box.
        assert_eq!(layout.solid.left, layout.style_box.left + 3);
        assert_eq!(layout.solid.right - layout.solid.left, 64);
        assert_eq!(layout.blur.left, layout.solid.right + 6);
        assert_eq!(layout.blur.right, layout.style_box.right - 3);

        // The colour well at the right of its row, the detectors' chips
        // at the right of theirs.
        assert_eq!(layout.well.right, 396);
        assert_eq!(layout.well.right - layout.well.left, 62);
        assert_eq!(layout.faces.right + 8, layout.text.left);
        assert_eq!(layout.text.right, 396);

        // The note, and the hairline that closes the body.
        assert_eq!(layout.note.top, 466);
        assert_eq!(layout.hairline_bottom.top, 496);

        // The footer: measured from the bottom, Reset at the left, the
        // making of the copy at the right.
        assert_eq!(layout.reset.left, 24);
        assert_eq!(layout.reset.right - layout.reset.left, 84);
        assert_eq!(layout.run.right, 396);
        assert_eq!(layout.run.right - layout.run.left, 168);
        assert_eq!(layout.run.top, 510);
        assert_eq!(layout.run.bottom, 544);
    }

    #[test]
    fn the_media_stands_as_large_as_the_stage_allows_and_no_larger() {
        // A landscape picture leads by its height here.
        assert_eq!(fit_of(372.0, 236.0, 160.0, 120.0), 236.0 / 120.0);
        // A small one is not held back all the same.
        assert_eq!(fit_of(372.0, 236.0, 64.0, 48.0), 236.0 / 48.0);
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
        let rects = [(0.0, 0.0, 100.0, 100.0)];
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
        // The pills on the edges, at the middle of each side, on a cover
        // big enough to wear them.
        assert_eq!(
            picked_at((50.0, 3.0), &rects, Some(0), 1.0),
            Pick::Corner(Spot::Top)
        );
        assert_eq!(
            picked_at((50.0, 97.0), &rects, Some(0), 1.0),
            Pick::Corner(Spot::Bottom)
        );
        assert_eq!(
            picked_at((3.0, 50.0), &rects, Some(0), 1.0),
            Pick::Corner(Spot::Left)
        );
        assert_eq!(
            picked_at((97.0, 50.0), &rects, Some(0), 1.0),
            Pick::Corner(Spot::Right)
        );
        // Deep inside: the whole cover comes along.
        assert_eq!(picked_at((50.0, 50.0), &rects, Some(0), 1.0), Pick::Track(0));
        // Wholly away from it: a new cover is drawn.
        assert_eq!(picked_at((150.0, 50.0), &rects, Some(0), 1.0), Pick::Create);

        // A cover too small to wear the pills takes no pill press: its
        // face answers away from the corners, and a press near a bare
        // edge is a new drawing.
        let small = [(0.0, 0.0, 30.0, 30.0)];
        assert_eq!(
            picked_at((15.0, 15.0), &small, Some(0), 1.0),
            Pick::Track(0)
        );
        assert_eq!(
            picked_at((15.0, 3.0), &small, Some(0), 1.0),
            Pick::Create
        );

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
        // The top pill: the bottom keeps its word.
        assert_eq!(
            resized(Spot::Top, anchored, (150.0, 50.0), 1000.0, 1000.0),
            (100.0, 50.0, 200.0, 150.0)
        );
        // The bottom pill: the top keeps its word.
        assert_eq!(
            resized(Spot::Bottom, anchored, (150.0, 250.0), 1000.0, 1000.0),
            (100.0, 100.0, 200.0, 150.0)
        );
        // The left pill: the right keeps its word.
        assert_eq!(
            resized(Spot::Left, anchored, (50.0, 150.0), 1000.0, 1000.0),
            (50.0, 100.0, 250.0, 100.0)
        );
        // The right pill: the left keeps its word.
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
        // A carry-along past the right edge stops at it.
        assert_eq!(
            placed((0.0, 0.0, 32.0, 32.0), (0.0, 0.0), (500.0, 0.0), 160.0, 120.0),
            (128.0, 0.0, 32.0, 32.0)
        );
        // A carry-along past the top-left stops at 0.
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
    fn a_cover_is_said_in_thousandths_of_the_media() {
        assert_eq!(
            areas_of(&[(0.0, 0.0, 800.0, 600.0)], 1600.0, 1200.0),
            vec![RedactArea {
                x_mille: 0,
                y_mille: 0,
                w_mille: 500,
                h_mille: 500,
                start_ms: None,
                end_ms: None,
            }]
        );
        // Rounding, half away from zero, at whatever thousandth a corner
        // lands on.
        assert_eq!(
            areas_of(&[(8.0, 8.0, 792.0, 592.0)], 1600.0, 1200.0),
            vec![RedactArea {
                x_mille: 5,
                y_mille: 7,
                w_mille: 495,
                h_mille: 493,
                start_ms: None,
                end_ms: None,
            }]
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
            &[(0.0, 0.0, 1600.0, 1200.0)],
            1600.0,
            1200.0,
        );
        assert_eq!(solid.redact_style, Some(RedactStyle::Solid));
        assert_eq!(solid.redact_color, Some(String::from("000000")));
        assert_eq!(solid.redact_areas[0].w_mille, 1000);
        assert_eq!(solid.redact_areas[0].h_mille, 1000);
        // A smear, then, and the third colour of the wheel.
        let blur = settings_from(
            1,
            2,
            &[(8.0, 8.0, 792.0, 592.0)],
            1600.0,
            1200.0,
        );
        assert_eq!(blur.redact_style, Some(RedactStyle::Blur));
        assert_eq!(blur.redact_color, Some(String::from("808080")));
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

    /// Makes a small test picture, a red-to-green ramp, so the capture
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

    #[test]
    fn showing_and_pumping_the_redact_image_card_does_not_panic() {
        crate::ui::wheel::install_platform();
        let (sender, _receiver) = std::sync::mpsc::channel();
        let mut window = RedactImageWindow::new(sender).expect("the window was made");
        window.show_for(
            "tools.redact_image",
            "Redact image",
            "Cover the parts a person does not show",
            &[PathBuf::from("not-there.png")],
        );
        pump(40);
        window.hide();
    }

    #[test]
    fn a_style_chip_answers_the_pointer_without_the_host_being_taken() {
        crate::ui::wheel::install_platform();
        let (sender, _receiver) = std::sync::mpsc::channel();
        let mut window = RedactImageWindow::new(sender).expect("the window was made");
        window.show_for(
            "tools.redact_image",
            "Redact image",
            "Cover the parts a person does not show",
            &[PathBuf::from("not-there.png")],
        );
        pump(10);
        // Where the Blur chip stands, said in the card's own units.
        let told = HOST.with(|slot| {
            let borrowed = slot.borrow();
            let Some(host) = borrowed.as_ref() else {
                return None;
            };
            let scale = scale_factor(host.window);
            let layout = Layout::of(host.width as i32, host.height as i32, scale);
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
            draw_window(window.window);
        }
        // The smear was taken, and the card wears it.
        let style = STATE.with(|slot| slot.borrow().style);
        assert_eq!(style, 1, "the blur chip was not taken");
        let card_says_solid = HOST.with(|slot| {
            let borrowed = slot.borrow();
            borrowed
                .as_ref()
                .map(|host| host.card.get_solid_style())
                .unwrap_or(true)
        });
        assert!(!card_says_solid, "the card does not wear the choice");
        window.hide();
    }

    #[test]
    fn a_cover_drawn_through_the_window_messages_stands_in_the_state() {
        crate::ui::wheel::install_platform();
        let folder = std::env::temp_dir().join("open-tangerine-redact-test");
        std::fs::create_dir_all(&folder).expect("the test folder was made");
        let picture = make_test_image(&folder);
        let (sender, _receiver) = std::sync::mpsc::channel();
        let mut window = RedactImageWindow::new(sender).expect("the window was made");
        window.show_for(
            "tools.redact_image",
            "Redact image",
            "Cover the parts a person does not show",
            &[picture],
        );
        pump(10);
        // Where the media stands, in window pixels, and the two places
        // the drag is taken between: off the seeded cover's face, on the
        // media itself, so the drag draws rather than carries.
        let said = STATE.with(|slot| {
            let state = slot.borrow();
            let at_media = |point: (f64, f64)| -> (i32, i32) {
                (
                    (state.media_box.0 + point.0 * state.fit) as i32,
                    (state.media_box.1 + point.1 * state.fit) as i32,
                )
            };
            Some((at_media((10.0, 120.0)), at_media((60.0, 170.0))))
        });
        let Some((press, let_go)) = said else {
            panic!("the media never stood");
        };
        let pack = |x: i32, y: i32| -> LPARAM {
            LPARAM(((y & 0xFFFF) << 16) as isize | (x & 0xFFFF) as isize)
        };
        // A press, a carry, and a let-go — the window's own road.
        unsafe {
            let _ = SendMessageW(
                window.window,
                WM_LBUTTONDOWN,
                Some(WPARAM(MK_LBUTTON.0 as usize)),
                Some(pack(press.0, press.1)),
            );
            let _ = SendMessageW(window.window, WM_MOUSEMOVE, Some(WPARAM(0)), Some(pack(let_go.0, let_go.1)));
            let _ = SendMessageW(window.window, WM_LBUTTONUP, Some(WPARAM(0)), Some(pack(let_go.0, let_go.1)));
        }
        pump(2);
        STATE.with(|slot| {
            let state = slot.borrow();
            assert_eq!(state.rects.len(), 2, "the drawn cover was not kept");
            assert_eq!(state.selected, Some(1), "the drawn cover stands chosen");
            let (_, _, width, height) = state.rects[1];
            assert!(
                (40.0..60.0).contains(&width) && (40.0..60.0).contains(&height),
                "the drawn cover kept the drag, not the air: {} × {}",
                width,
                height
            );
        });
        window.hide();
        let _ = std::fs::remove_dir_all(&folder);
    }

    /// Opens the cover window on a real picture, lets it come in and
    /// draw, and captures what a person would see: one cover standing
    /// chosen over the picture, its eight hands about it, and the rows
    /// of the look below.
    #[test]
    #[ignore = "writes a screenshot"]
    fn capture_the_redact_image_window() {
        crate::ui::wheel::install_platform();
        let folder = std::env::temp_dir().join("open-tangerine-capture");
        std::fs::create_dir_all(&folder).expect("the capture folder was made");
        let picture = make_test_image(&folder);
        let (sender, _receiver) = std::sync::mpsc::channel();
        let mut window = RedactImageWindow::new(sender).expect("the window was made");
        window.show_for(
            "tools.redact_image",
            "Redact image",
            "Cover the parts a person does not show",
            &[picture],
        );
        // Long enough for the fade to finish and the picture to be read.
        for round in 0..2400 {
            pump(1);
            if get_fade_done() && round > 200 {
                break;
            }
        }
        let shots = std::path::Path::new("target").join("shots");
        std::fs::create_dir_all(&shots).expect("the shots folder was made");
        // The buffer itself, before any window machinery touches it: if
        // the rows are here, the drawing is right and the capture lies;
        // if not, the renderer is the one clipping.
        HOST.with(|slot| {
            let borrowed = slot.borrow();
            if let Some(host) = borrowed.as_ref() {
                let mut rgba = Vec::with_capacity(host.buffer.len() * 4);
                for pixel in &host.buffer {
                    let (_, red, green, blue) = pixel.parts();
                    rgba.extend_from_slice(&[red, green, blue, 255]);
                }
                let _ = image::save_buffer(
                    shots.join("redact-image-buffer.png"),
                    &rgba,
                    host.width,
                    host.height,
                    image::ExtendedColorType::Rgba8,
                );
            }
        });
        capture(window.window, &shots.join("redact-image-window.png"));
        window.hide();
        let _ = std::fs::remove_dir_all(&folder);
    }
}
