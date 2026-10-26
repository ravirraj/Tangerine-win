//! The window a list of videos or PDFs is reordered and merged in.
//!
//! Measures card dimensions via Layout::of(), manages Slint MergeCard,
//! wears video frame or PDF first page thumbnails, displays filenames and
//! duration/page counts, supports reordering via stacked Up/Down chevrons,
//! mouse clicks, and keyboard shortcuts (Up/Down arrow navigation, Shift+Up/Down
//! swapping, Enter to run, Esc to close).

use std::cell::{Cell, RefCell};
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
use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, VK_CONTROL, VK_SHIFT};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, GetClientRect, LoadCursorW, PostQuitMessage, RegisterClassW,
    SetForegroundWindow, SetLayeredWindowAttributes, SetWindowPos, ShowWindow, CW_USEDEFAULT,
    IDC_ARROW, LWA_ALPHA, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOZORDER, SW_HIDE, SW_SHOW,
    WM_DESTROY, WM_DPICHANGED, WM_ERASEBKGND, WM_KEYDOWN, WM_LBUTTONDOWN, WM_PAINT, WNDCLASSW, WS_EX_LAYERED,
};

use crate::media::locator;
use crate::media::metadata::format_file_size;
use crate::platform::windows::channelswindow::format_clock;
use crate::platform::windows::toolwindow::ToolEvent;
use crate::platform::windows::windowhost::{fixed_style, outer_size_for, scale_factor};
use crate::radial::execution::ToolSettings;
use crate::ui::wheel::{window_for_next_component, Bgra8, MergeCard, MergeItemData, Panel};

pub const CARD_W: i32 = 400;
pub const HEADER_UNITS: f32 = 64.0;
pub const SECTION_UNITS: f32 = 30.0;
pub const ITEM_UNITS: f32 = 54.0;
pub const FOOTNOTE_UNITS: f32 = 26.0;
pub const FOOTER_UNITS: f32 = 64.0;

#[derive(Clone, Debug, PartialEq)]
pub struct ItemLayout {
    pub row: Panel,
    pub up: Panel,
    pub down: Panel,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    pub client_width: i32,
    pub client_height: i32,
    pub header: Panel,
    pub detail: Panel,
    pub hairline_top: Panel,
    pub section: Panel,
    pub items: Vec<ItemLayout>,
    pub footnote: Panel,
    pub hairline_bottom: Panel,
    pub helper: Panel,
    pub cancel: Panel,
    pub run: Panel,
}

impl Layout {
    pub const WIDTH_UNITS: f32 = 400.0;

    pub fn card_height(item_count: usize) -> f32 {
        HEADER_UNITS
            + SECTION_UNITS
            + (ITEM_UNITS * (item_count.max(1) as f32))
            + FOOTNOTE_UNITS
            + FOOTER_UNITS
    }

    pub fn of(client_width: i32, client_height: i32, item_count: usize) -> Self {
        let scale = (client_width as f32 / Self::WIDTH_UNITS).max(0.5);
        let w = client_width as f32;
        let h = client_height as f32;
        let pad = 18.0 * scale;
        let content_w = w - pad * 2.0;

        let header = Panel {
            x: pad,
            y: 14.0 * scale,
            width: content_w,
            height: 22.0 * scale,
        };
        let detail = Panel {
            x: pad,
            y: 38.0 * scale,
            width: content_w,
            height: 18.0 * scale,
        };
        let hairline_top = Panel {
            x: 0.0,
            y: 62.0 * scale,
            width: w,
            height: 1.0,
        };
        let section = Panel {
            x: pad,
            y: 68.0 * scale,
            width: content_w,
            height: 22.0 * scale,
        };

        let items_start_y = 92.0 * scale;
        let row_h = ITEM_UNITS * scale;
        let mut items = Vec::with_capacity(item_count);
        for i in 0..item_count {
            let row_y = items_start_y + (i as f32) * row_h;
            let row = Panel {
                x: 0.0,
                y: row_y,
                width: w,
                height: row_h,
            };
            let up = Panel {
                x: w - 48.0 * scale,
                y: row_y + 6.0 * scale,
                width: 32.0 * scale,
                height: 20.0 * scale,
            };
            let down = Panel {
                x: w - 48.0 * scale,
                y: row_y + 28.0 * scale,
                width: 32.0 * scale,
                height: 20.0 * scale,
            };
            items.push(ItemLayout { row, up, down });
        }

        let footnote_y = items_start_y + (item_count as f32) * row_h + 4.0 * scale;
        let footnote = Panel {
            x: pad,
            y: footnote_y,
            width: content_w,
            height: 18.0 * scale,
        };

        let hairline_bottom = Panel {
            x: 0.0,
            y: h - 65.0 * scale,
            width: w,
            height: 1.0,
        };
        let helper = Panel {
            x: pad,
            y: h - 52.0 * scale,
            width: w - 220.0 * scale,
            height: 36.0 * scale,
        };
        let cancel = Panel {
            x: w - 200.0 * scale,
            y: h - 52.0 * scale,
            width: 76.0 * scale,
            height: 36.0 * scale,
        };
        let run = Panel {
            x: w - 116.0 * scale,
            y: h - 52.0 * scale,
            width: 98.0 * scale,
            height: 36.0 * scale,
        };

        Self {
            client_width,
            client_height,
            header,
            detail,
            hairline_top,
            section,
            items,
            footnote,
            hairline_bottom,
            helper,
            cancel,
            run,
        }
    }
}

#[derive(Clone)]
pub struct ItemState {
    pub path: PathBuf,
    pub name: String,
    pub detail: String,
    pub ext_badge: String,
    pub thumb_buffer: Option<SharedPixelBuffer<slint::Rgba8Pixel>>,
}

#[derive(Default)]
struct State {
    operation: String,
    is_pdf: bool,
    card_title: String,
    detail: String,
    section_title: String,
    footnote_text: String,
    run_label: String,
    helper_text: String,
    items: Vec<ItemState>,
    focused_index: isize, // -1 for none, 0..N-1
    button_focus: i32,    // 0=none, 1=cancel, 2=run
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
    card: MergeCard,
    software: Rc<MinimalSoftwareWindow>,
    buffer: Vec<Bgra8>,
    width: u32,
    height: u32,
}

impl Host {
    fn draw(&mut self) {
        let stride = self.width as usize;
        let mut buffer = std::mem::take(&mut self.buffer);
        self.software.draw_if_needed(|renderer| {
            renderer.render(&mut buffer, stride);
        });
        self.buffer = buffer;
        unsafe { self.put_on_window() };
    }

    unsafe fn put_on_window(&self) {
        crate::platform::windows::windowhost::blit_card_buffer(
            self.window,
            &self.buffer,
            self.width,
            self.height,
        );
    }
}

pub struct MergeWindow {
    pub window: HWND,
    #[allow(dead_code)]
    pub sender: Sender<ToolEvent>,
}

impl MergeWindow {
    pub fn new(sender: Sender<ToolEvent>) -> Result<Self, String> {
        let class_name = w!("OpenTangerineMergeCard");
        let instance = unsafe { GetModuleHandleW(None).map_err(|e| e.to_string())? };
        let wc = WNDCLASSW {
            lpfnWndProc: Some(merge_card_proc),
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
        let card = MergeCard::new().map_err(|e| e.to_string())?;

        let initial_h = Layout::card_height(2).round() as i32;
        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_LAYERED,
                class_name,
                w!("Merge"),
                fixed_style(),
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                CARD_W,
                initial_h,
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
                &preference as *const _ as *const core::ffi::c_void,
                std::mem::size_of::<i32>() as u32,
            );
            let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), 0, LWA_ALPHA);
        }

        WINDOW.with(|slot| slot.set(hwnd.0 as usize));

        // Wire Slint callbacks
        let snd_run = sender.clone();
        card.on_run_chosen(move || {
            let (operation, files) = STATE.with(|s| {
                let st = s.borrow();
                (
                    st.operation.clone(),
                    st.items.iter().map(|it| it.path.clone()).collect::<Vec<_>>(),
                )
            });
            if files.len() >= 2 {
                let _ = snd_run.send(ToolEvent::Run {
                    operation,
                    inputs: files,
                    settings: ToolSettings::default(),
                });
                unsafe {
                    let _ = ShowWindow(hwnd, SW_HIDE);
                };
            }
        });

        let snd_cancel = sender.clone();
        card.on_cancel_chosen(move || {
            let _ = snd_cancel.send(ToolEvent::Cancelled);
            unsafe {
                let _ = ShowWindow(hwnd, SW_HIDE);
            };
        });

        card.on_move_up_chosen(|idx| {
            move_up(idx as usize);
        });

        card.on_move_down_chosen(|idx| {
            move_down(idx as usize);
        });

        card.on_item_chosen(|idx| {
            STATE.with(|s| {
                let mut st = s.borrow_mut();
                st.focused_index = idx as isize;
                st.button_focus = 0;
            });
            refresh_card();
        });

        HOST.with(|slot| {
            *slot.borrow_mut() = Some(Host {
                window: hwnd,
                card,
                software,
                buffer: Vec::new(),
                width: CARD_W as u32,
                height: initial_h as u32,
            });
        });

        Ok(Self {
            window: hwnd,
            sender,
        })
    }

    pub fn show_for(
        &self,
        operation: &str,
        _title: &str,
        _detail: &str,
        files: &[PathBuf],
    ) {
        if files.is_empty() {
            return;
        }

        let is_pdf = files
            .first()
            .and_then(|p| p.extension())
            .and_then(|e| e.to_str())
            .map(|e| e.eq_ignore_ascii_case("pdf"))
            .unwrap_or(false);

        let tools = locator::find();

        let mut item_states = Vec::with_capacity(files.len());
        for file in files {
            let name = file
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("file")
                .to_string();
            let ext = file
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_uppercase();
            let size = std::fs::metadata(file).map(|m| m.len()).unwrap_or(0);
            let size_str = format_file_size(size);

            if is_pdf {
                let pages = crate::media::pdf::page_count(file).unwrap_or(1);
                let detail = format!("{size_str} \u{b7} {pages} page{}", if pages == 1 { "" } else { "s" });
                let thumb_buffer = crate::media::pdf::render_first_page(file, 80).ok().map(|img| {
                    let rgba = img.to_rgba8();
                    SharedPixelBuffer::clone_from_slice(rgba.as_raw(), rgba.width(), rgba.height())
                });
                item_states.push(ItemState {
                    path: file.clone(),
                    name,
                    detail,
                    ext_badge: ext,
                    thumb_buffer,
                });
            } else {
                let duration = tools
                    .as_ref()
                    .and_then(|t| crate::media::frames::duration_of(t, file))
                    .unwrap_or(0.0);
                let detail = format!("{size_str} \u{b7} {}", format_clock(duration));
                let thumb_buffer = tools.as_ref().and_then(|t| {
                    let frame = crate::media::frames::frame_at(t, file, 0.5)
                        .or_else(|| crate::media::frames::frame_at(t, file, 0.0))?;
                    let small = frame.thumbnail(80, 80);
                    let rgba = small.to_rgba8();
                    Some(SharedPixelBuffer::clone_from_slice(
                        rgba.as_raw(),
                        rgba.width(),
                        rgba.height(),
                    ))
                });
                item_states.push(ItemState {
                    path: file.clone(),
                    name,
                    detail,
                    ext_badge: ext,
                    thumb_buffer,
                });
            }
        }

        let (card_title, detail_str, section_title, footnote_text, run_label, helper_text) = if is_pdf {
            (
                "Merge PDFs".to_string(),
                format!(
                    "{} document{}",
                    files.len(),
                    if files.len() == 1 { "" } else { "s" }
                ),
                "Document order".to_string(),
                "Documents are merged in list order into one PDF.".to_string(),
                "Merge PDFs".to_string(),
                "Enter to merge \u{b7} Esc to close".to_string(),
            )
        } else {
            (
                "Join Videos".to_string(),
                format!(
                    "{} video{}",
                    files.len(),
                    if files.len() == 1 { "" } else { "s" }
                ),
                "Playback order".to_string(),
                "Clips are scaled to fit the first clip. Output is MP4.".to_string(),
                "Join Videos".to_string(),
                "Enter to join \u{b7} Esc to close".to_string(),
            )
        };

        STATE.with(|slot| {
            let mut st = slot.borrow_mut();
            st.operation = operation.to_string();
            st.is_pdf = is_pdf;
            st.card_title = card_title;
            st.detail = detail_str;
            st.section_title = section_title;
            st.footnote_text = footnote_text;
            st.run_label = run_label;
            st.helper_text = helper_text;
            st.items = item_states;
            st.focused_index = 0;
            st.button_focus = 0;
        });

        let scale = scale_factor(self.window);
        let phys_w = (CARD_W as f32 * scale).round() as i32;
        let height_units = Layout::card_height(files.len());
        let phys_h = (height_units * scale).round() as i32;
        let (outer_w, outer_h) = outer_size_for(self.window, phys_w, phys_h);

        unsafe {
            let _ = SetWindowPos(
                self.window,
                None,
                0,
                0,
                outer_w,
                outer_h,
                SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }

        apply_layout(phys_w, phys_h, files.len());
        refresh_card();

        unsafe {
            let _ = ShowWindow(self.window, SW_SHOW);
            let _ = SetForegroundWindow(self.window);
        }
    }

    #[allow(dead_code)]
    pub fn hide(&self) {
        unsafe {
            let _ = ShowWindow(self.window, SW_HIDE);
        }
    }
}

fn move_up(idx: usize) {
    if idx > 0 {
        STATE.with(|slot| {
            let mut st = slot.borrow_mut();
            if idx < st.items.len() {
                st.items.swap(idx, idx - 1);
                st.focused_index = (idx - 1) as isize;
            }
        });
        refresh_card();
    }
}

fn move_down(idx: usize) {
    STATE.with(|slot| {
        let mut st = slot.borrow_mut();
        if idx + 1 < st.items.len() {
            st.items.swap(idx, idx + 1);
            st.focused_index = (idx + 1) as isize;
        }
    });
    refresh_card();
}

fn apply_layout(width: i32, height: i32, item_count: usize) {
    let layout = Layout::of(width, height, item_count);
    HOST.with(|slot| {
        if let Some(host) = slot.borrow_mut().as_mut() {
            let scale = width as f32 / CARD_W as f32;
            host.card.set_card_width(width as f32 / scale);
            host.card.set_card_height(height as f32 / scale);
            host.card.set_ui_scale(scale);

            host.card.set_header_box(layout.header);
            host.card.set_detail_box(layout.detail);
            host.card.set_hairline_top(layout.hairline_top);
            host.card.set_section_box(layout.section);
            host.card.set_footnote_box(layout.footnote);
            host.card.set_hairline_bottom(layout.hairline_bottom);
            host.card.set_helper_box(layout.helper);
            host.card.set_cancel_box(layout.cancel);
            host.card.set_run_box(layout.run);

            host.software
                .set_size(PhysicalSize::new(width as u32, height as u32));
            host.width = width as u32;
            host.height = height as u32;
            host.buffer = vec![Bgra8::default(); (width as usize) * (height as usize)];
        }
    });
}

fn refresh_card() {
    STATE.with(|slot| {
        let st = slot.borrow();
        HOST.with(|h_slot| {
            if let Some(host) = h_slot.borrow_mut().as_mut() {
                host.card.set_file_name(st.card_title.as_str().into());
                host.card.set_detail(st.detail.as_str().into());
                host.card.set_section_title(st.section_title.as_str().into());
                host.card.set_footnote_text(st.footnote_text.as_str().into());
                host.card.set_run_label(st.run_label.as_str().into());
                host.card.set_run_enabled(st.items.len() >= 2);
                host.card.set_focused_index(st.focused_index as i32);
                host.card.set_button_focus(st.button_focus);

                let layout = Layout::of(host.width as i32, host.height as i32, st.items.len());
                let items_slint: Vec<MergeItemData> = st
                    .items
                    .iter()
                    .enumerate()
                    .map(|(idx, it)| {
                        let can_move_up = idx > 0;
                        let can_move_down = idx < st.items.len() - 1;
                        let row_layout = &layout.items[idx];
                        let (thumb, has_thumb) = match &it.thumb_buffer {
                            Some(buf) => (slint::Image::from_rgba8(buf.clone()), true),
                            None => (slint::Image::default(), false),
                        };
                        MergeItemData {
                            index: idx as i32,
                            display_index: (idx + 1).to_string().into(),
                            name: it.name.clone().into(),
                            detail: it.detail.clone().into(),
                            thumb,
                            has_thumb,
                            ext_badge: it.ext_badge.clone().into(),
                            focused: st.focused_index == idx as isize,
                            can_move_up,
                            can_move_down,
                            y: row_layout.row.y,
                            height: row_layout.row.height,
                        }
                    })
                    .collect();

                host.card.set_items(ModelRc::new(VecModel::from(items_slint)));
                host.software.request_redraw();
            }
        });
    });
    let window = window_handle();
    if !window.0.is_null() {
        let _ = unsafe { InvalidateRect(Some(window), None, true) };
    }
}

unsafe extern "system" fn merge_card_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
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
            let count = STATE.with(|s| s.borrow().items.len());
            let layout = Layout::of(rect.right, rect.bottom, count);

            // Check items and chevrons
            for (idx, item) in layout.items.iter().enumerate() {
                if x >= item.up.x
                    && x <= item.up.x + item.up.width
                    && y >= item.up.y
                    && y <= item.up.y + item.up.height
                {
                    move_up(idx);
                    return LRESULT(0);
                }
                if x >= item.down.x
                    && x <= item.down.x + item.down.width
                    && y >= item.down.y
                    && y <= item.down.y + item.down.height
                {
                    move_down(idx);
                    return LRESULT(0);
                }
                if x >= item.row.x
                    && x <= item.row.x + item.row.width
                    && y >= item.row.y
                    && y <= item.row.y + item.row.height
                {
                    STATE.with(|s| {
                        let mut st = s.borrow_mut();
                        st.focused_index = idx as isize;
                        st.button_focus = 0;
                    });
                    refresh_card();
                    return LRESULT(0);
                }
            }

            // Check cancel button
            if x >= layout.cancel.x
                && x <= layout.cancel.x + layout.cancel.width
                && y >= layout.cancel.y
                && y <= layout.cancel.y + layout.cancel.height
            {
                HOST.with(|slot| {
                    if let Some(host) = slot.borrow().as_ref() {
                        host.card.invoke_cancel_chosen();
                    }
                });
                return LRESULT(0);
            }

            // Check run button
            if x >= layout.run.x
                && x <= layout.run.x + layout.run.width
                && y >= layout.run.y
                && y <= layout.run.y + layout.run.height
            {
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
            let key = wparam.0 as u32;
            let shift_down = unsafe { GetKeyState(VK_SHIFT.0 as i32) } < 0;
            let ctrl_down = unsafe { GetKeyState(VK_CONTROL.0 as i32) } < 0;
            let is_mod = shift_down || ctrl_down;

            match key {
                0x1B => {
                    // Escape -> Cancel
                    HOST.with(|slot| {
                        if let Some(host) = slot.borrow().as_ref() {
                            host.card.invoke_cancel_chosen();
                        }
                    });
                }
                0x0D => {
                    // Enter -> Run or Cancel depending on button_focus
                    let btn_focus = STATE.with(|s| s.borrow().button_focus);
                    HOST.with(|slot| {
                        if let Some(host) = slot.borrow().as_ref() {
                            if btn_focus == 1 {
                                host.card.invoke_cancel_chosen();
                            } else {
                                host.card.invoke_run_chosen();
                            }
                        }
                    });
                }
                0x09 => {
                    // Tab: walk through items, then cancel, then run, then cycle
                    let count = STATE.with(|s| s.borrow().items.len());
                    STATE.with(|s| {
                        let mut st = s.borrow_mut();
                        if shift_down {
                            if st.button_focus == 2 {
                                st.button_focus = 1;
                            } else if st.button_focus == 1 {
                                st.button_focus = 0;
                                st.focused_index = (count as isize) - 1;
                            } else if st.focused_index > 0 {
                                st.focused_index -= 1;
                            } else {
                                st.button_focus = 2;
                                st.focused_index = -1;
                            }
                        } else if st.focused_index >= 0 && (st.focused_index as usize) < count - 1 {
                            st.focused_index += 1;
                        } else if st.focused_index == (count as isize) - 1 {
                            st.focused_index = -1;
                            st.button_focus = 1;
                        } else if st.button_focus == 1 {
                            st.button_focus = 2;
                        } else {
                            st.button_focus = 0;
                            st.focused_index = 0;
                        }
                    });
                    refresh_card();
                }
                0x26 => {
                    // Up Arrow: move item if mod, else move focus
                    let (focused, count) = STATE.with(|s| {
                        let st = s.borrow();
                        (st.focused_index, st.items.len())
                    });
                    if is_mod && focused > 0 {
                        move_up(focused as usize);
                    } else if !is_mod {
                        STATE.with(|s| {
                            let mut st = s.borrow_mut();
                            if st.button_focus > 0 {
                                st.button_focus = 0;
                                st.focused_index = (count as isize) - 1;
                            } else if st.focused_index > 0 {
                                st.focused_index -= 1;
                            }
                        });
                        refresh_card();
                    }
                }
                0x28 => {
                    // Down Arrow: move item if mod, else move focus
                    let (focused, count) = STATE.with(|s| {
                        let st = s.borrow();
                        (st.focused_index, st.items.len())
                    });
                    if is_mod && focused >= 0 && (focused as usize) < count - 1 {
                        move_down(focused as usize);
                    } else if !is_mod {
                        STATE.with(|s| {
                            let mut st = s.borrow_mut();
                            if st.focused_index >= 0 && (st.focused_index as usize) < count - 1 {
                                st.focused_index += 1;
                            } else if st.focused_index == (count as isize) - 1 {
                                st.focused_index = -1;
                                st.button_focus = 2;
                            }
                        });
                        refresh_card();
                    }
                }
                _ => {}
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            unsafe { PostQuitMessage(0) };
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
            let scale = scale_factor(hwnd);
            let count = STATE.with(|s| s.borrow().items.len());
            let phys_w = (CARD_W as f32 * scale).round() as i32;
            let height_units = Layout::card_height(count);
            let phys_h = (height_units * scale).round() as i32;
            apply_layout(phys_w, phys_h, count);
            refresh_card();
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_measures_boxes_proportionally_for_given_item_count() {
        let h2 = Layout::card_height(2).round() as i32;
        let layout2 = Layout::of(400, h2, 2);
        assert_eq!(layout2.client_width, 400);
        assert_eq!(layout2.client_height, h2);
        assert_eq!(layout2.items.len(), 2);
        assert_eq!(layout2.header.y, 14.0);
        assert_eq!(layout2.hairline_top.y, 62.0);
        assert_eq!(layout2.section.y, 68.0);
        assert_eq!(layout2.items[0].row.y, 92.0);
        assert_eq!(layout2.items[1].row.y, 92.0 + ITEM_UNITS);
        assert!(layout2.footnote.y >= layout2.items[1].row.y + ITEM_UNITS);
        assert!(layout2.hairline_bottom.y > layout2.footnote.y);
        assert_eq!(layout2.cancel.width, 76.0);
        assert_eq!(layout2.run.width, 98.0);

        let h4 = Layout::card_height(4).round() as i32;
        let layout4 = Layout::of(400, h4, 4);
        assert_eq!(layout4.items.len(), 4);
        assert_eq!(layout4.items[3].row.y, 92.0 + 3.0 * ITEM_UNITS);
    }

    #[test]
    fn card_height_calculation_scales_with_items() {
        let h2 = Layout::card_height(2);
        let h3 = Layout::card_height(3);
        let h4 = Layout::card_height(4);
        assert_eq!(h3 - h2, ITEM_UNITS);
        assert_eq!(h4 - h3, ITEM_UNITS);
        assert!(h2 >= 200.0);
    }

    #[test]
    fn swapping_items_maintains_bounds_and_order() {
        let mut items = vec![
            ItemState {
                path: PathBuf::from("a.mp4"),
                name: "a.mp4".into(),
                detail: "".into(),
                ext_badge: "MP4".into(),
                thumb_buffer: None,
            },
            ItemState {
                path: PathBuf::from("b.mp4"),
                name: "b.mp4".into(),
                detail: "".into(),
                ext_badge: "MP4".into(),
                thumb_buffer: None,
            },
            ItemState {
                path: PathBuf::from("c.mp4"),
                name: "c.mp4".into(),
                detail: "".into(),
                ext_badge: "MP4".into(),
                thumb_buffer: None,
            },
        ];

        // Swap 0 and 1
        items.swap(0, 1);
        assert_eq!(items[0].name, "b.mp4");
        assert_eq!(items[1].name, "a.mp4");
        assert_eq!(items[2].name, "c.mp4");

        // Swap 1 and 2
        items.swap(1, 2);
        assert_eq!(items[1].name, "c.mp4");
        assert_eq!(items[2].name, "a.mp4");
    }
}

#[cfg(test)]
pub mod show_tests {
    use super::*;
    use windows::Win32::UI::WindowsAndMessaging::{
        DispatchMessageW, PeekMessageW, TranslateMessage, MSG, PM_REMOVE,
    };

    #[test]
    fn showing_and_pumping_the_merge_card_does_not_panic() {
        crate::ui::wheel::install_platform();
        let (tx, _rx) = std::sync::mpsc::channel();
        let window = MergeWindow::new(tx).expect("merge window created");
        let scratch = std::env::temp_dir().join(format!("ot-merge-show-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&scratch);
        let file1 = scratch.join("clip1.mp4");
        let file2 = scratch.join("clip2.mp4");
        std::fs::write(&file1, b"mock video 1").expect("file1");
        std::fs::write(&file2, b"mock video 2").expect("file2");

        window.show_for("tools.merge", "Join", "Join videos", &[file1, file2]);
        unsafe {
            let mut msg = MSG::default();
            for _ in 0..10 {
                while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }
        }
        window.hide();
        let _ = std::fs::remove_dir_all(&scratch);
    }

    #[test]
    #[ignore = "captures the join videos window"]
    fn capture_the_join_videos_window() {
        crate::ui::wheel::install_platform();
        let (tx, _rx) = std::sync::mpsc::channel();
        let window = MergeWindow::new(tx).expect("merge window created");
        let scratch = std::env::temp_dir().join(format!("ot-join-cap-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&scratch);
        let file1 = scratch.join("Sunset drive.mp4");
        let file2 = scratch.join("Beach waves.mp4");
        let file3 = scratch.join("Campfire night.mp4");
        std::fs::write(&file1, b"mock video 1").expect("file1");
        std::fs::write(&file2, b"mock video 2").expect("file2");
        std::fs::write(&file3, b"mock video 3").expect("file3");

        window.show_for("tools.merge", "Join Videos", "Join videos", &[file1, file2, file3]);
        unsafe {
            let mut msg = MSG::default();
            for _ in 0..30 {
                while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
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
                    shots.join("join-videos-buffer.png"),
                    &rgba,
                    host.width,
                    host.height,
                    image::ExtendedColorType::Rgba8,
                );
            }
        });

        crate::platform::windows::windowhost::screenshot::capture(
            window.window,
            &shots.join("join-videos-window.png"),
        );

        window.hide();
        let _ = std::fs::remove_dir_all(&scratch);
    }

    #[test]
    #[ignore = "captures the merge pdf window"]
    fn capture_the_merge_pdf_window() {
        crate::ui::wheel::install_platform();
        let (tx, _rx) = std::sync::mpsc::channel();
        let window = MergeWindow::new(tx).expect("merge window created");
        let scratch = std::env::temp_dir().join(format!("ot-pdf-cap-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&scratch);
        let file1 = scratch.join("Annual Report.pdf");
        let file2 = scratch.join("Financial Appendix.pdf");
        std::fs::write(&file1, b"%PDF-1.4 mock pdf 1").expect("file1");
        std::fs::write(&file2, b"%PDF-1.4 mock pdf 2").expect("file2");

        window.show_for("tools.merge", "Merge PDFs", "Merge documents", &[file1, file2]);
        unsafe {
            let mut msg = MSG::default();
            for _ in 0..30 {
                while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
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
                    shots.join("merge-pdf-buffer.png"),
                    &rgba,
                    host.width,
                    host.height,
                    image::ExtendedColorType::Rgba8,
                );
            }
        });

        crate::platform::windows::windowhost::screenshot::capture(
            window.window,
            &shots.join("merge-pdf-window.png"),
        );

        window.hide();
        let _ = std::fs::remove_dir_all(&scratch);
    }
}
