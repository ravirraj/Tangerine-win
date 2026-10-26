//! The window a tool opens in.
//!
//! A tool on the wheel does not run the moment it is picked: it opens a
//! window of its own, the way the reference product does, with the file
//! in it, the choices the tool offers, and a way to say no.
//!
//! The card is drawn by the same renderer that draws the wheel, into a
//! buffer that is put on a plain window of the system; the rounded boxes,
//! the words and the hover of the buttons all come from the one drawing
//! the whole product is drawn with. Only the typing is the system's own:
//! the two fields are real system fields laid over the tiles the card
//! draws, so the keyboard, the focus and the reader for the blind behave
//! the way they do everywhere.
//!
//! The keyboard walks the card the way it walks every card: Tab and the
//! arrows go from field to field to Cancel to Run, Enter does what is
//! under the keyboard, and Escape says no to the whole thing.
//!
//! What the window is asked comes back as a message: the window never
//! runs anything itself. The messages of this window are pumped by the
//! loop that pumps the wheel's, because it takes every message the thread
//! is sent.

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
    GetKeyState, SetFocus, VK_DOWN, VK_ESCAPE, VK_LEFT, VK_RETURN, VK_RIGHT, VK_SHIFT, VK_SPACE,
    VK_TAB, VK_UP,
};
use windows::Win32::UI::Shell::{DefSubclassProc, SetWindowSubclass};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, GetAncestor, GetDlgItem, GetWindowTextLengthW,
    GetWindowTextW, GetClientRect, KillTimer, LoadCursorW, MoveWindow, RegisterClassW, SendMessageW,
    SetForegroundWindow, SetLayeredWindowAttributes, SetTimer, SetWindowTextW, ShowWindow,
    SW_HIDE, SW_SHOW, CW_USEDEFAULT, GA_ROOT, HMENU, IDC_ARROW, LWA_ALPHA, MINMAXINFO, WNDCLASSW,
    WM_CLOSE, WM_COMMAND, WM_CTLCOLOREDIT, WM_DESTROY, WM_ERASEBKGND, WM_GETMINMAXINFO,
    WM_KEYDOWN, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_PAINT, WM_SIZE, WM_TIMER,
    WM_SETFONT, WS_CHILD, WS_EX_LAYERED, WS_MAXIMIZEBOX, WS_OVERLAPPEDWINDOW, WS_TABSTOP,
    WS_VISIBLE, ES_NUMBER, WINDOW_EX_STYLE, WINDOW_STYLE,
};

use crate::platform::windows::windowhost::{scale_factor, screen_scale, to_wide};
use crate::radial::execution::ToolSettings;
use crate::ui::wheel::{window_for_next_component, Bgra8, Panel, ToolCard};

/// The tells of the system's fields: the field gained or lost the focus.
const EN_SETFOCUS: u32 = 0x0100;
const EN_KILLFOCUS: u32 = 0x0200;

/// The timer the window comes in softly on.
const FADE_TIMER_ID: usize = 51;

/// The card, in the drawing's own units, before the display stretches it.
const CARD_W: i32 = 460;
const CARD_H: i32 = 560;
/// How small the card may be made.
const MIN_W: i32 = 420;
const MIN_H: i32 = 460;

/// What the tool window tells the program.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ToolEvent {
    /// The tool was told to run, with what the window was asked.
    Run {
        operation: String,
        inputs: Vec<PathBuf>,
        settings: ToolSettings,
    },
    /// The window was closed without running anything.
    Cancelled,
}

/// What the window is open for, where the drawing and the pressing both
/// read it.
struct Shown {
    wants_size: bool,
}

thread_local! {
    static EVENTS: std::cell::RefCell<Option<Sender<ToolEvent>>> =
        const { std::cell::RefCell::new(None) };
    static OPEN_FOR: std::cell::RefCell<Option<(String, Vec<PathBuf>)>> =
        const { std::cell::RefCell::new(None) };
    static SHOWN: std::cell::RefCell<Shown> =
        std::cell::RefCell::new(Shown { wants_size: false });
    static HOST: std::cell::RefCell<Option<Host>> = const { std::cell::RefCell::new(None) };
    // The handles the card's own callbacks need, kept apart from the host
    // so a callback fired in the middle of a drawing never has to borrow
    // what is already borrowed. A handle of zero means "not there".
    static WINDOW: std::cell::Cell<isize> = const { std::cell::Cell::new(0) };
    static WIDTH_FIELD: std::cell::Cell<isize> = const { std::cell::Cell::new(0) };
    static HEIGHT_FIELD: std::cell::Cell<isize> = const { std::cell::Cell::new(0) };
    // Where the keyboard is: 0 width, 1 height, 2 cancel, 3 run.
    static FOCUS_INDEX: std::cell::Cell<i32> = const { std::cell::Cell::new(0) };
    // How far along the soft coming-in is.
    static FADE_ALPHA: std::cell::Cell<u8> = const { std::cell::Cell::new(255) };
}

/// The window handle, if there is one.
fn window_handle() -> HWND {
    HWND(WINDOW.get() as *mut _)
}

/// The window, its card, and the buffer the card is drawn into.
///
/// Kept in one place where the window's procedure and the program both
/// reach them, because the procedure is a plain function of the system.
struct Host {
    window: HWND,
    card: ToolCard,
    software: Rc<MinimalSoftwareWindow>,
    buffer: Vec<Bgra8>,
    width: u32,
    height: u32,
    width_field: HWND,
    height_field: HWND,
}

/// The window one tool is asked in.
pub struct ToolWindow {
    window: HWND,
}

// The window belongs to the thread that made it and is used by no other.
unsafe impl Send for ToolWindow {}

impl ToolWindow {
    /// Makes the window, hidden until a tool is picked.
    pub fn new(events: Sender<ToolEvent>) -> Result<ToolWindow, String> {
        EVENTS.with(|slot| *slot.borrow_mut() = Some(events));

        // A window is put aside for the card before the card is made, so
        // the two find each other.
        let software = window_for_next_component();
        let card = ToolCard::new().map_err(|error| error.to_string())?;

        let instance = unsafe { GetModuleHandleW(None).map_err(|error| error.to_string())? };
        let class_name = w!("OpenTangerineTool");
        // The card is made at the size the display stretches it to, and
        // the card's own white is what fills it while it comes in, so a
        // dark theme's idea of "window colour" never shows through.
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

        // The fields are the only children: everything else is drawn. The
        // card draws the word and the unit around each; the field itself
        // takes only the digits of a size.
        let field_font = unsafe { interface_font(window) };
        let make_field = |id: isize| unsafe {
            let field = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("EDIT"),
                w!(""),
                WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(ES_NUMBER as u32),
                0,
                0,
                100,
                20,
                Some(window),
                Some(HMENU(id as *mut _)),
                Some(instance.into()),
                None,
            )
            .unwrap_or_default();
            SendMessageW(
                field,
                WM_SETFONT,
                Some(WPARAM(field_font.0 as usize)),
                Some(LPARAM(1)),
            );
            let _ = SetWindowSubclass(field, Some(field_procedure), 1, 0);
            field
        };
        let width_field = make_field(50);
        let height_field = make_field(60);

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
        }

        let mut host = Host {
            window,
            card,
            software,
            buffer: Vec::new(),
            width: 0,
            height: 0,
            width_field,
            height_field,
        };
        host.resize();
        HOST.with(|slot| *slot.borrow_mut() = Some(host));
        WINDOW.with(|slot| slot.set(window.0 as isize));
        WIDTH_FIELD.with(|slot| slot.set(width_field.0 as isize));
        HEIGHT_FIELD.with(|slot| slot.set(height_field.0 as isize));

        Ok(ToolWindow { window })
    }

    /// Shows the window for one operation on the given files.
    pub fn show_for(
        &mut self,
        operation: &str,
        label: &str,
        detail: &str,
        inputs: &[PathBuf],
        preview: Option<&image::DynamicImage>,
    ) {
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

        // A size is asked only by the tools that have one.
        let wants_size = operation == "tools.resize";
        SHOWN.with(|shown| shown.borrow_mut().wants_size = wants_size);

        // Everything the card needs is set first, while the host is held
        // only for that and nothing is shown yet. The picture is made at
        // the size of its own box, so it is never drawn smaller than it
        // is shown.
        let (width_field, height_field, card_window) = HOST.with(|slot| {
            let mut borrowed = slot.borrow_mut();
            let Some(host) = borrowed.as_mut() else {
                return (HWND::default(), HWND::default(), HWND::default());
            };
            host.card.set_file_name(said.into());
            host.card.set_detail(detail.into());
            host.card.set_wants_size(wants_size);
            host.card.set_width_focused(false);
            host.card.set_height_focused(false);
            let layout = host.lay_out();
            let box_w = (layout.preview.right - layout.preview.left).max(1) as u32;
            let box_h = (layout.preview.bottom - layout.preview.top).max(1) as u32;
            let picture = preview.map(|picture| {
                let small = picture.thumbnail(box_w, box_h);
                let rgba = small.to_rgba8();
                let buffer = SharedPixelBuffer::clone_from_slice(
                    rgba.as_raw(),
                    small.width(),
                    small.height(),
                );
                slint::Image::from_rgba8(buffer)
            });
            host.card.set_has_preview(picture.is_some());
            if let Some(picture) = picture {
                host.card.set_preview(picture);
            }
            host.software.request_redraw();
            (host.width_field, host.height_field, host.window)
        });

        // And only then is anything shown: the messages that come of
        // showing a window can arrive at once, and the one that draws the
        // card must not find the card still held.
        if card_window.0.is_null() {
            return;
        }
        unsafe {
            let _ = SetWindowTextW(card_window, PCWSTR::from_raw(to_wide(label).as_ptr()));
            for field in [width_field, height_field] {
                if !field.0.is_null() {
                    let _ = SetWindowTextW(field, PCWSTR::from_raw(to_wide("").as_ptr()));
                    let _ = ShowWindow(field, if wants_size { SW_SHOW } else { SW_HIDE });
                }
            }
            let _ = InvalidateRect(Some(card_window), None, true);
            // The window comes in softly, from nothing to whole in a few
            // beats of the timer.
            FADE_ALPHA.with(|slot| slot.set(0));
            let _ = SetLayeredWindowAttributes(card_window, COLORREF(0), 0, LWA_ALPHA);
            let _ = SetForegroundWindow(card_window);
            let _ = ShowWindow(card_window, SW_SHOW);
            let _ = SetTimer(Some(card_window), FADE_TIMER_ID, 16, None);
            // The keyboard starts where the tool has something to take:
            // the first field when there are fields, the go button when
            // there are not.
            let first = if wants_size { 0 } else { 3 };
            FOCUS_INDEX.with(|slot| slot.set(first));
            apply_focus(card_window, first);
        }
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

    /// Tells the card where everything is, and puts the fields in their
    /// tiles. Both read the same numbers, so what is drawn and where the
    /// typing happens can never disagree.
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
        self.card.set_width_tile(panel(layout.width_tile));
        self.card.set_height_tile(panel(layout.height_tile));
        self.card.set_helper_box(panel(layout.helper));
        self.card.set_cancel_box(panel(layout.cancel));
        self.card.set_run_box(panel(layout.run));

        // Each field is laid between the word and the unit its tile draws.
        let at = |value: i32| (value as f32 * scale) as i32;
        for (field, tile) in [
            (self.width_field, layout.width_tile),
            (self.height_field, layout.height_tile),
        ] {
            unsafe {
                let _ = MoveWindow(
                    field,
                    tile.left + at(58),
                    tile.top + at(6),
                    (tile.right - tile.left) - at(58) - at(28),
                    (tile.bottom - tile.top) - at(12),
                    true,
                );
            }
        }
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
        crate::platform::windows::windowhost::blit_card_buffer(
            self.window,
            &self.buffer,
            self.width,
            self.height,
        );
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
    preview: RECT,
    width_tile: RECT,
    height_tile: RECT,
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
            preview: RECT::default(),
            width_tile: RECT::default(),
            height_tile: RECT::default(),
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

        // The body: the picture takes everything between the hairlines,
        // less the tiles when the tool asks for a size.
        let body_top = layout.hairline_top.bottom + at(14);
        let wants_size = SHOWN.with(|shown| shown.borrow().wants_size);
        if wants_size {
            let tile_bottom = footer_top - at(16);
            let tile_top = tile_bottom - at(36);
            let tile_width = (whole - margin * 2 - at(12)) / 2;
            layout.width_tile = RECT {
                left: margin,
                top: tile_top,
                right: margin + tile_width,
                bottom: tile_bottom,
            };
            layout.height_tile = RECT {
                left: margin + tile_width + at(12),
                top: tile_top,
                right: whole - margin,
                bottom: tile_bottom,
            };
            layout.preview = RECT {
                left: margin,
                top: body_top,
                right: whole - margin,
                bottom: tile_top - at(12),
            };
        } else {
            layout.preview = RECT {
                left: margin,
                top: body_top,
                right: whole - margin,
                bottom: footer_top - at(16),
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

/// Puts the keyboard on one stop of the card: a field or a button.
unsafe fn apply_focus(window: HWND, index: i32) {
    // The focus is given before anything of the card is touched: giving
    // it tells the field's window at once, and that telling must not
    // arrive while the card is held.
    let field = match index {
        0 => WIDTH_FIELD.with(|slot| HWND(slot.get() as *mut _)),
        1 => HEIGHT_FIELD.with(|slot| HWND(slot.get() as *mut _)),
        _ => HWND::default(),
    };
    if index <= 1 && !field.0.is_null() {
        unsafe { let _ = SetFocus(Some(field)); };
    } else {
        unsafe { let _ = SetFocus(Some(window)); };
    }
    HOST.with(|slot| {
        let mut borrowed = slot.borrow_mut();
        if let Some(host) = borrowed.as_mut() {
            host.card
                .set_button_focus(if index >= 2 { index - 1 } else { 0 });
            host.software.request_redraw();
        }
    });
    let _ = unsafe { InvalidateRect(Some(window), None, true) };
}

/// Walks the keyboard to the next stop of the card, or the one before.
unsafe fn cycle_focus(window: HWND, forward: bool) {
    let wants_size = SHOWN.with(|shown| shown.borrow().wants_size);
    let stops: &[i32] = if wants_size { &[0, 1, 2, 3] } else { &[2, 3] };
    let current = FOCUS_INDEX.with(|slot| slot.get());
    let at = stops.iter().position(|&stop| stop == current).unwrap_or(0);
    let count = stops.len();
    let next = if forward {
        (at + 1) % count
    } else {
        (at + count - 1) % count
    };
    let index = stops[next];
    FOCUS_INDEX.with(|slot| slot.set(index));
    unsafe { apply_focus(window, index) };
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
            let x = (lparam.0 & 0xffff) as i16 as f32;
            let y = ((lparam.0 >> 16) & 0xffff) as i16 as f32;
            let position = LogicalPosition::new(x, y);
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
        WM_COMMAND => {
            // A field taking or losing the focus lights the border of its
            // tile, and tells the card where the keyboard now is.
            let happened = ((wparam.0 >> 16) & 0xffff) as u32;
            if happened == EN_SETFOCUS || happened == EN_KILLFOCUS {
                let field = HWND(lparam.0 as *mut _);
                let on = happened == EN_SETFOCUS;
                let is_width =
                    WIDTH_FIELD.with(|slot| slot.get()) == field.0 as isize;
                let is_height =
                    HEIGHT_FIELD.with(|slot| slot.get()) == field.0 as isize;
                HOST.with(|slot| {
                    let mut borrowed = slot.borrow_mut();
                    if let Some(host) = borrowed.as_mut() {
                        if is_width {
                            host.card.set_width_focused(on);
                        }
                        if is_height {
                            host.card.set_height_focused(on);
                        }
                        host.software.request_redraw();
                    }
                });
                if on && is_width {
                    FOCUS_INDEX.with(|slot| slot.set(0));
                }
                if on && is_height {
                    FOCUS_INDEX.with(|slot| slot.set(1));
                }
                let _ = unsafe { InvalidateRect(Some(window), None, true) };
            }
            LRESULT(0)
        }
        WM_CTLCOLOREDIT => {
            // The field is white with dark words, over the white of its
            // tile: the border the eye follows is the card's, not the
            // system's.
            let context = windows::Win32::Graphics::Gdi::HDC(lparam.0 as *mut _);
            unsafe {
                windows::Win32::Graphics::Gdi::SetTextColor(context, COLORREF(0x0024_2424));
                windows::Win32::Graphics::Gdi::SetBkColor(context, COLORREF(0x00FF_FFFF));
            }
            LRESULT(unsafe { CreateSolidBrush(COLORREF(0x00FF_FFFF)).0 as isize })
        }
        WM_KEYDOWN => {
            let key = wparam.0 as u16;
            let shift = unsafe { GetKeyState(VK_SHIFT.0 as i32) < 0 };
            let forward =
                (key == VK_TAB.0 && !shift) || key == VK_DOWN.0 || key == VK_RIGHT.0;
            let backward =
                (key == VK_TAB.0 && shift) || key == VK_UP.0 || key == VK_LEFT.0;
            if forward || backward {
                unsafe { cycle_focus(window, forward) };
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
                let _ = ShowWindow(window, SW_HIDE);
            }
            LRESULT(0)
        }
        WM_DESTROY => LRESULT(0),
        _ => unsafe { DefWindowProcW(window, message, wparam, lparam) },
    }
}

/// The keys a field takes before the field sees them: Enter runs, Escape
/// says no, and Tab and the arrows walk the card, so the field never puts
/// a new line where a number belongs.
unsafe extern "system" fn field_procedure(
    field: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _which: usize,
    _given: usize,
) -> LRESULT {
    if message == WM_KEYDOWN {
        let key = wparam.0 as u16;
        if key == VK_RETURN.0 {
            let parent = unsafe { GetAncestor(field, GA_ROOT) };
            unsafe { run(parent) };
            return LRESULT(0);
        }
        if key == VK_ESCAPE.0 {
            let parent = unsafe { GetAncestor(field, GA_ROOT) };
            unsafe { cancel(parent) };
            return LRESULT(0);
        }
        let shift = unsafe { GetKeyState(VK_SHIFT.0 as i32) < 0 };
        let forward =
            (key == VK_TAB.0 && !shift) || key == VK_DOWN.0 || key == VK_RIGHT.0;
        let backward =
            (key == VK_TAB.0 && shift) || key == VK_UP.0 || key == VK_LEFT.0;
        if forward || backward {
            let parent = unsafe { GetAncestor(field, GA_ROOT) };
            unsafe { cycle_focus(parent, forward) };
            return LRESULT(0);
        }
    }
    unsafe { DefSubclassProc(field, message, wparam, lparam) }
}

/// Runs what the window was open for, with what the fields say.
unsafe fn run(window: HWND) {
    let settings = unsafe {
        ToolSettings {
            width: number(&field_text(GetDlgItem(Some(window), 50).unwrap_or_default())),
            height: number(&field_text(GetDlgItem(Some(window), 60).unwrap_or_default())),
            ..Default::default()
        }
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
    send(ToolEvent::Cancelled);
    unsafe { let _ = ShowWindow(window, SW_HIDE); };
}

/// What a field says.
unsafe fn field_text(field: HWND) -> String {
    let length = unsafe { GetWindowTextLengthW(field) };
    let mut text = vec![0u16; (length + 1) as usize];
    unsafe {
        let _ = GetWindowTextW(field, &mut text);
    }
    String::from_utf16_lossy(&text[..length.max(0) as usize])
}

/// A number in words, if the words say one.
fn number(text: &str) -> Option<u32> {
    text.trim().parse::<u32>().ok().filter(|value| *value > 0)
}

/// The font the system's fields are given, in the face people read.
unsafe fn interface_font(window: HWND) -> windows::Win32::Graphics::Gdi::HFONT {
    let scale = scale_factor(window);
    unsafe {
        windows::Win32::Graphics::Gdi::CreateFontW(
            -(13.0 * scale) as i32,
            0,
            0,
            0,
            400,
            0,
            0,
            0,
            windows::Win32::Graphics::Gdi::DEFAULT_CHARSET,
            windows::Win32::Graphics::Gdi::OUT_DEFAULT_PRECIS,
            windows::Win32::Graphics::Gdi::CLIP_DEFAULT_PRECIS,
            windows::Win32::Graphics::Gdi::CLEARTYPE_QUALITY,
            windows::Win32::Graphics::Gdi::DEFAULT_PITCH.0 as u32,
            w!("Segoe UI"),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_number_is_read_from_words() {
        assert_eq!(number("800"), Some(800));
        assert_eq!(number(" 800 "), Some(800));
        assert_eq!(number(""), None);
        assert_eq!(number("wide"), None);
        assert_eq!(number("0"), None);
    }

    #[test]
    fn the_layout_gives_the_buttons_the_bottom_and_the_picture_the_rest() {
        SHOWN.with(|shown| shown.borrow_mut().wants_size = true);
        let layout = Layout::of(460, 560, 1.0);
        // The buttons sit inside the window, next to each other.
        assert!(layout.cancel.right <= layout.run.left);
        assert!(layout.run.bottom <= 560);
        // The two hairlines bracket the body.
        assert!(layout.hairline_top.bottom < layout.preview.top);
        assert!(layout.preview.bottom < layout.hairline_bottom.top);
        // The picture sits above the tiles, the tiles above the footer.
        assert!(layout.preview.bottom <= layout.width_tile.top);
        assert!(layout.width_tile.bottom <= layout.hairline_bottom.top);
        assert_eq!(layout.width_tile.top, layout.height_tile.top);
        assert_eq!(layout.width_tile.bottom, layout.height_tile.bottom);
        // The helper says its piece without running into the buttons.
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
    /// panic: the program's own loop does exactly this when a tool is
    /// picked from the wheel.
    #[test]
    fn showing_and_pumping_the_card_does_not_panic() {
        crate::ui::wheel::install_platform();
        let (sender, _receiver) = std::sync::mpsc::channel();
        let mut window = ToolWindow::new(sender).expect("the window was made");
        let picture = image::DynamicImage::new_rgb8(8, 6);
        window.show_for(
            "tools.crop",
            "Crop",
            "Crop the picture",
            &[PathBuf::from("one.png")],
            Some(&picture),
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
