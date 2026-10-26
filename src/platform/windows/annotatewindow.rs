//! Win32 host and Slint card integration for the Annotate Photos window (P3-14 / `tools.annotate`).

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
    WM_LBUTTONDOWN, WM_PAINT,
};

use crate::media::annotate::{
    render_annotations, Annotation,
};
use crate::platform::windows::toolwindow::ToolEvent;
use crate::platform::windows::windowhost::{scale_factor, to_wide};
use crate::radial::execution::ToolSettings;
use crate::ui::wheel::{window_for_next_component, AnnotateCard, Bgra8, Panel};

pub const CARD_W: i32 = 440;
pub const CARD_H: i32 = 660;

pub const SWATCH_COLORS: [[u8; 4]; 6] = [
    [255, 92, 0, 255],   // Tangerine #FF5C00
    [10, 132, 255, 255], // Electric Blue #0A84FF
    [255, 59, 48, 255],  // Red #FF3B30
    [52, 199, 89, 255],  // Green #34C759
    [255, 255, 255, 255],// White #FFFFFF
    [0, 0, 0, 255],      // Black #000000
];

pub const THICKNESS_VALUES: [f32; 4] = [2.0, 4.0, 8.0, 12.0];

#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    pub client_width: i32,
    pub client_height: i32,
    pub header: Panel,
    pub detail: Panel,
    pub hairline_top: Panel,
    pub stage: Panel,
    pub tool_label: Panel,
    pub tool_chips: [Panel; 5],
    pub color_label: Panel,
    pub color_swatches: [Panel; 6],
    pub thickness_label: Panel,
    pub thickness_chips: [Panel; 4],
    pub text_label: Panel,
    pub text_field: Panel,
    pub hairline_bottom: Panel,
    pub reset: Panel,
    pub helper: Panel,
    pub cancel: Panel,
    pub run: Panel,
}

impl Layout {
    pub fn of(w: i32, h: i32) -> Self {
        let p = |x: f32, y: f32, width: f32, height: f32| Panel {
            x,
            y,
            width,
            height,
        };

        let chip5_w = (400.0 - 32.0) / 5.0; // ~73.6 px
        let tool_chips = [
            p(20.0, 362.0, chip5_w, 28.0),
            p(20.0 + (chip5_w + 8.0), 362.0, chip5_w, 28.0),
            p(20.0 + (chip5_w + 8.0) * 2.0, 362.0, chip5_w, 28.0),
            p(20.0 + (chip5_w + 8.0) * 3.0, 362.0, chip5_w, 28.0),
            p(20.0 + (chip5_w + 8.0) * 4.0, 362.0, chip5_w, 28.0),
        ];

        let swatch_step = 50.0;
        let color_swatches = [
            p(20.0, 416.0, 26.0, 26.0),
            p(20.0 + swatch_step, 416.0, 26.0, 26.0),
            p(20.0 + swatch_step * 2.0, 416.0, 26.0, 26.0),
            p(20.0 + swatch_step * 3.0, 416.0, 26.0, 26.0),
            p(20.0 + swatch_step * 4.0, 416.0, 26.0, 26.0),
            p(20.0 + swatch_step * 5.0, 416.0, 26.0, 26.0),
        ];

        let chip4_w = (400.0 - 24.0) / 4.0; // ~94 px
        let thickness_chips = [
            p(20.0, 468.0, chip4_w, 28.0),
            p(20.0 + chip4_w + 8.0, 468.0, chip4_w, 28.0),
            p(20.0 + (chip4_w + 8.0) * 2.0, 468.0, chip4_w, 28.0),
            p(20.0 + (chip4_w + 8.0) * 3.0, 468.0, chip4_w, 28.0),
        ];

        Self {
            client_width: w,
            client_height: h,
            header: p(20.0, 14.0, 400.0, 24.0),
            detail: p(20.0, 38.0, 400.0, 18.0),
            hairline_top: p(0.0, 62.0, w as f32, 1.0),
            stage: p(20.0, 74.0, 400.0, 260.0),
            tool_label: p(20.0, 344.0, 400.0, 16.0),
            tool_chips,
            color_label: p(20.0, 398.0, 400.0, 16.0),
            color_swatches,
            thickness_label: p(20.0, 450.0, 400.0, 16.0),
            thickness_chips,
            text_label: p(20.0, 504.0, 400.0, 16.0),
            text_field: p(20.0, 524.0, 400.0, 32.0),
            hairline_bottom: p(0.0, 576.0, w as f32, 1.0),
            reset: p(20.0, 594.0, 56.0, 36.0),
            helper: p(84.0, 594.0, 130.0, 36.0),
            cancel: p(222.0, 594.0, 64.0, 36.0),
            run: p(294.0, 594.0, 126.0, 36.0),
        }
    }
}

pub struct AnnotateWindow {
    window: HWND,
    renderer_window: Rc<MinimalSoftwareWindow>,
    card: AnnotateCard,
    buffer: Vec<Bgra8>,
    events: Rc<RefCell<Vec<ToolEvent>>>,
    sender: Sender<ToolEvent>,
    files: Vec<PathBuf>,
    source_image: Option<DynamicImage>,
    annotations: Vec<Annotation>,
    selected_tool: usize,
    selected_color: usize,
    selected_thickness: usize,
    callout_text: String,
}

thread_local! {
    static HOST_RC: RefCell<Option<Rc<RefCell<AnnotateWindow>>>> = const { RefCell::new(None) };
}

impl AnnotateWindow {
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
            annotations: Vec::new(),
            selected_tool: 0,
            selected_color: 0,
            selected_thickness: 1,
            callout_text: "Note".to_string(),
        };

        win.wire_callbacks();
        win.sync_card();
        Ok(win)
    }

    fn create_window() -> Result<(HWND, Rc<MinimalSoftwareWindow>, AnnotateCard), String> {
        let renderer_window = window_for_next_component();
        let card = AnnotateCard::new().map_err(|e| e.to_string())?;

        let scale = scale_factor(HWND(std::ptr::null_mut()));
        let client_w = ((CARD_W as f32) * scale).round() as i32;
        let client_h = ((CARD_H as f32) * scale).round() as i32;

        let window = crate::platform::windows::windowhost::create_layered_window(
            "OpenTangerineAnnotateCardWindow",
            "Annotate Photo — Open Tangerine",
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

        self.card.on_tool_chosen(|idx| {
            HOST_RC.with(|cell| {
                if let Some(host_rc) = cell.borrow().as_ref() {
                    host_rc.borrow_mut().selected_tool = (idx as usize).min(4);
                    host_rc.borrow_mut().sync_card();
                }
            });
        });

        self.card.on_color_chosen(|idx| {
            HOST_RC.with(|cell| {
                if let Some(host_rc) = cell.borrow().as_ref() {
                    host_rc.borrow_mut().selected_color = (idx as usize).min(5);
                    host_rc.borrow_mut().sync_card();
                }
            });
        });

        self.card.on_thickness_chosen(|idx| {
            HOST_RC.with(|cell| {
                if let Some(host_rc) = cell.borrow().as_ref() {
                    host_rc.borrow_mut().selected_thickness = (idx as usize).min(3);
                    host_rc.borrow_mut().sync_card();
                }
            });
        });

        self.card.on_text_changed(|text| {
            HOST_RC.with(|cell| {
                if let Some(host_rc) = cell.borrow().as_ref() {
                    host_rc.borrow_mut().callout_text = text.to_string();
                }
            });
        });

        self.card.on_reset_defaults_chosen(|| {
            HOST_RC.with(|cell| {
                if let Some(host_rc) = cell.borrow().as_ref() {
                    host_rc.borrow_mut().annotations.clear();
                    host_rc.borrow_mut().update_preview();
                    host_rc.borrow_mut().sync_card();
                }
            });
        });

        self.card.on_run_chosen(|| {
            HOST_RC.with(|cell| {
                if let Some(host_rc) = cell.borrow().as_ref() {
                    let host = host_rc.borrow();
                    let mut settings = ToolSettings::default();
                    settings.annotations_data = host.annotations.clone();

                    let ev = ToolEvent::Run {
                        operation: "tools.annotate".to_string(),
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
        let layout = Layout::of(CARD_W, CARD_H);
        self.card.set_header_box(layout.header);
        self.card.set_detail_box(layout.detail);
        self.card.set_hairline_top(layout.hairline_top);
        self.card.set_stage_box(layout.stage);

        self.card.set_tool_label_box(layout.tool_label);
        self.card.set_tool_chips(slint::ModelRc::from(std::rc::Rc::new(slint::VecModel::from(layout.tool_chips.to_vec()))));

        self.card.set_color_label_box(layout.color_label);
        self.card.set_color_swatches(slint::ModelRc::from(std::rc::Rc::new(slint::VecModel::from(layout.color_swatches.to_vec()))));

        self.card.set_thickness_label_box(layout.thickness_label);
        self.card.set_thickness_chips(slint::ModelRc::from(std::rc::Rc::new(slint::VecModel::from(layout.thickness_chips.to_vec()))));

        self.card.set_text_label_box(layout.text_label);
        self.card.set_text_field_box(layout.text_field);
        self.card.set_callout_text(self.callout_text.clone().into());

        self.card.set_hairline_bottom(layout.hairline_bottom);
        self.card.set_reset_box(layout.reset);
        self.card.set_helper_box(layout.helper);
        self.card.set_cancel_box(layout.cancel);
        self.card.set_run_box(layout.run);

        self.card.set_selected_tool(self.selected_tool as i32);
        self.card.set_selected_color(self.selected_color as i32);
        self.card.set_selected_thickness(self.selected_thickness as i32);
    }

    pub fn update_preview(&mut self) {
        let img = self.source_image.clone().unwrap_or_else(|| {
            DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(400, 260, image::Rgba([240, 240, 242, 255])))
        });

        let rendered = render_annotations(&img, &self.annotations);
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
                let downscaled = img.resize(400, 260, image::imageops::FilterType::Triangle);
                self.source_image = Some(downscaled);
            }
        }

        // Add a sample arrow and text if empty so user immediately sees capability
        if self.annotations.is_empty() {
            self.annotations.push(Annotation::Arrow {
                x0: 0.15,
                y0: 0.25,
                x1: 0.55,
                y1: 0.65,
                color: [255, 92, 0, 255],
                width: 4.0,
            });
            self.annotations.push(Annotation::Text {
                x: 0.12,
                y: 0.18,
                text: "Look here!".to_string(),
                color: [255, 92, 0, 255],
                scale: 2,
            });
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
                        let layout = Layout::of(CARD_W, CARD_H);
                        // If click is inside the preview stage, add an annotation
                        if x >= layout.stage.x && x <= layout.stage.x + layout.stage.width
                            && y >= layout.stage.y && y <= layout.stage.y + layout.stage.height
                        {
                            let nx = (x - layout.stage.x) / layout.stage.width;
                            let ny = (y - layout.stage.y) / layout.stage.height;
                            let color = SWATCH_COLORS[host.selected_color];
                            let width = THICKNESS_VALUES[host.selected_thickness];

                            let new_ann = match host.selected_tool {
                                1 => Annotation::Rect {
                                    x: (nx - 0.1).max(0.0),
                                    y: (ny - 0.1).max(0.0),
                                    w: 0.2,
                                    h: 0.2,
                                    color,
                                    width,
                                    fill: false,
                                },
                                2 => Annotation::Text {
                                    x: nx,
                                    y: ny,
                                    text: host.callout_text.clone(),
                                    color,
                                    scale: (width / 2.0).round().max(1.0) as u32,
                                },
                                4 => Annotation::Blur {
                                    x: (nx - 0.1).max(0.0),
                                    y: (ny - 0.1).max(0.0),
                                    w: 0.2,
                                    h: 0.2,
                                },
                                _ => Annotation::Arrow {
                                    x0: (nx - 0.15).max(0.0),
                                    y0: (ny - 0.15).max(0.0),
                                    x1: nx,
                                    y1: ny,
                                    color,
                                    width,
                                },
                            };
                            host.annotations.push(new_ann);
                            host.update_preview();
                            host.sync_card();
                        }
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
                            settings.annotations_data = host.annotations.clone();

                            let ev = ToolEvent::Run {
                                operation: "tools.annotate".to_string(),
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

impl Drop for AnnotateWindow {
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
        let layout = Layout::of(CARD_W, CARD_H);
        assert_eq!(layout.tool_chips.len(), 5);
        assert_eq!(layout.color_swatches.len(), 6);
        assert_eq!(layout.thickness_chips.len(), 4);
        assert!(layout.run.width > 50.0);
    }

    #[test]
    fn showing_and_pumping_the_annotate_card_does_not_panic() {
        crate::ui::wheel::install_platform();
        let (tx, _rx) = std::sync::mpsc::channel();
        let mut window = AnnotateWindow::new(tx).expect("annotate window can be made");
        window.show_for(
            "tools.annotate",
            "Annotate",
            "Annotate photo",
            &[PathBuf::from("photo.jpg")],
        );
        window.hide();
    }

    #[test]
    fn capture_the_annotate_window() {
        crate::ui::wheel::install_platform();
        let (tx, _rx) = std::sync::mpsc::channel();
        let mut window = AnnotateWindow::new(tx).expect("annotate window can be made");
        window.show_for(
            "tools.annotate",
            "Annotate",
            "Annotate photo",
            &[PathBuf::from("photo.jpg")],
        );
        let shot_path = std::path::Path::new("target").join("shots").join("annotate-window.png");
        window.save_capture(&shot_path).expect("annotate window captured");
        assert!(shot_path.is_file(), "annotate-window.png was written");
    }
}
