//! Win32 host and Slint card integration for the Create Collage window (P3-18 / `tools.collage`).

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::mpsc::Sender;

use image::DynamicImage;
use slint::platform::software_renderer::MinimalSoftwareWindow;
use slint::{Image, SharedPixelBuffer};
use windows::core::PCWSTR;
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, EndPaint, InvalidateRect, PAINTSTRUCT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    DefWindowProcW, DestroyWindow, PostQuitMessage,
    SetForegroundWindow, SetLayeredWindowAttributes, SetWindowPos,
    SetWindowTextW, ShowWindow, LWA_ALPHA, SWP_NOACTIVATE,
    SWP_NOMOVE, SWP_NOZORDER, SW_HIDE, SW_SHOW, WM_DESTROY, WM_DPICHANGED, WM_ERASEBKGND, WM_KEYDOWN,
    WM_LBUTTONDOWN, WM_MOUSEMOVE, WM_LBUTTONUP, WM_PAINT,
};

use crate::media::collage::{
    render_collage, CollageLayout, CollageSettings,
};
use crate::platform::windows::toolwindow::ToolEvent;
use crate::platform::windows::windowhost::{scale_factor, to_wide};
use crate::radial::execution::ToolSettings;
use crate::ui::wheel::{window_for_next_component, Bgra8, CollageCard, Panel};

pub const CARD_W: i32 = 440;
pub const CARD_H: i32 = 680;

pub const SWATCH_COLORS: [[u8; 4]; 5] = [
    [255, 255, 255, 255], // White #FFFFFF
    [255, 92, 0, 255],   // Tangerine #FF5C00
    [44, 44, 46, 255],   // Dark Gray #2C2C2E
    [229, 229, 234, 255],// Light Gray #E5E5EA
    [0, 0, 0, 255],       // Black #000000
];

pub const SWATCH_HEXES: [&str; 5] = [
    "#FFFFFF",
    "#FF5C00",
    "#2C2C2E",
    "#E5E5EA",
    "#000000",
];

#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    pub client_width: i32,
    pub client_height: i32,
    pub header: Panel,
    pub detail: Panel,
    pub hairline_top: Panel,
    pub stage: Panel,
    pub layout_label: Panel,
    pub layout_chips: [Panel; 4],
    pub spacing_label: Panel,
    pub spacing_track: Panel,
    pub spacing_thumb: Panel,
    pub corner_label: Panel,
    pub corner_track: Panel,
    pub corner_thumb: Panel,
    pub aspect_label: Panel,
    pub aspect_chips: [Panel; 3],
    pub color_label: Panel,
    pub color_swatches: [Panel; 5],
    pub hairline_bottom: Panel,
    pub reset: Panel,
    pub helper: Panel,
    pub cancel: Panel,
    pub run: Panel,
}

impl Layout {
    pub fn of(w: i32, h: i32, spacing_val: f32, corner_val: f32) -> Self {
        let p = |x: f32, y: f32, width: f32, height: f32| Panel {
            x,
            y,
            width,
            height,
        };

        let chip4_w = (400.0 - 24.0) / 4.0; // ~94 px
        let layout_chips = [
            p(20.0, 350.0, chip4_w, 28.0),
            p(20.0 + (chip4_w + 8.0), 350.0, chip4_w, 28.0),
            p(20.0 + (chip4_w + 8.0) * 2.0, 350.0, chip4_w, 28.0),
            p(20.0 + (chip4_w + 8.0) * 3.0, 350.0, chip4_w, 28.0),
        ];

        let spacing_track = p(20.0, 404.0, 400.0, 4.0);
        let spacing_frac = (spacing_val / 60.0).clamp(0.0, 1.0);
        let spacing_thumb = p(20.0 + spacing_frac * (400.0 - 16.0), 398.0, 16.0, 16.0);

        let corner_track = p(20.0, 444.0, 400.0, 4.0);
        let corner_frac = (corner_val / 40.0).clamp(0.0, 1.0);
        let corner_thumb = p(20.0 + corner_frac * (400.0 - 16.0), 438.0, 16.0, 16.0);

        let chip3_w = (400.0 - 16.0) / 3.0; // ~128 px
        let aspect_chips = [
            p(20.0, 490.0, chip3_w, 28.0),
            p(20.0 + chip3_w + 8.0, 490.0, chip3_w, 28.0),
            p(20.0 + (chip3_w + 8.0) * 2.0, 490.0, chip3_w, 28.0),
        ];

        let swatch_step = 54.0;
        let color_swatches = [
            p(20.0, 548.0, 26.0, 26.0),
            p(20.0 + swatch_step, 548.0, 26.0, 26.0),
            p(20.0 + swatch_step * 2.0, 548.0, 26.0, 26.0),
            p(20.0 + swatch_step * 3.0, 548.0, 26.0, 26.0),
            p(20.0 + swatch_step * 4.0, 548.0, 26.0, 26.0),
        ];

        Self {
            client_width: w,
            client_height: h,
            header: p(20.0, 14.0, 400.0, 24.0),
            detail: p(20.0, 38.0, 400.0, 18.0),
            hairline_top: p(0.0, 62.0, w as f32, 1.0),
            stage: p(20.0, 74.0, 400.0, 250.0),
            layout_label: p(20.0, 332.0, 400.0, 16.0),
            layout_chips,
            spacing_label: p(20.0, 386.0, 400.0, 16.0),
            spacing_track,
            spacing_thumb,
            corner_label: p(20.0, 426.0, 400.0, 16.0),
            corner_track,
            corner_thumb,
            aspect_label: p(20.0, 470.0, 400.0, 16.0),
            aspect_chips,
            color_label: p(20.0, 528.0, 400.0, 16.0),
            color_swatches,
            hairline_bottom: p(0.0, 594.0, w as f32, 1.0),
            reset: p(20.0, 612.0, 56.0, 36.0),
            helper: p(84.0, 612.0, 130.0, 36.0),
            cancel: p(222.0, 612.0, 64.0, 36.0),
            run: p(294.0, 612.0, 126.0, 36.0),
        }
    }
}

pub struct CollageWindow {
    window: HWND,
    renderer_window: Rc<MinimalSoftwareWindow>,
    card: CollageCard,
    buffer: Vec<Bgra8>,
    events: Rc<RefCell<Vec<ToolEvent>>>,
    sender: Sender<ToolEvent>,
    files: Vec<PathBuf>,
    preview_images: Vec<DynamicImage>,
    selected_layout: usize,
    spacing: f32,
    corner_radius: f32,
    selected_aspect: usize,
    selected_color: usize,
    dragging_slider: Option<u8>, // 1 = spacing, 2 = corner
}

thread_local! {
    static HOST_RC: RefCell<Option<Rc<RefCell<CollageWindow>>>> = const { RefCell::new(None) };
}

impl CollageWindow {
    pub fn new(sender: Sender<ToolEvent>) -> Result<Self, String> {
        let (window, renderer_window, card) = Self::create_window()?;
        let events = Rc::new(RefCell::new(Vec::new()));
        let buffer = vec![Bgra8::default(); (CARD_W * CARD_H) as usize];

        let mut win = Self {
            window,
            renderer_window,
            card,
            buffer,
            events,
            sender,
            files: Vec::new(),
            preview_images: Vec::new(),
            selected_layout: 0,
            spacing: 16.0,
            corner_radius: 16.0,
            selected_aspect: 0,
            selected_color: 0,
            dragging_slider: None,
        };

        win.wire_callbacks();
        win.sync_card();
        Ok(win)
    }

    fn create_window() -> Result<(HWND, Rc<MinimalSoftwareWindow>, CollageCard), String> {
        let renderer_window = window_for_next_component();
        let card = CollageCard::new().map_err(|e| e.to_string())?;

        let scale = scale_factor(HWND(std::ptr::null_mut()));
        let client_w = ((CARD_W as f32) * scale).round() as i32;
        let client_h = ((CARD_H as f32) * scale).round() as i32;

        let window = crate::platform::windows::windowhost::create_layered_window(
            "OpenTangerineCollageCardWindow",
            "Create Collage — Open Tangerine",
            Some(Self::wnd_proc),
            client_w,
            client_h,
        )?;

        renderer_window.set_size(slint::PhysicalSize::new(CARD_W as u32, CARD_H as u32));
        Ok((window, renderer_window, card))
    }

    fn wire_callbacks(&mut self) {
        let events = self.events.clone();
        let sender = self.sender.clone();
        let win_hwnd = self.window;
        self.card.on_cancel_chosen(move || {
            events.borrow_mut().push(ToolEvent::Cancelled);
            let _ = sender.send(ToolEvent::Cancelled);
            unsafe {
                let _ = ShowWindow(win_hwnd, SW_HIDE);
            }
        });

        let events_dismiss = self.events.clone();
        let sender_dismiss = self.sender.clone();
        self.card.on_dismiss_chosen(move || {
            events_dismiss.borrow_mut().push(ToolEvent::Cancelled);
            let _ = sender_dismiss.send(ToolEvent::Cancelled);
            unsafe {
                let _ = ShowWindow(win_hwnd, SW_HIDE);
            }
        });

        self.card.on_layout_chosen(|idx| {
            HOST_RC.with(|cell| {
                if let Some(host_rc) = cell.borrow().as_ref() {
                    host_rc.borrow_mut().selected_layout = (idx as usize).min(3);
                    host_rc.borrow_mut().update_preview();
                    host_rc.borrow_mut().sync_card();
                }
            });
        });

        self.card.on_aspect_chosen(|idx| {
            HOST_RC.with(|cell| {
                if let Some(host_rc) = cell.borrow().as_ref() {
                    host_rc.borrow_mut().selected_aspect = (idx as usize).min(2);
                    host_rc.borrow_mut().update_preview();
                    host_rc.borrow_mut().sync_card();
                }
            });
        });

        self.card.on_color_chosen(|idx| {
            HOST_RC.with(|cell| {
                if let Some(host_rc) = cell.borrow().as_ref() {
                    host_rc.borrow_mut().selected_color = (idx as usize).min(4);
                    host_rc.borrow_mut().update_preview();
                    host_rc.borrow_mut().sync_card();
                }
            });
        });

        self.card.on_reset_defaults_chosen(|| {
            HOST_RC.with(|cell| {
                if let Some(host_rc) = cell.borrow().as_ref() {
                    host_rc.borrow_mut().selected_layout = 0;
                    host_rc.borrow_mut().spacing = 16.0;
                    host_rc.borrow_mut().corner_radius = 16.0;
                    host_rc.borrow_mut().selected_aspect = 0;
                    host_rc.borrow_mut().selected_color = 0;
                    host_rc.borrow_mut().update_preview();
                    host_rc.borrow_mut().sync_card();
                }
            });
        });

        self.card.on_run_chosen(|| {
            HOST_RC.with(|cell| {
                if let Some(host_rc) = cell.borrow().as_ref() {
                    let host = host_rc.borrow();
                    let layout_str = match host.selected_layout {
                        1 => "columns",
                        2 => "rows",
                        3 => "featured",
                        _ => "grid",
                    };
                    let aspect_str = match host.selected_aspect {
                        1 => "16:9",
                        2 => "9:16",
                        _ => "1:1",
                    };
                    let bg_hex = SWATCH_HEXES[host.selected_color];

                    let mut settings = ToolSettings::default();
                    settings.collage_layout = Some(layout_str.to_string());
                    settings.collage_spacing = Some(host.spacing.round() as u32);
                    settings.collage_corner_radius = Some(host.corner_radius.round() as u32);
                    settings.collage_bg_color = Some(bg_hex.to_string());
                    settings.collage_aspect = Some(aspect_str.to_string());

                    let ev = ToolEvent::Run {
                        operation: "tools.collage".to_string(),
                        inputs: host.files.clone(),
                        settings,
                    };
                    host.events.borrow_mut().push(ev.clone());
                    let _ = host.sender.send(ev);
                    unsafe {
                        let _ = ShowWindow(host.window, SW_HIDE);
                    }
                }
            });
        });
    }

    pub fn sync_card(&mut self) {
        let layout = Layout::of(CARD_W, CARD_H, self.spacing, self.corner_radius);
        self.card.set_header_box(layout.header);
        self.card.set_detail_box(layout.detail);
        self.card.set_hairline_top(layout.hairline_top);
        self.card.set_stage_box(layout.stage);

        self.card.set_layout_label_box(layout.layout_label);
        self.card.set_layout_chips(slint::ModelRc::from(std::rc::Rc::new(slint::VecModel::from(layout.layout_chips.to_vec()))));

        self.card.set_spacing_label_box(layout.spacing_label);
        self.card.set_spacing_track_box(layout.spacing_track);
        self.card.set_spacing_thumb_box(layout.spacing_thumb);
        self.card.set_spacing_text(format!("{:.0} px", self.spacing).into());

        self.card.set_corner_label_box(layout.corner_label);
        self.card.set_corner_track_box(layout.corner_track);
        self.card.set_corner_thumb_box(layout.corner_thumb);
        self.card.set_corner_text(format!("{:.0} px", self.corner_radius).into());

        self.card.set_aspect_label_box(layout.aspect_label);
        self.card.set_aspect_chips(slint::ModelRc::from(std::rc::Rc::new(slint::VecModel::from(layout.aspect_chips.to_vec()))));

        self.card.set_color_label_box(layout.color_label);
        self.card.set_color_swatches(slint::ModelRc::from(std::rc::Rc::new(slint::VecModel::from(layout.color_swatches.to_vec()))));

        self.card.set_hairline_bottom(layout.hairline_bottom);
        self.card.set_reset_box(layout.reset);
        self.card.set_helper_box(layout.helper);
        self.card.set_cancel_box(layout.cancel);
        self.card.set_run_box(layout.run);

        self.card.set_selected_layout(self.selected_layout as i32);
        self.card.set_selected_aspect(self.selected_aspect as i32);
        self.card.set_selected_color(self.selected_color as i32);
    }

    pub fn update_preview(&mut self) {
        if self.preview_images.is_empty() {
            // Provide placeholder preview blocks if no files loaded
            let img1 = DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(300, 300, image::Rgba([255, 92, 0, 255])));
            let img2 = DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(300, 300, image::Rgba([10, 132, 255, 255])));
            let img3 = DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(300, 300, image::Rgba([48, 209, 88, 255])));
            self.preview_images = vec![img1, img2, img3];
        }

        let layout = match self.selected_layout {
            1 => CollageLayout::Columns,
            2 => CollageLayout::Rows,
            3 => CollageLayout::Featured,
            _ => CollageLayout::Grid,
        };

        let (stage_w, stage_h) = match self.selected_aspect {
            1 => (400, 250), // Landscape
            2 => (250, 375), // Portrait
            _ => (320, 320), // Square
        };

        // Scale down spacing and corner radius proportionally for stage preview
        let preview_scale = stage_w as f32 / 2400.0;
        let scaled_spacing = (self.spacing * preview_scale * 5.0).max(4.0);
        let scaled_corner = (self.corner_radius * preview_scale * 5.0).max(0.0);
        let color = SWATCH_COLORS[self.selected_color];

        let settings = CollageSettings {
            layout,
            width: stage_w,
            height: stage_h,
            spacing: scaled_spacing,
            corner_radius: scaled_corner,
            background: color,
        };

        if let Ok(rendered) = render_collage(&self.preview_images, &settings) {
            let buffer = SharedPixelBuffer::clone_from_slice(rendered.as_raw(), stage_w, stage_h);
            self.card.set_preview_image(Image::from_rgba8(buffer));
            self.card.set_has_preview(true);
        }

        unsafe {
            let _ = InvalidateRect(Some(self.window), None, true);
        }
    }

    pub fn show_for(&mut self, _id: &str, label: &str, _detail: &str, files: &[PathBuf]) {
        self.files = files.to_vec();
        let file_name = if files.len() > 1 {
            format!("{} photos selected", files.len())
        } else {
            files
                .first()
                .and_then(|p| p.file_name())
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "Photos".to_string())
        };

        // Downscale source images to ~400px for snappy interactive previews
        self.preview_images.clear();
        for p in files {
            if let Ok(img) = image::open(p) {
                let downscaled = img.resize(400, 400, image::imageops::FilterType::Triangle);
                self.preview_images.push(downscaled);
            }
        }

        self.card.set_file_name(file_name.into());
        self.card.set_detail(label.into());
        self.update_preview();
        self.sync_card();

        unsafe {
            let title = to_wide(&format!("Open Tangerine — {label}"));
            let _ = SetWindowTextW(self.window, PCWSTR(title.as_ptr()));
            let _ = SetLayeredWindowAttributes(self.window, COLORREF(0), 255, LWA_ALPHA);
            let _ = SetWindowPos(
                self.window,
                None,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE | windows::Win32::UI::WindowsAndMessaging::SWP_SHOWWINDOW,
            );
            let _ = ShowWindow(self.window, SW_SHOW);
            crate::platform::windows::windowhost::hide_from_taskbar(self.window);
            let _ = SetForegroundWindow(self.window);
        }
    }

    pub fn hide(&mut self) {
        unsafe {
            let _ = ShowWindow(self.window, SW_HIDE);
        }
    }

    #[allow(dead_code)]
    pub fn is_visible(&self) -> bool {
        unsafe { windows::Win32::UI::WindowsAndMessaging::IsWindowVisible(self.window).as_bool() }
    }

    pub fn render_to_buffer(&mut self) -> (i32, i32) {
        let w = CARD_W;
        let h = CARD_H;
        if self.buffer.len() != (w * h) as usize {
            self.buffer.resize((w * h) as usize, Bgra8::default());
        }
        self.sync_card();
        self.renderer_window.draw_if_needed(|renderer| {
            renderer.render(&mut self.buffer, w as usize);
        });
        (w, h)
    }

    pub fn save_capture(&mut self, path: &Path) -> Result<(), String> {
        let (w, h) = self.render_to_buffer();
        crate::platform::windows::windowhost::save_buffer_as_png(&self.buffer, w as u32, h as u32, path)
    }

    #[allow(dead_code)]
    pub fn take_events(&self) -> Vec<ToolEvent> {
        std::mem::take(&mut *self.events.borrow_mut())
    }

    unsafe extern "system" fn wnd_proc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        match msg {
            WM_ERASEBKGND => LRESULT(1),
            WM_PAINT => {
                let mut ps = PAINTSTRUCT::default();
                unsafe {
                    let _hdc = BeginPaint(hwnd, &mut ps);
                    let _ = EndPaint(hwnd, &ps);
                }
                LRESULT(0)
            }
            WM_LBUTTONDOWN => {
                let x = (lparam.0 & 0xFFFF) as i16 as f32;
                let y = ((lparam.0 >> 16) & 0xFFFF) as i16 as f32;
                HOST_RC.with(|cell| {
                    if let Some(host_rc) = cell.borrow().as_ref() {
                        let mut host = host_rc.borrow_mut();
                        let layout = Layout::of(CARD_W, CARD_H, host.spacing, host.corner_radius);

                        // Spacing slider track hit test
                        if x >= layout.spacing_track.x && x <= layout.spacing_track.x + layout.spacing_track.width
                            && y >= layout.spacing_track.y - 10.0 && y <= layout.spacing_track.y + 14.0
                        {
                            host.dragging_slider = Some(1);
                            let frac = ((x - layout.spacing_track.x) / layout.spacing_track.width).clamp(0.0, 1.0);
                            host.spacing = frac * 60.0;
                            host.update_preview();
                            host.sync_card();
                            return;
                        }

                        // Corner radius slider track hit test
                        if x >= layout.corner_track.x && x <= layout.corner_track.x + layout.corner_track.width
                            && y >= layout.corner_track.y - 10.0 && y <= layout.corner_track.y + 14.0
                        {
                            host.dragging_slider = Some(2);
                            let frac = ((x - layout.corner_track.x) / layout.corner_track.width).clamp(0.0, 1.0);
                            host.corner_radius = frac * 40.0;
                            host.update_preview();
                            host.sync_card();
                        }
                    }
                });
                LRESULT(0)
            }
            WM_MOUSEMOVE => {
                if (wparam.0 & 0x0001) != 0 { // MK_LBUTTON
                    let x = (lparam.0 & 0xFFFF) as i16 as f32;
                    HOST_RC.with(|cell| {
                        if let Some(host_rc) = cell.borrow().as_ref() {
                            let mut host = host_rc.borrow_mut();
                            let dragging = host.dragging_slider;
                            let layout = Layout::of(CARD_W, CARD_H, host.spacing, host.corner_radius);
                            if dragging == Some(1) {
                                let frac = ((x - layout.spacing_track.x) / layout.spacing_track.width).clamp(0.0, 1.0);
                                host.spacing = frac * 60.0;
                                host.update_preview();
                                host.sync_card();
                            } else if dragging == Some(2) {
                                let frac = ((x - layout.corner_track.x) / layout.corner_track.width).clamp(0.0, 1.0);
                                host.corner_radius = frac * 40.0;
                                host.update_preview();
                                host.sync_card();
                            }
                        }
                    });
                }
                LRESULT(0)
            }
            WM_LBUTTONUP => {
                HOST_RC.with(|cell| {
                    if let Some(host_rc) = cell.borrow().as_ref() {
                        host_rc.borrow_mut().dragging_slider = None;
                    }
                });
                LRESULT(0)
            }
            WM_KEYDOWN => {
                if wparam.0 == 0x1B { // Esc
                    unsafe { let _ = ShowWindow(hwnd, SW_HIDE); }
                    return LRESULT(0);
                }
                if wparam.0 == 0x09 { // Tab
                    HOST_RC.with(|cell| {
                        if let Some(host_rc) = cell.borrow().as_ref() {
                            let mut host = host_rc.borrow_mut();
                            host.selected_layout = (host.selected_layout + 1) % 4;
                            host.update_preview();
                            host.sync_card();
                        }
                    });
                    return LRESULT(0);
                }
                if wparam.0 == 0x0D { // Enter
                    HOST_RC.with(|cell| {
                        if let Some(host_rc) = cell.borrow().as_ref() {
                            let host = host_rc.borrow();
                            let layout_str = match host.selected_layout {
                                1 => "columns",
                                2 => "rows",
                                3 => "featured",
                                _ => "grid",
                            };
                            let aspect_str = match host.selected_aspect {
                                1 => "16:9",
                                2 => "9:16",
                                _ => "1:1",
                            };
                            let bg_hex = SWATCH_HEXES[host.selected_color];

                            let mut settings = ToolSettings::default();
                            settings.collage_layout = Some(layout_str.to_string());
                            settings.collage_spacing = Some(host.spacing.round() as u32);
                            settings.collage_corner_radius = Some(host.corner_radius.round() as u32);
                            settings.collage_bg_color = Some(bg_hex.to_string());
                            settings.collage_aspect = Some(aspect_str.to_string());

                            let ev = ToolEvent::Run {
                                operation: "tools.collage".to_string(),
                                inputs: host.files.clone(),
                                settings,
                            };
                            host.events.borrow_mut().push(ev.clone());
                            let _ = host.sender.send(ev);
                            unsafe { let _ = ShowWindow(hwnd, SW_HIDE); }
                        }
                    });
                    return LRESULT(0);
                }
                unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
            }
            WM_DESTROY => {
                unsafe { PostQuitMessage(0); }
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
                HOST_RC.with(|cell| {
                    if let Some(host_rc) = cell.borrow().as_ref() {
                        let mut host = host_rc.borrow_mut();
                        let scale = scale_factor(hwnd);
                        let client_w = ((CARD_W as f32) * scale).round() as u32;
                        let client_h = ((CARD_H as f32) * scale).round() as u32;
                        let _ = host.renderer_window.set_size(slint::PhysicalSize::new(client_w, client_h));
                        host.sync_card();
                    }
                });
                let _ = unsafe { InvalidateRect(Some(hwnd), None, true) };
                LRESULT(0)
            }
            _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
        }
    }
}

impl Drop for CollageWindow {
    fn drop(&mut self) {
        if !self.window.0.is_null() {
            unsafe {
                let _ = DestroyWindow(self.window);
            }
        }
    }
}

#[cfg(test)]
pub mod show_tests {
    use super::*;

    #[test]
    fn layout_measures_sane_panels() {
        let layout = Layout::of(CARD_W, CARD_H, 16.0, 16.0);
        assert_eq!(layout.layout_chips.len(), 4);
        assert_eq!(layout.aspect_chips.len(), 3);
        assert_eq!(layout.color_swatches.len(), 5);
        assert!(layout.run.width > 50.0);
    }

    #[test]
    fn showing_and_pumping_the_collage_card_does_not_panic() {
        crate::ui::wheel::install_platform();
        let (tx, _rx) = std::sync::mpsc::channel();
        let mut window = CollageWindow::new(tx).expect("collage window can be made");
        window.show_for(
            "tools.collage",
            "Collage",
            "Create photo collage",
            &[PathBuf::from("img1.jpg"), PathBuf::from("img2.jpg")],
        );
        window.hide();
    }

    #[test]
    fn capture_the_collage_window() {
        crate::ui::wheel::install_platform();
        let (tx, _rx) = std::sync::mpsc::channel();
        let mut window = CollageWindow::new(tx).expect("collage window can be made");
        window.show_for(
            "tools.collage",
            "Collage",
            "Create photo collage",
            &[PathBuf::from("img1.jpg"), PathBuf::from("img2.jpg")],
        );
        let shot_path = std::path::Path::new("target").join("shots").join("collage-window.png");
        window.save_capture(&shot_path).expect("collage window captured");
        assert!(shot_path.is_file(), "collage-window.png was written");
    }
}
