//! The window PDF pages are inspected, reordered, rotated, duplicated and removed in.
//!
//! Visual and behavioral twin of the reference's Organize PDF tool view:
//! - Left pane: an interactive 3-column scrollable grid of page cards, each showing
//!   page thumbnail (rotated to current angle), 1-based index, optional source label,
//!   and selection highlight (3px accent border).
//! - Right pane: three sidebar cards:
//!   1. Selection: count readout, All / None buttons.
//!   2. Pages: Rotate left (-90°), Rotate right (+90°), Duplicate, Remove, Reverse order.
//!   3. Summary: total pages, multi-document combination readout.
//! - Footer: helper shortcuts on the left, Cancel and Save copy on the right.
//!
//! Layout-twin pattern: Slint card only draws; the Win32 window host measures
//! all boxes in `Layout::of(client_width, client_height, count)` and hands them to
//! the card via `Panel` properties, while hit-testing and keyboard walk match
//! the same coordinate math.

use std::cell::{Cell, RefCell};
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc::Sender;

use slint::platform::software_renderer::MinimalSoftwareWindow;
use slint::{ModelRc, PhysicalSize, SharedPixelBuffer, VecModel};
use windows::core::w;
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreateSolidBrush, EndPaint, InvalidateRect, PAINTSTRUCT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, VK_BACK, VK_CONTROL, VK_DELETE, VK_DOWN, VK_ESCAPE, VK_LEFT, VK_RETURN,
    VK_RIGHT, VK_SHIFT, VK_SPACE, VK_TAB, VK_UP,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, GetClientRect, LoadCursorW, PostQuitMessage, RegisterClassW,
    SetForegroundWindow, SetLayeredWindowAttributes, SetWindowPos, ShowWindow, CW_USEDEFAULT,
    IDC_ARROW, LWA_ALPHA, SWP_NOACTIVATE, SWP_NOZORDER, SW_HIDE, SW_SHOW,
    WM_DESTROY, WM_DPICHANGED, WM_ERASEBKGND, WM_KEYDOWN, WM_LBUTTONDOWN, WM_PAINT, WNDCLASSW, WS_EX_LAYERED,
};

use crate::platform::windows::toolwindow::ToolEvent;
use crate::platform::windows::windowhost::{fixed_style, scale_factor};
use crate::radial::execution::ToolSettings;
use crate::ui::wheel::{
    window_for_next_component, Bgra8, OrganizePdfCard, PageCardData, Panel,
};

pub const CARD_W: i32 = 820;
pub const CARD_H: i32 = 560;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Box4 {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Box4 {
    pub fn contains(&self, px: f32, py: f32) -> bool {
        px >= self.x && px <= self.x + self.w && py >= self.y && py <= self.y + self.h
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    pub client_width: i32,
    pub client_height: i32,
    pub header: Panel,
    pub detail: Panel,
    pub hairline_top: Panel,
    pub hairline_split: Panel,
    pub pages_box: Panel,
    pub pages: Vec<Box4>,
    pub sidebar_selection_box: Panel,
    pub sidebar_selection_title: Panel,
    pub selection_status: Panel,
    pub btn_all: Panel,
    pub btn_none: Panel,
    pub sidebar_pages_box: Panel,
    pub sidebar_pages_title: Panel,
    pub btn_rot_left: Panel,
    pub btn_rot_right: Panel,
    pub btn_duplicate: Panel,
    pub btn_remove: Panel,
    pub btn_reverse: Panel,
    pub sidebar_summary_box: Panel,
    pub sidebar_summary_title: Panel,
    pub summary_count: Panel,
    pub summary_desc: Panel,
    pub hairline_bottom: Panel,
    pub helper: Panel,
    pub cancel: Panel,
    pub run: Panel,
}

impl Layout {
    pub const WIDTH_UNITS: f32 = 820.0;

    pub fn of(client_width: i32, client_height: i32, page_count: usize) -> Self {
        let scale = (client_width as f32 / Self::WIDTH_UNITS).max(0.5);
        let w = client_width as f32;
        let h = client_height as f32;

        let header = Panel {
            x: 20.0 * scale,
            y: 16.0 * scale,
            width: 480.0 * scale,
            height: 22.0 * scale,
        };
        let detail = Panel {
            x: 20.0 * scale,
            y: 40.0 * scale,
            width: 480.0 * scale,
            height: 18.0 * scale,
        };
        let hairline_top = Panel {
            x: 0.0,
            y: 68.0 * scale,
            width: w,
            height: 1.0,
        };
        let hairline_split = Panel {
            x: 530.0 * scale,
            y: 69.0 * scale,
            width: 1.0,
            height: (h - 69.0 * scale - 68.0 * scale).max(0.0),
        };

        // Grid of pages
        let grid_x = 24.0 * scale;
        let grid_y = 80.0 * scale;
        let cell_w = 148.0 * scale;
        let cell_h = 186.0 * scale;
        let gap_x = 16.0 * scale;
        let gap_y = 16.0 * scale;
        let cols = 3;

        let mut pages = Vec::with_capacity(page_count);
        for idx in 0..page_count {
            let row = (idx / cols) as f32;
            let col = (idx % cols) as f32;
            let px = grid_x + col * (cell_w + gap_x);
            let py = grid_y + row * (cell_h + gap_y);
            pages.push(Box4 {
                x: px,
                y: py,
                w: cell_w,
                h: cell_h,
            });
        }

        let pages_box = Panel {
            x: 20.0 * scale,
            y: 76.0 * scale,
            width: 494.0 * scale,
            height: 406.0 * scale,
        };

        // Sidebar
        let sb_x = 546.0 * scale;
        let sb_w = 254.0 * scale;

        // Section 1: Selection
        let sidebar_selection_box = Panel {
            x: sb_x,
            y: 80.0 * scale,
            width: sb_w,
            height: 96.0 * scale,
        };
        let sidebar_selection_title = Panel {
            x: sb_x + 14.0 * scale,
            y: 92.0 * scale,
            width: sb_w - 28.0 * scale,
            height: 16.0 * scale,
        };
        let selection_status = Panel {
            x: sb_x + 14.0 * scale,
            y: 112.0 * scale,
            width: sb_w - 28.0 * scale,
            height: 18.0 * scale,
        };
        let btn_all = Panel {
            x: sb_x + 14.0 * scale,
            y: 138.0 * scale,
            width: 108.0 * scale,
            height: 26.0 * scale,
        };
        let btn_none = Panel {
            x: sb_x + 132.0 * scale,
            y: 138.0 * scale,
            width: 108.0 * scale,
            height: 26.0 * scale,
        };

        // Section 2: Pages
        let sidebar_pages_box = Panel {
            x: sb_x,
            y: 188.0 * scale,
            width: sb_w,
            height: 186.0 * scale,
        };
        let sidebar_pages_title = Panel {
            x: sb_x + 14.0 * scale,
            y: 198.0 * scale,
            width: sb_w - 28.0 * scale,
            height: 16.0 * scale,
        };
        let btn_rot_left = Panel {
            x: sb_x + 14.0 * scale,
            y: 220.0 * scale,
            width: 108.0 * scale,
            height: 26.0 * scale,
        };
        let btn_rot_right = Panel {
            x: sb_x + 132.0 * scale,
            y: 220.0 * scale,
            width: 108.0 * scale,
            height: 26.0 * scale,
        };
        let btn_duplicate = Panel {
            x: sb_x + 14.0 * scale,
            y: 254.0 * scale,
            width: sb_w - 28.0 * scale,
            height: 26.0 * scale,
        };
        let btn_remove = Panel {
            x: sb_x + 14.0 * scale,
            y: 288.0 * scale,
            width: sb_w - 28.0 * scale,
            height: 26.0 * scale,
        };
        let btn_reverse = Panel {
            x: sb_x + 14.0 * scale,
            y: 322.0 * scale,
            width: sb_w - 28.0 * scale,
            height: 26.0 * scale,
        };

        // Section 3: Summary
        let sidebar_summary_box = Panel {
            x: sb_x,
            y: 386.0 * scale,
            width: sb_w,
            height: 96.0 * scale,
        };
        let sidebar_summary_title = Panel {
            x: sb_x + 14.0 * scale,
            y: 398.0 * scale,
            width: sb_w - 28.0 * scale,
            height: 16.0 * scale,
        };
        let summary_count = Panel {
            x: sb_x + 14.0 * scale,
            y: 418.0 * scale,
            width: sb_w - 28.0 * scale,
            height: 20.0 * scale,
        };
        let summary_desc = Panel {
            x: sb_x + 14.0 * scale,
            y: 442.0 * scale,
            width: sb_w - 28.0 * scale,
            height: 32.0 * scale,
        };

        // Footer
        let hairline_bottom = Panel {
            x: 0.0,
            y: h - 68.0 * scale,
            width: w,
            height: 1.0,
        };
        let helper = Panel {
            x: 20.0 * scale,
            y: h - 52.0 * scale,
            width: 480.0 * scale,
            height: 32.0 * scale,
        };
        let cancel = Panel {
            x: w - 210.0 * scale,
            y: h - 52.0 * scale,
            width: 88.0 * scale,
            height: 32.0 * scale,
        };
        let run = Panel {
            x: w - 112.0 * scale,
            y: h - 52.0 * scale,
            width: 92.0 * scale,
            height: 32.0 * scale,
        };

        Self {
            client_width,
            client_height,
            header,
            detail,
            hairline_top,
            hairline_split,
            pages_box,
            pages,
            sidebar_selection_box,
            sidebar_selection_title,
            selection_status,
            btn_all,
            btn_none,
            sidebar_pages_box,
            sidebar_pages_title,
            btn_rot_left,
            btn_rot_right,
            btn_duplicate,
            btn_remove,
            btn_reverse,
            sidebar_summary_box,
            sidebar_summary_title,
            summary_count,
            summary_desc,
            hairline_bottom,
            helper,
            cancel,
            run,
        }
    }
}

#[derive(Clone)]
pub struct PageState {
    pub source_doc: usize,
    pub source_page: usize,
    pub source_label: String,
    pub rotation_degrees: i32,
    pub base_thumb: Option<image::RgbaImage>,
}

#[derive(Default)]
struct State {
    files: Vec<PathBuf>,
    primary_name: String,
    pages: Vec<PageState>,
    selected: BTreeSet<usize>,
    focused_page: Option<usize>,
    button_focus: i32, // 0=none, 1=cancel, 2=save
}

thread_local! {
    static HOST: RefCell<Option<Host>> = const { RefCell::new(None) };
    static STATE: RefCell<State> = RefCell::new(State::default());
    static WINDOW: Cell<usize> = const { Cell::new(0) };
}

fn window_handle() -> HWND {
    HWND(WINDOW.with(|slot| slot.get()) as *mut _)
}

struct Host {
    window: HWND,
    card: OrganizePdfCard,
    software: Rc<MinimalSoftwareWindow>,
    buffer: Vec<Bgra8>,
    width: u32,
    height: u32,
}

impl Host {
    fn resize(&mut self) {
        let mut rect = RECT::default();
        let _ = unsafe { GetClientRect(self.window, &mut rect) };
        let w = (rect.right - rect.left).max(1) as u32;
        let h = (rect.bottom - rect.top).max(1) as u32;
        if w != self.width || h != self.height || self.buffer.len() != (w * h) as usize {
            self.width = w;
            self.height = h;
            self.buffer = vec![Bgra8::default(); (w * h) as usize];
            self.software.set_size(PhysicalSize::new(w, h));
            self.software.request_redraw();
        }
    }

    fn draw(&mut self) {
        if self.width == 0 || self.height == 0 || self.buffer.len() != (self.width * self.height) as usize {
            self.resize();
        }
        let stride = self.width as usize;
        let mut buffer = std::mem::take(&mut self.buffer);
        if buffer.len() != (self.width * self.height) as usize {
            buffer = vec![Bgra8::default(); (self.width * self.height) as usize];
        }
        self.software.draw_if_needed(|renderer| {
            renderer.render(&mut buffer, stride);
        });
        self.buffer = buffer;
        unsafe { self.put_on_window() };
    }

    unsafe fn put_on_window(&self) {
        unsafe {
            let dc = windows::Win32::Graphics::Gdi::GetDC(Some(self.window));
            let mut info = windows::Win32::Graphics::Gdi::BITMAPINFO::default();
            info.bmiHeader.biSize =
                std::mem::size_of::<windows::Win32::Graphics::Gdi::BITMAPINFOHEADER>() as u32;
            info.bmiHeader.biWidth = self.width as i32;
            info.bmiHeader.biHeight = -(self.height as i32);
            info.bmiHeader.biPlanes = 1;
            info.bmiHeader.biBitCount = 32;
            info.bmiHeader.biCompression = windows::Win32::Graphics::Gdi::BI_RGB.0 as u32;
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
                self.buffer.as_ptr() as *const _,
                &info,
                windows::Win32::Graphics::Gdi::DIB_RGB_COLORS,
            );
            windows::Win32::Graphics::Gdi::ReleaseDC(Some(self.window), dc);
        }
    }
}

pub struct OrganizePdfWindow {
    pub window: HWND,
    #[allow(dead_code)]
    pub sender: Sender<ToolEvent>,
}

fn outer_size(window: HWND, client_w: i32, client_h: i32) -> (i32, i32) {
    let mut rect = RECT {
        left: 0,
        top: 0,
        right: client_w,
        bottom: client_h,
    };
    unsafe {
        let _ = windows::Win32::UI::HiDpi::AdjustWindowRectExForDpi(
            &mut rect,
            fixed_style(),
            false,
            windows::Win32::UI::WindowsAndMessaging::WINDOW_EX_STYLE::default(),
            (scale_factor(window) * 96.0).round() as u32,
        );
    }
    (rect.right - rect.left, rect.bottom - rect.top)
}

impl OrganizePdfWindow {
    pub fn new(sender: Sender<ToolEvent>) -> Result<Self, String> {
        let class_name = w!("OpenTangerineOrganizePdfCard");
        let instance = unsafe { GetModuleHandleW(None).map_err(|e| e.to_string())? };
        let wc = WNDCLASSW {
            lpfnWndProc: Some(organize_card_proc),
            hInstance: instance.into(),
            lpszClassName: class_name,
            hCursor: unsafe { LoadCursorW(None, IDC_ARROW) }.unwrap_or_default(),
            hbrBackground: unsafe { CreateSolidBrush(COLORREF(0x00FF_FFFF)) },
            ..Default::default()
        };
        unsafe {
            let _ = RegisterClassW(&wc);
        }

        let software = window_for_next_component();
        let card = OrganizePdfCard::new().map_err(|e| e.to_string())?;

        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_LAYERED,
                class_name,
                w!("Organize Pages"),
                fixed_style(),
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                CARD_W,
                CARD_H,
                None,
                None,
                Some(instance.into()),
                None,
            )
            .map_err(|e| e.to_string())?
        };

        unsafe {
            use windows::Win32::Graphics::Dwm::{
                DwmSetWindowAttribute, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND,
            };
            let preference = DWMWCP_ROUND;
            let _ = DwmSetWindowAttribute(
                hwnd,
                DWMWA_WINDOW_CORNER_PREFERENCE,
                &preference as *const _ as *const _,
                std::mem::size_of_val(&preference) as u32,
            );
            let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), 255, LWA_ALPHA);
        }

        WINDOW.with(|slot| slot.set(hwnd.0 as usize));

        let snd_run = sender.clone();
        card.on_run_chosen(move || {
            execute_save(&snd_run);
        });

        let snd_cancel = sender.clone();
        card.on_cancel_chosen(move || {
            let _ = snd_cancel.send(ToolEvent::Cancelled);
            unsafe {
                let _ = ShowWindow(hwnd, SW_HIDE);
            };
        });

        card.on_page_clicked(|idx| {
            let multi = unsafe {
                GetKeyState(VK_SHIFT.0 as i32) < 0 || GetKeyState(VK_CONTROL.0 as i32) < 0
            };
            toggle_select(idx as usize, multi);
        });

        card.on_select_all_chosen(|| {
            select_all();
        });

        card.on_select_none_chosen(|| {
            select_none();
        });

        card.on_rotate_left_chosen(|| {
            rotate_left();
        });

        card.on_rotate_right_chosen(|| {
            rotate_right();
        });

        card.on_duplicate_chosen(|| {
            duplicate();
        });

        card.on_remove_chosen(|| {
            remove_selected();
        });

        card.on_reverse_order_chosen(|| {
            reverse_order();
        });

        software.set_size(PhysicalSize::new(CARD_W as u32, CARD_H as u32));

        HOST.with(|slot| {
            *slot.borrow_mut() = Some(Host {
                window: hwnd,
                card,
                software,
                buffer: vec![Bgra8::default(); (CARD_W * CARD_H) as usize],
                width: CARD_W as u32,
                height: CARD_H as u32,
            });
        });

        Ok(Self {
            window: hwnd,
            sender,
        })
    }

    pub fn hide(&self) {
        unsafe {
            let _ = ShowWindow(self.window, SW_HIDE);
        };
    }

    pub fn show_for(
        &self,
        _operation: &str,
        _title: &str,
        _detail: &str,
        files: &[PathBuf],
    ) {
        if files.is_empty() {
            return;
        }

        let first_name = files[0]
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("document.pdf")
            .to_string();

        let mut pages = Vec::new();
        for (doc_idx, file) in files.iter().enumerate() {
            let doc_name = file
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("doc")
                .to_string();

            // Try rendering pages with PDFium
            match crate::media::pdf::render_pages(file, 72, crate::media::pdf::Encoding::Png) {
                Ok(rendered) => {
                    for (page_idx, png_bytes) in rendered.into_iter().enumerate() {
                        let rgba = image::load_from_memory(&png_bytes)
                            .ok()
                            .map(|img| img.to_rgba8());
                        pages.push(PageState {
                            source_doc: doc_idx,
                            source_page: page_idx,
                            source_label: doc_name.clone(),
                            rotation_degrees: 0,
                            base_thumb: rgba,
                        });
                    }
                }
                Err(_) => {
                    // Fallback to page count or at least 1 page placeholder
                    let count = crate::media::pdf::page_count(file).unwrap_or(1);
                    for page_idx in 0..count {
                        pages.push(PageState {
                            source_doc: doc_idx,
                            source_page: page_idx,
                            source_label: doc_name.clone(),
                            rotation_degrees: 0,
                            base_thumb: None,
                        });
                    }
                }
            }
        }

        STATE.with(|s| {
            let mut st = s.borrow_mut();
            st.files = files.to_vec();
            st.primary_name = first_name;
            st.pages = pages;
            st.selected.clear();
            st.focused_page = if st.pages.is_empty() { None } else { Some(0) };
            st.button_focus = 0;
        });

        let hwnd = self.window;
        let (outer_w, outer_h) = outer_size(hwnd, CARD_W, CARD_H);
        let (cx, cy) = center_on_primary(outer_w, outer_h);

        unsafe {
            let _ = SetWindowPos(
                hwnd,
                None,
                cx,
                cy,
                outer_w,
                outer_h,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }

        refresh_card();

        unsafe {
            let _ = ShowWindow(hwnd, SW_SHOW);
            let _ = SetForegroundWindow(hwnd);
        }
    }
}

fn center_on_primary(width: i32, height: i32) -> (i32, i32) {
    let screen_w = unsafe { windows::Win32::UI::WindowsAndMessaging::GetSystemMetrics(windows::Win32::UI::WindowsAndMessaging::SM_CXSCREEN) };
    let screen_h = unsafe { windows::Win32::UI::WindowsAndMessaging::GetSystemMetrics(windows::Win32::UI::WindowsAndMessaging::SM_CYSCREEN) };
    let x = (screen_w - width) / 2;
    let y = (screen_h - height) / 2;
    (x.max(0), y.max(0))
}

fn select_all() {
    STATE.with(|s| {
        let mut st = s.borrow_mut();
        st.selected = (0..st.pages.len()).collect();
    });
    refresh_card();
}

fn select_none() {
    STATE.with(|s| {
        let mut st = s.borrow_mut();
        st.selected.clear();
    });
    refresh_card();
}

fn toggle_select(idx: usize, multi: bool) {
    STATE.with(|s| {
        let mut st = s.borrow_mut();
        if idx >= st.pages.len() {
            return;
        }
        st.focused_page = Some(idx);
        st.button_focus = 0;
        if multi {
            if st.selected.contains(&idx) {
                st.selected.remove(&idx);
            } else {
                st.selected.insert(idx);
            }
        } else {
            if st.selected.len() == 1 && st.selected.contains(&idx) {
                st.selected.clear();
            } else {
                st.selected.clear();
                st.selected.insert(idx);
            }
        }
    });
    refresh_card();
}

fn rotate_left() {
    STATE.with(|s| {
        let mut st = s.borrow_mut();
        let targets: Vec<usize> = if st.selected.is_empty() {
            (0..st.pages.len()).collect()
        } else {
            st.selected.iter().copied().collect()
        };
        for idx in targets {
            if let Some(page) = st.pages.get_mut(idx) {
                page.rotation_degrees = ((page.rotation_degrees - 90) % 360 + 360) % 360;
            }
        }
    });
    refresh_card();
}

fn rotate_right() {
    STATE.with(|s| {
        let mut st = s.borrow_mut();
        let targets: Vec<usize> = if st.selected.is_empty() {
            (0..st.pages.len()).collect()
        } else {
            st.selected.iter().copied().collect()
        };
        for idx in targets {
            if let Some(page) = st.pages.get_mut(idx) {
                page.rotation_degrees = ((page.rotation_degrees + 90) % 360 + 360) % 360;
            }
        }
    });
    refresh_card();
}

fn duplicate() {
    STATE.with(|s| {
        let mut st = s.borrow_mut();
        if st.selected.is_empty() {
            return;
        }
        let mut new_pages = Vec::with_capacity(st.pages.len() + st.selected.len());
        let mut new_selected = BTreeSet::new();
        for (idx, page) in st.pages.iter().enumerate() {
            let was_sel = st.selected.contains(&idx);
            new_pages.push(page.clone());
            if was_sel {
                let dup_idx = new_pages.len();
                new_pages.push(page.clone());
                new_selected.insert(dup_idx);
            }
        }
        st.pages = new_pages;
        st.selected = new_selected;
    });
    refresh_card();
}

fn remove_selected() {
    STATE.with(|s| {
        let mut st = s.borrow_mut();
        if st.selected.is_empty() || st.selected.len() >= st.pages.len() {
            return;
        }
        let mut new_pages = Vec::with_capacity(st.pages.len() - st.selected.len());
        for (idx, page) in st.pages.iter().enumerate() {
            if !st.selected.contains(&idx) {
                new_pages.push(page.clone());
            }
        }
        st.pages = new_pages;
        st.selected.clear();
        if let Some(focused) = st.focused_page {
            if focused >= st.pages.len() {
                st.focused_page = if st.pages.is_empty() {
                    None
                } else {
                    Some(st.pages.len() - 1)
                };
            }
        }
    });
    refresh_card();
}

fn reverse_order() {
    STATE.with(|s| {
        let mut st = s.borrow_mut();
        let len = st.pages.len();
        if len <= 1 {
            return;
        }
        st.pages.reverse();
        let new_sel: BTreeSet<usize> = st
            .selected
            .iter()
            .map(|&idx| len - 1 - idx)
            .collect();
        st.selected = new_sel;
        if let Some(f) = st.focused_page {
            st.focused_page = Some(len - 1 - f);
        }
    });
    refresh_card();
}

fn execute_save(sender: &Sender<ToolEvent>) {
    let (files, page_specs) = STATE.with(|s| {
        let st = s.borrow();
        let specs: Vec<(usize, usize, i32)> = st
            .pages
            .iter()
            .map(|p| (p.source_doc, p.source_page, p.rotation_degrees))
            .collect();
        (st.files.clone(), specs)
    });

    if page_specs.is_empty() {
        return;
    }

    let _ = sender.send(ToolEvent::Run {
        operation: "tools.organize_pdf".to_string(),
        inputs: files,
        settings: ToolSettings {
            pdf_pages: page_specs,
            ..Default::default()
        },
    });

    let hwnd = window_handle();
    unsafe {
        let _ = ShowWindow(hwnd, SW_HIDE);
    };
}

pub fn refresh_card() {
    let hwnd = window_handle();
    let mut rect = RECT::default();
    let _ = unsafe { GetClientRect(hwnd, &mut rect) };
    let client_w = if rect.right > 0 { rect.right } else { CARD_W };
    let client_h = if rect.bottom > 0 { rect.bottom } else { CARD_H };

    let (
        files_count,
        primary_name,
        page_count,
        selected_count,
        pages_data,
        button_focus,
        can_dup,
        can_rem,
    ) = STATE.with(|s| {
        let st = s.borrow();
        let count = st.pages.len();
        let sel_count = st.selected.len();
        let can_dup = sel_count > 0;
        let can_rem = sel_count > 0 && sel_count < count;

        let layout = Layout::of(client_w, client_h, count);

        let mut data = Vec::with_capacity(count);
        for (idx, page) in st.pages.iter().enumerate() {
            let bbox = layout.pages.get(idx).copied().unwrap_or(Box4 {
                x: 0.0,
                y: 0.0,
                w: 148.0,
                h: 186.0,
            });

            let thumb = if let Some(base) = &page.base_thumb {
                let rot = ((page.rotation_degrees % 360) + 360) % 360;
                let rotated = match rot {
                    90 => image::imageops::rotate90(base),
                    180 => image::imageops::rotate180(base),
                    270 => image::imageops::rotate270(base),
                    _ => base.clone(),
                };
                let buf = SharedPixelBuffer::clone_from_slice(
                    rotated.as_raw(),
                    rotated.width(),
                    rotated.height(),
                );
                slint::Image::from_rgba8(buf)
            } else {
                slint::Image::default()
            };

            data.push(PageCardData {
                index: idx as i32,
                source_doc: page.source_doc as i32,
                source_page: page.source_page as i32,
                display_index: (idx + 1).to_string().into(),
                source_label: page.source_label.clone().into(),
                has_source_label: st.files.len() > 1,
                thumb,
                has_thumb: page.base_thumb.is_some(),
                rotation: page.rotation_degrees,
                selected: st.selected.contains(&idx),
                x: bbox.x,
                y: bbox.y,
                width: bbox.w,
                height: bbox.h,
            });
        }

        (
            st.files.len(),
            st.primary_name.clone(),
            count,
            sel_count,
            data,
            st.button_focus,
            can_dup,
            can_rem,
        )
    });

    let layout = Layout::of(client_w, client_h, page_count);

    HOST.with(|slot| {
        if let Some(host) = slot.borrow_mut().as_mut() {
            let card = &host.card;
            card.set_file_name(primary_name.into());
            let detail = format!("{page_count} page{}", if page_count == 1 { "" } else { "s" });
            card.set_detail(detail.into());

            // Geometry panels
            card.set_header_box(layout.header);
            card.set_detail_box(layout.detail);
            card.set_hairline_top(layout.hairline_top);
            card.set_hairline_split(layout.hairline_split);
            card.set_pages_box(layout.pages_box);

            // Selection
            card.set_sidebar_selection_box(layout.sidebar_selection_box);
            card.set_sidebar_selection_title_box(layout.sidebar_selection_title);
            card.set_selection_status_box(layout.selection_status);
            let sel_text = if selected_count == 0 {
                "Click pages to select. Drag to reorder.".to_string()
            } else {
                format!("{selected_count} selected")
            };
            card.set_selection_status_text(sel_text.into());
            card.set_btn_all_box(layout.btn_all);
            card.set_btn_none_box(layout.btn_none);

            // Pages
            card.set_sidebar_pages_box(layout.sidebar_pages_box);
            card.set_sidebar_pages_title_box(layout.sidebar_pages_title);
            card.set_btn_rot_left_box(layout.btn_rot_left);
            card.set_btn_rot_right_box(layout.btn_rot_right);
            card.set_btn_duplicate_box(layout.btn_duplicate);
            card.set_btn_remove_box(layout.btn_remove);
            card.set_btn_reverse_box(layout.btn_reverse);
            card.set_can_duplicate(can_dup);
            card.set_can_remove(can_rem);

            // Summary
            card.set_sidebar_summary_box(layout.sidebar_summary_box);
            card.set_sidebar_summary_title_box(layout.sidebar_summary_title);
            card.set_summary_count_box(layout.summary_count);
            let count_text = format!("{page_count} page{}", if page_count == 1 { "" } else { "s" });
            card.set_summary_count_text(count_text.into());
            card.set_summary_desc_box(layout.summary_desc);
            card.set_has_summary_desc(files_count > 1);
            if files_count > 1 {
                card.set_summary_desc_text(
                    format!("Pages from {files_count} documents are combined into one.").into(),
                );
            }

            // Footer
            card.set_hairline_bottom(layout.hairline_bottom);
            card.set_helper_box(layout.helper);
            card.set_cancel_box(layout.cancel);
            card.set_run_box(layout.run);
            card.set_save_enabled(page_count > 0);
            card.set_button_focus(button_focus);

            // Pages items model
            let model = Rc::new(VecModel::from(pages_data));
            card.set_pages(ModelRc::from(model));
        }
    });

    unsafe {
        let _ = InvalidateRect(Some(hwnd), None, false);
    };
}

unsafe extern "system" fn organize_card_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_DESTROY => {
            unsafe { PostQuitMessage(0) };
            LRESULT(0)
        }
        WM_ERASEBKGND => LRESULT(1),
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            let _ = unsafe { BeginPaint(hwnd, &mut ps) };
            HOST.with(|slot| {
                if let Some(host) = slot.borrow_mut().as_mut() {
                    host.draw();
                }
            });
            unsafe {
                let _ = EndPaint(hwnd, &ps);
            };
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            let x = (lparam.0 & 0xffff) as i16 as f32;
            let y = ((lparam.0 >> 16) & 0xffff) as i16 as f32;
            let mut rect = RECT::default();
            let _ = unsafe { GetClientRect(hwnd, &mut rect) };
            let count = STATE.with(|s| s.borrow().pages.len());
            let layout = Layout::of(rect.right, rect.bottom, count);

            // Hit test page cards
            for (idx, bbox) in layout.pages.iter().enumerate() {
                if bbox.contains(x, y) {
                    let multi = unsafe {
                        GetKeyState(VK_SHIFT.0 as i32) < 0 || GetKeyState(VK_CONTROL.0 as i32) < 0
                    };
                    toggle_select(idx, multi);
                    return LRESULT(0);
                }
            }

            // Hit test sidebar buttons
            if in_panel(x, y, &layout.btn_all) {
                select_all();
                return LRESULT(0);
            }
            if in_panel(x, y, &layout.btn_none) {
                select_none();
                return LRESULT(0);
            }
            if in_panel(x, y, &layout.btn_rot_left) {
                rotate_left();
                return LRESULT(0);
            }
            if in_panel(x, y, &layout.btn_rot_right) {
                rotate_right();
                return LRESULT(0);
            }
            if in_panel(x, y, &layout.btn_duplicate) {
                duplicate();
                return LRESULT(0);
            }
            if in_panel(x, y, &layout.btn_remove) {
                remove_selected();
                return LRESULT(0);
            }
            if in_panel(x, y, &layout.btn_reverse) {
                reverse_order();
                return LRESULT(0);
            }

            // Hit test footer buttons
            if in_panel(x, y, &layout.cancel) {
                HOST.with(|slot| {
                    if let Some(host) = slot.borrow().as_ref() {
                        host.card.invoke_cancel_chosen();
                    }
                });
                return LRESULT(0);
            }
            if in_panel(x, y, &layout.run) {
                HOST.with(|slot| {
                    if let Some(host) = slot.borrow().as_ref() {
                        host.card.invoke_run_chosen();
                    }
                });
                return LRESULT(0);
            }

            LRESULT(0)
        }
        WM_KEYDOWN => {
            let key = wparam.0 as i32;
            match windows::Win32::UI::Input::KeyboardAndMouse::VIRTUAL_KEY(key as u16) {
                VK_ESCAPE => {
                    HOST.with(|slot| {
                        if let Some(host) = slot.borrow().as_ref() {
                            host.card.invoke_cancel_chosen();
                        }
                    });
                }
                VK_RETURN => {
                    let btn = STATE.with(|s| s.borrow().button_focus);
                    if btn == 1 {
                        HOST.with(|slot| {
                            if let Some(host) = slot.borrow().as_ref() {
                                host.card.invoke_cancel_chosen();
                            }
                        });
                    } else {
                        HOST.with(|slot| {
                            if let Some(host) = slot.borrow().as_ref() {
                                host.card.invoke_run_chosen();
                            }
                        });
                    }
                }
                VK_SPACE => {
                    let focused = STATE.with(|s| s.borrow().focused_page);
                    if let Some(idx) = focused {
                        toggle_select(idx, true);
                    }
                }
                VK_DELETE | VK_BACK => {
                    remove_selected();
                }
                VK_LEFT => {
                    let shift = unsafe { GetKeyState(VK_SHIFT.0 as i32) < 0 };
                    if shift {
                        rotate_left();
                    } else {
                        STATE.with(|s| {
                            let mut st = s.borrow_mut();
                            if let Some(cur) = st.focused_page {
                                if cur > 0 {
                                    st.focused_page = Some(cur - 1);
                                }
                            }
                        });
                        refresh_card();
                    }
                }
                VK_RIGHT => {
                    let shift = unsafe { GetKeyState(VK_SHIFT.0 as i32) < 0 };
                    if shift {
                        rotate_right();
                    } else {
                        STATE.with(|s| {
                            let mut st = s.borrow_mut();
                            let count = st.pages.len();
                            if let Some(cur) = st.focused_page {
                                if cur + 1 < count {
                                    st.focused_page = Some(cur + 1);
                                }
                            }
                        });
                        refresh_card();
                    }
                }
                VK_UP => {
                    STATE.with(|s| {
                        let mut st = s.borrow_mut();
                        if let Some(cur) = st.focused_page {
                            if cur >= 3 {
                                st.focused_page = Some(cur - 3);
                            }
                        }
                    });
                    refresh_card();
                }
                VK_DOWN => {
                    STATE.with(|s| {
                        let mut st = s.borrow_mut();
                        let count = st.pages.len();
                        if let Some(cur) = st.focused_page {
                            if cur + 3 < count {
                                st.focused_page = Some(cur + 3);
                            }
                        }
                    });
                    refresh_card();
                }
                VK_TAB => {
                    STATE.with(|s| {
                        let mut st = s.borrow_mut();
                        st.button_focus = (st.button_focus + 1) % 3;
                    });
                    refresh_card();
                }
                _ => {
                    // Check for 'L' / 'R' keys for quick rotation
                    if key == 0x4C {
                        // 'L'
                        rotate_left();
                    } else if key == 0x52 {
                        // 'R'
                        rotate_right();
                    }
                }
            }
            LRESULT(0)
        }
        WM_DPICHANGED => {
            let rect_ptr = lparam.0 as *const RECT;
            if !rect_ptr.is_null() {
                let r = unsafe { *rect_ptr };
                let _ = unsafe {
                    SetWindowPos(
                        hwnd,
                        None,
                        r.left,
                        r.top,
                        r.right - r.left,
                        r.bottom - r.top,
                        SWP_NOZORDER | SWP_NOACTIVATE,
                    )
                };
            }
            HOST.with(|slot| {
                if let Some(host) = slot.borrow_mut().as_mut() {
                    let scale = scale_factor(hwnd);
                    let phys_w = (CARD_W as f32 * scale).round() as u32;
                    let phys_h = (CARD_H as f32 * scale).round() as u32;
                    host.software.set_size(slint::PhysicalSize::new(phys_w, phys_h));
                    host.width = phys_w;
                    host.height = phys_h;
                    host.buffer = vec![Bgra8::default(); (phys_w * phys_h) as usize];
                }
            });
            refresh_card();
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

fn in_panel(x: f32, y: f32, panel: &Panel) -> bool {
    x >= panel.x && x <= panel.x + panel.width && y >= panel.y && y <= panel.y + panel.height
}

#[cfg(test)]
pub mod tests {
    use super::*;

    #[test]
    fn layout_matches_spec() {
        let layout = Layout::of(CARD_W, CARD_H, 4);

        // Header and top hairline
        assert_eq!(layout.header.x, 20.0);
        assert!(layout.hairline_top.y > layout.header.y);

        // Split hairline
        assert_eq!(layout.hairline_split.x, 530.0);

        // 4 pages arranged in 3 columns -> row 0 has 3, row 1 has 1
        assert_eq!(layout.pages.len(), 4);
        assert_eq!(layout.pages[0].x, 24.0);
        assert_eq!(layout.pages[0].y, 80.0);
        assert_eq!(layout.pages[1].x, 24.0 + 148.0 + 16.0);
        assert_eq!(layout.pages[1].y, 80.0);
        assert_eq!(layout.pages[3].x, 24.0);
        assert_eq!(layout.pages[3].y, 80.0 + 186.0 + 16.0);

        // Sidebar
        assert!(layout.sidebar_selection_box.x >= 530.0);
        assert!(layout.sidebar_pages_box.y > layout.sidebar_selection_box.y);
        assert!(layout.sidebar_summary_box.y > layout.sidebar_pages_box.y);

        // Footer
        assert!(layout.hairline_bottom.y > layout.sidebar_summary_box.y);
        assert!(layout.cancel.x < layout.run.x);
        assert_eq!(layout.client_width, CARD_W);
        assert_eq!(layout.client_height, CARD_H);
    }

    #[test]
    fn page_selection_and_transforms() {
        STATE.with(|s| {
            let mut st = s.borrow_mut();
            st.pages = vec![
                PageState {
                    source_doc: 0,
                    source_page: 0,
                    source_label: "doc.pdf".into(),
                    rotation_degrees: 0,
                    base_thumb: None,
                },
                PageState {
                    source_doc: 0,
                    source_page: 1,
                    source_label: "doc.pdf".into(),
                    rotation_degrees: 0,
                    base_thumb: None,
                },
                PageState {
                    source_doc: 0,
                    source_page: 2,
                    source_label: "doc.pdf".into(),
                    rotation_degrees: 0,
                    base_thumb: None,
                },
            ];
            st.selected.clear();
        });

        // Select all
        select_all();
        STATE.with(|s| assert_eq!(s.borrow().selected.len(), 3));

        // Select none
        select_none();
        STATE.with(|s| assert!(s.borrow().selected.is_empty()));

        // Toggle single select
        toggle_select(1, false);
        STATE.with(|s| {
            let st = s.borrow();
            assert_eq!(st.selected.len(), 1);
            assert!(st.selected.contains(&1));
        });

        // Rotate right (+90°) on selected page 1
        rotate_right();
        STATE.with(|s| {
            let st = s.borrow();
            assert_eq!(st.pages[0].rotation_degrees, 0);
            assert_eq!(st.pages[1].rotation_degrees, 90);
            assert_eq!(st.pages[2].rotation_degrees, 0);
        });

        // Rotate left (-90°) on selected page 1 -> returns to 0°
        rotate_left();
        STATE.with(|s| {
            let st = s.borrow();
            assert_eq!(st.pages[1].rotation_degrees, 0);
        });

        // Duplicate page 1
        duplicate();
        STATE.with(|s| {
            let st = s.borrow();
            assert_eq!(st.pages.len(), 4);
            assert_eq!(st.pages[1].source_page, 1);
            assert_eq!(st.pages[2].source_page, 1);
        });

        // Reverse order
        reverse_order();
        STATE.with(|s| {
            let st = s.borrow();
            assert_eq!(st.pages[0].source_page, 2);
            assert_eq!(st.pages[3].source_page, 0);
        });

        // Remove selected
        remove_selected();
        STATE.with(|s| {
            let st = s.borrow();
            assert_eq!(st.pages.len(), 3);
        });
    }
}

#[cfg(test)]
pub mod show_tests {
    use super::*;
    use std::sync::mpsc::channel;

    #[test]
    fn showing_and_pumping_the_organize_pdf_card_does_not_panic() {
        let (tx, _rx) = channel();
        let win = match OrganizePdfWindow::new(tx) {
            Ok(w) => w,
            Err(e) => {
                eprintln!("Skipping: {e}");
                return;
            }
        };

        let temp_dir = std::env::temp_dir().join(format!("ot-test-org-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&temp_dir);
        let sample = temp_dir.join("test.pdf");
        let _ = std::fs::write(&sample, b"%PDF-1.4 empty");

        win.show_for("tools.organize_pdf", "Organize Pages", "4 pages", &[sample]);

        // Pump message loop briefly
        let mut msg = windows::Win32::UI::WindowsAndMessaging::MSG::default();
        for _ in 0..10 {
            unsafe {
                if windows::Win32::UI::WindowsAndMessaging::PeekMessageW(
                    &mut msg,
                    Some(win.window),
                    0,
                    0,
                    windows::Win32::UI::WindowsAndMessaging::PM_REMOVE,
                )
                .as_bool()
                {
                    let _ = windows::Win32::UI::WindowsAndMessaging::TranslateMessage(&msg);
                    windows::Win32::UI::WindowsAndMessaging::DispatchMessageW(&msg);
                }
            }
        }

        unsafe {
            let _ = ShowWindow(win.window, SW_HIDE);
        };
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    #[ignore = "captures the organize pdf window"]
    fn capture_the_organize_pdf_window() {
        crate::ui::wheel::install_platform();
        let (tx, _rx) = std::sync::mpsc::channel();
        let window = match OrganizePdfWindow::new(tx) {
            Ok(w) => w,
            Err(e) => {
                eprintln!("Skipping: {e}");
                return;
            }
        };

        let temp_dir = std::env::temp_dir().join(format!("ot-cap-org-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&temp_dir);
        let sample = temp_dir.join("Quarterly Report.pdf");

        // Write a multi-page PDF with 4 styled pages
        let page_stream = |num: usize| {
            let text = format!("Page {num}");
            let stream = format!("BT /F1 24 Tf 40 140 Td ({text}) Tj ET");
            format!(
                "{} 0 obj << /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Contents {} 0 R /Resources << /Font << /F1 10 0 R >> >> >> endobj\n\
                 {} 0 obj << /Length {} >> stream\n{}\nendstream endobj\n",
                num * 2 + 1,
                num * 2 + 2,
                num * 2 + 2,
                stream.len(),
                stream
            )
        };
        let doc = format!(
            "%PDF-1.4\n\
             1 0 obj << /Type /Catalog /Pages 2 0 R >> endobj\n\
             2 0 obj << /Type /Pages /Kids [3 0 R 5 0 R 7 0 R 9 0 R] /Count 4 >> endobj\n\
             {}{}{}{}\
             10 0 obj << /Type /Font /Subtype /Type1 /BaseFont /Helvetica >> endobj\n\
             trailer << /Root 1 0 R /Size 12 >>",
            page_stream(1),
            page_stream(2),
            page_stream(3),
            page_stream(4)
        );
        std::fs::write(&sample, doc.as_bytes()).expect("write sample pdf");

        window.show_for("tools.organize_pdf", "Organize Pages", "4 pages", &[sample]);

        // Select page 1 for selection highlight
        toggle_select(1, false);

        unsafe {
            let mut msg = windows::Win32::UI::WindowsAndMessaging::MSG::default();
            for _ in 0..30 {
                while windows::Win32::UI::WindowsAndMessaging::PeekMessageW(
                    &mut msg,
                    None,
                    0,
                    0,
                    windows::Win32::UI::WindowsAndMessaging::PM_REMOVE,
                )
                .as_bool()
                {
                    let _ = windows::Win32::UI::WindowsAndMessaging::TranslateMessage(&msg);
                    windows::Win32::UI::WindowsAndMessaging::DispatchMessageW(&msg);
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        }

        let shots = std::path::Path::new("target").join("shots");
        let _ = std::fs::create_dir_all(&shots);
        HOST.with(|slot| {
            let borrowed = slot.borrow();
            if let Some(host) = borrowed.as_ref() {
                let mut rgba = Vec::with_capacity(host.buffer.len() * 4);
                for pixel in &host.buffer {
                    let (_, r, g, b) = pixel.parts();
                    rgba.extend_from_slice(&[r, g, b, 255]);
                }
                let _ = image::save_buffer(
                    shots.join("organize-pdf-buffer.png"),
                    &rgba,
                    host.width,
                    host.height,
                    image::ExtendedColorType::Rgba8,
                );
            }
        });

        crate::platform::windows::windowhost::screenshot::capture(
            window.window,
            &shots.join("organize-pdf-window.png"),
        );

        window.hide();
        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}
