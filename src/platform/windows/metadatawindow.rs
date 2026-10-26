//! The window a file's metadata is inspected and edited in.
//!
//! Visual and behavioral twin of the reference's Edit Metadata tool view:
//! - Left pane: a white card holding a search field at top, the editable
//!   metadata fields (Title, Description, Author, Keywords, Copyright,
//!   Date, Comment) with changed dots when modified, and file system
//!   readouts (Created, Modified, Name, Size).
//! - Right pane: three sidebar cards: Fields instruction and Undo,
//!   Remove options (Remove all, Remove camera/GPS/processing data), and
//!   Summary count of total and sensitive fields.
//! - Footer: file name on the left, Cancel and Save copy actions on the right.
//!
//! Layout-twin pattern: Slint card only draws; the Win32 window host measures
//! all boxes in `Layout::of(scale)` and hands them to the card via `Panel`
//! properties, while hit-testing and keyboard walk match the same `Box4` math.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc::Sender;

use slint::platform::software_renderer::MinimalSoftwareWindow;
use slint::PhysicalSize;
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{COLORREF, HMODULE, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreateFontIndirectW, CreateSolidBrush, DrawTextW, EndPaint, GetDC, InvalidateRect,
    ReleaseDC, SelectObject, SetBkMode, SetTextColor,
    DT_LEFT, DT_NOPREFIX, DT_SINGLELINE, DT_VCENTER,
    HFONT, LOGFONTW, PAINTSTRUCT, TRANSPARENT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::AdjustWindowRectExForDpi;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    EnableWindow, GetFocus, GetKeyState, SetFocus, VK_DOWN, VK_ESCAPE, VK_RETURN, VK_SHIFT,
    VK_SPACE, VK_TAB, VK_UP,
};
use windows::Win32::UI::Shell::{DefSubclassProc, SetWindowSubclass};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, GetClientRect, GetWindowTextLengthW, GetWindowTextW,
    KillTimer, LoadCursorW, RegisterClassW, SendMessageW, SetForegroundWindow,
    SetLayeredWindowAttributes, SetTimer, SetWindowPos, SetWindowTextW, ShowWindow, CW_USEDEFAULT,
    EN_CHANGE, HMENU, IDC_ARROW, LWA_ALPHA, SWP_NOACTIVATE, SWP_NOZORDER, SWP_SHOWWINDOW, SW_HIDE,
    SW_SHOW, WM_CLOSE, WM_COMMAND, WM_CTLCOLOREDIT, WM_DPICHANGED, WM_ERASEBKGND, WM_KEYDOWN,
    WM_LBUTTONDOWN, WM_PAINT, WM_SETFONT, WM_SIZE, WM_TIMER, WNDCLASSW, WS_CHILD, WS_EX_LAYERED,
    WS_TABSTOP, WINDOW_EX_STYLE,
};

use crate::media::metadata::{format_file_size, format_system_time, MetadataField, MetadataTag};
use crate::media::probe::Kind;
use crate::platform::windows::toolwindow::ToolEvent;
use crate::platform::windows::windowhost::{fixed_style, scale_factor, to_wide};
use crate::radial::execution::ToolSettings;
use crate::ui::wheel::{window_for_next_component, Bgra8, MetadataCard, Panel};

/// The timer the window comes in softly on.
const FADE_TIMER_ID: usize = 54;

/// Win32 message to set cue banner (placeholder) text in an edit control.
const EM_SETCUEBANNER: u32 = 0x1501;

/// The card's width in the drawing's own units.
pub const CARD_W: i32 = 820;
/// The card's height in the drawing's own units.
pub const CARD_H: i32 = 560;

#[derive(Clone, Copy, Debug, PartialEq)]
struct Box4 {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
}

impl Box4 {
    fn contains(&self, px: f32, py: f32) -> bool {
        px >= self.x && px <= self.x + self.w && py >= self.y && py <= self.y + self.h
    }
}

fn to_panel(b: Box4, scale: f32) -> Panel {
    Panel {
        x: (b.x * scale).round() as f32,
        y: (b.y * scale).round() as f32,
        width: (b.w * scale).round() as f32,
        height: (b.h * scale).round() as f32,
    }
}

#[derive(Clone, Debug, PartialEq)]
struct Layout {
    card_w: f32,
    card_h: f32,
    left_card_box: Box4,
    search_tile_box: Box4,
    search_field_box: Box4,
    hairline_search_box: Box4,
    fields_header_box: Box4,
    tag_label_boxes: [Box4; 7],
    tag_tile_boxes: [Box4; 7],
    tag_edit_boxes: [Box4; 7],
    tag_dot_boxes: [Box4; 7],
    file_header_box: Box4,
    file_row_boxes: [Box4; 4],
    file_label_boxes: [Box4; 4],
    file_value_boxes: [Box4; 4],
    sidebar_fields_card_box: Box4,
    sidebar_fields_title_box: Box4,
    sidebar_fields_text_box: Box4,
    undo_button_box: Box4,
    sidebar_remove_card_box: Box4,
    sidebar_remove_title_box: Box4,
    remove_all_check_box: Box4,
    remove_sensitive_check_box: Box4,
    sidebar_remove_hint_box: Box4,
    sidebar_summary_card_box: Box4,
    sidebar_summary_title_box: Box4,
    sidebar_summary_text_box: Box4,
    hairline_bottom_box: Box4,
    footer_icon_box: Box4,
    footer_file_box: Box4,
    footer_helper_box: Box4,
    footer_cancel_box: Box4,
    footer_save_box: Box4,
}

impl Layout {
    fn of(_scale: f32) -> Self {
        let card_w = CARD_W as f32;
        let card_h = CARD_H as f32;

        let left_card_box = Box4 {
            x: 20.0,
            y: 20.0,
            w: 474.0,
            h: 468.0,
        };
        let search_tile_box = Box4 {
            x: 240.0,
            y: 30.0,
            w: 240.0,
            h: 28.0,
        };
        let search_field_box = Box4 {
            x: 244.0,
            y: 32.0,
            w: 232.0,
            h: 24.0,
        };
        let hairline_search_box = Box4 {
            x: 20.0,
            y: 68.0,
            w: 474.0,
            h: 1.0,
        };
        let fields_header_box = Box4 {
            x: 34.0,
            y: 76.0,
            w: 200.0,
            h: 20.0,
        };

        let mut tag_label_boxes = [Box4 { x: 0.0, y: 0.0, w: 0.0, h: 0.0 }; 7];
        let mut tag_tile_boxes = [Box4 { x: 0.0, y: 0.0, w: 0.0, h: 0.0 }; 7];
        let mut tag_edit_boxes = [Box4 { x: 0.0, y: 0.0, w: 0.0, h: 0.0 }; 7];
        let mut tag_dot_boxes = [Box4 { x: 0.0, y: 0.0, w: 0.0, h: 0.0 }; 7];

        for i in 0..7 {
            let row_y = 100.0 + i as f32 * 34.0;
            tag_label_boxes[i] = Box4 {
                x: 34.0,
                y: row_y,
                w: 106.0,
                h: 28.0,
            };
            tag_tile_boxes[i] = Box4 {
                x: 146.0,
                y: row_y,
                w: 334.0,
                h: 28.0,
            };
            tag_edit_boxes[i] = Box4 {
                x: 152.0,
                y: row_y + 2.0,
                w: 306.0,
                h: 24.0,
            };
            tag_dot_boxes[i] = Box4 {
                x: 146.0 + 334.0 - 16.0,
                y: row_y + 11.0,
                w: 6.0,
                h: 6.0,
            };
        }

        let file_header_box = Box4 {
            x: 34.0,
            y: 344.0,
            w: 200.0,
            h: 20.0,
        };
        let mut file_row_boxes = [Box4 { x: 0.0, y: 0.0, w: 0.0, h: 0.0 }; 4];
        let mut file_label_boxes = [Box4 { x: 0.0, y: 0.0, w: 0.0, h: 0.0 }; 4];
        let mut file_value_boxes = [Box4 { x: 0.0, y: 0.0, w: 0.0, h: 0.0 }; 4];

        for j in 0..4 {
            let file_y = 368.0 + j as f32 * 26.0;
            file_row_boxes[j] = Box4 {
                x: 30.0,
                y: file_y,
                w: 454.0,
                h: 24.0,
            };
            file_label_boxes[j] = Box4 {
                x: 38.0,
                y: file_y,
                w: 100.0,
                h: 24.0,
            };
            file_value_boxes[j] = Box4 {
                x: 146.0,
                y: file_y,
                w: 330.0,
                h: 24.0,
            };
        }

        let sidebar_fields_card_box = Box4 {
            x: 510.0,
            y: 20.0,
            w: 290.0,
            h: 84.0,
        };
        let sidebar_fields_title_box = Box4 {
            x: 524.0,
            y: 30.0,
            w: 262.0,
            h: 20.0,
        };
        let sidebar_fields_text_box = Box4 {
            x: 524.0,
            y: 52.0,
            w: 262.0,
            h: 44.0,
        };
        let undo_button_box = Box4 {
            x: 524.0,
            y: 72.0,
            w: 110.0,
            h: 24.0,
        };

        let sidebar_remove_card_box = Box4 {
            x: 510.0,
            y: 114.0,
            w: 290.0,
            h: 180.0,
        };
        let sidebar_remove_title_box = Box4 {
            x: 524.0,
            y: 124.0,
            w: 262.0,
            h: 20.0,
        };
        let remove_all_check_box = Box4 {
            x: 524.0,
            y: 148.0,
            w: 262.0,
            h: 24.0,
        };
        let remove_sensitive_check_box = Box4 {
            x: 524.0,
            y: 176.0,
            w: 262.0,
            h: 32.0,
        };
        let sidebar_remove_hint_box = Box4 {
            x: 524.0,
            y: 212.0,
            w: 262.0,
            h: 72.0,
        };

        let sidebar_summary_card_box = Box4 {
            x: 510.0,
            y: 304.0,
            w: 290.0,
            h: 64.0,
        };
        let sidebar_summary_title_box = Box4 {
            x: 524.0,
            y: 314.0,
            w: 262.0,
            h: 20.0,
        };
        let sidebar_summary_text_box = Box4 {
            x: 524.0,
            y: 338.0,
            w: 262.0,
            h: 20.0,
        };

        let hairline_bottom_box = Box4 {
            x: 0.0,
            y: 504.0,
            w: 820.0,
            h: 1.0,
        };
        let footer_icon_box = Box4 {
            x: 20.0,
            y: 522.0,
            w: 18.0,
            h: 18.0,
        };
        let footer_file_box = Box4 {
            x: 44.0,
            y: 520.0,
            w: 340.0,
            h: 24.0,
        };
        let footer_helper_box = Box4 {
            x: 20.0,
            y: 516.0,
            w: 350.0,
            h: 32.0,
        };
        let footer_cancel_box = Box4 {
            x: 613.0,
            y: 516.0,
            w: 75.0,
            h: 32.0,
        };
        let footer_save_box = Box4 {
            x: 700.0,
            y: 516.0,
            w: 100.0,
            h: 32.0,
        };

        Self {
            card_w,
            card_h,
            left_card_box,
            search_tile_box,
            search_field_box,
            hairline_search_box,
            fields_header_box,
            tag_label_boxes,
            tag_tile_boxes,
            tag_edit_boxes,
            tag_dot_boxes,
            file_header_box,
            file_row_boxes,
            file_label_boxes,
            file_value_boxes,
            sidebar_fields_card_box,
            sidebar_fields_title_box,
            sidebar_fields_text_box,
            undo_button_box,
            sidebar_remove_card_box,
            sidebar_remove_title_box,
            remove_all_check_box,
            remove_sensitive_check_box,
            sidebar_remove_hint_box,
            sidebar_summary_card_box,
            sidebar_summary_title_box,
            sidebar_summary_text_box,
            hairline_bottom_box,
            footer_icon_box,
            footer_file_box,
            footer_helper_box,
            footer_cancel_box,
            footer_save_box,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Focus {
    Search,
    Tag(usize),
    Undo,
    RemoveAll,
    RemoveSensitive,
    Cancel,
    Save,
}

#[derive(Clone, Debug)]
struct MetadataState {
    files: Vec<PathBuf>,
    file_name: String,
    created_str: String,
    modified_str: String,
    size_str: String,
    kind: Option<Kind>,
    is_pdf: bool,
    tags: HashMap<MetadataTag, String>,
    original_tags: HashMap<MetadataTag, String>,
    active_tag_defs: Vec<MetadataTag>,
    fields: Vec<MetadataField>,
    remove_all: bool,
    remove_sensitive: bool,
    search_text: String,
    focus: Focus,
    fade_alpha: u8,
}

impl Default for MetadataState {
    fn default() -> Self {
        Self {
            files: Vec::new(),
            file_name: String::new(),
            created_str: String::new(),
            modified_str: String::new(),
            size_str: String::new(),
            kind: None,
            is_pdf: false,
            tags: HashMap::new(),
            original_tags: HashMap::new(),
            active_tag_defs: Vec::new(),
            fields: Vec::new(),
            remove_all: false,
            remove_sensitive: false,
            search_text: String::new(),
            focus: Focus::Tag(0),
            fade_alpha: 0,
        }
    }
}

thread_local! {
    static HOST: RefCell<Option<MetadataHost>> = const { RefCell::new(None) };
    static STATE: RefCell<MetadataState> = RefCell::new(MetadataState::default());
    static WINDOW: Cell<isize> = const { Cell::new(0) };
    static SEARCH_FIELD: Cell<isize> = const { Cell::new(0) };
    static TAG_FIELDS: Cell<[isize; 7]> = const { Cell::new([0; 7]) };
}

fn window_handle() -> HWND {
    HWND(WINDOW.with(|slot| slot.get()) as *mut _)
}

fn search_field_handle() -> HWND {
    HWND(SEARCH_FIELD.with(|slot| slot.get()) as *mut _)
}

fn tag_field_handle(idx: usize) -> HWND {
    HWND(TAG_FIELDS.with(|slot| slot.get()[idx]) as *mut _)
}

unsafe fn interface_font(window: HWND) -> HFONT {
    let mut logfont = LOGFONTW::default();
    let name = to_wide("Segoe UI");
    for (i, &c) in name.iter().take(31).enumerate() {
        logfont.lfFaceName[i] = c;
    }
    let dpi = (scale_factor(window) * 96.0).round() as i32;
    logfont.lfHeight = -((13 * dpi) / 72);
    unsafe { CreateFontIndirectW(&logfont) }
}

struct MetadataHost {
    window: HWND,
    search_edit: HWND,
    tag_edits: [HWND; 7],
    sender: Sender<ToolEvent>,
    card: MetadataCard,
    software: Rc<MinimalSoftwareWindow>,
    width: u32,
    height: u32,
    buffer: Vec<Bgra8>,
}

impl MetadataHost {
    fn draw(&mut self) {
        if self.width == 0 || self.height == 0 {
            self.resize();
        }
        if self.width == 0 || self.height == 0 {
            return;
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
        crate::platform::windows::windowhost::blit_card_buffer(
            self.window,
            &self.buffer,
            self.width,
            self.height,
        );
    }

    fn sync_to_card(&mut self, state: &MetadataState, scale: f32) {
        let layout = Layout::of(scale);

        let card = &mut self.card;
        card.set_ui_scale(scale);
        card.set_card_width(layout.card_w * scale);
        card.set_card_height(layout.card_h * scale);
        card.set_search_text(state.search_text.clone().into());
        card.set_search_focused(state.focus == Focus::Search);

        let tag_count = state.active_tag_defs.len();
        card.set_tag_count(tag_count as i32);

        let mut changed_tags: Vec<String> = Vec::new();
        for (tag, val) in &state.tags {
            let orig = state.original_tags.get(tag).map(|s| s.trim()).unwrap_or("");
            if val.trim() != orig {
                changed_tags.push(tag.label(state.kind, state.is_pdf).to_string());
            }
        }
        changed_tags.sort();
        let has_changes = !changed_tags.is_empty();

        // Tag rows bindings
        for i in 0..7 {
            if i < tag_count {
                let tag = state.active_tag_defs[i];
                let label = tag.label(state.kind, state.is_pdf);
                let val = state.tags.get(&tag).cloned().unwrap_or_default();
                let orig = state.original_tags.get(&tag).cloned().unwrap_or_default();
                let placeholder = tag.placeholder(state.kind);
                let changed = val.trim() != orig.trim();
                let focused = state.focus == Focus::Tag(i);

                match i {
                    0 => {
                        card.set_tag_0_label(label.into());
                        card.set_tag_0_value(val.into());
                        card.set_tag_0_placeholder(placeholder.into());
                        card.set_tag_0_changed(changed);
                        card.set_tag_0_focused(focused);
                    }
                    1 => {
                        card.set_tag_1_label(label.into());
                        card.set_tag_1_value(val.into());
                        card.set_tag_1_placeholder(placeholder.into());
                        card.set_tag_1_changed(changed);
                        card.set_tag_1_focused(focused);
                    }
                    2 => {
                        card.set_tag_2_label(label.into());
                        card.set_tag_2_value(val.into());
                        card.set_tag_2_placeholder(placeholder.into());
                        card.set_tag_2_changed(changed);
                        card.set_tag_2_focused(focused);
                    }
                    3 => {
                        card.set_tag_3_label(label.into());
                        card.set_tag_3_value(val.into());
                        card.set_tag_3_placeholder(placeholder.into());
                        card.set_tag_3_changed(changed);
                        card.set_tag_3_focused(focused);
                    }
                    4 => {
                        card.set_tag_4_label(label.into());
                        card.set_tag_4_value(val.into());
                        card.set_tag_4_placeholder(placeholder.into());
                        card.set_tag_4_changed(changed);
                        card.set_tag_4_focused(focused);
                    }
                    5 => {
                        card.set_tag_5_label(label.into());
                        card.set_tag_5_value(val.into());
                        card.set_tag_5_placeholder(placeholder.into());
                        card.set_tag_5_changed(changed);
                        card.set_tag_5_focused(focused);
                    }
                    6 => {
                        card.set_tag_6_label(label.into());
                        card.set_tag_6_value(val.into());
                        card.set_tag_6_placeholder(placeholder.into());
                        card.set_tag_6_changed(changed);
                        card.set_tag_6_focused(focused);
                    }
                    _ => {}
                }
            }
        }

        // File rows bindings
        card.set_file_0_value(state.created_str.clone().into());
        card.set_file_1_value(state.modified_str.clone().into());
        card.set_file_2_value(state.file_name.clone().into());
        card.set_file_3_value(state.size_str.clone().into());

        // Sidebar bindings
        let desc = if has_changes {
            format!(
                "{} field{} changed: {}.",
                changed_tags.len(),
                if changed_tags.len() == 1 { "" } else { "s" },
                changed_tags.join(", ")
            )
        } else {
            "Type in a field to add it or change it; empty it to remove it. Only what you change is written.".to_string()
        };
        card.set_fields_desc(desc.into());
        card.set_has_changes(has_changes);
        card.set_undo_focused(state.focus == Focus::Undo);

        card.set_remove_all(state.remove_all);
        card.set_remove_all_focused(state.focus == Focus::RemoveAll);
        card.set_remove_sensitive(state.remove_sensitive);
        card.set_remove_sensitive_focused(state.focus == Focus::RemoveSensitive);
        card.set_remove_sensitive_disabled(state.remove_all);

        let sensitive_count = state.fields.iter().filter(|f| f.sensitive).count();
        let summary = format!("{} fields \u{b7} {} sensitive", state.fields.len(), sensitive_count);
        card.set_summary_text(summary.into());

        // Footer bindings
        card.set_footer_file_name(state.file_name.clone().into());
        let file_count = state.files.len();
        let save_title = if file_count <= 1 {
            "Save copy".to_string()
        } else {
            format!("Apply to {file_count} files")
        };
        card.set_save_title(save_title.into());
        let save_enabled = state.remove_all || state.remove_sensitive || has_changes;
        card.set_save_enabled(save_enabled);
        let bf = match state.focus {
            Focus::Cancel => 1,
            Focus::Save => 2,
            _ => 0,
        };
        card.set_button_focus(bf);

        // Panels
        card.set_left_card_box(to_panel(layout.left_card_box, scale));
        card.set_search_tile_box(to_panel(layout.search_tile_box, scale));
        card.set_hairline_search_box(to_panel(layout.hairline_search_box, scale));
        card.set_fields_header_box(to_panel(layout.fields_header_box, scale));

        card.set_tag_0_label_box(to_panel(layout.tag_label_boxes[0], scale));
        card.set_tag_0_tile_box(to_panel(layout.tag_tile_boxes[0], scale));
        card.set_tag_0_dot_box(to_panel(layout.tag_dot_boxes[0], scale));

        card.set_tag_1_label_box(to_panel(layout.tag_label_boxes[1], scale));
        card.set_tag_1_tile_box(to_panel(layout.tag_tile_boxes[1], scale));
        card.set_tag_1_dot_box(to_panel(layout.tag_dot_boxes[1], scale));

        card.set_tag_2_label_box(to_panel(layout.tag_label_boxes[2], scale));
        card.set_tag_2_tile_box(to_panel(layout.tag_tile_boxes[2], scale));
        card.set_tag_2_dot_box(to_panel(layout.tag_dot_boxes[2], scale));

        card.set_tag_3_label_box(to_panel(layout.tag_label_boxes[3], scale));
        card.set_tag_3_tile_box(to_panel(layout.tag_tile_boxes[3], scale));
        card.set_tag_3_dot_box(to_panel(layout.tag_dot_boxes[3], scale));

        card.set_tag_4_label_box(to_panel(layout.tag_label_boxes[4], scale));
        card.set_tag_4_tile_box(to_panel(layout.tag_tile_boxes[4], scale));
        card.set_tag_4_dot_box(to_panel(layout.tag_dot_boxes[4], scale));

        card.set_tag_5_label_box(to_panel(layout.tag_label_boxes[5], scale));
        card.set_tag_5_tile_box(to_panel(layout.tag_tile_boxes[5], scale));
        card.set_tag_5_dot_box(to_panel(layout.tag_dot_boxes[5], scale));

        card.set_tag_6_label_box(to_panel(layout.tag_label_boxes[6], scale));
        card.set_tag_6_tile_box(to_panel(layout.tag_tile_boxes[6], scale));
        card.set_tag_6_dot_box(to_panel(layout.tag_dot_boxes[6], scale));

        card.set_file_header_box(to_panel(layout.file_header_box, scale));
        card.set_file_0_row_box(to_panel(layout.file_row_boxes[0], scale));
        card.set_file_0_label_box(to_panel(layout.file_label_boxes[0], scale));
        card.set_file_0_value_box(to_panel(layout.file_value_boxes[0], scale));

        card.set_file_1_row_box(to_panel(layout.file_row_boxes[1], scale));
        card.set_file_1_label_box(to_panel(layout.file_label_boxes[1], scale));
        card.set_file_1_value_box(to_panel(layout.file_value_boxes[1], scale));

        card.set_file_2_row_box(to_panel(layout.file_row_boxes[2], scale));
        card.set_file_2_label_box(to_panel(layout.file_label_boxes[2], scale));
        card.set_file_2_value_box(to_panel(layout.file_value_boxes[2], scale));

        card.set_file_3_row_box(to_panel(layout.file_row_boxes[3], scale));
        card.set_file_3_label_box(to_panel(layout.file_label_boxes[3], scale));
        card.set_file_3_value_box(to_panel(layout.file_value_boxes[3], scale));

        card.set_sidebar_fields_box(to_panel(layout.sidebar_fields_card_box, scale));
        card.set_sidebar_fields_title_box(to_panel(layout.sidebar_fields_title_box, scale));
        card.set_sidebar_fields_text_box(to_panel(layout.sidebar_fields_text_box, scale));
        card.set_undo_button_box(to_panel(layout.undo_button_box, scale));

        card.set_sidebar_remove_box(to_panel(layout.sidebar_remove_card_box, scale));
        card.set_sidebar_remove_title_box(to_panel(layout.sidebar_remove_title_box, scale));
        card.set_remove_all_box(to_panel(layout.remove_all_check_box, scale));
        card.set_remove_sensitive_box(to_panel(layout.remove_sensitive_check_box, scale));
        card.set_sidebar_remove_hint_box(to_panel(layout.sidebar_remove_hint_box, scale));

        card.set_sidebar_summary_box(to_panel(layout.sidebar_summary_card_box, scale));
        card.set_sidebar_summary_title_box(to_panel(layout.sidebar_summary_title_box, scale));
        card.set_sidebar_summary_text_box(to_panel(layout.sidebar_summary_text_box, scale));

        card.set_hairline_bottom_box(to_panel(layout.hairline_bottom_box, scale));
        card.set_footer_icon_box(to_panel(layout.footer_icon_box, scale));
        card.set_footer_file_box(to_panel(layout.footer_file_box, scale));
        card.set_footer_helper_box(to_panel(layout.footer_helper_box, scale));
        card.set_footer_cancel_box(to_panel(layout.footer_cancel_box, scale));
        card.set_footer_save_box(to_panel(layout.footer_save_box, scale));

        // Sync native edit controls
        unsafe {
            // Search edit
            let sx = (layout.search_field_box.x * scale).round() as i32;
            let sy = (layout.search_field_box.y * scale).round() as i32;
            let sw = (layout.search_field_box.w * scale).round() as i32;
            let sh = (layout.search_field_box.h * scale).round() as i32;
            let _ = SetWindowPos(
                self.search_edit,
                None,
                sx,
                sy,
                sw,
                sh,
                SWP_NOZORDER | SWP_NOACTIVATE | SWP_SHOWWINDOW,
            );

            // Tag edits
            for i in 0..7 {
                let edit = self.tag_edits[i];
                if i < tag_count {
                    let tx = (layout.tag_edit_boxes[i].x * scale).round() as i32;
                    let ty = (layout.tag_edit_boxes[i].y * scale).round() as i32;
                    let tw = (layout.tag_edit_boxes[i].w * scale).round() as i32;
                    let th = (layout.tag_edit_boxes[i].h * scale).round() as i32;
                    let _ = SetWindowPos(
                        edit,
                        None,
                        tx,
                        ty,
                        tw,
                        th,
                        SWP_NOZORDER | SWP_NOACTIVATE | SWP_SHOWWINDOW,
                    );
                    let _ = EnableWindow(edit, !state.remove_all);
                } else {
                    let _ = ShowWindow(edit, SW_HIDE);
                }
            }
        }
    }

    fn resize(&mut self) {
        let mut rect = RECT::default();
        let _ = unsafe { GetClientRect(self.window, &mut rect) };
        let w = (rect.right - rect.left).max(1) as u32;
        let h = (rect.bottom - rect.top).max(1) as u32;
        if w != self.width || h != self.height {
            self.width = w;
            self.height = h;
            self.buffer = vec![Bgra8::default(); (w * h) as usize];
            let scale = scale_factor(self.window);
            self.card.set_card_width(w as f32 / scale);
            self.card.set_card_height(h as f32 / scale);
            self.software.set_size(PhysicalSize::new(w, h));
            self.software.request_redraw();
        }
    }
}

pub struct MetadataWindow {
    pub window: HWND,
}

impl MetadataWindow {
    pub fn new(sender: Sender<ToolEvent>) -> Result<Self, String> {
        let instance: HMODULE = unsafe { GetModuleHandleW(PCWSTR::null()) }
            .map_err(|e| format!("GetModuleHandleW: {e:?}"))?;

        let class_name = to_wide("OpenTangerineMetadataWindow");
        let bg_brush = unsafe { CreateSolidBrush(COLORREF(0x00F4F6F7)) };
        let wc = WNDCLASSW {
            lpfnWndProc: Some(window_procedure),
            hInstance: instance.into(),
            lpszClassName: PCWSTR(class_name.as_ptr()),
            hCursor: unsafe { LoadCursorW(None, IDC_ARROW).unwrap_or_default() },
            hbrBackground: bg_brush,
            ..Default::default()
        };
        unsafe {
            RegisterClassW(&wc);
        }

        let software = window_for_next_component();
        let card = MetadataCard::new().map_err(|e| e.to_string())?;

        let window = unsafe {
            CreateWindowExW(
                WS_EX_LAYERED,
                PCWSTR(class_name.as_ptr()),
                PCWSTR(to_wide("Edit metadata").as_ptr()),
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
            .map_err(|e| format!("CreateWindowExW failed: {e:?}"))?
        };

        let field_font = unsafe { interface_font(window) };

        let make_field = |id: isize| unsafe {
            let edit = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("EDIT"),
                w!(""),
                WS_CHILD | WS_TABSTOP,
                0,
                0,
                100,
                24,
                Some(window),
                Some(HMENU(id as *mut _)),
                Some(instance.into()),
                None,
            )
            .unwrap_or_default();
            SendMessageW(edit, WM_SETFONT, Some(WPARAM(field_font.0 as usize)), Some(LPARAM(1)));
            let _ = SetWindowSubclass(edit, Some(field_procedure), id as usize, 0);
            edit
        };

        let search_edit = make_field(100);
        let tag_edits = [
            make_field(101),
            make_field(102),
            make_field(103),
            make_field(104),
            make_field(105),
            make_field(106),
            make_field(107),
        ];

        WINDOW.with(|slot| slot.set(window.0 as isize));
        SEARCH_FIELD.with(|slot| slot.set(search_edit.0 as isize));
        TAG_FIELDS.with(|slot| {
            slot.set([
                tag_edits[0].0 as isize,
                tag_edits[1].0 as isize,
                tag_edits[2].0 as isize,
                tag_edits[3].0 as isize,
                tag_edits[4].0 as isize,
                tag_edits[5].0 as isize,
                tag_edits[6].0 as isize,
            ])
        });

        // Slint callbacks
        {
            card.on_search_clicked(move || {
                let w = window_handle();
                if !w.0.is_null() {
                    STATE.with(|slot| slot.borrow_mut().focus = Focus::Search);
                    unsafe { let _ = SetFocus(Some(search_field_handle())); }
                    redraw(w);
                }
            });

            card.on_tag_clicked(move |idx| {
                let w = window_handle();
                if !w.0.is_null() {
                    let u = idx as usize;
                    STATE.with(|slot| slot.borrow_mut().focus = Focus::Tag(u));
                    if u < 7 {
                        unsafe { let _ = SetFocus(Some(tag_field_handle(u))); }
                    }
                    redraw(w);
                }
            });

            card.on_undo_clicked(move || {
                let w = window_handle();
                if !w.0.is_null() {
                    undo_changes(w);
                }
            });

            card.on_remove_all_toggled(move || {
                let w = window_handle();
                if !w.0.is_null() {
                    toggle_remove_all(w);
                }
            });

            card.on_remove_sensitive_toggled(move || {
                let w = window_handle();
                if !w.0.is_null() {
                    toggle_remove_sensitive(w);
                }
            });

            card.on_cancel_chosen(move || {
                let w = window_handle();
                if !w.0.is_null() {
                    unsafe { cancel(w) };
                }
            });

            card.on_save_chosen(move || {
                let w = window_handle();
                if !w.0.is_null() {
                    unsafe { run(w) };
                }
            });
        }

        let host = MetadataHost {
            window,
            search_edit,
            tag_edits,
            sender: sender.clone(),
            card,
            software,
            width: 0,
            height: 0,
            buffer: Vec::new(),
        };

        HOST.with(|slot| *slot.borrow_mut() = Some(host));

        Ok(Self { window })
    }

    pub fn show_for(&mut self, _op: &str, _title: &str, _detail: &str, files: &[PathBuf]) {
        if files.is_empty() {
            return;
        }

        let primary = &files[0];
        let file_name = primary
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();

        let ext = primary
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();

        let is_pdf = ext == "pdf";
        let kind = match ext.as_str() {
            "jpg" | "jpeg" | "png" | "webp" | "gif" | "bmp" | "tiff" | "tif" | "heic" | "avif" => {
                Some(Kind::Image)
            }
            "mp4" | "mov" | "m4v" | "mkv" | "webm" | "avi" => Some(Kind::Video),
            "mp3" | "wav" | "m4a" | "aac" | "flac" | "ogg" => Some(Kind::Audio),
            _ => None,
        };

        let fs_meta = std::fs::metadata(primary).ok();
        let size_str = fs_meta
            .as_ref()
            .map(|m| format_file_size(m.len()))
            .unwrap_or_else(|| "0 B".to_string());

        let created_str = fs_meta
            .as_ref()
            .and_then(|m| m.created().ok())
            .map(format_system_time)
            .unwrap_or_else(|| "Unknown".to_string());

        let modified_str = fs_meta
            .as_ref()
            .and_then(|m| m.modified().ok())
            .map(format_system_time)
            .unwrap_or_else(|| "Unknown".to_string());

        let tools = crate::media::locator::find();
        let fields = crate::media::metadata::read_fields(primary, tools.as_ref());
        let tags = crate::media::metadata::read_tags(primary, tools.as_ref());
        let active_tag_defs = MetadataTag::for_kind(kind, is_pdf);

        STATE.with(|slot| {
            let mut st = slot.borrow_mut();
            st.files = files.to_vec();
            st.file_name = file_name.clone();
            st.created_str = created_str;
            st.modified_str = modified_str;
            st.size_str = size_str;
            st.kind = kind;
            st.is_pdf = is_pdf;
            st.tags = tags.clone();
            st.original_tags = tags.clone();
            st.active_tag_defs = active_tag_defs.to_vec();
            st.fields = fields;
            st.remove_all = false;
            st.remove_sensitive = false;
            st.search_text.clear();
            st.focus = Focus::Tag(0);
            st.fade_alpha = 0;
        });

        // Populate native tag edit controls text without holding HOST borrow
        let search_handle = search_field_handle();
        let search_hint = to_wide("Search everything in the file");
        let empty_wide = to_wide("");
        unsafe {
            let _ = SetWindowTextW(search_handle, PCWSTR(empty_wide.as_ptr()));
            let _ = SendMessageW(
                search_handle,
                EM_SETCUEBANNER,
                Some(WPARAM(1)),
                Some(LPARAM(search_hint.as_ptr() as isize)),
            );
            for i in 0..7 {
                let edit = tag_field_handle(i);
                let text = if i < active_tag_defs.len() {
                    let tag = active_tag_defs[i];
                    tags.get(&tag).cloned().unwrap_or_default()
                } else {
                    String::new()
                };
                let placeholder = if i < active_tag_defs.len() {
                    active_tag_defs[i].placeholder(kind)
                } else {
                    ""
                };
                let wide_text = to_wide(&text);
                let wide_ph = to_wide(placeholder);
                let _ = SetWindowTextW(edit, PCWSTR(wide_text.as_ptr()));
                let _ = SendMessageW(
                    edit,
                    EM_SETCUEBANNER,
                    Some(WPARAM(1)),
                    Some(LPARAM(wide_ph.as_ptr() as isize)),
                );
            }
        }

        let scale = scale_factor(self.window);
        let dpi = (96.0 * scale).round() as u32;
        let mut rect = RECT {
            left: 0,
            top: 0,
            right: (CARD_W as f32 * scale).round() as i32,
            bottom: (CARD_H as f32 * scale).round() as i32,
        };
        let _ = unsafe {
            AdjustWindowRectExForDpi(&mut rect, fixed_style(), false, WINDOW_EX_STYLE::default(), dpi)
        };
        let width = rect.right - rect.left;
        let height = rect.bottom - rect.top;

        unsafe {
            let _ = SetWindowPos(
                self.window,
                None,
                0,
                0,
                width,
                height,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
            let _ = SetLayeredWindowAttributes(self.window, COLORREF(0), 0, LWA_ALPHA);
            let _ = ShowWindow(self.window, SW_SHOW);
            let _ = SetForegroundWindow(self.window);
            let _ = SetTimer(Some(self.window), FADE_TIMER_ID, 16, None);
        }

        redraw(self.window);

        // Set initial focus to first tag edit control
        if !active_tag_defs.is_empty() {
            unsafe { let _ = SetFocus(Some(tag_field_handle(0))); }
        }
    }
}

fn redraw(window: HWND) {
    let scale = scale_factor(window);
    let state = STATE.with(|slot| slot.borrow().clone());
    HOST.with(|slot| {
        if let Some(host) = slot.borrow_mut().as_mut() {
            host.resize();
            host.sync_to_card(&state, scale);
            host.software.request_redraw();
            host.draw();
        }
    });
    unsafe {
        let _ = InvalidateRect(Some(window), None, true);
    }
}

fn undo_changes(window: HWND) {
    let (original, active_defs) = STATE.with(|slot| {
        let mut st = slot.borrow_mut();
        st.tags = st.original_tags.clone();
        (st.tags.clone(), st.active_tag_defs.clone())
    });

    for i in 0..active_defs.len().min(7) {
        let tag = active_defs[i];
        let text = original.get(&tag).cloned().unwrap_or_default();
        let edit = tag_field_handle(i);
        let wide = to_wide(&text);
        unsafe {
            let _ = SetWindowTextW(edit, PCWSTR(wide.as_ptr()));
        }
    }

    redraw(window);
}

fn toggle_remove_all(window: HWND) {
    STATE.with(|slot| {
        let mut st = slot.borrow_mut();
        st.remove_all = !st.remove_all;
        if st.remove_all {
            st.remove_sensitive = false;
        }
    });
    redraw(window);
}

fn toggle_remove_sensitive(window: HWND) {
    STATE.with(|slot| {
        let mut st = slot.borrow_mut();
        if !st.remove_all {
            st.remove_sensitive = !st.remove_sensitive;
        }
    });
    redraw(window);
}

unsafe fn run(window: HWND) {
    let (files, tags, original_tags, remove_all, remove_sensitive) = STATE.with(|slot| {
        let st = slot.borrow();
        (
            st.files.clone(),
            st.tags.clone(),
            st.original_tags.clone(),
            st.remove_all,
            st.remove_sensitive,
        )
    });

    let mut changed_metadata: Vec<(String, String)> = Vec::new();
    if !remove_all {
        for (tag, value) in tags {
            let orig = original_tags.get(&tag).map(|s| s.trim()).unwrap_or("");
            if value.trim() != orig {
                changed_metadata.push((tag.ffmpeg_key().to_string(), value.trim().to_string()));
            }
        }
    }

    let save_enabled = remove_all || remove_sensitive || !changed_metadata.is_empty();
    if !save_enabled {
        return;
    }

    let settings = ToolSettings {
        metadata_tags: changed_metadata,
        metadata_remove_all: remove_all,
        metadata_remove_sensitive: remove_sensitive,
        ..Default::default()
    };

    let sender = HOST.with(|slot| slot.borrow().as_ref().map(|h| h.sender.clone()));
    if let Some(sender) = sender {
        let _ = sender.send(ToolEvent::Run {
            operation: "tools.metadata".to_string(),
            inputs: files,
            settings,
        });
    }

    unsafe {
        let _ = ShowWindow(window, SW_HIDE);
    }
}

unsafe fn cancel(window: HWND) {
    let sender = HOST.with(|slot| slot.borrow().as_ref().map(|h| h.sender.clone()));
    if let Some(sender) = sender {
        let _ = sender.send(ToolEvent::Cancelled);
    }
    unsafe {
        let _ = ShowWindow(window, SW_HIDE);
    }
}

unsafe extern "system" fn field_procedure(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    id: usize,
    _subclass_data: usize,
) -> LRESULT {
    match message {
        WM_PAINT => {
            let res = unsafe { DefSubclassProc(window, message, wparam, lparam) };
            let len = unsafe { GetWindowTextLengthW(window) };
            if len == 0 {
                let focus_hwnd = unsafe { GetFocus() };
                let is_focused = focus_hwnd == window;
                let ph: Option<String> = if id == 100 {
                    Some("Search everything in the file".to_string())
                } else if id >= 101 && id <= 107 {
                    let idx = id - 101;
                    STATE.with(|s| {
                        let st = s.borrow();
                        if idx < st.active_tag_defs.len() {
                            let p = st.active_tag_defs[idx].placeholder(st.kind);
                            if !p.is_empty() {
                                Some(p.to_string())
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    })
                } else {
                    None
                };

                if let Some(text) = ph {
                    let mut rect = RECT::default();
                    unsafe {
                        let _ = GetClientRect(window, &mut rect);
                        let dc = GetDC(Some(window));
                        let font = interface_font(window_handle());
                        let old_font = SelectObject(dc, font.into());
                        let _ = SetTextColor(dc, COLORREF(0x008E8E93));
                        let _ = SetBkMode(dc, TRANSPARENT);
                        let mut wide = to_wide(&text);
                        rect.left += if is_focused { 5 } else { 2 };
                        let _ = DrawTextW(
                            dc,
                            &mut wide[..text.encode_utf16().count()],
                            &mut rect,
                            DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX,
                        );
                        let _ = SelectObject(dc, old_font);
                        let _ = ReleaseDC(Some(window), dc);
                    }
                }
            }
            return res;
        }
        windows::Win32::UI::WindowsAndMessaging::WM_SETFOCUS
        | windows::Win32::UI::WindowsAndMessaging::WM_KILLFOCUS => {
            let res = unsafe { DefSubclassProc(window, message, wparam, lparam) };
            let _ = unsafe { InvalidateRect(Some(window), None, true) };
            return res;
        }
        WM_KEYDOWN => {
            let key = wparam.0 as i32;
            let parent = window_handle();
            if key == VK_RETURN.0 as i32 {
                unsafe { run(parent) };
                return LRESULT(0);
            }
            if key == VK_ESCAPE.0 as i32 {
                unsafe { cancel(parent) };
                return LRESULT(0);
            }
            if key == VK_TAB.0 as i32 {
                let shift = unsafe { GetKeyState(VK_SHIFT.0 as i32) } < 0;
                step_focus(parent, shift);
                return LRESULT(0);
            }
            if key == VK_UP.0 as i32 {
                if id > 101 && id <= 107 {
                    let prev = id - 1;
                    unsafe { let _ = SetFocus(Some(tag_field_handle(prev - 101))); }
                    STATE.with(|s| s.borrow_mut().focus = Focus::Tag(prev - 101));
                    redraw(parent);
                    return LRESULT(0);
                }
            }
            if key == VK_DOWN.0 as i32 {
                if id >= 101 && id < 107 {
                    let active_len = STATE.with(|s| s.borrow().active_tag_defs.len());
                    if id - 101 + 1 < active_len {
                        let next = id + 1;
                        unsafe { let _ = SetFocus(Some(tag_field_handle(next - 101))); }
                        STATE.with(|s| s.borrow_mut().focus = Focus::Tag(next - 101));
                        redraw(parent);
                        return LRESULT(0);
                    }
                }
            }
        }
        _ => {}
    }
    unsafe { DefSubclassProc(window, message, wparam, lparam) }
}

fn step_focus(window: HWND, reverse: bool) {
    let (focus, tag_count, has_changes, remove_all) = STATE.with(|slot| {
        let st = slot.borrow();
        (
            st.focus,
            st.active_tag_defs.len(),
            st.tags.iter().any(|(k, v)| {
                v.trim() != st.original_tags.get(k).map(|s| s.trim()).unwrap_or("")
            }),
            st.remove_all,
        )
    });

    let mut sequence = vec![Focus::Search];
    for i in 0..tag_count {
        if !remove_all {
            sequence.push(Focus::Tag(i));
        }
    }
    if has_changes {
        sequence.push(Focus::Undo);
    }
    sequence.push(Focus::RemoveAll);
    if !remove_all {
        sequence.push(Focus::RemoveSensitive);
    }
    sequence.push(Focus::Cancel);
    sequence.push(Focus::Save);

    let current_idx = sequence.iter().position(|f| *f == focus).unwrap_or(0);
    let next_idx = if reverse {
        if current_idx == 0 {
            sequence.len().saturating_sub(1)
        } else {
            current_idx - 1
        }
    } else {
        (current_idx + 1) % sequence.len()
    };

    let next_focus = sequence[next_idx];
    STATE.with(|slot| slot.borrow_mut().focus = next_focus);

    match next_focus {
        Focus::Search => unsafe { let _ = SetFocus(Some(search_field_handle())); },
        Focus::Tag(i) => unsafe {
            if i < 7 {
                let _ = SetFocus(Some(tag_field_handle(i)));
            }
        },
        _ => unsafe { let _ = SetFocus(Some(window)); },
    }

    redraw(window);
}

unsafe extern "system" fn window_procedure(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_TIMER => {
            if wparam.0 == FADE_TIMER_ID {
                let alpha = STATE.with(|slot| {
                    let mut st = slot.borrow_mut();
                    st.fade_alpha = st.fade_alpha.saturating_add(51);
                    st.fade_alpha
                });
                let _ = unsafe { SetLayeredWindowAttributes(window, COLORREF(0), alpha, LWA_ALPHA) };
                if alpha == 255 {
                    let _ = unsafe { KillTimer(Some(window), FADE_TIMER_ID) };
                }
                return LRESULT(0);
            }
        }
        WM_ERASEBKGND => return LRESULT(1),
        WM_SIZE => {
            HOST.with(|slot| {
                if let Some(host) = slot.borrow_mut().as_mut() {
                    host.resize();
                    host.software.request_redraw();
                }
            });
            redraw(window);
            return LRESULT(0);
        }
        WM_PAINT => {
            let mut paint = PAINTSTRUCT::default();
            let _dc = unsafe { BeginPaint(window, &mut paint) };
            HOST.with(|slot| {
                if let Some(host) = slot.borrow_mut().as_mut() {
                    host.resize();
                    host.software.request_redraw();
                    host.draw();
                }
            });
            let _ = unsafe { EndPaint(window, &paint) };
            return LRESULT(0);
        }
        WM_CTLCOLOREDIT => {
            let context = windows::Win32::Graphics::Gdi::HDC(wparam.0 as *mut _);
            let edit_hwnd = HWND(lparam.0 as *mut _);
            let is_search = edit_hwnd == search_field_handle();
            let bg_color = if is_search {
                COLORREF(0x00F7F6F4)
            } else {
                COLORREF(0x00FFFFFF)
            };
            unsafe {
                windows::Win32::Graphics::Gdi::SetTextColor(context, COLORREF(0x001D_1D1F));
                windows::Win32::Graphics::Gdi::SetBkColor(context, bg_color);
                return LRESULT(CreateSolidBrush(bg_color).0 as isize);
            }
        }
        WM_COMMAND => {
            let code = (wparam.0 >> 16) as u16;
            let id = (wparam.0 & 0xFFFF) as u16;
            if code == EN_CHANGE as u16 {
                let edit = windows::Win32::Foundation::HWND(lparam.0 as *mut _);
                let len = unsafe { GetWindowTextLengthW(edit) } as usize;
                let mut buf = vec![0u16; len + 1];
                let read_len = unsafe { GetWindowTextW(edit, &mut buf) } as usize;
                let text = String::from_utf16_lossy(&buf[..read_len]);

                if id == 100 {
                    STATE.with(|s| s.borrow_mut().search_text = text);
                    redraw(window);
                } else if id >= 101 && id <= 107 {
                    let idx = (id - 101) as usize;
                    let tag = STATE.with(|s| {
                        if idx < s.borrow().active_tag_defs.len() {
                            Some(s.borrow().active_tag_defs[idx])
                        } else {
                            None
                        }
                    });
                    if let Some(t) = tag {
                        STATE.with(|s| s.borrow_mut().tags.insert(t, text));
                        redraw(window);
                    }
                }
            }
            return LRESULT(0);
        }
        WM_LBUTTONDOWN => {
            let x = (lparam.0 & 0xFFFF) as i16 as f32;
            let y = ((lparam.0 >> 16) & 0xFFFF) as i16 as f32;
            let scale = scale_factor(window);
            let layout = Layout::of(scale);
            let unscaled_x = x / scale;
            let unscaled_y = y / scale;

            if layout.search_tile_box.contains(unscaled_x, unscaled_y) {
                STATE.with(|s| s.borrow_mut().focus = Focus::Search);
                unsafe { let _ = SetFocus(Some(search_field_handle())); }
                redraw(window);
                return LRESULT(0);
            }

            let (tag_count, remove_all) = STATE.with(|s| (s.borrow().active_tag_defs.len(), s.borrow().remove_all));
            if !remove_all {
                for i in 0..tag_count.min(7) {
                    if layout.tag_tile_boxes[i].contains(unscaled_x, unscaled_y)
                        || layout.tag_label_boxes[i].contains(unscaled_x, unscaled_y)
                    {
                        STATE.with(|s| s.borrow_mut().focus = Focus::Tag(i));
                        unsafe { let _ = SetFocus(Some(tag_field_handle(i))); }
                        redraw(window);
                        return LRESULT(0);
                    }
                }
            }

            let has_changes = STATE.with(|s| {
                s.borrow().tags.iter().any(|(k, v)| {
                    v.trim() != s.borrow().original_tags.get(k).map(|s| s.trim()).unwrap_or("")
                })
            });

            if has_changes && layout.undo_button_box.contains(unscaled_x, unscaled_y) {
                undo_changes(window);
                return LRESULT(0);
            }

            if layout.remove_all_check_box.contains(unscaled_x, unscaled_y) {
                toggle_remove_all(window);
                return LRESULT(0);
            }

            if layout.remove_sensitive_check_box.contains(unscaled_x, unscaled_y) {
                toggle_remove_sensitive(window);
                return LRESULT(0);
            }

            if layout.footer_cancel_box.contains(unscaled_x, unscaled_y) {
                unsafe { cancel(window) };
                return LRESULT(0);
            }

            if layout.footer_save_box.contains(unscaled_x, unscaled_y) {
                unsafe { run(window) };
                return LRESULT(0);
            }

            return LRESULT(0);
        }
        WM_KEYDOWN => {
            let key = wparam.0 as i32;
            if key == VK_ESCAPE.0 as i32 {
                unsafe { cancel(window) };
                return LRESULT(0);
            }
            if key == VK_RETURN.0 as i32 {
                unsafe { run(window) };
                return LRESULT(0);
            }
            if key == VK_TAB.0 as i32 {
                let shift = unsafe { GetKeyState(VK_SHIFT.0 as i32) } < 0;
                step_focus(window, shift);
                return LRESULT(0);
            }
            if key == VK_SPACE.0 as i32 {
                let focus = STATE.with(|s| s.borrow().focus);
                if focus == Focus::RemoveAll {
                    toggle_remove_all(window);
                    return LRESULT(0);
                } else if focus == Focus::RemoveSensitive {
                    toggle_remove_sensitive(window);
                    return LRESULT(0);
                } else if focus == Focus::Undo {
                    undo_changes(window);
                    return LRESULT(0);
                }
            }
        }
        WM_DPICHANGED => {
            let rect = unsafe { &*(lparam.0 as *const RECT) };
            let _ = unsafe {
                SetWindowPos(
                    window,
                    None,
                    rect.left,
                    rect.top,
                    rect.right - rect.left,
                    rect.bottom - rect.top,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                )
            };
            redraw(window);
            return LRESULT(0);
        }
        WM_CLOSE => {
            unsafe { cancel(window) };
            return LRESULT(0);
        }
        _ => {}
    }
    unsafe { DefWindowProcW(window, message, wparam, lparam) }
}

#[cfg(test)]
pub mod show_tests {
    use super::*;
    use std::sync::mpsc::channel;
    use windows::Win32::UI::WindowsAndMessaging::{
        DispatchMessageW, PeekMessageW, TranslateMessage, MSG, PM_REMOVE,
    };

    fn pump_messages(count: usize) {
        for _ in 0..count {
            unsafe {
                let mut msg = MSG::default();
                while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    #[test]
    fn metadata_layout_dimensions_are_pinned() {
        let layout = Layout::of(1.0);
        assert_eq!(layout.card_w, 820.0);
        assert_eq!(layout.card_h, 560.0);
        assert_eq!(layout.left_card_box.w, 474.0);
        assert_eq!(layout.sidebar_fields_card_box.w, 290.0);
        assert_eq!(layout.tag_label_boxes.len(), 7);
    }

    #[test]
    fn showing_and_pumping_the_metadata_card_does_not_panic() {
        crate::ui::wheel::install_platform();
        let (sender, _receiver) = channel();
        let mut window = MetadataWindow::new(sender).expect("the metadata window was made");
        let path = PathBuf::from("photo.png");
        window.show_for("tools.metadata", "Edit metadata", "", &[path]);
        pump_messages(20);
        unsafe { cancel(window.window) };
    }

    #[test]
    #[ignore = "captures the composited metadata window"]
    fn capture_the_metadata_window() {
        crate::ui::wheel::install_platform();
        use crate::platform::windows::windowhost::screenshot::capture;
        let (sender, _receiver) = channel();
        let mut window = MetadataWindow::new(sender).expect("the metadata window was made");

        let out_dir = std::path::Path::new("target").join("shots");
        let _ = std::fs::create_dir_all(&out_dir);
        let test_file = out_dir.join("photo.png");
        let _ = std::fs::write(&test_file, vec![0u8; 1_468_006]);

        window.show_for("tools.metadata", "Edit metadata", "", &[test_file]);

        STATE.with(|slot| {
            let mut st = slot.borrow_mut();
            st.tags.insert(MetadataTag::Date, "2026:10:19 14:30:00".to_string());
            st.original_tags.insert(MetadataTag::Date, "2026:10:19 14:30:00".to_string());
            st.fields = (0..17)
                .map(|i| MetadataField {
                    group: "Exif".to_string(),
                    key: format!("Field {i}"),
                    value: "val".to_string(),
                    sensitive: i < 3,
                })
                .collect();
            st.focus = Focus::Search;
        });

        let date_edit = tag_field_handle(5);
        let date_wide = to_wide("2026:10:19 14:30:00");
        unsafe {
            let _ = SetWindowTextW(date_edit, PCWSTR(date_wide.as_ptr()));
            let _ = SetFocus(Some(search_field_handle()));
        }
        redraw(window.window);

        for _ in 0..100 {
            unsafe {
                let mut msg = MSG::default();
                while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
            let alpha = STATE.with(|s| s.borrow().fade_alpha);
            if alpha == 255 {
                break;
            }
        }
        pump_messages(10);

        let window_shot = out_dir.join("metadata-window.png");
        capture(window.window, &window_shot);
        assert!(window_shot.is_file(), "metadata-window.png was written");

        unsafe { cancel(window.window) };
    }

    /// Audit probe: the same window captured three times inside this one
    /// process, so title bytes of a, b and c say whether the picture race
    /// of the system chrome is per call or per process.
    #[test]
    #[ignore = "captures the same metadata window three times in a row"]
    fn capture_probe_the_metadata_window_thrice() {
        crate::ui::wheel::install_platform();
        use crate::platform::windows::windowhost::screenshot::capture;
        let (sender, _receiver) = channel();
        let mut window = MetadataWindow::new(sender).expect("the metadata window was made");

        let out_dir = std::path::Path::new("target").join("shots");
        let _ = std::fs::create_dir_all(&out_dir);
        let test_file = out_dir.join("photo.png");
        let _ = std::fs::write(&test_file, vec![0u8; 1_468_006]);

        window.show_for("tools.metadata", "Edit metadata", "", &[test_file]);

        STATE.with(|slot| {
            let mut st = slot.borrow_mut();
            st.tags.insert(MetadataTag::Date, "2026:10:19 14:30:00".to_string());
            st.original_tags.insert(MetadataTag::Date, "2026:10:19 14:30:00".to_string());
            st.fields = (0..17)
                .map(|i| MetadataField {
                    group: "Exif".to_string(),
                    key: format!("Field {i}"),
                    value: "val".to_string(),
                    sensitive: i < 3,
                })
                .collect();
            st.focus = Focus::Search;
        });
        redraw(window.window);

        for _ in 0..100 {
            unsafe {
                let mut msg = MSG::default();
                while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
            let alpha = STATE.with(|s| s.borrow().fade_alpha);
            if alpha == 255 {
                break;
            }
        }
        pump_messages(10);

        capture(window.window, &out_dir.join("meta-probe-a.png"));
        pump_messages(5);
        std::thread::sleep(std::time::Duration::from_millis(250));
        capture(window.window, &out_dir.join("meta-probe-b.png"));
        pump_messages(5);
        std::thread::sleep(std::time::Duration::from_millis(500));
        capture(window.window, &out_dir.join("meta-probe-c.png"));
        unsafe { cancel(window.window) };
    }
}
