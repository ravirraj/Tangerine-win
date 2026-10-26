//! Win32 host and Slint card integration for the Edit Photos window (P3-13 / `tools.edit_image`).

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
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, LoadCursorW, PostQuitMessage,
    RegisterClassW, SetForegroundWindow, SetLayeredWindowAttributes, SetWindowPos,
    SetWindowTextW, ShowWindow, CW_USEDEFAULT, IDC_ARROW, LWA_ALPHA, SWP_NOACTIVATE,
    SWP_NOMOVE, SWP_NOZORDER, SW_HIDE, SW_SHOW, WM_DESTROY, WM_DPICHANGED, WM_ERASEBKGND, WM_KEYDOWN,
    WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_PAINT, WNDCLASSW, WS_EX_LAYERED,
};

use crate::media::adjustments::{
    apply_adjustments, PhotoAdjustments,
};
use crate::platform::windows::toolwindow::ToolEvent;
use crate::platform::windows::windowhost::{fixed_style, scale_factor, to_wide};
use crate::radial::execution::ToolSettings;
use crate::ui::wheel::{window_for_next_component, Bgra8, EditImageCard, Panel};

pub const CARD_W: i32 = 440;
pub const CARD_H: i32 = 680;

#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    pub client_width: i32,
    pub client_height: i32,
    pub header: Panel,
    pub detail: Panel,
    pub hairline_top: Panel,
    pub stage: Panel,
    pub exposure_label: Panel,
    pub exposure_track: Panel,
    pub exposure_thumb: Panel,
    pub brightness_label: Panel,
    pub brightness_track: Panel,
    pub brightness_thumb: Panel,
    pub contrast_label: Panel,
    pub contrast_track: Panel,
    pub contrast_thumb: Panel,
    pub saturation_label: Panel,
    pub saturation_track: Panel,
    pub saturation_thumb: Panel,
    pub warmth_label: Panel,
    pub warmth_track: Panel,
    pub warmth_thumb: Panel,
    pub vignette_label: Panel,
    pub vignette_track: Panel,
    pub vignette_thumb: Panel,
    pub hairline_bottom: Panel,
    pub reset: Panel,
    pub helper: Panel,
    pub cancel: Panel,
    pub run: Panel,
}

impl Layout {
    pub fn of(w: i32, h: i32, adj: &PhotoAdjustments) -> Self {
        let p = |x: f32, y: f32, width: f32, height: f32| Panel {
            x,
            y,
            width,
            height,
        };

        // Track width is 400px (20 to 420)
        let tw = 400.0;
        let thumb_w = 16.0;

        let frac_exp = ((adj.exposure_x100 as f32 + 200.0) / 400.0).clamp(0.0, 1.0);
        let exp_track = p(20.0, 344.0, tw, 4.0);
        let exp_thumb = p(20.0 + frac_exp * (tw - thumb_w), 338.0, thumb_w, thumb_w);

        let frac_bri = ((adj.brightness_x100 as f32 + 50.0) / 100.0).clamp(0.0, 1.0);
        let bri_track = p(20.0, 386.0, tw, 4.0);
        let bri_thumb = p(20.0 + frac_bri * (tw - thumb_w), 380.0, thumb_w, thumb_w);

        let frac_con = ((adj.contrast_x100 as f32 - 50.0) / 100.0).clamp(0.0, 1.0);
        let con_track = p(20.0, 428.0, tw, 4.0);
        let con_thumb = p(20.0 + frac_con * (tw - thumb_w), 422.0, thumb_w, thumb_w);

        let frac_sat = (adj.saturation_x100 as f32 / 200.0).clamp(0.0, 1.0);
        let sat_track = p(20.0, 470.0, tw, 4.0);
        let sat_thumb = p(20.0 + frac_sat * (tw - thumb_w), 464.0, thumb_w, thumb_w);

        let frac_war = ((adj.temperature as f32 + 100.0) / 200.0).clamp(0.0, 1.0);
        let war_track = p(20.0, 512.0, tw, 4.0);
        let war_thumb = p(20.0 + frac_war * (tw - thumb_w), 506.0, thumb_w, thumb_w);

        let frac_vig = (adj.vignette_x100 as f32 / 200.0).clamp(0.0, 1.0);
        let vig_track = p(20.0, 554.0, tw, 4.0);
        let vig_thumb = p(20.0 + frac_vig * (tw - thumb_w), 548.0, thumb_w, thumb_w);

        Self {
            client_width: w,
            client_height: h,
            header: p(20.0, 14.0, 400.0, 24.0),
            detail: p(20.0, 38.0, 400.0, 18.0),
            hairline_top: p(0.0, 62.0, w as f32, 1.0),
            stage: p(20.0, 74.0, 400.0, 240.0),
            exposure_label: p(20.0, 326.0, tw, 16.0),
            exposure_track: exp_track,
            exposure_thumb: exp_thumb,
            brightness_label: p(20.0, 368.0, tw, 16.0),
            brightness_track: bri_track,
            brightness_thumb: bri_thumb,
            contrast_label: p(20.0, 410.0, tw, 16.0),
            contrast_track: con_track,
            contrast_thumb: con_thumb,
            saturation_label: p(20.0, 452.0, tw, 16.0),
            saturation_track: sat_track,
            saturation_thumb: sat_thumb,
            warmth_label: p(20.0, 494.0, tw, 16.0),
            warmth_track: war_track,
            warmth_thumb: war_thumb,
            vignette_label: p(20.0, 536.0, tw, 16.0),
            vignette_track: vig_track,
            vignette_thumb: vig_thumb,
            hairline_bottom: p(0.0, 594.0, w as f32, 1.0),
            reset: p(20.0, 612.0, 56.0, 36.0),
            helper: p(84.0, 612.0, 130.0, 36.0),
            cancel: p(222.0, 612.0, 64.0, 36.0),
            run: p(294.0, 612.0, 126.0, 36.0),
        }
    }
}

pub struct EditImageWindow {
    window: HWND,
    renderer_window: Rc<MinimalSoftwareWindow>,
    card: EditImageCard,
    buffer: Vec<Bgra8>,
    events: Rc<RefCell<Vec<ToolEvent>>>,
    sender: Sender<ToolEvent>,
    files: Vec<PathBuf>,
    source_image: Option<DynamicImage>,
    adjustments: PhotoAdjustments,
    show_original: bool,
    dragging_slider: Option<u8>, // 1..6
}

thread_local! {
    static HOST_RC: RefCell<Option<Rc<RefCell<EditImageWindow>>>> = const { RefCell::new(None) };
}

impl EditImageWindow {
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
            source_image: None,
            adjustments: PhotoAdjustments {
                exposure_x100: 0,
                brightness_x100: 0,
                contrast_x100: 100,
                saturation_x100: 100,
                temperature: 0,
                tint: 0,
                sharpness_x100: 0,
                vignette_x100: 0,
            },
            show_original: false,
            dragging_slider: None,
        };

        win.wire_callbacks();
        win.sync_card();
        Ok(win)
    }

    fn create_window() -> Result<(HWND, Rc<MinimalSoftwareWindow>, EditImageCard), String> {
        let renderer_window = window_for_next_component();
        let card = EditImageCard::new().map_err(|e| e.to_string())?;

        let class_name = to_wide("OpenTangerineEditImageCardWindow");
        let hinstance = unsafe { GetModuleHandleW(PCWSTR::null()).map_err(|e| e.to_string())? };
        let wc = WNDCLASSW {
            lpfnWndProc: Some(Self::wnd_proc),
            hInstance: hinstance.into(),
            lpszClassName: PCWSTR(class_name.as_ptr()),
            hCursor: unsafe { LoadCursorW(None, IDC_ARROW) }.unwrap_or_default(),
            hbrBackground: windows::Win32::Graphics::Gdi::HBRUSH(std::ptr::null_mut()),
            ..Default::default()
        };
        unsafe {
            let _ = RegisterClassW(&wc);
        }

        let scale = scale_factor(HWND(std::ptr::null_mut()));
        let client_w = ((CARD_W as f32) * scale).round() as i32;
        let client_h = ((CARD_H as f32) * scale).round() as i32;
        let mut rect = RECT {
            left: 0,
            top: 0,
            right: client_w,
            bottom: client_h,
        };
        let style = fixed_style();
        let _ = unsafe {
            windows::Win32::UI::WindowsAndMessaging::AdjustWindowRectEx(
                &mut rect,
                style,
                false,
                WS_EX_LAYERED,
            )
        };

        let title = to_wide("Adjust Photo — Open Tangerine");
        let window = unsafe {
            CreateWindowExW(
                WS_EX_LAYERED,
                PCWSTR(class_name.as_ptr()),
                PCWSTR(title.as_ptr()),
                style,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                rect.right - rect.left,
                rect.bottom - rect.top,
                None,
                None,
                Some(hinstance.into()),
                None,
            )
            .map_err(|e| e.to_string())?
        };

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

        self.card.on_compare_pressed(|holding| {
            HOST_RC.with(|cell| {
                if let Some(host_rc) = cell.borrow().as_ref() {
                    host_rc.borrow_mut().show_original = holding;
                    host_rc.borrow_mut().update_preview();
                }
            });
        });

        self.card.on_reset_defaults_chosen(|| {
            HOST_RC.with(|cell| {
                if let Some(host_rc) = cell.borrow().as_ref() {
                    let mut host = host_rc.borrow_mut();
                    host.adjustments = PhotoAdjustments {
                        exposure_x100: 0,
                        brightness_x100: 0,
                        contrast_x100: 100,
                        saturation_x100: 100,
                        temperature: 0,
                        tint: 0,
                        sharpness_x100: 0,
                        vignette_x100: 0,
                    };
                    host.update_preview();
                    host.sync_card();
                }
            });
        });

        self.card.on_run_chosen(|| {
            HOST_RC.with(|cell| {
                if let Some(host_rc) = cell.borrow().as_ref() {
                    let host = host_rc.borrow();
                    let mut settings = ToolSettings::default();
                    settings.photo_adjustments = Some(host.adjustments);

                    let ev = ToolEvent::Run {
                        operation: "tools.edit_image".to_string(),
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
        let layout = Layout::of(CARD_W, CARD_H, &self.adjustments);
        self.card.set_header_box(layout.header);
        self.card.set_detail_box(layout.detail);
        self.card.set_hairline_top(layout.hairline_top);
        self.card.set_stage_box(layout.stage);

        // 1. Exposure
        self.card.set_exposure_label_box(layout.exposure_label);
        self.card.set_exposure_track_box(layout.exposure_track);
        self.card.set_exposure_thumb_box(layout.exposure_thumb);
        self.card.set_exposure_text(format!("{:+.1} EV", self.adjustments.exposure_x100 as f32 / 100.0).into());

        // 2. Brightness
        self.card.set_brightness_label_box(layout.brightness_label);
        self.card.set_brightness_track_box(layout.brightness_track);
        self.card.set_brightness_thumb_box(layout.brightness_thumb);
        self.card.set_brightness_text(format!("{:+.2}", self.adjustments.brightness_x100 as f32 / 100.0).into());

        // 3. Contrast
        self.card.set_contrast_label_box(layout.contrast_label);
        self.card.set_contrast_track_box(layout.contrast_track);
        self.card.set_contrast_thumb_box(layout.contrast_thumb);
        self.card.set_contrast_text(format!("{:.1}x", self.adjustments.contrast_x100 as f32 / 100.0).into());

        // 4. Saturation
        self.card.set_saturation_label_box(layout.saturation_label);
        self.card.set_saturation_track_box(layout.saturation_track);
        self.card.set_saturation_thumb_box(layout.saturation_thumb);
        self.card.set_saturation_text(format!("{:.1}x", self.adjustments.saturation_x100 as f32 / 100.0).into());

        // 5. Warmth
        self.card.set_warmth_label_box(layout.warmth_label);
        self.card.set_warmth_track_box(layout.warmth_track);
        self.card.set_warmth_thumb_box(layout.warmth_thumb);
        self.card.set_warmth_text(format!("{:+}", self.adjustments.temperature).into());

        // 6. Vignette
        self.card.set_vignette_label_box(layout.vignette_label);
        self.card.set_vignette_track_box(layout.vignette_track);
        self.card.set_vignette_thumb_box(layout.vignette_thumb);
        self.card.set_vignette_text(format!("{:.0}%", self.adjustments.vignette_x100 as f32 / 2.0).into());

        self.card.set_hairline_bottom(layout.hairline_bottom);
        self.card.set_reset_box(layout.reset);
        self.card.set_helper_box(layout.helper);
        self.card.set_cancel_box(layout.cancel);
        self.card.set_run_box(layout.run);
    }

    pub fn update_preview(&mut self) {
        let img = self.source_image.clone().unwrap_or_else(|| {
            DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(400, 240, image::Rgba([180, 190, 200, 255])))
        });

        let rendered = if self.show_original {
            img.to_rgba8()
        } else {
            apply_adjustments(&img, &self.adjustments)
        };

        let buffer = SharedPixelBuffer::clone_from_slice(rendered.as_raw(), rendered.width(), rendered.height());
        self.card.set_preview_image(Image::from_rgba8(buffer));
        self.card.set_has_preview(true);

        unsafe {
            let _ = InvalidateRect(Some(self.window), None, true);
        }
    }

    pub fn show_for(&mut self, _id: &str, label: &str, _detail: &str, files: &[PathBuf]) {
        self.files = files.to_vec();
        let file_name = files
            .first()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "photo.jpg".to_string());

        if let Some(first) = files.first() {
            if let Ok(img) = image::open(first) {
                // Downscale for responsive stage rendering
                let downscaled = img.resize(400, 240, image::imageops::FilterType::Triangle);
                self.source_image = Some(downscaled);
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
        let mut rgba = image::RgbaImage::new(w as u32, h as u32);
        for y in 0..h {
            for x in 0..w {
                let pixel = self.buffer[(y * w + x) as usize];
                let (a, r, g, b) = pixel.parts();
                rgba.put_pixel(x as u32, y as u32, image::Rgba([r, g, b, a]));
            }
        }
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        rgba.save(path).map_err(|e| e.to_string())
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
                        let layout = Layout::of(CARD_W, CARD_H, &host.adjustments);

                        let check_slider = |track: &Panel, y_pos: f32| -> bool {
                            x >= track.x && x <= track.x + track.width && y_pos >= track.y - 10.0 && y_pos <= track.y + 14.0
                        };

                        if check_slider(&layout.exposure_track, y) {
                            host.dragging_slider = Some(1);
                            let frac = ((x - layout.exposure_track.x) / layout.exposure_track.width).clamp(0.0, 1.0);
                            host.adjustments.exposure_x100 = (frac * 400.0 - 200.0).round() as i32;
                            host.update_preview();
                            host.sync_card();
                        } else if check_slider(&layout.brightness_track, y) {
                            host.dragging_slider = Some(2);
                            let frac = ((x - layout.brightness_track.x) / layout.brightness_track.width).clamp(0.0, 1.0);
                            host.adjustments.brightness_x100 = (frac * 100.0 - 50.0).round() as i32;
                            host.update_preview();
                            host.sync_card();
                        } else if check_slider(&layout.contrast_track, y) {
                            host.dragging_slider = Some(3);
                            let frac = ((x - layout.contrast_track.x) / layout.contrast_track.width).clamp(0.0, 1.0);
                            host.adjustments.contrast_x100 = (frac * 100.0 + 50.0).round() as i32;
                            host.update_preview();
                            host.sync_card();
                        } else if check_slider(&layout.saturation_track, y) {
                            host.dragging_slider = Some(4);
                            let frac = ((x - layout.saturation_track.x) / layout.saturation_track.width).clamp(0.0, 1.0);
                            host.adjustments.saturation_x100 = (frac * 200.0).round() as i32;
                            host.update_preview();
                            host.sync_card();
                        } else if check_slider(&layout.warmth_track, y) {
                            host.dragging_slider = Some(5);
                            let frac = ((x - layout.warmth_track.x) / layout.warmth_track.width).clamp(0.0, 1.0);
                            host.adjustments.temperature = (frac * 200.0 - 100.0).round() as i32;
                            host.update_preview();
                            host.sync_card();
                        } else if check_slider(&layout.vignette_track, y) {
                            host.dragging_slider = Some(6);
                            let frac = ((x - layout.vignette_track.x) / layout.vignette_track.width).clamp(0.0, 1.0);
                            host.adjustments.vignette_x100 = (frac * 200.0).round() as i32;
                            host.update_preview();
                            host.sync_card();
                        }
                    }
                });
                LRESULT(0)
            }
            WM_MOUSEMOVE => {
                if (wparam.0 & 0x0001) != 0 {
                    let x = (lparam.0 & 0xFFFF) as i16 as f32;
                    HOST_RC.with(|cell| {
                        if let Some(host_rc) = cell.borrow().as_ref() {
                            let mut host = host_rc.borrow_mut();
                            let dragging = host.dragging_slider;
                            let layout = Layout::of(CARD_W, CARD_H, &host.adjustments);
                            let tw = layout.exposure_track.width;
                            let ox = layout.exposure_track.x;
                            let frac = ((x - ox) / tw).clamp(0.0, 1.0);

                            match dragging {
                                Some(1) => host.adjustments.exposure_x100 = (frac * 400.0 - 200.0).round() as i32,
                                Some(2) => host.adjustments.brightness_x100 = (frac * 100.0 - 50.0).round() as i32,
                                Some(3) => host.adjustments.contrast_x100 = (frac * 100.0 + 50.0).round() as i32,
                                Some(4) => host.adjustments.saturation_x100 = (frac * 200.0).round() as i32,
                                Some(5) => host.adjustments.temperature = (frac * 200.0 - 100.0).round() as i32,
                                Some(6) => host.adjustments.vignette_x100 = (frac * 200.0).round() as i32,
                                _ => return,
                            }
                            host.update_preview();
                            host.sync_card();
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
                if wparam.0 == 0x0D { // Enter
                    HOST_RC.with(|cell| {
                        if let Some(host_rc) = cell.borrow().as_ref() {
                            let host = host_rc.borrow();
                            let mut settings = ToolSettings::default();
                            settings.photo_adjustments = Some(host.adjustments);

                            let ev = ToolEvent::Run {
                                operation: "tools.edit_image".to_string(),
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

impl Drop for EditImageWindow {
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
        let adj = PhotoAdjustments::default();
        let layout = Layout::of(CARD_W, CARD_H, &adj);
        assert!(layout.exposure_track.width > 200.0);
        assert!(layout.brightness_track.width > 200.0);
        assert!(layout.run.width > 50.0);
    }

    #[test]
    fn showing_and_pumping_the_edit_image_card_does_not_panic() {
        crate::ui::wheel::install_platform();
        let (tx, _rx) = std::sync::mpsc::channel();
        let mut window = EditImageWindow::new(tx).expect("edit image window can be made");
        window.show_for(
            "tools.edit_image",
            "Adjust",
            "Adjust photo",
            &[PathBuf::from("photo.jpg")],
        );
        window.hide();
    }

    #[test]
    fn capture_the_edit_image_window() {
        crate::ui::wheel::install_platform();
        let (tx, _rx) = std::sync::mpsc::channel();
        let mut window = EditImageWindow::new(tx).expect("edit image window can be made");
        window.show_for(
            "tools.edit_image",
            "Adjust",
            "Adjust photo",
            &[PathBuf::from("photo.jpg")],
        );
        let shot_path = std::path::Path::new("target").join("shots").join("edit-image-window.png");
        window.save_capture(&shot_path).expect("edit image window captured");
        assert!(shot_path.is_file(), "edit-image-window.png was written");
    }
}
