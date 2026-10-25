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
//! the two fields are real system fields laid over the boxes the card
//! draws, so the keyboard, the focus and the reader for the blind behave
//! the way they do everywhere.
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
use windows::Win32::Graphics::Gdi::{BeginPaint, InvalidateRect};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{SetFocus, VK_ESCAPE, VK_RETURN};
use windows::Win32::UI::Shell::{DefSubclassProc, SetWindowSubclass};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, GetAncestor, GetDlgItem, GetWindowTextLengthW,
    GetWindowTextW, GetClientRect, LoadCursorW, MoveWindow, RegisterClassW, SendMessageW,
    SetWindowTextW, ShowWindow, CW_USEDEFAULT, GA_ROOT, HMENU, IDC_ARROW, WNDCLASSW, WM_CLOSE,
    WM_COMMAND, WM_CTLCOLOREDIT, WM_DESTROY, WM_ERASEBKGND, WM_KEYDOWN, WM_LBUTTONDOWN,
    WM_LBUTTONUP, WM_MOUSEMOVE, WM_PAINT, WM_SIZE, WM_SETFONT, WS_CHILD, WS_OVERLAPPEDWINDOW,
    WS_TABSTOP, WS_VISIBLE, ES_CENTER, WINDOW_EX_STYLE, WINDOW_STYLE,
};

use crate::radial::execution::ToolSettings;
use crate::ui::wheel::{window_for_next_component, Bgra8, Panel, ToolCard};

/// What a field is told when it is given a word to hold out for the
/// person to fill.
const EM_SETCUEBANNER: u32 = 0x1501;
/// The tells of the system's fields: the field gained or lost the focus.
const EN_SETFOCUS: u32 = 0x0100;
const EN_KILLFOCUS: u32 = 0x0200;

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
        let window = unsafe {
            let class = WNDCLASSW {
                lpfnWndProc: Some(window_procedure),
                hInstance: instance.into(),
                lpszClassName: class_name,
                hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
                hbrBackground: windows::Win32::Graphics::Gdi::GetSysColorBrush(
                    windows::Win32::Graphics::Gdi::COLOR_WINDOW,
                ),
                ..Default::default()
            };
            RegisterClassW(&class);

            CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                class_name,
                w!("Open Tangerine"),
                WS_OVERLAPPEDWINDOW,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                460,
                560,
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

        // The fields are the only children: everything else is drawn. Each
        // holds a word out for the person to fill, and takes the keyboard
        // the way every field does.
        let field_font = unsafe { interface_font(window) };
        let make_field = |cue: &str, id: isize| unsafe {
            let field = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("EDIT"),
                w!(""),
                WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(ES_CENTER as u32),
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
            let cue_wide: Vec<u16> = cue.encode_utf16().chain([0]).collect();
            SendMessageW(
                field,
                EM_SETCUEBANNER,
                Some(WPARAM(1)),
                Some(LPARAM(cue_wide.as_ptr() as isize)),
            );
            SetWindowSubclass(field, Some(field_procedure), 1, 0);
            field
        };
        let width_field = make_field("Width", 50);
        let height_field = make_field("Height", 60);

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
                    send(ToolEvent::Cancelled);
                    unsafe {
                        ShowWindow(window, windows::Win32::UI::WindowsAndMessaging::SW_HIDE)
                    };
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
        // only for that and nothing is shown yet.
        let (width_field, height_field, card_window) = HOST.with(|slot| {
            let mut borrowed = slot.borrow_mut();
            let Some(host) = borrowed.as_mut() else {
                return (HWND::default(), HWND::default(), HWND::default());
            };
            host.card.set_title_text(detail.into());
            host.card.set_file_text(said.into());
            host.card.set_wants_size(wants_size);
            let picture = preview.map(|picture| {
                let small = picture.thumbnail(400, 300);
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
            host.lay_out();
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
            SetWindowTextW(card_window, PCWSTR::from_raw(to_wide(label).as_ptr()));
            for (field, slot) in [
                (width_field, &WIDTH_FIELD),
                (height_field, &HEIGHT_FIELD),
            ] {
                if !field.0.is_null() {
                    SetWindowTextW(field, PCWSTR::from_raw(to_wide("").as_ptr()));
                    ShowWindow(
                        field,
                        if wants_size {
                            windows::Win32::UI::WindowsAndMessaging::SW_SHOW
                        } else {
                            windows::Win32::UI::WindowsAndMessaging::SW_HIDE
                        },
                    );
                }
                let _ = slot;
            }
            InvalidateRect(Some(card_window), None, true);
            ShowWindow(card_window, windows::Win32::UI::WindowsAndMessaging::SW_SHOW);
            SetFocus(Some(if wants_size {
                width_field
            } else {
                card_window
            }));
        }
    }

    /// Hides the window.
    pub fn hide(&self) {
        unsafe {
            ShowWindow(self.window, windows::Win32::UI::WindowsAndMessaging::SW_HIDE);
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

    /// Tells the card where everything is, and puts the fields in their
    /// boxes. Both read the same numbers, so what is drawn and where the
    /// typing happens can never disagree.
    fn lay_out(&mut self) {
        let scale = scale_factor(self.window);
        let layout = Layout::of(self.width as i32, self.height as i32, scale);
        let panel = |rect: RECT| Panel {
            x: rect.left as f32,
            y: rect.top as f32,
            width: (rect.right - rect.left) as f32,
            height: (rect.bottom - rect.top) as f32,
        };
        self.card.set_title_box(panel(layout.title));
        self.card.set_file_box(panel(layout.file));
        self.card.set_preview_box(panel(layout.preview));
        self.card.set_width_box(panel(layout.width_box));
        self.card.set_height_box(panel(layout.height_box));
        self.card.set_cancel_box(panel(layout.cancel));
        self.card.set_run_box(panel(layout.run));

        let inset = (10.0 * scale) as i32;
        for (field, box_rect) in [
            (self.width_field, layout.width_box),
            (self.height_field, layout.height_box),
        ] {
            unsafe {
                let _ = MoveWindow(
                    field,
                    box_rect.left + inset,
                    box_rect.top + inset,
                    box_rect.right - box_rect.left - inset * 2,
                    box_rect.bottom - box_rect.top - inset * 2,
                    true,
                );
            }
        }
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
    title: RECT,
    file: RECT,
    preview: RECT,
    width_box: RECT,
    height_box: RECT,
    cancel: RECT,
    run: RECT,
}

impl Layout {
    fn of(whole: i32, side: i32, scale: f32) -> Layout {
        let at = |value: i32| (value as f32 * scale) as i32;
        let margin = at(24);

        let mut layout = Layout {
            title: RECT {
                left: margin,
                top: margin,
                right: whole - margin,
                bottom: margin + at(30),
            },
            file: RECT {
                left: margin,
                top: margin + at(34),
                right: whole - margin,
                bottom: margin + at(56),
            },
            preview: RECT::default(),
            width_box: RECT::default(),
            height_box: RECT::default(),
            cancel: RECT::default(),
            run: RECT::default(),
        };

        // The buttons sit at the bottom, and the picture takes the rest.
        let button = at(112);
        let button_height = at(38);
        layout.run = RECT {
            left: whole - margin - button,
            top: side - margin - button_height,
            right: whole - margin,
            bottom: side - margin,
        };
        layout.cancel = RECT {
            left: whole - margin - button * 2 - at(10),
            top: side - margin - button_height,
            right: whole - margin - button - at(10),
            bottom: side - margin,
        };

        let wants_size = SHOWN.with(|shown| shown.borrow().wants_size);
        let fields_height = if wants_size { at(72) } else { 0 };
        let preview_top = layout.file.bottom + at(14);
        let preview_bottom = layout.cancel.top - at(20) - fields_height;
        layout.preview = RECT {
            left: margin,
            top: preview_top,
            right: whole - margin,
            bottom: preview_bottom,
        };

        if wants_size {
            let fields_top = preview_bottom + at(12);
            let box_height = at(40);
            let box_width = (whole - margin * 2 - at(12)) / 2;
            layout.width_box = RECT {
                left: margin,
                top: fields_top,
                right: margin + box_width,
                bottom: fields_top + box_height,
            };
            layout.height_box = RECT {
                left: margin + box_width + at(12),
                top: fields_top,
                right: whole - margin,
                bottom: fields_top + box_height,
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

unsafe extern "system" fn window_procedure(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_PAINT => {
            let mut paint = windows::Win32::Graphics::Gdi::PAINTSTRUCT::default();
            let dc = BeginPaint(window, &mut paint);
            HOST.with(|slot| {
                let mut borrowed = slot.borrow_mut();
                if let Some(host) = borrowed.as_mut() {
                    host.resize();
                    host.draw();
                    let _ = dc;
                }
            });
            let _ = windows::Win32::Graphics::Gdi::EndPaint(window, &paint);
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
            let _ = InvalidateRect(Some(window), None, true);
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
            // A field taking or losing the focus changes nothing the card
            // draws, but the card is drawn again all the same, so nothing
            // is left of an old frame.
            let happened = ((wparam.0 >> 16) & 0xffff) as u32;
            if happened == EN_SETFOCUS || happened == EN_KILLFOCUS {
                let _ = InvalidateRect(Some(window), None, true);
            }
            LRESULT(0)
        }
        WM_CTLCOLOREDIT => {
            // The field is the colour of the window, with no frame of its
            // own: the frame is drawn by the card.
            let context = windows::Win32::Graphics::Gdi::HDC(lparam.0 as *mut _);
            unsafe {
                windows::Win32::Graphics::Gdi::SetTextColor(context, COLORREF(0x0024_2424));
                windows::Win32::Graphics::Gdi::SetBkColor(context, COLORREF(0x00FF_FFFF));
            }
            LRESULT(
                windows::Win32::Graphics::Gdi::GetSysColorBrush(
                    windows::Win32::Graphics::Gdi::COLOR_WINDOW,
                )
                .0 as isize,
            )
        }
        WM_ERASEBKGND => LRESULT(1),
        WM_CLOSE => {
            send(ToolEvent::Cancelled);
            unsafe {
                ShowWindow(window, windows::Win32::UI::WindowsAndMessaging::SW_HIDE);
            }
            LRESULT(0)
        }
        WM_DESTROY => LRESULT(0),
        _ => DefWindowProcW(window, message, wparam, lparam),
    }
}

/// The Enter key in a field runs the tool; Escape closes the window. The
/// keys are taken before the field sees them, so the field never puts a
/// new line where a number belongs.
unsafe extern "system" fn field_procedure(
    field: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _which: usize,
    _given: usize,
) -> LRESULT {
    if message == WM_KEYDOWN && (wparam.0 as u16) == VK_RETURN.0 {
        let parent = GetAncestor(field, GA_ROOT);
        run(parent);
        return LRESULT(0);
    }
    if message == WM_KEYDOWN && (wparam.0 as u16) == VK_ESCAPE.0 {
        let parent = GetAncestor(field, GA_ROOT);
        send(ToolEvent::Cancelled);
        ShowWindow(parent, windows::Win32::UI::WindowsAndMessaging::SW_HIDE);
        return LRESULT(0);
    }
    DefSubclassProc(field, message, wparam, lparam)
}

/// Runs what the window was open for, with what the fields say.
unsafe fn run(window: HWND) {
    let settings = ToolSettings {
        width: number(&field_text(GetDlgItem(Some(window), 50).unwrap_or_default())),
        height: number(&field_text(GetDlgItem(Some(window), 60).unwrap_or_default())),
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
    ShowWindow(window, windows::Win32::UI::WindowsAndMessaging::SW_HIDE);
}

/// What a field says.
unsafe fn field_text(field: HWND) -> String {
    let length = GetWindowTextLengthW(field);
    let mut text = vec![0u16; (length + 1) as usize];
    let _ = GetWindowTextW(field, &mut text);
    String::from_utf16_lossy(&text[..length.max(0) as usize])
}

/// A number in words, if the words say one.
fn number(text: &str) -> Option<u32> {
    text.trim().parse::<u32>().ok().filter(|value| *value > 0)
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

/// The font the system's fields are given, in the face people read.
unsafe fn interface_font(window: HWND) -> windows::Win32::Graphics::Gdi::HFONT {
    let scale = scale_factor(window);
    windows::Win32::Graphics::Gdi::CreateFontW(
        -(14.0 * scale) as i32,
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

/// Words in the wide letters the system reads.
fn to_wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain([0]).collect()
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
        // The picture sits above the fields, the fields above the buttons.
        assert!(layout.preview.bottom <= layout.width_box.top);
        assert!(layout.width_box.bottom <= layout.cancel.top);
        assert_eq!(layout.width_box.top, layout.height_box.top);
    }
}

#[cfg(test)]
mod show_tests {
    use super::*;
    use windows::Win32::UI::WindowsAndMessaging::{
        DispatchMessageW, GetMessageW, MSG, PM_REMOVE, PeekMessageW, TranslateMessage,
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
