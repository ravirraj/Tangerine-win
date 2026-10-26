//! The window a file or a batch of files is compressed in.
//!
//! A file does not compress the moment it is picked: it opens a window
//! with the file's facts in front of the person — the name, the weight
//! it carries now and what it will save as — the choice between balanced
//! and strong presets with a hint line, video options when any video is
//! in the batch, a round checkbox for a target file size with its
//! numeric field and unit chips, a footnote line, and Cancel and
//! Compress actions.
//!
//! The card is drawn by the Slint renderer, wearing the shell's
//! skeleton; the hit-testing, keyboard walk and number typing are read
//! by the window itself, which holds the identical geometry.

use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc::Sender;

use slint::platform::software_renderer::MinimalSoftwareWindow;
use slint::PhysicalSize;
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{COLORREF, HMODULE, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreateFontIndirectW, CreateSolidBrush, EndPaint, InvalidateRect,
    HFONT, LOGFONTW, PAINTSTRUCT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, SetFocus, VK_ESCAPE, VK_LEFT, VK_RETURN, VK_RIGHT, VK_SHIFT, VK_SPACE, VK_TAB,
};
use windows::Win32::UI::Shell::{DefSubclassProc, SetWindowSubclass};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, GetClientRect, GetWindowTextW, KillTimer, LoadCursorW,
    RegisterClassW, SendMessageW, SetForegroundWindow, SetLayeredWindowAttributes, SetTimer,
    SetWindowPos, ShowWindow, CW_USEDEFAULT, ES_NUMBER, HMENU, IDC_ARROW, LWA_ALPHA,
    SWP_NOACTIVATE, SWP_NOZORDER, SWP_SHOWWINDOW, SW_HIDE, SW_SHOW, WM_CLOSE, WM_COMMAND,
    WM_CTLCOLOREDIT, WM_DPICHANGED, WM_ERASEBKGND, WM_KEYDOWN, WM_LBUTTONDOWN, WM_PAINT,
    WM_SETFONT, WM_SIZE, WM_TIMER, WNDCLASSW, WS_CHILD, WS_EX_LAYERED, WS_TABSTOP,
    WINDOW_EX_STYLE, WINDOW_STYLE,
};

use crate::media::probe::Kind;
use crate::platform::windows::toolwindow::ToolEvent;
use crate::platform::windows::windowhost::{
    fixed_style, outer_size_for, scale_factor, to_wide,
};
use crate::radial::execution::ToolSettings;
use crate::ui::wheel::{window_for_next_component, Bgra8, CompressCard, Panel};

/// The timer the window comes in softly on.
const FADE_TIMER_ID: usize = 53;

/// The card's width in the drawing's own units.
const CARD_W: i32 = 420;

/// Stack heights in scale-independent units.
const HEADER_UNITS: i32 = 56;

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
    header_box: Box4,
    detail_box: Box4,
    hairline_top: Box4,
    preset_label_box: Box4,
    preset_balanced_box: Box4,
    preset_strong_box: Box4,
    preset_hint_box: Box4,
    video_codec_label_box: Box4,
    codec_h264_box: Box4,
    codec_hevc_box: Box4,
    video_codec_hint_box: Box4,
    video_res_label_box: Box4,
    res_orig_box: Box4,
    res_4k_box: Box4,
    res_1080_box: Box4,
    res_720_box: Box4,
    res_480_box: Box4,
    video_fps_label_box: Box4,
    fps_orig_box: Box4,
    fps_60_box: Box4,
    fps_30_box: Box4,
    fps_24_box: Box4,
    hairline_mid: Box4,
    target_check_box: Box4,
    target_tile_box: Box4,
    target_field_box: Box4,
    target_kb_box: Box4,
    target_mb_box: Box4,
    target_caption_box: Box4,
    target_hint_box: Box4,
    footnote_box: Box4,
    hairline_bottom: Box4,
    helper_box: Box4,
    cancel_box: Box4,
    run_box: Box4,
}

impl Layout {
    fn of(_scale: f32, has_video: bool, target_enabled: bool) -> Self {
        let card_w = CARD_W as f32;
        let mut y = 14.0f32;

        let header_box = Box4 { x: 18.0, y, w: card_w - 36.0, h: 22.0 };
        let detail_box = Box4 { x: 18.0, y: y + 22.0, w: card_w - 36.0, h: 18.0 };
        y = HEADER_UNITS as f32;

        let hairline_top = Box4 { x: 0.0, y, w: card_w, h: 1.0 };
        y += 12.0;

        let preset_label_box = Box4 { x: 18.0, y, w: card_w - 36.0, h: 18.0 };
        y += 22.0;

        let chip_w = (card_w - 36.0 - 10.0) / 2.0;
        let preset_balanced_box = Box4 { x: 18.0, y, w: chip_w, h: 30.0 };
        let preset_strong_box = Box4 { x: 18.0 + chip_w + 10.0, y, w: chip_w, h: 30.0 };
        y += 36.0;

        let preset_hint_box = Box4 { x: 18.0, y, w: card_w - 36.0, h: 16.0 };
        y += 24.0;

        let (
            video_codec_label_box,
            codec_h264_box,
            codec_hevc_box,
            video_codec_hint_box,
            video_res_label_box,
            res_orig_box,
            res_4k_box,
            res_1080_box,
            res_720_box,
            res_480_box,
            video_fps_label_box,
            fps_orig_box,
            fps_60_box,
            fps_30_box,
            fps_24_box,
        ) = if has_video {
            let cl_box = Box4 { x: 18.0, y, w: card_w - 36.0, h: 16.0 };
            y += 20.0;

            let c_h264 = Box4 { x: 18.0, y, w: 86.0, h: 26.0 };
            let c_hevc = Box4 { x: 110.0, y, w: 86.0, h: 26.0 };
            y += 30.0;

            let c_hint = Box4 { x: 18.0, y, w: card_w - 36.0, h: 14.0 };
            y += 20.0;

            let rl_box = Box4 { x: 18.0, y, w: card_w - 36.0, h: 16.0 };
            y += 20.0;

            let rw = (card_w - 36.0 - 24.0) / 5.0;
            let r_orig = Box4 { x: 18.0, y, w: rw, h: 26.0 };
            let r_4k = Box4 { x: 18.0 + (rw + 6.0), y, w: rw, h: 26.0 };
            let r_1080 = Box4 { x: 18.0 + (rw + 6.0) * 2.0, y, w: rw, h: 26.0 };
            let r_720 = Box4 { x: 18.0 + (rw + 6.0) * 3.0, y, w: rw, h: 26.0 };
            let r_480 = Box4 { x: 18.0 + (rw + 6.0) * 4.0, y, w: rw, h: 26.0 };
            y += 32.0;

            let fl_box = Box4 { x: 18.0, y, w: card_w - 36.0, h: 16.0 };
            y += 20.0;

            let fw = (card_w - 36.0 - 18.0) / 4.0;
            let f_orig = Box4 { x: 18.0, y, w: fw, h: 26.0 };
            let f_60 = Box4 { x: 18.0 + (fw + 6.0), y, w: fw, h: 26.0 };
            let f_30 = Box4 { x: 18.0 + (fw + 6.0) * 2.0, y, w: fw, h: 26.0 };
            let f_24 = Box4 { x: 18.0 + (fw + 6.0) * 3.0, y, w: fw, h: 26.0 };
            y += 32.0;

            (
                cl_box, c_h264, c_hevc, c_hint,
                rl_box, r_orig, r_4k, r_1080, r_720, r_480,
                fl_box, f_orig, f_60, f_30, f_24,
            )
        } else {
            let zero = Box4 { x: 0.0, y: 0.0, w: 0.0, h: 0.0 };
            (
                zero, zero, zero, zero,
                zero, zero, zero, zero, zero, zero,
                zero, zero, zero, zero, zero,
            )
        };

        let hairline_mid = Box4 { x: 0.0, y, w: card_w, h: 1.0 };
        y += 12.0;

        let target_check_box = Box4 { x: 18.0, y, w: card_w - 36.0, h: 24.0 };
        y += 30.0;

        let (
            target_tile_box,
            target_field_box,
            target_kb_box,
            target_mb_box,
            target_caption_box,
            target_hint_box,
        ) = if target_enabled {
            let t_tile = Box4 { x: 18.0, y, w: 80.0, h: 30.0 };
            let t_field = Box4 { x: 22.0, y: y + 4.0, w: 72.0, h: 22.0 };
            let t_kb = Box4 { x: 106.0, y, w: 46.0, h: 30.0 };
            let t_mb = Box4 { x: 158.0, y, w: 46.0, h: 30.0 };
            let t_cap = Box4 { x: 212.0, y, w: 80.0, h: 30.0 };
            y += 36.0;

            let t_hint = Box4 { x: 18.0, y, w: card_w - 36.0, h: 16.0 };
            y += 24.0;

            (t_tile, t_field, t_kb, t_mb, t_cap, t_hint)
        } else {
            let zero = Box4 { x: 0.0, y: 0.0, w: 0.0, h: 0.0 };
            (zero, zero, zero, zero, zero, zero)
        };

        let footnote_box = Box4 { x: 18.0, y, w: card_w - 36.0, h: 18.0 };
        y += 24.0;

        let hairline_bottom = Box4 { x: 0.0, y, w: card_w, h: 1.0 };
        y += 12.0;

        let helper_box = Box4 { x: 18.0, y: y + 10.0, w: 180.0, h: 20.0 };
        let cancel_box = Box4 { x: card_w - 18.0 - 196.0, y, w: 90.0, h: 38.0 };
        let run_box = Box4 { x: card_w - 18.0 - 96.0, y, w: 96.0, h: 38.0 };
        let card_h = y + 50.0;

        Self {
            card_w,
            card_h,
            header_box,
            detail_box,
            hairline_top,
            preset_label_box,
            preset_balanced_box,
            preset_strong_box,
            preset_hint_box,
            video_codec_label_box,
            codec_h264_box,
            codec_hevc_box,
            video_codec_hint_box,
            video_res_label_box,
            res_orig_box,
            res_4k_box,
            res_1080_box,
            res_720_box,
            res_480_box,
            video_fps_label_box,
            fps_orig_box,
            fps_60_box,
            fps_30_box,
            fps_24_box,
            hairline_mid,
            target_check_box,
            target_tile_box,
            target_field_box,
            target_kb_box,
            target_mb_box,
            target_caption_box,
            target_hint_box,
            footnote_box,
            hairline_bottom,
            helper_box,
            cancel_box,
            run_box,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Focus {
    Preset(usize),
    Codec(usize),
    Res(usize),
    Fps(usize),
    TargetCheck,
    TargetField,
    TargetUnit(usize),
    Cancel,
    Run,
}

#[derive(Clone)]
struct State {
    fade_alpha: u8,
    preset_strong: bool,
    codec_hevc: bool,
    res_choice: usize,
    fps_choice: usize,
    target_enabled: bool,
    target_unit_mb: bool,
    target_value: u64,
    has_video: bool,
    inputs: Vec<PathBuf>,
    file_name: String,
    detail: String,
    footnote: String,
    focus: Focus,
    sender: Option<Sender<ToolEvent>>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            fade_alpha: 0,
            preset_strong: false,
            codec_hevc: false,
            res_choice: 0,
            fps_choice: 0,
            target_enabled: false,
            target_unit_mb: true,
            target_value: 1,
            has_video: false,
            inputs: Vec::new(),
            file_name: String::new(),
            detail: String::new(),
            footnote: String::new(),
            focus: Focus::Run,
            sender: None,
        }
    }
}

thread_local! {
    static HOST: std::cell::RefCell<Option<Host>> = const { std::cell::RefCell::new(None) };
    static STATE: std::cell::RefCell<State> = std::cell::RefCell::new(State::default());
    static WINDOW: std::cell::Cell<isize> = const { std::cell::Cell::new(0) };
    static FIELD: std::cell::Cell<isize> = const { std::cell::Cell::new(0) };
}

struct Host {
    window: HWND,
    card: CompressCard,
    software: Rc<MinimalSoftwareWindow>,
    buffer: Vec<Bgra8>,
    width: u32,
    height: u32,
    field: HWND,
}

impl Host {
    fn sync_to_card(&mut self, state: &State, scale: f32) {
        let layout = Layout::of(scale, state.has_video, state.target_enabled);
        let card = &self.card;

        card.set_ui_scale(scale);
        card.set_card_width(layout.card_w * scale);
        card.set_card_height(layout.card_h * scale);

        card.set_file_name(state.file_name.clone().into());
        card.set_detail(state.detail.clone().into());
        card.set_footnote(state.footnote.clone().into());

        card.set_has_video(state.has_video);
        card.set_target_enabled(state.target_enabled);
        card.set_preset_strong(state.preset_strong);
        card.set_codec_hevc(state.codec_hevc);
        card.set_res_choice(state.res_choice as i32);
        card.set_fps_choice(state.fps_choice as i32);
        card.set_target_unit_mb(state.target_unit_mb);
        card.set_target_value_text(state.target_value.to_string().into());

        card.set_preset_hint(
            if state.preset_strong {
                "Smallest output; quality loss is visible."
            } else {
                "Smaller files with less quality loss."
            }
            .into(),
        );
        card.set_video_hint(
            if state.codec_hevc {
                "HEVC keeps the same picture in about two thirds of the size."
            } else {
                "H.264 plays everywhere."
            }
            .into(),
        );

        // Focus bindings
        let (pf, cf, rf, ff, tcf, tff, uf, bf) = match state.focus {
            Focus::Preset(idx) => (idx as i32 + 1, 0, 0, 0, false, false, 0, 0),
            Focus::Codec(idx) => (0, idx as i32 + 1, 0, 0, false, false, 0, 0),
            Focus::Res(idx) => (0, 0, idx as i32 + 1, 0, false, false, 0, 0),
            Focus::Fps(idx) => (0, 0, 0, idx as i32 + 1, false, false, 0, 0),
            Focus::TargetCheck => (0, 0, 0, 0, true, false, 0, 0),
            Focus::TargetField => (0, 0, 0, 0, false, true, 0, 0),
            Focus::TargetUnit(idx) => (0, 0, 0, 0, false, false, idx as i32 + 1, 0),
            Focus::Cancel => (0, 0, 0, 0, false, false, 0, 1),
            Focus::Run => (0, 0, 0, 0, false, false, 0, 2),
        };
        card.set_preset_focus(pf);
        card.set_codec_focus(cf);
        card.set_res_focus(rf);
        card.set_fps_focus(ff);
        card.set_target_check_focus(tcf);
        card.set_target_field_focus(tff);
        card.set_unit_focus(uf);
        card.set_button_focus(bf);

        // Panels
        card.set_header_box(to_panel(layout.header_box, scale));
        card.set_detail_box(to_panel(layout.detail_box, scale));
        card.set_hairline_top(to_panel(layout.hairline_top, scale));
        card.set_preset_label_box(to_panel(layout.preset_label_box, scale));
        card.set_preset_balanced_box(to_panel(layout.preset_balanced_box, scale));
        card.set_preset_strong_box(to_panel(layout.preset_strong_box, scale));
        card.set_preset_hint_box(to_panel(layout.preset_hint_box, scale));

        card.set_video_codec_label_box(to_panel(layout.video_codec_label_box, scale));
        card.set_codec_h264_box(to_panel(layout.codec_h264_box, scale));
        card.set_codec_hevc_box(to_panel(layout.codec_hevc_box, scale));
        card.set_video_codec_hint_box(to_panel(layout.video_codec_hint_box, scale));

        card.set_video_res_label_box(to_panel(layout.video_res_label_box, scale));
        card.set_res_orig_box(to_panel(layout.res_orig_box, scale));
        card.set_res_4k_box(to_panel(layout.res_4k_box, scale));
        card.set_res_1080_box(to_panel(layout.res_1080_box, scale));
        card.set_res_720_box(to_panel(layout.res_720_box, scale));
        card.set_res_480_box(to_panel(layout.res_480_box, scale));

        card.set_video_fps_label_box(to_panel(layout.video_fps_label_box, scale));
        card.set_fps_orig_box(to_panel(layout.fps_orig_box, scale));
        card.set_fps_60_box(to_panel(layout.fps_60_box, scale));
        card.set_fps_30_box(to_panel(layout.fps_30_box, scale));
        card.set_fps_24_box(to_panel(layout.fps_24_box, scale));

        card.set_hairline_mid(to_panel(layout.hairline_mid, scale));
        card.set_target_check_box(to_panel(layout.target_check_box, scale));
        card.set_target_tile_box(to_panel(layout.target_tile_box, scale));
        card.set_target_kb_box(to_panel(layout.target_kb_box, scale));
        card.set_target_mb_box(to_panel(layout.target_mb_box, scale));
        card.set_target_caption_box(to_panel(layout.target_caption_box, scale));
        card.set_target_hint_box(to_panel(layout.target_hint_box, scale));

        card.set_footnote_box(to_panel(layout.footnote_box, scale));
        card.set_hairline_bottom(to_panel(layout.hairline_bottom, scale));
        card.set_helper_box(to_panel(layout.helper_box, scale));
        card.set_cancel_box(to_panel(layout.cancel_box, scale));
        card.set_run_box(to_panel(layout.run_box, scale));

        // Sync native field
        unsafe {
            if state.target_enabled {
                let fx = (layout.target_field_box.x * scale).round() as i32;
                let fy = (layout.target_field_box.y * scale).round() as i32;
                let fw = (layout.target_field_box.w * scale).round() as i32;
                let fh = (layout.target_field_box.h * scale).round() as i32;
                let _ = SetWindowPos(
                    self.field,
                    None,
                    fx,
                    fy,
                    fw,
                    fh,
                    SWP_NOZORDER | SWP_NOACTIVATE | SWP_SHOWWINDOW,
                );
            } else {
                let _ = ShowWindow(self.field, SW_HIDE);
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
}

pub struct CompressWindow {
    pub(crate) window: HWND,
}

impl CompressWindow {
    pub fn new(sender: Sender<ToolEvent>) -> Result<Self, String> {
        STATE.with(|slot| slot.borrow_mut().sender = Some(sender));
        let instance: HMODULE = unsafe { GetModuleHandleW(PCWSTR::null()) }.map_err(|e| e.to_string())?;
        let class_name = to_wide("OpenTangerineCompressWindow");

        let white_brush = unsafe { CreateSolidBrush(COLORREF(0x00F4F6F7)) };
        let wnd_class = WNDCLASSW {
            lpfnWndProc: Some(window_procedure),
            hInstance: instance.into(),
            lpszClassName: PCWSTR(class_name.as_ptr()),
            hCursor: unsafe { LoadCursorW(None, IDC_ARROW) }.unwrap_or_default(),
            hbrBackground: white_brush,
            ..Default::default()
        };
        unsafe { RegisterClassW(&wnd_class) };

        let window = unsafe {
            CreateWindowExW(
                WS_EX_LAYERED,
                PCWSTR(class_name.as_ptr()),
                PCWSTR(to_wide("Compress").as_ptr()),
                fixed_style(),
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                CARD_W,
                480,
                None,
                None,
                Some(instance.into()),
                None,
            )
        }
        .map_err(|e| e.to_string())?;

        unsafe {
            let _ = SetLayeredWindowAttributes(window, COLORREF(0), 0, LWA_ALPHA);
        }

        let software = window_for_next_component();
        let card = CompressCard::new().map_err(|e| e.to_string())?;

        // Native numeric field
        let field_font = unsafe { interface_font(window) };
        let field = unsafe {
            let edit = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("EDIT"),
                w!("1"),
                WS_CHILD | WS_TABSTOP | WINDOW_STYLE(ES_NUMBER as u32),
                0,
                0,
                100,
                24,
                Some(window),
                Some(HMENU(50 as *mut _)),
                Some(instance.into()),
                None,
            )
            .unwrap_or_default();
            SendMessageW(edit, WM_SETFONT, Some(WPARAM(field_font.0 as usize)), Some(LPARAM(1)));
            let _ = SetWindowSubclass(edit, Some(field_procedure), 1, 0);
            edit
        };

        // Wire Slint callbacks
        {
            card.on_preset_chosen(move |strong| {
                let w = window_handle();
                if !w.0.is_null() {
                    STATE.with(|slot| {
                        let mut st = slot.borrow_mut();
                        st.preset_strong = strong;
                        st.focus = Focus::Preset(if strong { 1 } else { 0 });
                    });
                    redraw(w);
                }
            });
            card.on_codec_chosen(move |hevc| {
                let w = window_handle();
                if !w.0.is_null() {
                    STATE.with(|slot| {
                        let mut st = slot.borrow_mut();
                        st.codec_hevc = hevc;
                        st.focus = Focus::Codec(if hevc { 1 } else { 0 });
                    });
                    redraw(w);
                }
            });
            card.on_res_chosen(move |choice| {
                let w = window_handle();
                if !w.0.is_null() {
                    STATE.with(|slot| {
                        let mut st = slot.borrow_mut();
                        st.res_choice = choice.max(0) as usize;
                        st.focus = Focus::Res(st.res_choice);
                    });
                    redraw(w);
                }
            });
            card.on_fps_chosen(move |choice| {
                let w = window_handle();
                if !w.0.is_null() {
                    STATE.with(|slot| {
                        let mut st = slot.borrow_mut();
                        st.fps_choice = choice.max(0) as usize;
                        st.focus = Focus::Fps(st.fps_choice);
                    });
                    redraw(w);
                }
            });
            card.on_target_toggled(move || {
                let w = window_handle();
                if !w.0.is_null() {
                    toggle_target(w);
                }
            });
            card.on_unit_chosen(move |is_mb| {
                let w = window_handle();
                if !w.0.is_null() {
                    STATE.with(|slot| {
                        let mut st = slot.borrow_mut();
                        st.target_unit_mb = is_mb;
                        st.focus = Focus::TargetUnit(if is_mb { 1 } else { 0 });
                    });
                    redraw(w);
                }
            });
            card.on_run_chosen(move || {
                let w = window_handle();
                if !w.0.is_null() {
                    unsafe { run(w) };
                }
            });
            card.on_cancel_chosen(move || {
                let w = window_handle();
                if !w.0.is_null() {
                    unsafe { cancel(w) };
                }
            });
        }

        let host = Host {
            window,
            card,
            software,
            buffer: Vec::new(),
            width: 0,
            height: 0,
            field,
        };
        HOST.with(|slot| *slot.borrow_mut() = Some(host));
        WINDOW.with(|slot| slot.set(window.0 as isize));
        FIELD.with(|slot| slot.set(field.0 as isize));

        Ok(Self { window })
    }

    pub fn show_for(&mut self, _operation: &str, _label: &str, _detail: &str, files: &[PathBuf]) {
        let has_video = files.iter().any(|path| {
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            matches!(crate::media::probe::kind_of(ext), Some(Kind::Video))
        });
        let count = files.len();
        let total_size: u64 = files
            .iter()
            .map(|p| std::fs::metadata(p).map(|m| m.len()).unwrap_or(0))
            .sum();

        let file_name = if count == 1 {
            files[0].file_name().and_then(|n| n.to_str()).unwrap_or("File").to_string()
        } else {
            format!("{count} files")
        };

        let formatted_size = format_file_size(total_size);
        let detail = if count == 1 {
            let ext = files[0].extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
            format!("{formatted_size} \u{b7} saves as {ext}")
        } else {
            format!("{formatted_size} total")
        };

        let footnote = format!(
            "{count} file{}, {formatted_size} total. Originals stay untouched; copies saved beside.",
            if count == 1 { "" } else { "s" }
        );

        STATE.with(|slot| {
            let mut st = slot.borrow_mut();
            st.inputs = files.to_vec();
            st.has_video = has_video;
            st.file_name = file_name;
            st.detail = detail;
            st.footnote = footnote;
            st.fade_alpha = 0;
            st.focus = Focus::Run;
        });

        let scale = scale_factor(self.window);
        let layout = Layout::of(scale, has_video, false);
        let outer = outer_size_for(self.window, (layout.card_w * scale).round() as i32, (layout.card_h * scale).round() as i32);

        unsafe {
            let _ = SetWindowPos(
                self.window,
                None,
                0,
                0,
                outer.0,
                outer.1,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
            let _ = SetLayeredWindowAttributes(self.window, COLORREF(0), 0, LWA_ALPHA);
            let _ = ShowWindow(self.window, SW_SHOW);
            let _ = SetForegroundWindow(self.window);
            let _ = SetTimer(Some(self.window), FADE_TIMER_ID, 20, None);
        }

        redraw(self.window);
    }
}

fn window_handle() -> HWND {
    HWND(WINDOW.with(|slot| slot.get()) as *mut _)
}

fn field_handle() -> HWND {
    HWND(FIELD.with(|slot| slot.get()) as *mut _)
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
        let _ = InvalidateRect(Some(window), None, false);
    }
}

fn toggle_target(window: HWND) {
    let (has_video, target_enabled) = STATE.with(|slot| {
        let mut st = slot.borrow_mut();
        st.target_enabled = !st.target_enabled;
        st.focus = Focus::TargetCheck;
        (st.has_video, st.target_enabled)
    });
    let scale = scale_factor(window);
    let layout = Layout::of(scale, has_video, target_enabled);
    let outer = outer_size_for(window, (layout.card_w * scale).round() as i32, (layout.card_h * scale).round() as i32);
    unsafe {
        let _ = SetWindowPos(window, None, 0, 0, outer.0, outer.1, SWP_NOZORDER | SWP_NOACTIVATE);
    }
    redraw(window);
}

unsafe fn run(window: HWND) {
    let (inputs, strong, target_bytes, max_long_edge, max_fps, sender) = STATE.with(|slot| {
        let st = slot.borrow();
        let target_bytes = if st.target_enabled && st.target_value > 0 {
            Some(st.target_value * if st.target_unit_mb { 1_000_000 } else { 1_000 })
        } else {
            None
        };
        let max_long_edge = match st.res_choice {
            1 => Some(3840),
            2 => Some(1920),
            3 => Some(1280),
            4 => Some(854),
            _ => None,
        };
        let max_fps = match st.fps_choice {
            1 => Some(60),
            2 => Some(30),
            3 => Some(24),
            _ => None,
        };
        (st.inputs.clone(), st.preset_strong, target_bytes, max_long_edge, max_fps, st.sender.clone())
    });

    let settings = ToolSettings {
        compress_strong: strong,
        target_bytes,
        max_long_edge,
        max_fps,
        ..Default::default()
    };

    if let Some(sender) = sender {
        let _ = sender.send(ToolEvent::Run {
            operation: "tools.compress".to_string(),
            inputs,
            settings,
        });
    }

    unsafe {
        let _ = ShowWindow(window, SW_HIDE);
    }
}

unsafe fn cancel(window: HWND) {
    let sender = STATE.with(|slot| slot.borrow().sender.clone());
    if let Some(sender) = sender {
        let _ = sender.send(ToolEvent::Cancelled);
    }
    unsafe {
        let _ = ShowWindow(window, SW_HIDE);
    }
}

unsafe fn interface_font(window: HWND) -> HFONT {
    let mut log_font = LOGFONTW::default();
    let name = to_wide("Segoe UI");
    for (i, ch) in name.iter().take(31).enumerate() {
        log_font.lfFaceName[i] = *ch;
    }
    let dpi = (scale_factor(window) * 96.0).round() as i32;
    log_font.lfHeight = -((13 * dpi) / 72);
    unsafe { CreateFontIndirectW(&log_font) }
}

fn format_file_size(bytes: u64) -> String {
    if bytes >= 1_000_000_000 {
        format!("{:.1} GB", bytes as f64 / 1_000_000_000.0)
    } else if bytes >= 1_000_000 {
        format!("{:.1} MB", bytes as f64 / 1_000_000.0)
    } else if bytes >= 1_000 {
        format!("{:.0} KB", bytes as f64 / 1_000.0)
    } else {
        format!("{bytes} B")
    }
}

unsafe extern "system" fn field_procedure(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    _subclass_data: usize,
) -> LRESULT {
    match message {
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
                let shift = (unsafe { GetKeyState(VK_SHIFT.0 as i32) } as u16 & 0x8000) != 0;
                cycle_focus(parent, !shift);
                return LRESULT(0);
            }
        }
        _ => {}
    }
    unsafe { DefSubclassProc(window, message, wparam, lparam) }
}

fn cycle_focus(window: HWND, forward: bool) {
    let has_video = STATE.with(|slot| slot.borrow().has_video);
    let target_enabled = STATE.with(|slot| slot.borrow().target_enabled);

    let mut chain = vec![Focus::Preset(0), Focus::TargetCheck];
    if target_enabled {
        chain.push(Focus::TargetField);
        chain.push(Focus::TargetUnit(0));
    }
    if has_video {
        chain.insert(1, Focus::Codec(0));
        chain.insert(2, Focus::Res(0));
        chain.insert(3, Focus::Fps(0));
    }
    chain.push(Focus::Cancel);
    chain.push(Focus::Run);

    STATE.with(|slot| {
        let mut st = slot.borrow_mut();
        let cur_idx = chain.iter().position(|f| {
            std::mem::discriminant(f) == std::mem::discriminant(&st.focus)
        }).unwrap_or(0);
        let next_idx = if forward {
            (cur_idx + 1) % chain.len()
        } else {
            (cur_idx + chain.len() - 1) % chain.len()
        };
        st.focus = chain[next_idx];
        if matches!(st.focus, Focus::TargetField) {
            let field = field_handle();
            unsafe { let _ = SetFocus(Some(field)); }
        } else {
            unsafe { let _ = SetFocus(Some(window)); }
        }
    });
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
                    host.draw();
                }
            });
            let _ = unsafe { EndPaint(window, &paint) };
            return LRESULT(0);
        }
        WM_LBUTTONDOWN => {
            let x = (lparam.0 & 0xFFFF) as i16 as f32;
            let y = ((lparam.0 >> 16) & 0xFFFF) as i16 as f32;
            let scale = scale_factor(window);
            let (has_video, target_enabled) = STATE.with(|slot| {
                let st = slot.borrow();
                (st.has_video, st.target_enabled)
            });
            let layout = Layout::of(scale, has_video, target_enabled);
            let unscaled_x = x / scale;
            let unscaled_y = y / scale;

            if layout.preset_balanced_box.contains(unscaled_x, unscaled_y) {
                STATE.with(|slot| {
                    let mut st = slot.borrow_mut();
                    st.preset_strong = false;
                    st.focus = Focus::Preset(0);
                });
                redraw(window);
                return LRESULT(0);
            }
            if layout.preset_strong_box.contains(unscaled_x, unscaled_y) {
                STATE.with(|slot| {
                    let mut st = slot.borrow_mut();
                    st.preset_strong = true;
                    st.focus = Focus::Preset(1);
                });
                redraw(window);
                return LRESULT(0);
            }
            if has_video {
                if layout.codec_h264_box.contains(unscaled_x, unscaled_y) {
                    STATE.with(|slot| {
                        let mut st = slot.borrow_mut();
                        st.codec_hevc = false;
                        st.focus = Focus::Codec(0);
                    });
                    redraw(window);
                    return LRESULT(0);
                }
                if layout.codec_hevc_box.contains(unscaled_x, unscaled_y) {
                    STATE.with(|slot| {
                        let mut st = slot.borrow_mut();
                        st.codec_hevc = true;
                        st.focus = Focus::Codec(1);
                    });
                    redraw(window);
                    return LRESULT(0);
                }
                let res_boxes = [
                    layout.res_orig_box,
                    layout.res_4k_box,
                    layout.res_1080_box,
                    layout.res_720_box,
                    layout.res_480_box,
                ];
                for (idx, b) in res_boxes.iter().enumerate() {
                    if b.contains(unscaled_x, unscaled_y) {
                        STATE.with(|slot| {
                            let mut st = slot.borrow_mut();
                            st.res_choice = idx;
                            st.focus = Focus::Res(idx);
                        });
                        redraw(window);
                        return LRESULT(0);
                    }
                }
                let fps_boxes = [
                    layout.fps_orig_box,
                    layout.fps_60_box,
                    layout.fps_30_box,
                    layout.fps_24_box,
                ];
                for (idx, b) in fps_boxes.iter().enumerate() {
                    if b.contains(unscaled_x, unscaled_y) {
                        STATE.with(|slot| {
                            let mut st = slot.borrow_mut();
                            st.fps_choice = idx;
                            st.focus = Focus::Fps(idx);
                        });
                        redraw(window);
                        return LRESULT(0);
                    }
                }
            }
            if layout.target_check_box.contains(unscaled_x, unscaled_y) {
                toggle_target(window);
                return LRESULT(0);
            }
            if target_enabled {
                if layout.target_kb_box.contains(unscaled_x, unscaled_y) {
                    STATE.with(|slot| {
                        let mut st = slot.borrow_mut();
                        st.target_unit_mb = false;
                        st.focus = Focus::TargetUnit(0);
                    });
                    redraw(window);
                    return LRESULT(0);
                }
                if layout.target_mb_box.contains(unscaled_x, unscaled_y) {
                    STATE.with(|slot| {
                        let mut st = slot.borrow_mut();
                        st.target_unit_mb = true;
                        st.focus = Focus::TargetUnit(1);
                    });
                    redraw(window);
                    return LRESULT(0);
                }
            }
            if layout.cancel_box.contains(unscaled_x, unscaled_y) {
                unsafe { cancel(window) };
                return LRESULT(0);
            }
            if layout.run_box.contains(unscaled_x, unscaled_y) {
                unsafe { run(window) };
                return LRESULT(0);
            }
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
                let shift = (unsafe { GetKeyState(VK_SHIFT.0 as i32) } as u16 & 0x8000) != 0;
                cycle_focus(window, !shift);
                return LRESULT(0);
            }
            if key == VK_SPACE.0 as i32 {
                let focus = STATE.with(|slot| slot.borrow().focus);
                if matches!(focus, Focus::TargetCheck) {
                    toggle_target(window);
                    return LRESULT(0);
                }
            }
            if key == VK_LEFT.0 as i32 || key == VK_RIGHT.0 as i32 {
                let right = key == VK_RIGHT.0 as i32;
                STATE.with(|slot| {
                    let mut st = slot.borrow_mut();
                    match st.focus {
                        Focus::Preset(_) => {
                            st.preset_strong = !st.preset_strong;
                            st.focus = Focus::Preset(if st.preset_strong { 1 } else { 0 });
                        }
                        Focus::Codec(_) => {
                            st.codec_hevc = !st.codec_hevc;
                            st.focus = Focus::Codec(if st.codec_hevc { 1 } else { 0 });
                        }
                        Focus::Res(idx) => {
                            let next = if right { (idx + 1) % 5 } else { (idx + 4) % 5 };
                            st.res_choice = next;
                            st.focus = Focus::Res(next);
                        }
                        Focus::Fps(idx) => {
                            let next = if right { (idx + 1) % 4 } else { (idx + 3) % 4 };
                            st.fps_choice = next;
                            st.focus = Focus::Fps(next);
                        }
                        Focus::TargetUnit(_) => {
                            st.target_unit_mb = !st.target_unit_mb;
                            st.focus = Focus::TargetUnit(if st.target_unit_mb { 1 } else { 0 });
                        }
                        _ => {}
                    }
                });
                redraw(window);
                return LRESULT(0);
            }
        }
        WM_COMMAND => {
            let id = (wparam.0 & 0xFFFF) as u16;
            let code = ((wparam.0 >> 16) & 0xFFFF) as u16;
            if id == 50 && code == 0x0300 {
                // EN_CHANGE
                let field = field_handle();
                let mut buf = [0u16; 32];
                let len = unsafe { GetWindowTextW(field, &mut buf) } as usize;
                let text = String::from_utf16_lossy(&buf[..len]);
                if let Ok(val) = text.parse::<u64>() {
                    STATE.with(|slot| slot.borrow_mut().target_value = val);
                }
            }
        }
        WM_CTLCOLOREDIT => {
            let brush = unsafe { CreateSolidBrush(COLORREF(0x00FFFFFF)) };
            return LRESULT(brush.0 as isize);
        }
        WM_DPICHANGED => {
            let scale = scale_factor(window);
            let (has_video, target_enabled) = STATE.with(|slot| {
                let st = slot.borrow();
                (st.has_video, st.target_enabled)
            });
            let layout = Layout::of(scale, has_video, target_enabled);
            let outer = outer_size_for(window, (layout.card_w * scale).round() as i32, (layout.card_h * scale).round() as i32);
            let _ = unsafe { SetWindowPos(window, None, 0, 0, outer.0, outer.1, SWP_NOZORDER | SWP_NOACTIVATE) };
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
mod tests {
    use super::*;

    #[test]
    fn compress_layout_heights_are_pinned() {
        let base = Layout::of(1.0, false, false);
        assert_eq!(base.card_w, 420.0);
        assert!((base.card_h - 278.0).abs() < 2.0, "base card height: {}", base.card_h);

        let target_on = Layout::of(1.0, false, true);
        assert!(target_on.card_h > base.card_h, "target revealed increases height");
        assert!((target_on.card_h - 338.0).abs() < 2.0, "target on height: {}", target_on.card_h);

        let video = Layout::of(1.0, true, false);
        assert!(video.card_h > base.card_h, "video rows increase height");
        assert!((video.card_h - 452.0).abs() < 2.0, "video height: {}", video.card_h);

        let video_target = Layout::of(1.0, true, true);
        assert!(video_target.card_h > video.card_h);
        assert!((video_target.card_h - 512.0).abs() < 2.0, "video+target height: {}", video_target.card_h);
    }

    #[test]
    fn compress_format_file_size_converts_units() {
        assert_eq!(format_file_size(500), "500 B");
        assert_eq!(format_file_size(2_000), "2 KB");
        assert_eq!(format_file_size(3_400_000), "3.4 MB");
        assert_eq!(format_file_size(1_500_000_000), "1.5 GB");
    }
}

#[cfg(test)]
pub(crate) mod show_tests {
    use super::*;
    use std::sync::mpsc::channel;
    use windows::Win32::UI::WindowsAndMessaging::{DispatchMessageW, PeekMessageW, TranslateMessage, MSG, PM_REMOVE};

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
    fn showing_and_pumping_the_image_compress_card_does_not_panic() {
        crate::ui::wheel::install_platform();
        let (sender, _receiver) = channel();
        let mut window = CompressWindow::new(sender).expect("the compress window was made");
        let path = PathBuf::from("photo.jpg");
        window.show_for("tools.compress", "Compress", "3.4 MB \u{b7} saves as jpg", &[path]);
        pump_messages(20);
        unsafe { cancel(window.window) };
    }

    #[test]
    fn showing_and_pumping_the_video_compress_card_does_not_panic() {
        crate::ui::wheel::install_platform();
        let (sender, _receiver) = channel();
        let mut window = CompressWindow::new(sender).expect("the compress window was made");
        let path = PathBuf::from("clip.mp4");
        window.show_for("tools.compress", "Compress", "12.5 MB \u{b7} saves as mp4", &[path]);
        pump_messages(20);
        unsafe { cancel(window.window) };
    }

    #[test]
    #[ignore = "captures the composited compress window"]
    fn capture_the_compress_window() {
        crate::ui::wheel::install_platform();
        use crate::platform::windows::windowhost::screenshot::capture;
        let (sender, _receiver) = channel();
        let mut window = CompressWindow::new(sender).expect("the compress window was made");
        let path = PathBuf::from("Oranges.png");
        window.show_for("tools.compress", "Compress", "3.4 MB \u{b7} saves as png", &[path]);
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

        let out_dir = std::path::Path::new("target").join("shots");
        let _ = std::fs::create_dir_all(&out_dir);

        let window_shot = out_dir.join("compress-window.png");
        capture(window.window, &window_shot);
        assert!(window_shot.is_file(), "compress-window.png was written");

        unsafe { cancel(window.window) };
    }

    #[test]
    #[ignore = "captures the composited compress video window"]
    fn capture_the_compress_video_window() {
        crate::ui::wheel::install_platform();
        use crate::platform::windows::windowhost::screenshot::capture;
        let (sender, _receiver) = channel();
        let mut window = CompressWindow::new(sender).expect("the compress window was made");
        let path = PathBuf::from("clip.mp4");
        window.show_for("tools.compress", "Compress", "12.5 MB \u{b7} saves as mp4", &[path]);
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

        let out_dir = std::path::Path::new("target").join("shots");
        let _ = std::fs::create_dir_all(&out_dir);

        let window_shot = out_dir.join("compress-video-window.png");
        capture(window.window, &window_shot);
        assert!(window_shot.is_file(), "compress-video-window.png was written");

        unsafe { cancel(window.window) };
    }
}
