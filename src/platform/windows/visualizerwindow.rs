//! Win32 host and Slint card integration for the Audio Visualizer window (B-10.16 / P3-9).

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::mpsc::Sender;

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
    WM_PAINT,
};

use crate::media::visualizer::{
    render_visualizer_frame, VisualizerSettings, VisualizerShape, VisualizerStyle,
};
use crate::platform::windows::toolwindow::ToolEvent;
use crate::platform::windows::windowhost::{scale_factor, to_wide};
use crate::radial::execution::ToolSettings;
use crate::ui::wheel::{window_for_next_component, Bgra8, Panel, VisualizerCard};

pub const CARD_W: i32 = 420;
pub const CARD_H: i32 = 640;

pub const SWATCH_COLORS: [[u8; 4]; 6] = [
    [255, 92, 0, 255],   // Tangerine #FF5C00
    [10, 132, 255, 255], // Electric Blue #0A84FF
    [48, 209, 88, 255],  // Neon Green #30D158
    [191, 90, 242, 255], // Purple #BF5AF2
    [255, 214, 10, 255], // Gold #FFD60A
    [255, 255, 255, 255],// White #FFFFFF
];

#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    pub client_width: i32,
    pub client_height: i32,
    pub header: Panel,
    pub detail: Panel,
    pub hairline_top: Panel,
    pub stage: Panel,
    pub shape_label: Panel,
    pub shape_chips: [Panel; 3],
    pub style_label: Panel,
    pub style_chips: [Panel; 3],
    pub color_label: Panel,
    pub color_swatches: [Panel; 6],
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

        let chip_w = (380.0 - 16.0) / 3.0; // ~121.33 px

        let shape_chips = [
            p(20.0, 368.0, chip_w, 28.0),
            p(20.0 + chip_w + 8.0, 368.0, chip_w, 28.0),
            p(20.0 + (chip_w + 8.0) * 2.0, 368.0, chip_w, 28.0),
        ];

        let style_chips = [
            p(20.0, 432.0, chip_w, 28.0),
            p(20.0 + chip_w + 8.0, 432.0, chip_w, 28.0),
            p(20.0 + (chip_w + 8.0) * 2.0, 432.0, chip_w, 28.0),
        ];

        let swatch_step = 60.0;
        let color_swatches = [
            p(20.0, 494.0, 26.0, 26.0),
            p(20.0 + swatch_step, 494.0, 26.0, 26.0),
            p(20.0 + swatch_step * 2.0, 494.0, 26.0, 26.0),
            p(20.0 + swatch_step * 3.0, 494.0, 26.0, 26.0),
            p(20.0 + swatch_step * 4.0, 494.0, 26.0, 26.0),
            p(20.0 + swatch_step * 5.0, 494.0, 26.0, 26.0),
        ];

        Self {
            client_width: w,
            client_height: h,
            header: p(20.0, 14.0, 380.0, 24.0),
            detail: p(20.0, 38.0, 380.0, 18.0),
            hairline_top: p(0.0, 62.0, w as f32, 1.0),
            stage: p(20.0, 74.0, 380.0, 260.0),
            shape_label: p(20.0, 346.0, 380.0, 18.0),
            shape_chips,
            style_label: p(20.0, 410.0, 380.0, 18.0),
            style_chips,
            color_label: p(20.0, 472.0, 380.0, 18.0),
            color_swatches,
            hairline_bottom: p(0.0, 560.0, w as f32, 1.0),
            reset: p(20.0, 578.0, 56.0, 36.0),
            helper: p(84.0, 578.0, 120.0, 36.0),
            cancel: p(212.0, 578.0, 64.0, 36.0),
            run: p(284.0, 578.0, 116.0, 36.0),
        }
    }
}

pub struct VisualizerWindow {
    window: HWND,
    renderer_window: Rc<MinimalSoftwareWindow>,
    card: VisualizerCard,
    buffer: Vec<Bgra8>,
    events: Rc<RefCell<Vec<ToolEvent>>>,
    sender: Sender<ToolEvent>,
    layout: Layout,
    files: Vec<PathBuf>,
    selected_shape: usize,
    selected_style: usize,
    selected_color: usize,
}

thread_local! {
    static HOST_RC: RefCell<Option<Rc<RefCell<VisualizerWindow>>>> = const { RefCell::new(None) };
}

impl VisualizerWindow {
    pub fn new(sender: Sender<ToolEvent>) -> Result<Self, String> {
        let (window, renderer_window, card) = Self::create_window()?;
        let events = Rc::new(RefCell::new(Vec::new()));
        let layout = Layout::of(CARD_W, CARD_H);
        let buffer = vec![Bgra8::default(); (CARD_W * CARD_H) as usize];

        let mut win = Self {
            window,
            renderer_window,
            card,
            buffer,
            events,
            sender,
            layout,
            files: Vec::new(),
            selected_shape: 0,
            selected_style: 0,
            selected_color: 0,
        };

        win.wire_callbacks();
        win.sync_card();
        Ok(win)
    }

    fn create_window() -> Result<(HWND, Rc<MinimalSoftwareWindow>, VisualizerCard), String> {
        let renderer_window = window_for_next_component();
        let card = VisualizerCard::new().map_err(|e| e.to_string())?;

        let scale = scale_factor(HWND(std::ptr::null_mut()));
        let client_w = ((CARD_W as f32) * scale).round() as i32;
        let client_h = ((CARD_H as f32) * scale).round() as i32;

        let window = crate::platform::windows::windowhost::create_layered_window(
            "OpenTangerineVisualizerCardWindow",
            "Audio Visualizer — Open Tangerine",
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

        self.card.on_shape_chosen(|idx| {
            HOST_RC.with(|cell| {
                if let Some(host_rc) = cell.borrow().as_ref() {
                    host_rc.borrow_mut().selected_shape = (idx as usize).min(2);
                    host_rc.borrow_mut().update_preview();
                    host_rc.borrow_mut().sync_card();
                }
            });
        });

        self.card.on_style_chosen(|idx| {
            HOST_RC.with(|cell| {
                if let Some(host_rc) = cell.borrow().as_ref() {
                    host_rc.borrow_mut().selected_style = (idx as usize).min(2);
                    host_rc.borrow_mut().update_preview();
                    host_rc.borrow_mut().sync_card();
                }
            });
        });

        self.card.on_color_chosen(|idx| {
            HOST_RC.with(|cell| {
                if let Some(host_rc) = cell.borrow().as_ref() {
                    host_rc.borrow_mut().selected_color = (idx as usize).min(5);
                    host_rc.borrow_mut().update_preview();
                    host_rc.borrow_mut().sync_card();
                }
            });
        });

        self.card.on_reset_defaults_chosen(|| {
            HOST_RC.with(|cell| {
                if let Some(host_rc) = cell.borrow().as_ref() {
                    host_rc.borrow_mut().selected_shape = 0;
                    host_rc.borrow_mut().selected_style = 0;
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
                    let shape_str = match host.selected_shape {
                        1 => "9:16",
                        2 => "1:1",
                        _ => "16:9",
                    };
                    let style_str = match host.selected_style {
                        1 => "bars",
                        2 => "still",
                        _ => "waveform",
                    };
                    let color = SWATCH_COLORS[host.selected_color];
                    let hex = format!("#{:02X}{:02X}{:02X}", color[0], color[1], color[2]);

                    let mut settings = ToolSettings::default();
                    settings.visualizer_shape = Some(shape_str.to_string());
                    settings.visualizer_style = Some(style_str.to_string());
                    settings.visualizer_accent = Some(hex);

                    let ev = ToolEvent::Run {
                        operation: "tools.audio_to_video".to_string(),
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
        self.card.set_shape_label_box(layout.shape_label);
        self.card.set_shape_chips(slint::ModelRc::from(std::rc::Rc::new(slint::VecModel::from(layout.shape_chips.to_vec()))));
        self.card.set_style_label_box(layout.style_label);
        self.card.set_style_chips(slint::ModelRc::from(std::rc::Rc::new(slint::VecModel::from(layout.style_chips.to_vec()))));
        self.card.set_color_label_box(layout.color_label);
        self.card.set_color_swatches(slint::ModelRc::from(std::rc::Rc::new(slint::VecModel::from(layout.color_swatches.to_vec()))));
        self.card.set_hairline_bottom(layout.hairline_bottom);
        self.card.set_reset_box(layout.reset);
        self.card.set_helper_box(layout.helper);
        self.card.set_cancel_box(layout.cancel);
        self.card.set_run_box(layout.run);

        self.card.set_selected_shape(self.selected_shape as i32);
        self.card.set_selected_style(self.selected_style as i32);
        self.card.set_selected_color(self.selected_color as i32);
    }

    pub fn update_preview(&mut self) {
        let shape = match self.selected_shape {
            1 => VisualizerShape::Portrait9x16,
            2 => VisualizerShape::Square1x1,
            _ => VisualizerShape::Landscape16x9,
        };
        let style = match self.selected_style {
            1 => VisualizerStyle::Bars,
            2 => VisualizerStyle::StillImage,
            _ => VisualizerStyle::Waveform,
        };
        let color = SWATCH_COLORS[self.selected_color];

        let settings = VisualizerSettings {
            style,
            shape,
            accent_color: color,
            bg_color: [0, 0, 0, 255],
            image_path: None,
        };

        // Synthesize dynamic preview waveform peaks
        let sample_count = 64;
        let mut peaks = Vec::with_capacity(sample_count);
        for i in 0..sample_count {
            let t = (i as f32) / (sample_count as f32) * std::f32::consts::PI * 4.0;
            let val = ((t.sin().abs() * 0.7 + (t * 2.3).cos().abs() * 0.3) * 0.85 + 0.1).clamp(0.05, 1.0);
            peaks.push(val);
        }

        let stage_w = 380;
        let stage_h = 260;
        let frame = render_visualizer_frame(&peaks, stage_w, stage_h, &settings);
        let buffer = SharedPixelBuffer::clone_from_slice(frame.as_raw(), stage_w, stage_h);
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
            .unwrap_or_else(|| "Audio file.wav".to_string());

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
        let w = self.layout.client_width;
        let h = self.layout.client_height;
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
            WM_KEYDOWN => {
                if wparam.0 == 0x1B { // Esc
                    unsafe { let _ = ShowWindow(hwnd, SW_HIDE); }
                    return LRESULT(0);
                }
                if wparam.0 == 0x09 { // Tab -> cycle shape
                    HOST_RC.with(|cell| {
                        if let Some(host_rc) = cell.borrow().as_ref() {
                            let mut host = host_rc.borrow_mut();
                            host.selected_shape = (host.selected_shape + 1) % 3;
                            host.sync_card();
                        }
                    });
                    return LRESULT(0);
                }
                if wparam.0 == 0x0D { // Enter
                    HOST_RC.with(|cell| {
                        if let Some(host_rc) = cell.borrow().as_ref() {
                            let host = host_rc.borrow();
                            let shape_str = match host.selected_shape {
                                1 => "9:16",
                                2 => "1:1",
                                _ => "16:9",
                            };
                            let style_str = match host.selected_style {
                                1 => "bars",
                                2 => "still",
                                _ => "waveform",
                            };
                            let color = SWATCH_COLORS[host.selected_color];
                            let hex = format!("#{:02X}{:02X}{:02X}", color[0], color[1], color[2]);

                            let mut settings = ToolSettings::default();
                            settings.visualizer_shape = Some(shape_str.to_string());
                            settings.visualizer_style = Some(style_str.to_string());
                            settings.visualizer_accent = Some(hex);

                            let ev = ToolEvent::Run {
                                operation: "tools.audio_to_video".to_string(),
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

impl Drop for VisualizerWindow {
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
        assert_eq!(layout.shape_chips.len(), 3);
        assert_eq!(layout.style_chips.len(), 3);
        assert_eq!(layout.color_swatches.len(), 6);
        assert!(layout.run.width > 50.0);
    }

    #[test]
    fn showing_and_pumping_the_visualizer_card_does_not_panic() {
        crate::ui::wheel::install_platform();
        let (tx, _rx) = std::sync::mpsc::channel();
        let mut window = VisualizerWindow::new(tx).expect("visualizer window can be made");
        window.show_for(
            "tools.audio_to_video",
            "Audio Visualizer",
            "Create MP4 video",
            &[PathBuf::from("voice.wav")],
        );
        window.hide();
    }

    #[test]
    fn capture_the_visualizer_window() {
        crate::ui::wheel::install_platform();
        let (tx, _rx) = std::sync::mpsc::channel();
        let mut window = VisualizerWindow::new(tx).expect("visualizer window can be made");
        window.show_for(
            "tools.audio_to_video",
            "Audio Visualizer",
            "Create MP4 video",
            &[PathBuf::from("voice.wav")],
        );
        let shot_path = std::path::Path::new("target").join("shots").join("visualizer-window.png");
        window.save_capture(&shot_path).expect("visualizer window captured");
        assert!(shot_path.is_file(), "visualizer-window.png was written");
    }
}
