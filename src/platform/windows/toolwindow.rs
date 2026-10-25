//! The window a tool opens in.
//!
//! A tool on the wheel does not run the moment it is picked: it opens a
//! window of its own, the way the reference product does, with the file
//! in it, the choices the tool offers, and a way to say no. The window is
//! a real window of the system, with the system's own fields and buttons,
//! so the keyboard, the focus and the reader for the blind are the ones
//! every other window has.
//!
//! What the window is asked comes back as a message: the window never runs
//! anything itself. The program that draws keeps deciding what happens.
//! The messages of this window are pumped by the loop that pumps the
//! wheel's, because it takes every message the thread is sent.

use std::path::PathBuf;
use std::sync::mpsc::Sender;

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    CreateBitmap, DeleteObject, GetDC, GetStockObject, GetSysColorBrush, COLOR_WINDOW,
    DEFAULT_GUI_FONT, HBITMAP, HBRUSH, HFONT, HDC,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, GetDlgItem, GetWindowTextLengthW, GetWindowTextW, MoveWindow,
    RegisterClassW, SendMessageW, SetWindowTextW, ShowWindow, CW_USEDEFAULT, HMENU, IDC_ARROW,
    WNDCLASSW, WM_CLOSE, WM_COMMAND, WM_CTLCOLOREDIT, WM_CTLCOLORSTATIC, WM_DESTROY,
    WM_ERASEBKGND, WM_SETFONT, WS_BORDER, WS_CHILD, WS_EX_CLIENTEDGE, WS_OVERLAPPEDWINDOW,
    WS_TABSTOP, WS_VISIBLE, WINDOW_EX_STYLE, WINDOW_STYLE,
};

use crate::radial::execution::ToolSettings;

/// The styles the system's fields carry, spelled as numbers because the
/// window makes them from parts.
const SS_CENTER: u32 = 0x0001;
const SS_BITMAP: u32 = 0x000E;
const BS_DEFPUSHBUTTON: u32 = 0x0001;
/// What a picture field is told when it is given a picture.
const STM_SETIMAGE: u32 = 0x0172;

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

/// Where the window's procedure sends what the buttons say.
thread_local! {
    static EVENTS: std::cell::RefCell<Option<Sender<ToolEvent>>> =
        const { std::cell::RefCell::new(None) };
}

/// What the window is open for, put where its procedure can find it.
thread_local! {
    static OPEN_FOR: std::cell::RefCell<Option<(String, Vec<PathBuf>)>> =
        const { std::cell::RefCell::new(None) };
}

/// The window one tool is asked in.
pub struct ToolWindow {
    window: HWND,
    title: HWND,
    file: HWND,
    preview: HWND,
    width_label: HWND,
    width_field: HWND,
    height_label: HWND,
    height_field: HWND,
    cancel: HWND,
    run: HWND,
    bitmap: Option<HBITMAP>,
    font: HFONT,
}

// The window belongs to the thread that made it and is used by no other.
unsafe impl Send for ToolWindow {}

/// The name the window class is registered under.
const CLASS_NAME: PCWSTR = w!("OpenTangerineTool");

impl ToolWindow {
    /// Makes the window, hidden until a tool is picked.
    pub fn new(events: Sender<ToolEvent>) -> Result<ToolWindow, String> {
        EVENTS.with(|slot| *slot.borrow_mut() = Some(events));

        let instance = unsafe { GetModuleHandleW(None).map_err(|error| error.to_string())? };
        unsafe {
            let class = WNDCLASSW {
                lpfnWndProc: Some(window_procedure),
                hInstance: instance.into(),
                lpszClassName: CLASS_NAME,
                hCursor: windows::Win32::UI::WindowsAndMessaging::LoadCursorW(None, IDC_ARROW)
                    .unwrap_or_default(),
                hbrBackground: HBRUSH(GetSysColorBrush(COLOR_WINDOW).0),
                ..Default::default()
            };
            RegisterClassW(&class);
        }

        let window = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                CLASS_NAME,
                w!("Open Tangerine"),
                WS_OVERLAPPEDWINDOW,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                460,
                580,
                None,
                None,
                Some(instance.into()),
                None,
            )
        }
        .map_err(|error| error.to_string())?;

        let made = ToolWindow {
            window,
            title: make_child(window, w!("STATIC"), "", SS_CENTER, false, 10),
            file: make_child(window, w!("STATIC"), "", 0, false, 20),
            preview: make_child(window, w!("STATIC"), "", SS_BITMAP, false, 30),
            width_label: make_child(window, w!("STATIC"), "Width", 0, false, 40),
            width_field: make_child(window, w!("EDIT"), "", WS_BORDER.0 | WS_TABSTOP.0, true, 50),
            height_label: make_child(window, w!("STATIC"), "Height", 0, false, 60),
            height_field: make_child(window, w!("EDIT"), "", WS_BORDER.0 | WS_TABSTOP.0, true, 70),
            cancel: make_child(window, w!("BUTTON"), "Cancel", WS_TABSTOP.0, false, 80),
            run: make_child(
                window,
                w!("BUTTON"),
                "Run",
                WS_TABSTOP.0 | BS_DEFPUSHBUTTON,
                false,
                90,
            ),
            bitmap: None,
            font: HFONT(unsafe { GetStockObject(DEFAULT_GUI_FONT) }.0),
        };
        made.apply_font();
        Ok(made)
    }

    /// The font every field and button is given.
    fn apply_font(&self) {
        for field in [
            self.title,
            self.file,
            self.width_label,
            self.width_field,
            self.height_label,
            self.height_field,
            self.cancel,
            self.run,
        ] {
            unsafe {
                SendMessageW(field, WM_SETFONT, Some(WPARAM(self.font.0 as usize)), Some(LPARAM(1)));
            }
        }
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

        unsafe {
            SetWindowTextW(self.window, PCWSTR::from_raw(to_wide(&format!("{label} — Open Tangerine")).as_ptr()));
            SetWindowTextW(self.title, PCWSTR::from_raw(to_wide(detail).as_ptr()));
            SetWindowTextW(self.file, PCWSTR::from_raw(to_wide(&said).as_ptr()));
            SetWindowTextW(self.width_field, PCWSTR::from_raw(to_wide("").as_ptr()));
            SetWindowTextW(self.height_field, PCWSTR::from_raw(to_wide("").as_ptr()));

            // A size is asked only by the tools that have one.
            let wants_size = operation == "tools.resize";
            for (field, shown) in [
                (self.width_label, wants_size),
                (self.width_field, wants_size),
                (self.height_label, wants_size),
                (self.height_field, wants_size),
            ] {
                ShowWindow(
                    field,
                    if shown {
                        windows::Win32::UI::WindowsAndMessaging::SW_SHOW
                    } else {
                        windows::Win32::UI::WindowsAndMessaging::SW_HIDE
                    },
                );
            }

            self.set_preview(preview);
            self.place();
            ShowWindow(self.window, windows::Win32::UI::WindowsAndMessaging::SW_SHOW);
        }
    }

    /// Puts the picture in the window, or empties the place it would be.
    fn set_preview(&mut self, picture: Option<&image::DynamicImage>) {
        unsafe {
            // The field lets go of the old picture before it is taken away.
            SendMessageW(self.preview, STM_SETIMAGE, Some(WPARAM(0)), Some(LPARAM(0)));
            if let Some(old) = self.bitmap.take() {
                let _ = DeleteObject(old.into());
            }
            let bitmap = picture.map(|picture| {
                let small = picture.thumbnail(392, 264);
                let rgba = small.to_rgba8();
                let mut bgra: Vec<u8> = Vec::with_capacity(rgba.len());
                for pixel in rgba.pixels() {
                    bgra.extend_from_slice(&[pixel.0[2], pixel.0[1], pixel.0[0], pixel.0[3]]);
                }
                CreateBitmap(
                    small.width() as i32,
                    small.height() as i32,
                    1,
                    32,
                    Some(bgra.as_ptr() as *const core::ffi::c_void),
                )
            });
            if let Some(bitmap) = bitmap {
                SendMessageW(
                    self.preview,
                    STM_SETIMAGE,
                Some(WPARAM(0)),
                    Some(LPARAM(bitmap.0 as isize)),
                );
                self.bitmap = Some(bitmap);
            }
        }
    }

    /// Puts every part of the window where it belongs.
    ///
    /// The parts are placed by hand, the way the wheel is drawn by hand:
    /// a tool window is a small form, and a form does not need a layout
    /// engine to hold four fields and two buttons.
    fn place(&self) {
        let scale = scale_factor(self.window);
        let at = |value: i32| (value as f32 * scale) as i32;
        unsafe {
            let mut rect = RECT::default();
            let _ = windows::Win32::UI::WindowsAndMessaging::GetClientRect(self.window, &mut rect);
            let (whole, side) = (rect.right, rect.bottom);
            let margin = at(16);

            move_window(self.title, margin, margin, whole - margin * 2, at(28));
            move_window(self.file, margin, margin + at(34), whole - margin * 2, at(22));

            let preview_top = margin + at(66);
            let preview_height = at(280);
            move_window(
                self.preview,
                margin,
                preview_top,
                whole - margin * 2,
                preview_height,
            );

            let fields_top = preview_top + preview_height + at(18);
            move_window(self.width_label, margin, fields_top, at(44), at(22));
            move_window(
                self.width_field,
                margin + at(50),
                fields_top - at(2),
                at(96),
                at(26),
            );
            move_window(
                self.height_label,
                margin + at(176),
                fields_top,
                at(48),
                at(22),
            );
            move_window(
                self.height_field,
                margin + at(230),
                fields_top - at(2),
                at(96),
                at(26),
            );

            let button = at(100);
            let buttons_top = side - margin - at(34);
            move_window(
                self.cancel,
                whole - margin - button * 2 - at(10),
                buttons_top,
                button,
                at(34),
            );
            move_window(self.run, whole - margin - button, buttons_top, button, at(34));
        }
    }

    /// Hides the window.
    pub fn hide(&self) {
        unsafe {
            ShowWindow(self.window, windows::Win32::UI::WindowsAndMessaging::SW_HIDE);
        }
    }
}

impl Drop for ToolWindow {
    fn drop(&mut self) {
        if let Some(bitmap) = self.bitmap.take() {
            unsafe {
                let _ = DeleteObject(bitmap.into());
            }
        }
    }
}

unsafe extern "system" fn window_procedure(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_COMMAND => {
            // The low word says which field or button, by the number it was
            // given; the high word says what happened to it.
            let which = (wparam.0 & 0xffff) as i32;
            let happened = (wparam.0 >> 16) & 0xffff;
            if happened == 0 && which == 80 {
                // Cancel: nothing runs, and the person is not told anything,
                // because they were the one who said no.
                let _ = send(ToolEvent::Cancelled);
                ShowWindow(window, windows::Win32::UI::WindowsAndMessaging::SW_HIDE);
            } else if happened == 0 && which == 90 {
                // Run: what the fields say, for what the window was open.
                let settings = ToolSettings {
                    width: number(&field_text(GetDlgItem(Some(window), 50).unwrap_or_default())),
                    height: number(&field_text(GetDlgItem(Some(window), 60).unwrap_or_default())),
                };
                OPEN_FOR.with(|slot| {
                    let mut open_for = slot.borrow_mut();
                    if let Some((operation, inputs)) = open_for.take() {
                        let _ = send(ToolEvent::Run {
                            operation,
                            inputs,
                            settings,
                        });
                    }
                });
                ShowWindow(window, windows::Win32::UI::WindowsAndMessaging::SW_HIDE);
            }
            LRESULT(0)
        }
        WM_CLOSE => {
            let _ = send(ToolEvent::Cancelled);
            ShowWindow(window, windows::Win32::UI::WindowsAndMessaging::SW_HIDE);
            LRESULT(0)
        }
        WM_DESTROY => LRESULT(0),
        WM_CTLCOLOREDIT | WM_CTLCOLORSTATIC => {
            // The writing is black, on the colour of the window, the way
            // writing is.
            let context = HDC(lparam.0 as *mut _);
            unsafe {
                windows::Win32::Graphics::Gdi::SetTextColor(
                    context,
                    windows::Win32::Foundation::COLORREF(0),
                );
                windows::Win32::Graphics::Gdi::SetBkColor(
                    context,
                    windows::Win32::Foundation::COLORREF(0x00FF_FF_FF),
                );
            }
            LRESULT(GetSysColorBrush(COLOR_WINDOW).0 as isize)
        }
        WM_ERASEBKGND => {
            let context = HDC(wparam.0 as *mut _);
            let mut rect = RECT::default();
            unsafe {
                let _ = windows::Win32::UI::WindowsAndMessaging::GetClientRect(window, &mut rect);
                windows::Win32::Graphics::Gdi::FillRect(
                    context,
                    &rect,
                    GetSysColorBrush(COLOR_WINDOW),
                );
            }
            LRESULT(1)
        }
        _ => DefWindowProcW(window, message, wparam, lparam),
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

/// A child window of one kind, with a number of its own.
fn make_child(
    parent: HWND,
    kind: PCWSTR,
    text: &str,
    style: u32,
    sunken: bool,
    number: i32,
) -> HWND {
    let wide = to_wide(text);
    unsafe {
        CreateWindowExW(
            if sunken { WS_EX_CLIENTEDGE } else { WINDOW_EX_STYLE::default() },
            kind,
            PCWSTR::from_raw(wide.as_ptr()),
            WS_CHILD | WS_VISIBLE | WINDOW_STYLE(style),
            0,
            0,
            0,
            0,
            Some(parent),
            Some(HMENU(number as *mut _)),
            None,
            None,
        )
    }
    .unwrap_or_default()
}

/// Puts a child where it now belongs.
unsafe fn move_window(child: HWND, x: i32, y: i32, width: i32, height: i32) {
    let _ = MoveWindow(child, x, y, width, height, true);
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
        let context = GetDC(Some(window));
        let dots = windows::Win32::Graphics::Gdi::GetDeviceCaps(
            Some(context),
            windows::Win32::Graphics::Gdi::LOGPIXELSY,
        );
        let _ = windows::Win32::Graphics::Gdi::ReleaseDC(Some(window), context);
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
    fn a_number_is_read_from_words() {
        assert_eq!(number("800"), Some(800));
        assert_eq!(number(" 800 "), Some(800));
        assert_eq!(number(""), None);
        assert_eq!(number("wide"), None);
        assert_eq!(number("0"), None);
    }

    #[test]
    fn words_become_wide_and_back() {
        let wide = to_wide("Hello");
        assert_eq!(wide.last(), Some(&0));
        assert_eq!(String::from_utf16_lossy(&wide[..wide.len() - 1]), "Hello");
    }
}
