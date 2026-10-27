//! The window an image background or frame is chosen in.
//!
//! Measures card dimensions via Layout::of(), manages Slint FrameImageCard,
//! provides live preview of the image placed on background with padding,
//! rounded corners, and drop shadow. Offers gradient and solid color swatches,
//! blur, custom background image, spacing/corner/shadow sliders, and ratio chips.
//! Matches `tool-windows/frameImage.webp` and `press/add-background.png`.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc::Sender;

use slint::platform::software_renderer::MinimalSoftwareWindow;
use slint::{Image, PhysicalSize, SharedPixelBuffer};
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreateSolidBrush, EndPaint, InvalidateRect, PAINTSTRUCT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, LoadCursorW, PostQuitMessage, RegisterClassW,
    SetForegroundWindow, SetLayeredWindowAttributes, SetWindowPos, SetWindowTextW, ShowWindow,
    CW_USEDEFAULT, IDC_ARROW, LWA_ALPHA, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOZORDER, SW_HIDE,
    SW_SHOW, WM_DESTROY, WM_DPICHANGED, WM_ERASEBKGND, WM_KEYDOWN, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE,
    WM_PAINT, WNDCLASSW, WS_EX_LAYERED,
};

use crate::media::frame::{FrameBackground, FrameRatio, FrameSettings};
use crate::platform::windows::toolwindow::ToolEvent;
use crate::platform::windows::windowhost::{fixed_style, outer_size_for, scale_factor, to_wide};
use crate::radial::execution::ToolSettings;
use crate::ui::wheel::{window_for_next_component, Bgra8, FrameImageCard, Panel};

pub const CARD_W: i32 = 460;
pub const CARD_H: i32 = 660;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum SliderKind {
    Padding,
    Corner,
    Shadow,
}

pub const GRADIENT_PRESETS: [([u8; 4], [u8; 4], f32); 9] = [
    // 0 is Blur (handled specially)
    ([0, 0, 0, 0], [0, 0, 0, 0], 0.0),
    // 1: Sunset
    ([255, 90, 31, 255], [255, 51, 102, 255], 135.0),
    // 2: Purple Dream
    ([138, 35, 135, 255], [233, 64, 87, 255], 135.0),
    // 3: Ocean
    ([0, 180, 219, 255], [0, 131, 176, 255], 135.0),
    // 4: Emerald
    ([17, 153, 142, 255], [56, 239, 125, 255], 135.0),
    // 5: Sunburst
    ([255, 183, 94, 255], [237, 143, 3, 255], 135.0),
    // 6: Sky
    ([76, 161, 175, 255], [196, 224, 229, 255], 135.0),
    // 7: Neon Violet
    ([101, 78, 163, 255], [234, 175, 200, 255], 135.0),
    // 8: Dark Slate
    ([35, 37, 38, 255], [65, 67, 69, 255], 135.0),
];

pub const SOLID_PRESETS: [[u8; 4]; 10] = [
    [255, 255, 255, 255], // White
    [247, 246, 244, 255], // Cream / Neutral
    [254, 228, 64, 255],  // Lemon
    [46, 196, 182, 255],  // Teal
    [0, 100, 102, 255],   // Forest
    [0, 180, 216, 255],   // Cyan
    [58, 134, 255, 255],  // Blue
    [114, 9, 183, 255],   // Purple
    [43, 45, 66, 255],    // Slate
    [26, 26, 26, 255],    // Dark
];

#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    pub client_width: i32,
    pub client_height: i32,
    pub header: Panel,
    pub detail: Panel,
    pub hairline_top: Panel,
    pub stage: Panel,
    pub gradient_swatches: [Panel; 10],
    pub solid_swatches: [Panel; 10],
    pub padding_slider: Panel,
    pub corner_slider: Panel,
    pub shadow_slider: Panel,
    pub ratio_auto: Panel,
    pub ratio_square: Panel,
    pub ratio_wide: Panel,
    pub ratio_tall: Panel,
    pub hairline_bottom: Panel,
    pub reset: Panel,
    pub helper: Panel,
    pub run: Panel,
}

impl Layout {
    pub const WIDTH_UNITS: f32 = 460.0;
    #[allow(dead_code)]
    pub const HEIGHT_UNITS: f32 = 660.0;

    pub fn of(client_width: i32, client_height: i32) -> Self {
        let scale = (client_width as f32 / Self::WIDTH_UNITS).max(0.5);
        let w = client_width as f32;
        let pad = 20.0 * scale;
        let content_w = w - pad * 2.0;

        let header = Panel {
            x: pad,
            y: 14.0 * scale,
            width: content_w,
            height: 22.0 * scale,
        };
        let detail = Panel {
            x: pad,
            y: 36.0 * scale,
            width: content_w,
            height: 18.0 * scale,
        };
        let hairline_top = Panel {
            x: 0.0,
            y: 58.0 * scale,
            width: w,
            height: 1.0,
        };

        // Preview Stage: 420 x 230
        let stage = Panel {
            x: pad,
            y: 68.0 * scale,
            width: content_w,
            height: 230.0 * scale,
        };

        // Swatches: 10 in row 1, 10 in row 2
        let swatch_d = 26.0 * scale;
        let swatch_gap = (content_w - 10.0 * swatch_d) / 9.0;
        let mut gradient_swatches: [Panel; 10] = std::array::from_fn(|_| Panel::default());
        let mut solid_swatches: [Panel; 10] = std::array::from_fn(|_| Panel::default());
        let y_row1 = 312.0 * scale;
        let y_row2 = 346.0 * scale;
        for i in 0..10 {
            let sx = pad + (i as f32) * (swatch_d + swatch_gap);
            gradient_swatches[i] = Panel {
                x: sx,
                y: y_row1,
                width: swatch_d,
                height: swatch_d,
            };
            solid_swatches[i] = Panel {
                x: sx,
                y: y_row2,
                width: swatch_d,
                height: swatch_d,
            };
        }

        // Sliders
        let padding_slider = Panel {
            x: pad,
            y: 386.0 * scale,
            width: content_w,
            height: 40.0 * scale,
        };
        let corner_slider = Panel {
            x: pad,
            y: 430.0 * scale,
            width: content_w,
            height: 40.0 * scale,
        };
        let shadow_slider = Panel {
            x: pad,
            y: 474.0 * scale,
            width: content_w,
            height: 40.0 * scale,
        };

        // Ratio chips (4 chips across content_w)
        let chip_gap = 8.0 * scale;
        let chip_w = (content_w - 3.0 * chip_gap) / 4.0;
        let chip_h = 28.0 * scale;
        let y_ratio = 524.0 * scale;

        let ratio_auto = Panel {
            x: pad,
            y: y_ratio,
            width: chip_w,
            height: chip_h,
        };
        let ratio_square = Panel {
            x: pad + 1.0 * (chip_w + chip_gap),
            y: y_ratio,
            width: chip_w,
            height: chip_h,
        };
        let ratio_wide = Panel {
            x: pad + 2.0 * (chip_w + chip_gap),
            y: y_ratio,
            width: chip_w,
            height: chip_h,
        };
        let ratio_tall = Panel {
            x: pad + 3.0 * (chip_w + chip_gap),
            y: y_ratio,
            width: chip_w,
            height: chip_h,
        };

        let hairline_bottom = Panel {
            x: 0.0,
            y: 566.0 * scale,
            width: w,
            height: 1.0,
        };

        // Footer
        let y_footer = 580.0 * scale;
        let reset = Panel {
            x: pad,
            y: y_footer,
            width: 72.0 * scale,
            height: 36.0 * scale,
        };
        let run_w = 170.0 * scale;
        let run = Panel {
            x: w - pad - run_w,
            y: y_footer,
            width: run_w,
            height: 36.0 * scale,
        };
        let helper = Panel {
            x: pad + 76.0 * scale,
            y: y_footer,
            width: w - pad * 2.0 - 76.0 * scale - run_w - 8.0 * scale,
            height: 36.0 * scale,
        };

        Self {
            client_width,
            client_height,
            header,
            detail,
            hairline_top,
            stage,
            gradient_swatches,
            solid_swatches,
            padding_slider,
            corner_slider,
            shadow_slider,
            ratio_auto,
            ratio_square,
            ratio_wide,
            ratio_tall,
            hairline_bottom,
            reset,
            helper,
            run,
        }
    }
}

pub struct Host {
    pub window: HWND,
    pub card: FrameImageCard,
    pub software: Rc<MinimalSoftwareWindow>,
    pub width: u32,
    pub height: u32,
    pub buffer: Vec<Bgra8>,
    pub sender: Sender<ToolEvent>,
    pub operation_id: String,
    pub file_path: Option<PathBuf>,
    pub source_image: Option<image::DynamicImage>,
    pub selected_row: usize, // 0 = gradients/blur/custom, 1 = solids
    pub selected_col: usize, // 0..9
    pub padding: f32,        // 0.0 .. 0.3
    pub corner_radius: f32,  // 0.0 .. 50.0
    pub shadow: f32,         // 0.0 .. 50.0
    pub ratio_index: usize,  // 0=Auto, 1=1:1, 2=16:9, 3=4:5
    pub custom_bg_path: Option<PathBuf>,
    pub dragging_slider: Option<SliderKind>,
}

thread_local! {
    static CURRENT_HOST: RefCell<Option<Rc<RefCell<Host>>>> = const { RefCell::new(None) };
    static ACTIVE_HWND: Cell<HWND> = const { Cell::new(HWND(std::ptr::null_mut())) };
}

impl Host {
    pub fn resize(&mut self, w: u32, h: u32) {
        self.width = w.max(1);
        self.height = h.max(1);
        self.software
            .set_size(PhysicalSize::new(self.width, self.height));
        self.buffer = vec![Bgra8::default(); (self.width * self.height) as usize];
        self.card.set_card_width(w as f32);
        self.card.set_card_height(h as f32);
        self.apply_layout();
    }

    pub fn apply_layout(&self) {
        let layout = Layout::of(self.width as i32, self.height as i32);
        self.card.set_header_box(layout.header);
        self.card.set_detail_box(layout.detail);
        self.card.set_hairline_top(layout.hairline_top);
        self.card.set_stage_box(layout.stage);

        let grad_model = Rc::new(slint::VecModel::from(layout.gradient_swatches.to_vec()));
        self.card
            .set_gradient_swatches(slint::ModelRc::from(grad_model));
        let solid_model = Rc::new(slint::VecModel::from(layout.solid_swatches.to_vec()));
        self.card
            .set_solid_swatches(slint::ModelRc::from(solid_model));

        self.card.set_padding_box(layout.padding_slider);
        self.card.set_corner_box(layout.corner_slider);
        self.card.set_shadow_box(layout.shadow_slider);

        self.card.set_ratio_auto_box(layout.ratio_auto);
        self.card.set_ratio_square_box(layout.ratio_square);
        self.card.set_ratio_wide_box(layout.ratio_wide);
        self.card.set_ratio_tall_box(layout.ratio_tall);

        self.card.set_hairline_bottom(layout.hairline_bottom);
        self.card.set_reset_box(layout.reset);
        self.card.set_helper_box(layout.helper);
        self.card.set_run_box(layout.run);

        self.card.set_selected_row(self.selected_row as i32);
        self.card.set_selected_col(self.selected_col as i32);
        self.card.set_padding_val(self.padding);
        let px_pad = (self.padding * 2000.0).round() as i32;
        self.card.set_padding_text(slint::SharedString::from(format!("{} px", px_pad)));
        self.card.set_corner_val(self.corner_radius);
        self.card.set_corner_text(slint::SharedString::from(format!("{:.0} px", self.corner_radius)));
        self.card.set_shadow_val(self.shadow);
        self.card.set_shadow_text(slint::SharedString::from(format!("{:.0} px", self.shadow)));
        self.card.set_ratio_index(self.ratio_index as i32);
    }

    pub fn render_preview(&mut self) {
        let Some(source) = &self.source_image else {
            self.card.set_has_preview(false);
            return;
        };

        let aspect = match self.ratio_index {
            1 => FrameRatio::Square1x1,
            2 => FrameRatio::Wide16x9,
            3 => FrameRatio::Tall4x5,
            _ => FrameRatio::Auto,
        };

        let background = if self.selected_row == 0 {
            if self.selected_col == 0 {
                FrameBackground::Blur { radius: 30.0 }
            } else if self.selected_col == 9 {
                if let Some(path) = &self.custom_bg_path {
                    if let Ok(img) = image::open(path) {
                        FrameBackground::Image(img.to_rgba8())
                    } else {
                        FrameBackground::Blur { radius: 30.0 }
                    }
                } else {
                    FrameBackground::Blur { radius: 30.0 }
                }
            } else {
                let (ca, cb, angle) = GRADIENT_PRESETS[self.selected_col];
                FrameBackground::Gradient {
                    color_a: ca,
                    color_b: cb,
                    angle_deg: angle,
                }
            }
        } else {
            let col = self.selected_col.min(SOLID_PRESETS.len() - 1);
            FrameBackground::Color(SOLID_PRESETS[col])
        };

        let settings = FrameSettings {
            aspect,
            padding_fraction: self.padding,
            corner_radius: self.corner_radius,
            shadow_radius: self.shadow,
            background,
        };

        // Render preview with downscaled source for snappy feedback
        let preview_source = source.resize(480, 480, image::imageops::FilterType::Triangle);
        if let Ok(framed) = crate::media::frame::render_frame(&preview_source, &settings, false) {
            let (fw, fh) = (framed.width(), framed.height());
            let buffer = SharedPixelBuffer::clone_from_slice(framed.as_raw(), fw, fh);
            self.card.set_preview_image(Image::from_rgba8(buffer));
            self.card.set_has_preview(true);
        }

        unsafe {
            let _ = InvalidateRect(Some(self.window), None, true);
        }
    }

    pub fn reset_defaults(&mut self) {
        self.selected_row = 0;
        self.selected_col = 0;
        self.padding = 0.08;
        self.corner_radius = 24.0;
        self.shadow = 24.0;
        self.ratio_index = 0;
        self.apply_layout();
        self.render_preview();
    }

    pub fn run(&self) {
        let Some(path) = &self.file_path else {
            return;
        };

        let bg_kind = if self.selected_row == 0 {
            if self.selected_col == 0 {
                "blur"
            } else if self.selected_col == 9 {
                "image"
            } else {
                "gradient"
            }
        } else {
            "color"
        };

        let (color_a, color_b, gradient_angle) = if self.selected_row == 0 {
            if self.selected_col >= 1 && self.selected_col <= 8 {
                let (ca, cb, angle) = GRADIENT_PRESETS[self.selected_col];
                (
                    Some(format!("{:02X}{:02X}{:02X}", ca[0], ca[1], ca[2])),
                    Some(format!("{:02X}{:02X}{:02X}", cb[0], cb[1], cb[2])),
                    Some(angle as u32),
                )
            } else {
                (None, None, None)
            }
        } else {
            let c = SOLID_PRESETS[self.selected_col.min(SOLID_PRESETS.len() - 1)];
            (Some(format!("{:02X}{:02X}{:02X}", c[0], c[1], c[2])), None, None)
        };

        let aspect = match self.ratio_index {
            1 => "1:1",
            2 => "16:9",
            3 => "4:5",
            _ => "auto",
        };

        let settings = ToolSettings {
            frame_bg_kind: Some(bg_kind.to_string()),
            frame_color_a: color_a,
            frame_color_b: color_b,
            frame_gradient_angle: gradient_angle,
            frame_bg_image: self.custom_bg_path.clone(),
            frame_padding_x1000: Some((self.padding * 1000.0).round() as u32),
            frame_corner_radius: Some(self.corner_radius.round() as u32),
            frame_shadow: Some(self.shadow.round() as u32),
            frame_aspect: Some(aspect.to_string()),
            ..Default::default()
        };

        let _ = self.sender.send(ToolEvent::Run {
            operation: self.operation_id.clone(),
            inputs: vec![path.clone()],
            settings,
        });
        unsafe {
            let _ = ShowWindow(self.window, SW_HIDE);
        }
    }

    pub fn render_to_dc(&mut self, _dc: windows::Win32::Graphics::Gdi::HDC) {
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

pub struct FrameImageWindow {
    pub window: HWND,
    #[allow(dead_code)]
    pub sender: Sender<ToolEvent>,
}

impl FrameImageWindow {
    pub fn new(sender: Sender<ToolEvent>) -> Result<Self, String> {
        let class_name = w!("OpenTangerineFrameImageCard");
        let instance = unsafe { GetModuleHandleW(None).map_err(|e| e.to_string())? };
        let wc = WNDCLASSW {
            lpfnWndProc: Some(frame_card_proc),
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
        let card = FrameImageCard::new().map_err(|e| e.to_string())?;

        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_LAYERED,
                class_name,
                w!("Add Background"),
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

        let host = Rc::new(RefCell::new(Host {
            window: hwnd,
            card,
            software,
            width: CARD_W as u32,
            height: CARD_H as u32,
            buffer: vec![Bgra8::default(); (CARD_W * CARD_H) as usize],
            sender: sender.clone(),
            operation_id: "tools.frame_image".to_string(),
            file_path: None,
            source_image: None,
            selected_row: 0,
            selected_col: 0,
            padding: 0.08,
            corner_radius: 24.0,
            shadow: 24.0,
            ratio_index: 0,
            custom_bg_path: None,
            dragging_slider: None,
        }));

        CURRENT_HOST.with(|c| *c.borrow_mut() = Some(host));
        ACTIVE_HWND.with(|a| a.set(hwnd));

        Ok(Self {
            window: hwnd,
            sender,
        })
    }

    pub fn show_for(&self, operation_id: &str, _label: &str, _detail: &str, files: &[PathBuf]) {
        if files.is_empty() {
            return;
        }
        let input = &files[0];
        let file_name = input
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "image.png".to_string());

        let loaded_img = image::open(input).ok();

        CURRENT_HOST.with(|c| {
            if let Some(host_rc) = c.borrow().as_ref() {
                let mut host = host_rc.borrow_mut();
                host.operation_id = operation_id.to_string();
                host.file_path = Some(input.clone());
                host.source_image = loaded_img;
                host.card.set_file_name(slint::SharedString::from(&file_name));
                host.card.set_detail(slint::SharedString::from("Add Background"));

                host.reset_defaults();
            }
        });

        let (outer_w, outer_h) = outer_size_for(self.window, CARD_W, CARD_H);
        unsafe {
            let _ = SetWindowTextW(self.window, PCWSTR::from_raw(to_wide("Add Background").as_ptr()));
            let _ = SetWindowPos(
                self.window,
                None,
                0,
                0,
                outer_w,
                outer_h,
                SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
            );
            let _ = ShowWindow(self.window, SW_SHOW);
            let _ = SetForegroundWindow(self.window);
            let _ = InvalidateRect(Some(self.window), None, true);
        }
    }
}

unsafe extern "system" fn frame_card_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            let dc = unsafe { BeginPaint(hwnd, &mut ps) };
            CURRENT_HOST.with(|c| {
                if let Some(host_rc) = c.borrow().as_ref() {
                    host_rc.borrow_mut().render_to_dc(dc);
                }
            });
            unsafe {
                let _ = EndPaint(hwnd, &ps);
            };
            LRESULT(0)
        }
        WM_ERASEBKGND => LRESULT(1),
        WM_LBUTTONDOWN => {
            let x = (lparam.0 & 0xFFFF) as i16 as f32;
            let y = ((lparam.0 >> 16) & 0xFFFF) as i16 as f32;

            CURRENT_HOST.with(|c| {
                if let Some(host_rc) = c.borrow().as_ref() {
                    let mut host = host_rc.borrow_mut();
                    let layout = Layout::of(host.width as i32, host.height as i32);

                    // 1. Swatches Row 1
                    for (i, p) in layout.gradient_swatches.iter().enumerate() {
                        if point_in_panel(x, y, p) {
                            host.selected_row = 0;
                            host.selected_col = i;
                            host.apply_layout();
                            host.render_preview();
                            return;
                        }
                    }

                    // 2. Swatches Row 2
                    for (i, p) in layout.solid_swatches.iter().enumerate() {
                        if point_in_panel(x, y, p) {
                            host.selected_row = 1;
                            host.selected_col = i;
                            host.apply_layout();
                            host.render_preview();
                            return;
                        }
                    }

                    // 3. Ratio chips
                    if point_in_panel(x, y, &layout.ratio_auto) {
                        host.ratio_index = 0;
                        host.apply_layout();
                        host.render_preview();
                        return;
                    }
                    if point_in_panel(x, y, &layout.ratio_square) {
                        host.ratio_index = 1;
                        host.apply_layout();
                        host.render_preview();
                        return;
                    }
                    if point_in_panel(x, y, &layout.ratio_wide) {
                        host.ratio_index = 2;
                        host.apply_layout();
                        host.render_preview();
                        return;
                    }
                    if point_in_panel(x, y, &layout.ratio_tall) {
                        host.ratio_index = 3;
                        host.apply_layout();
                        host.render_preview();
                        return;
                    }

                    // 4. Sliders
                    if point_in_panel(x, y, &layout.padding_slider) {
                        host.dragging_slider = Some(SliderKind::Padding);
                        let frac = ((x - layout.padding_slider.x) / layout.padding_slider.width).clamp(0.0, 1.0);
                        host.padding = frac * 0.3;
                        host.apply_layout();
                        host.render_preview();
                        return;
                    }
                    if point_in_panel(x, y, &layout.corner_slider) {
                        host.dragging_slider = Some(SliderKind::Corner);
                        let frac = ((x - layout.corner_slider.x) / layout.corner_slider.width).clamp(0.0, 1.0);
                        host.corner_radius = frac * 50.0;
                        host.apply_layout();
                        host.render_preview();
                        return;
                    }
                    if point_in_panel(x, y, &layout.shadow_slider) {
                        host.dragging_slider = Some(SliderKind::Shadow);
                        let frac = ((x - layout.shadow_slider.x) / layout.shadow_slider.width).clamp(0.0, 1.0);
                        host.shadow = frac * 50.0;
                        host.apply_layout();
                        host.render_preview();
                        return;
                    }

                    // 5. Reset
                    if point_in_panel(x, y, &layout.reset) {
                        host.reset_defaults();
                        return;
                    }

                    // 6. Run
                    if point_in_panel(x, y, &layout.run) {
                        host.run();
                        return;
                    }
                }
            });
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            let x = (lparam.0 & 0xFFFF) as i16 as f32;
            let _y = ((lparam.0 >> 16) & 0xFFFF) as i16 as f32;

            CURRENT_HOST.with(|c| {
                if let Some(host_rc) = c.borrow().as_ref() {
                    let mut host = host_rc.borrow_mut();
                    if let Some(kind) = host.dragging_slider {
                        let layout = Layout::of(host.width as i32, host.height as i32);
                        match kind {
                            SliderKind::Padding => {
                                let frac = ((x - layout.padding_slider.x) / layout.padding_slider.width).clamp(0.0, 1.0);
                                host.padding = frac * 0.3;
                            }
                            SliderKind::Corner => {
                                let frac = ((x - layout.corner_slider.x) / layout.corner_slider.width).clamp(0.0, 1.0);
                                host.corner_radius = frac * 50.0;
                            }
                            SliderKind::Shadow => {
                                let frac = ((x - layout.shadow_slider.x) / layout.shadow_slider.width).clamp(0.0, 1.0);
                                host.shadow = frac * 50.0;
                            }
                        }
                        host.apply_layout();
                        host.render_preview();
                    }
                }
            });
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            CURRENT_HOST.with(|c| {
                if let Some(host_rc) = c.borrow().as_ref() {
                    host_rc.borrow_mut().dragging_slider = None;
                }
            });
            LRESULT(0)
        }
        WM_KEYDOWN => {
            match wparam.0 {
                0x1B => {
                    // Escape
                    unsafe {
                        let _ = ShowWindow(hwnd, SW_HIDE);
                    }
                }
                0x0D => {
                    // Enter
                    CURRENT_HOST.with(|c| {
                        if let Some(host_rc) = c.borrow().as_ref() {
                            host_rc.borrow().run();
                        }
                    });
                }
                0x52 => {
                    // 'R' -> reset
                    CURRENT_HOST.with(|c| {
                        if let Some(host_rc) = c.borrow().as_ref() {
                            host_rc.borrow_mut().reset_defaults();
                        }
                    });
                }
                0x09 => {
                    // Tab -> cycle presets
                    CURRENT_HOST.with(|c| {
                        if let Some(host_rc) = c.borrow().as_ref() {
                            let mut host = host_rc.borrow_mut();
                            host.selected_col = (host.selected_col + 1) % 6;
                            host.apply_layout();
                            host.render_preview();
                        }
                    });
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
            CURRENT_HOST.with(|cell| {
                if let Some(host_rc) = cell.borrow().as_ref() {
                    let mut host = host_rc.borrow_mut();
                    let scale = scale_factor(hwnd);
                    let phys_w = (CARD_W as f32 * scale).round() as u32;
                    let phys_h = (CARD_H as f32 * scale).round() as u32;
                    host.resize(phys_w, phys_h);
                }
            });
            let _ = unsafe { InvalidateRect(Some(hwnd), None, true) };
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

#[inline]
fn point_in_panel(x: f32, y: f32, p: &Panel) -> bool {
    x >= p.x && x <= p.x + p.width && y >= p.y && y <= p.y + p.height
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_places_swatches_and_sliders_proportionally() {
        let l = Layout::of(CARD_W, CARD_H);
        assert_eq!(l.client_width, CARD_W);
        assert_eq!(l.client_height, CARD_H);
        assert_eq!(l.gradient_swatches.len(), 10);
        assert_eq!(l.solid_swatches.len(), 10);
        assert!(l.stage.height > 100.0);
        assert!(l.padding_slider.y > l.solid_swatches[0].y);
        assert!(l.corner_slider.y > l.padding_slider.y);
        assert!(l.shadow_slider.y > l.corner_slider.y);
        assert!(l.ratio_auto.y > l.shadow_slider.y);
        assert!(l.run.y > l.hairline_bottom.y);
    }

    #[test]
    fn layout_ratio_chips_have_consistent_widths() {
        let l = Layout::of(CARD_W, CARD_H);
        assert!((l.ratio_auto.width - l.ratio_square.width).abs() < 1.0);
        assert!((l.ratio_square.width - l.ratio_wide.width).abs() < 1.0);
        assert!((l.ratio_wide.width - l.ratio_tall.width).abs() < 1.0);
    }
}

#[cfg(test)]
pub mod show_tests {
    use super::*;
    use std::path::Path;
    use std::sync::mpsc::channel;

    #[test]
    fn showing_and_pumping_the_frame_image_card_does_not_panic() {
        crate::ui::wheel::install_platform();
        let (tx, _rx) = channel();
        let win = FrameImageWindow::new(tx).expect("window created");
        let scratch = std::env::temp_dir().join(format!("ot-frame-pump-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&scratch);
        let sample = scratch.join("pump_photo.png");
        let img = image::DynamicImage::new_rgb8(120, 80);
        img.save(&sample).unwrap();

        win.show_for("tools.frame_image", "Add BG", "Add background", &[sample]);

        CURRENT_HOST.with(|c| {
            if let Some(host_rc) = c.borrow().as_ref() {
                let mut host = host_rc.borrow_mut();
                host.resize(CARD_W as u32, CARD_H as u32);
                let stride = host.width as usize;
                let mut buf = vec![Bgra8::default(); (host.width * host.height) as usize];
                host.software.draw_if_needed(|renderer| {
                    renderer.render(&mut buf, stride);
                });
            }
        });

        unsafe {
            let _ = ShowWindow(win.window, SW_HIDE);
        }
        let _ = std::fs::remove_dir_all(&scratch);
    }

    #[test]
    fn capture_the_frame_image_window() {
        crate::ui::wheel::install_platform();
        let (tx, _rx) = channel();
        let win = FrameImageWindow::new(tx).expect("window created");
        let scratch = std::env::temp_dir().join(format!("ot-frame-cap-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&scratch);
        let sample = scratch.join("sample_frame.png");
        
        // Make a visually interesting photo (e.g. gradient square)
        let mut img = image::RgbImage::new(240, 180);
        for y in 0..180 {
            for x in 0..240 {
                img.put_pixel(x, y, image::Rgb([(x * 255 / 240) as u8, (y * 255 / 180) as u8, 180]));
            }
        }
        img.save(&sample).unwrap();

        win.show_for("tools.frame_image", "Add BG", "Add Background", &[sample]);

        // Select Sunset gradient preset (Row 0, Col 1) for the shot
        CURRENT_HOST.with(|c| {
            if let Some(host_rc) = c.borrow().as_ref() {
                let mut host = host_rc.borrow_mut();
                host.selected_row = 0;
                host.selected_col = 1; // Sunset gradient
                host.apply_layout();
                host.render_preview();
            }
        });

        let mut buffer = Vec::new();
        let (mut w, mut h) = (0u32, 0u32);

        CURRENT_HOST.with(|c| {
            if let Some(host_rc) = c.borrow().as_ref() {
                let mut host = host_rc.borrow_mut();
                host.resize(CARD_W as u32, CARD_H as u32);
                w = host.width;
                h = host.height;
                let stride = w as usize;
                buffer = vec![Bgra8::default(); (w * h) as usize];
                let buf_slice = &mut buffer;
                host.software.draw_if_needed(|renderer| {
                    renderer.render(buf_slice, stride);
                });
            }
        });

        assert!(!buffer.is_empty(), "buffer was rendered");
        let shots_dir = Path::new("target").join("shots");
        let _ = std::fs::create_dir_all(&shots_dir);
        let shot_path = shots_dir.join("frame-image-window.png");

        let mut rgba = Vec::with_capacity(buffer.len() * 4);
        for pixel in &buffer {
            let (_, r, g, b) = pixel.parts();
            rgba.extend_from_slice(&[r, g, b, 255]);
        }
        let _ = image::save_buffer(
            &shot_path,
            &rgba,
            w,
            h,
            image::ExtendedColorType::Rgba8,
        );

        unsafe {
            let _ = ShowWindow(win.window, SW_HIDE);
        }
        let _ = std::fs::remove_dir_all(&scratch);
    }
}
