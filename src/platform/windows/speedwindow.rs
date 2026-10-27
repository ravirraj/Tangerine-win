//! The window a video speed is asked in.
//!
//! Measures card dimensions via Layout::of(), manages Slint SpeedCard,
//! wears the video's own first frame as the preview, holds the six speed
//! stops on a slider that snaps to them, says what the copy will run in,
//! handles keyboard navigation, and execution dispatch.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc::Sender;

use slint::platform::software_renderer::MinimalSoftwareWindow;
use slint::{PhysicalSize, SharedPixelBuffer};
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreateSolidBrush, EndPaint, InvalidateRect, PAINTSTRUCT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, GetClientRect, LoadCursorW, PostQuitMessage,
    RegisterClassW, SetForegroundWindow, SetLayeredWindowAttributes, SetWindowPos,
    SetWindowTextW, ShowWindow, CW_USEDEFAULT, IDC_ARROW, LWA_ALPHA, SWP_NOACTIVATE,
    SWP_NOMOVE, SWP_NOZORDER, SW_HIDE, SW_SHOW, WM_DESTROY, WM_DPICHANGED, WM_ERASEBKGND,
    WM_KEYDOWN, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_PAINT, WNDCLASSW, WS_EX_LAYERED,
};

use crate::media::locator;
use crate::platform::windows::channelswindow::format_clock;
use crate::platform::windows::toolwindow::ToolEvent;
use crate::platform::windows::windowhost::{fixed_style, outer_size_for, scale_factor, to_wide};
use crate::radial::execution::ToolSettings;
use crate::ui::wheel::{window_for_next_component, Bgra8, Panel, SpeedCard};

pub const CARD_W: i32 = 400;
pub const CARD_H: i32 = 560;

/// The six stops the speed snaps to, said in per cent of the video's own
/// pace: a quarter, a half, the video as it is, one and a half times,
/// twice, and four times. The card opens standing at the middle one.
const STOPS: [u32; 6] = [25, 50, 100, 150, 200, 400];

/// The boxes worked out for the card and its controls.
#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    pub client_width: i32,
    pub client_height: i32,
    pub header: Panel,
    pub detail: Panel,
    pub hairline_top: Panel,
    pub preview: Panel,
    pub speed: Panel,
    pub stops: Panel,
    pub runs: Panel,
    pub footnote: Panel,
    pub hairline_bottom: Panel,
    pub helper: Panel,
    pub cancel: Panel,
    pub run: Panel,
}

impl Layout {
    pub const WIDTH_UNITS: f32 = 400.0;
    #[allow(dead_code)]
    pub const HEIGHT_UNITS: f32 = 560.0;

    pub fn of(client_width: i32, client_height: i32) -> Self {
        let scale = (client_width as f32 / Self::WIDTH_UNITS).max(0.5);
        let w = client_width as f32;
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
        let preview = Panel {
            x: pad,
            y: 74.0 * scale,
            width: content_w,
            height: 150.0 * scale,
        };
        let speed = Panel {
            x: pad,
            y: 240.0 * scale,
            width: content_w,
            height: 48.0 * scale,
        };
        let stops = Panel {
            x: pad,
            y: 294.0 * scale,
            width: content_w,
            height: 16.0 * scale,
        };
        let runs = Panel {
            x: pad,
            y: 334.0 * scale,
            width: content_w,
            height: 20.0 * scale,
        };
        let footnote = Panel {
            x: pad,
            y: 366.0 * scale,
            width: content_w,
            height: 44.0 * scale,
        };
        let hairline_bottom = Panel {
            x: 0.0,
            y: 495.0 * scale,
            width: w,
            height: 1.0,
        };
        let helper = Panel {
            x: pad,
            y: 508.0 * scale,
            width: 170.0 * scale,
            height: 36.0 * scale,
        };
        let cancel = Panel {
            x: w - 200.0 * scale,
            y: 508.0 * scale,
            width: 76.0 * scale,
            height: 36.0 * scale,
        };
        let run = Panel {
            x: w - 116.0 * scale,
            y: 508.0 * scale,
            width: 98.0 * scale,
            height: 36.0 * scale,
        };

        Self {
            client_width,
            client_height,
            header,
            detail,
            hairline_top,
            preview,
            speed,
            stops,
            runs,
            footnote,
            hairline_bottom,
            helper,
            cancel,
            run,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DragTarget {
    None,
    Speed,
}

struct State {
    file_path: PathBuf,
    file_name: String,
    file_detail: String,
    duration: f64,
    speed_index: usize, // the stop the card runs at: 0..5 into STOPS
    slider_focus: usize, // 1 = the speed slider
    button_focus: usize, // 0 = none, 1 = cancel, 2 = run
    dragging: DragTarget,
}

impl Default for State {
    fn default() -> Self {
        Self {
            file_path: PathBuf::new(),
            file_name: String::new(),
            file_detail: "Video".to_string(),
            duration: 0.0,
            speed_index: 2, // the video as it is
            slider_focus: 0,
            button_focus: 0,
            dragging: DragTarget::None,
        }
    }
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
    card: SpeedCard,
    software: Rc<MinimalSoftwareWindow>,
    buffer: Vec<Bgra8>,
    width: u32,
    height: u32,
}

impl Host {
    fn draw(&mut self) {
        if self.width == 0 || self.height == 0 {
            return;
        }
        let needed = (self.width * self.height) as usize;
        let mut buffer = std::mem::take(&mut self.buffer);
        if buffer.len() != needed {
            buffer = vec![Bgra8::default(); needed];
        }
        let stride = self.width as usize;
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

pub struct SpeedWindow {
    pub window: HWND,
    #[allow(dead_code)]
    pub sender: Sender<ToolEvent>,
}

/// What the copy will run in, said in the clock's own face: the video's
/// whole length brought to the pace the stop asks for. Twice as fast is
/// half as long, and a quarter of the pace is four times the wait.
pub fn runs_said(duration: f64, percent: u32) -> String {
    let pace = (percent.max(1) as f64) / 100.0;
    format_clock(duration / pace)
}

/// The stop a point on the track stands at: the whole track is five
/// steps from one edge to the other, and the round of the share says
/// which of the six stops it lands nearest.
pub fn stop_at(ratio: f32) -> usize {
    ((ratio.clamp(0.0, 1.0) * 5.0).round() as i32).clamp(0, 5) as usize
}

impl SpeedWindow {
    pub fn new(sender: Sender<ToolEvent>) -> Result<Self, String> {
        let class_name = w!("OpenTangerineSpeedCard");
        let instance = unsafe { GetModuleHandleW(None).map_err(|e| e.to_string())? };
        let wc = WNDCLASSW {
            lpfnWndProc: Some(speed_card_proc),
            hInstance: instance.into(),
            lpszClassName: class_name,
            hCursor: unsafe { LoadCursorW(None, IDC_ARROW) }.unwrap_or_default(),
            hIcon: crate::platform::windows::windowhost::get_app_icon(),
            hbrBackground: unsafe { CreateSolidBrush(COLORREF(0x00FF_FFFF)) },
            ..Default::default()
        };
        unsafe {
            let _ = RegisterClassW(&wc);
        }

        let software = window_for_next_component();
        let card = SpeedCard::new().map_err(|e| e.to_string())?;

        let owner = crate::platform::windows::windowhost::root_owner_window();
        let hwnd = unsafe {
            let win = CreateWindowExW(
                WS_EX_LAYERED,
                class_name,
                w!("Speed"),
                fixed_style(),
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                CARD_W,
                CARD_H,
                if owner.0.is_null() { None } else { Some(owner) },
                None,
                Some(instance.into()),
                None,
            )
            .map_err(|e| e.to_string())?;
            crate::platform::windows::windowhost::prepare_tool_window(win);
            win
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
            let (path, settings) = STATE.with(|s| {
                let st = s.borrow();
                (
                    st.file_path.clone(),
                    ToolSettings {
                        speed_percent: Some(STOPS[st.speed_index]),
                        ..Default::default()
                    },
                )
            });
            let _ = snd_run.send(ToolEvent::Run {
                operation: "tools.speed".to_string(),
                inputs: vec![path],
                settings,
            });
            unsafe { let _ = ShowWindow(hwnd, SW_HIDE); };
        });

        let snd_cancel = sender.clone();
        card.on_cancel_chosen(move || {
            let _ = snd_cancel.send(ToolEvent::Cancelled);
            unsafe { let _ = ShowWindow(hwnd, SW_HIDE); };
        });

        card.on_speed_chosen(|idx| {
            STATE.with(|s| {
                let mut st = s.borrow_mut();
                st.speed_index = (idx.clamp(0, 5)) as usize;
                st.slider_focus = 1;
                st.button_focus = 0;
            });
            refresh_card();
        });

        card.set_run_label("Apply".into());

        HOST.with(|slot| {
            *slot.borrow_mut() = Some(Host {
                window: hwnd,
                card,
                software,
                buffer: Vec::new(),
                width: CARD_W as u32,
                height: CARD_H as u32,
            });
        });

        Ok(Self { window: hwnd, sender })
    }

    pub fn show_for(
        &self,
        _operation: &str,
        title: &str,
        _detail: &str,
        files: &[PathBuf],
    ) {
        if files.is_empty() {
            return;
        }

        let file_path = files[0].clone();
        let file_name = file_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("Video")
            .to_string();

        let tools = locator::find();
        let duration = tools
            .as_ref()
            .and_then(|t| crate::media::frames::duration_of(t, &file_path))
            .unwrap_or(0.0);
        let file_detail = if let Some(ref t) = tools {
            if let Some((w, h, _rate)) = crate::media::frames::shape_of(t, &file_path) {
                format!("{}×{} \u{b7} {}", w, h, format_clock(duration))
            } else {
                "Video".to_string()
            }
        } else {
            "Video".to_string()
        };

        STATE.with(|slot| {
            let mut st = slot.borrow_mut();
            st.file_path = file_path.clone();
            st.file_name = file_name;
            st.file_detail = file_detail;
            st.duration = duration;
            st.speed_index = 2;
            st.slider_focus = 0;
            st.button_focus = 0;
            st.dragging = DragTarget::None;
        });

        let scale = scale_factor(self.window);
        let phys_w = (CARD_W as f32 * scale).round() as i32;
        let phys_h = (CARD_H as f32 * scale).round() as i32;
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

        apply_layout(phys_w, phys_h);
        refresh_card();

        // The video's own beginning: the frame at zero is asked standing
        // up, made small for its box, and worn before the window shows
        // itself, so what appears is already the video's own card. All
        // of it is done here, standing still — the card holds no play of
        // its own; the video says itself by its first frame alone.
        if let Some(t) = tools.as_ref() {
            let layout = Layout::of(phys_w, phys_h);
            if let Some(frame) = crate::media::frames::frame_at(t, &file_path, 0.0) {
                let small = frame.thumbnail(
                    layout.preview.width.max(1.0) as u32,
                    layout.preview.height.max(1.0) as u32,
                );
                let rgba = small.to_rgba8();
                let buffer = SharedPixelBuffer::clone_from_slice(
                    rgba.as_raw(),
                    small.width(),
                    small.height(),
                );
                wear_preview(buffer);
            }
        }

        let label_text = if !title.is_empty() { title } else { "Speed" };
        let label_wide = to_wide(label_text);
        unsafe {
            let _ = SetWindowTextW(self.window, PCWSTR(label_wide.as_ptr()));
            let _ = SetLayeredWindowAttributes(self.window, COLORREF(0), 255, LWA_ALPHA);
            let _ = SetForegroundWindow(self.window);
            let _ = ShowWindow(self.window, SW_SHOW);
            crate::platform::windows::windowhost::hide_from_taskbar(self.window);
        }
    }

    pub fn hide(&self) {
        unsafe { let _ = ShowWindow(self.window, SW_HIDE); };
    }
}

/// Puts the first frame on the card, whole, and asks for the redraw.
fn wear_preview(buffer: SharedPixelBuffer<slint::Rgba8Pixel>) {
    HOST.with(|slot| {
        if let Some(host) = slot.borrow_mut().as_mut() {
            host.card.set_frame(slint::Image::from_rgba8(buffer));
            host.card.set_has_frame(true);
            host.software.request_redraw();
        }
    });
    let window = window_handle();
    if !window.0.is_null() {
        let _ = unsafe { InvalidateRect(Some(window), None, true) };
    }
}

fn apply_layout(width: i32, height: i32) {
    let layout = Layout::of(width, height);
    HOST.with(|slot| {
        if let Some(host) = slot.borrow_mut().as_mut() {
            let scale = width as f32 / CARD_W as f32;
            host.card.set_card_width(width as f32 / scale);
            host.card.set_card_height(height as f32 / scale);
            host.card.set_ui_scale(scale);

            host.card.set_header_box(layout.header);
            host.card.set_detail_box(layout.detail);
            host.card.set_hairline_top(layout.hairline_top);
            host.card.set_preview_box(layout.preview);
            host.card.set_speed_box(layout.speed);
            host.card.set_runs_box(layout.runs);
            host.card.set_footnote_box(layout.footnote);
            host.card.set_hairline_bottom(layout.hairline_bottom);
            host.card.set_helper_box(layout.helper);
            host.card.set_cancel_box(layout.cancel);
            host.card.set_run_box(layout.run);

            host.software.set_size(PhysicalSize::new(width as u32, height as u32));
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
                host.card.set_file_name(st.file_name.as_str().into());
                host.card.set_detail(st.file_detail.as_str().into());
                host.card.set_speed_index(st.speed_index as i32);
                host.card.set_runs_text(
                    runs_said(st.duration, STOPS[st.speed_index]).as_str().into(),
                );
                host.card.set_slider_focus(st.slider_focus as i32);
                host.card.set_button_focus(st.button_focus as i32);
                host.software.request_redraw();
            }
        });
    });
    let window = window_handle();
    if !window.0.is_null() {
        let _ = unsafe { InvalidateRect(Some(window), None, true) };
    }
}

unsafe extern "system" fn speed_card_proc(
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
            unsafe { let _ = EndPaint(hwnd, &ps); };
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            let x = (lparam.0 & 0xffff) as i16 as f32;
            let y = ((lparam.0 >> 16) & 0xffff) as i16 as f32;
            let mut rect = RECT::default();
            let _ = unsafe { GetClientRect(hwnd, &mut rect) };
            let layout = Layout::of(rect.right, rect.bottom);
            let scale = layout.client_width as f32 / Layout::WIDTH_UNITS;

            // The speed row: wherever the hand lands, the nearest stop
            // is the one the card runs at, and the row stays held to
            // keep following the hand.
            if x >= layout.speed.x
                && x <= layout.speed.x + layout.speed.width
                && y >= layout.speed.y
                && y <= layout.speed.y + layout.speed.height
            {
                let usable = layout.speed.width - 2.0 * scale;
                let ratio = if usable > 0.0 { (x - layout.speed.x) / usable } else { 0.0 };
                STATE.with(|s| {
                    let mut st = s.borrow_mut();
                    st.speed_index = stop_at(ratio);
                    st.slider_focus = 1;
                    st.button_focus = 0;
                    st.dragging = DragTarget::Speed;
                });
                refresh_card();
                return LRESULT(0);
            }

            // The stops under their own names: picking a name is picking
            // its stop, and the slider comes with it.
            if x >= layout.stops.x
                && x <= layout.stops.x + layout.stops.width
                && y >= layout.stops.y
                && y <= layout.stops.y + layout.stops.height
            {
                let usable = layout.stops.width - 2.0 * scale;
                let ratio = if usable > 0.0 { (x - layout.stops.x) / usable } else { 0.0 };
                STATE.with(|s| {
                    let mut st = s.borrow_mut();
                    st.speed_index = stop_at(ratio);
                    st.slider_focus = 1;
                    st.button_focus = 0;
                });
                refresh_card();
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

            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            let dragging = STATE.with(|s| s.borrow().dragging);
            if dragging == DragTarget::Speed {
                let x = (lparam.0 & 0xffff) as i16 as f32;
                let mut rect = RECT::default();
                let _ = unsafe { GetClientRect(hwnd, &mut rect) };
                let layout = Layout::of(rect.right, rect.bottom);
                let scale = layout.client_width as f32 / Layout::WIDTH_UNITS;
                let usable = layout.speed.width - 2.0 * scale;
                let ratio = if usable > 0.0 { (x - layout.speed.x) / usable } else { 0.0 };
                STATE.with(|s| {
                    s.borrow_mut().speed_index = stop_at(ratio);
                });
                refresh_card();
            }
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            STATE.with(|s| {
                s.borrow_mut().dragging = DragTarget::None;
            });
            LRESULT(0)
        }
        WM_KEYDOWN => {
            let key = wparam.0 as u32;
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
                    // Enter -> Run
                    HOST.with(|slot| {
                        if let Some(host) = slot.borrow().as_ref() {
                            host.card.invoke_run_chosen();
                        }
                    });
                }
                0x09 => {
                    // Tab: the slider, then the buttons, then back.
                    STATE.with(|s| {
                        let mut st = s.borrow_mut();
                        if st.slider_focus == 1 {
                            st.slider_focus = 0;
                            st.button_focus = 1;
                        } else if st.button_focus == 1 {
                            st.button_focus = 2;
                        } else if st.button_focus == 2 {
                            st.slider_focus = 0;
                            st.button_focus = 0;
                        } else {
                            st.slider_focus = 1;
                        }
                    });
                    refresh_card();
                }
                0x25 => {
                    // Left arrow: one stop slower.
                    let moved = STATE.with(|s| {
                        let mut st = s.borrow_mut();
                        if st.slider_focus == 1 && st.speed_index > 0 {
                            st.speed_index -= 1;
                            true
                        } else {
                            false
                        }
                    });
                    if moved {
                        refresh_card();
                    }
                }
                0x27 => {
                    // Right arrow: one stop faster.
                    let moved = STATE.with(|s| {
                        let mut st = s.borrow_mut();
                        if st.slider_focus == 1 && st.speed_index < 5 {
                            st.speed_index += 1;
                            true
                        } else {
                            false
                        }
                    });
                    if moved {
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
            let phys_w = (CARD_W as f32 * scale).round() as i32;
            let phys_h = (CARD_H as f32 * scale).round() as i32;
            apply_layout(phys_w, phys_h);
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
    fn a_speed_layout_measures_all_boxes_proportionally() {
        let layout = Layout::of(400, 560);
        assert_eq!(layout.client_width, 400);
        assert_eq!(layout.client_height, 560);
        assert_eq!(layout.header.x, 18.0);
        assert_eq!(layout.header.y, 14.0);
        assert_eq!(layout.header.height, 22.0);
        assert_eq!(layout.detail.x, 18.0);
        assert_eq!(layout.detail.y, 38.0);
        assert_eq!(layout.hairline_top.y, 62.0);
        assert_eq!(layout.preview.width, 364.0);
        assert_eq!(layout.preview.height, 150.0);
        assert_eq!(layout.speed.y, 240.0);
        assert_eq!(layout.speed.height, 48.0);
        // The stops sit under the track, where the card's own names are
        // laid by the same rule.
        assert_eq!(layout.stops.y, layout.speed.y + layout.speed.height + 6.0);
        assert_eq!(layout.runs.y, 334.0);
        assert_eq!(layout.runs.height, 20.0);
        assert_eq!(layout.footnote.y, 366.0);
        assert_eq!(layout.footnote.height, 44.0);
        assert_eq!(layout.run.width, 98.0);
        assert_eq!(layout.cancel.width, 76.0);
        assert!(layout.run.y > layout.footnote.y);
    }

    #[test]
    fn the_copy_runs_says_what_the_pace_makes() {
        assert_eq!(runs_said(4.0, 100), "0:04.00");
        assert_eq!(runs_said(4.0, 200), "0:02.00");
        assert_eq!(runs_said(4.0, 50), "0:08.00");
        assert_eq!(runs_said(4.0, 25), "0:16.00");
        assert_eq!(runs_said(0.0, 100), "0:00.00");
        assert_eq!(runs_said(2.05, 400), "0:00.51");
    }

    #[test]
    fn a_point_on_the_track_snaps_to_its_nearest_stop() {
        assert_eq!(stop_at(0.0), 0, "the left edge is the quarter");
        assert_eq!(stop_at(1.0), 5, "the right edge is the four-times");
        assert_eq!(stop_at(0.4), 2, "the exact middle is the video as it is");
        assert_eq!(stop_at(0.5), 3, "half past the middle is one and a half");
        assert_eq!(stop_at(-0.5), 0, "before the edge is held to the edge");
        assert_eq!(stop_at(2.0), 5, "past the edge is held to the edge");
    }

    #[test]
    fn the_stops_are_the_six_the_reference_names() {
        assert_eq!(STOPS, [25, 50, 100, 150, 200, 400]);
        assert_eq!(STOPS[2], 100, "the card opens standing on the video as it is");
    }
}

#[cfg(test)]
pub mod show_tests {
    use super::*;
    // The tests drive the window themselves, so the pump is charged here.
    use windows::Win32::UI::WindowsAndMessaging::{
        DispatchMessageW, MSG, PeekMessageW, PM_REMOVE, TranslateMessage,
    };

    #[test]
    fn showing_and_pumping_the_speed_card_does_not_panic() {
        crate::ui::wheel::install_platform();
        let (tx, _rx) = std::sync::mpsc::channel();
        let window = SpeedWindow::new(tx).expect("speed window created");
        let scratch = std::env::temp_dir().join(format!("ot-speed-show-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&scratch);
        let test_wav = scratch.join("test.wav");
        let pcm = crate::media::audio::encode_wav_bytes(&[0i16; 8000], 8000, 2);
        std::fs::write(&test_wav, pcm).expect("test wav written");

        window.show_for("tools.speed", "Speed", "Change video speed", &[test_wav]);
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
}

#[cfg(test)]
pub mod capture_tests {
    use super::*;
    // The tests drive the window themselves, so the pump is charged here.
    use windows::Win32::UI::WindowsAndMessaging::{
        DispatchMessageW, MSG, PeekMessageW, PM_REMOVE, TranslateMessage,
    };

    use crate::media::runner::ProcessRunner;

    /// A short video of its own: thirty red frames at a tenth of a
    /// second each, three seconds in all, made with the vendored
    /// conversion program itself.
    fn make_test_video(scratch: &std::path::Path) -> PathBuf {
        let ffmpeg = std::path::Path::new("vendor").join("ffmpeg").join("ffmpeg.exe");
        assert!(ffmpeg.is_file(), "the conversion program is not there");
        let raw = scratch.join("three.raw");
        std::fs::write(&raw, vec![255u8, 0, 0, 255].repeat(160 * 120 * 30))
            .expect("the pixels could be written");
        let input = scratch.join("clip.mp4");
        let making: Vec<std::ffi::OsString> = [
            "-hide_banner",
            "-nostdin",
            "-y",
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgba",
            "-s",
            "160x120",
            "-r",
            "10",
            "-i",
        ]
        .iter()
        .map(|word| std::ffi::OsString::from(*word))
        .chain([
            raw.as_os_str().to_os_string(),
            std::ffi::OsString::from("-c:v"),
            std::ffi::OsString::from("mpeg4"),
            std::ffi::OsString::from("-pix_fmt"),
            std::ffi::OsString::from("yuv420p"),
            std::ffi::OsString::from("-f"),
            std::ffi::OsString::from("mov"),
            input.as_os_str().to_os_string(),
        ])
        .collect();
        let made = crate::media::runner::SystemRunner
            .run(&ffmpeg, &making, None)
            .expect("the video was made");
        assert!(made.succeeded(), "the test video was not made: {}", made.stderr);
        input
    }

    /// One press of the left button at a point of the card, delivered as
    /// the system delivers it: down and up on the proc's own door.
    unsafe fn press(window: HWND, x: f32, y: f32) {
        let lparam = LPARAM(((((y as i32) << 16) | ((x as i32) & 0xffff)) as i64) as isize);
        unsafe {
            let _ = windows::Win32::UI::WindowsAndMessaging::SendMessageW(
                window,
                WM_LBUTTONDOWN,
                Some(WPARAM(0x0001)), // MK_LBUTTON
                Some(lparam),
            );
            let _ = windows::Win32::UI::WindowsAndMessaging::SendMessageW(
                window,
                WM_LBUTTONUP,
                Some(WPARAM(0)),
                Some(lparam),
            );
        }
    }

    #[test]
    #[ignore = "captures the video speed window"]
    fn capture_the_video_speed_window() {
        crate::ui::wheel::install_platform();
        let (tx, rx) = std::sync::mpsc::channel();
        let window = SpeedWindow::new(tx).expect("window created");
        let scratch = std::env::temp_dir().join(format!("ot-speed-cap-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&scratch);
        let video = make_test_video(&scratch);

        window.show_for("tools.speed", "Speed", "Change video speed", &[video.clone()]);
        unsafe {
            let mut msg = MSG::default();
            for _ in 0..40 {
                while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        }

        // Choosing the twice-as-fast stop the way a hand does: a press on
        // its own name in the row under the track. The proc is asked
        // through the system's own sending, down and up.
        let mut rect = RECT::default();
        let _ = unsafe { GetClientRect(window.window, &mut rect) };
        let layout = Layout::of(rect.right, rect.bottom);
        let scale = layout.client_width as f32 / Layout::WIDTH_UNITS;
        let _ = unsafe {
            press(
                window.window,
                layout.stops.x + 0.8 * (layout.stops.width - 2.0 * scale),
                layout.stops.y + layout.stops.height / 2.0,
            )
        };

        // The card says the stop the hand chose: the copy runs in half
        // the video's own length, whatever that length was said to be.
        let said = HOST.with(|slot| {
            slot.borrow()
                .as_ref()
                .map(|h| h.card.get_runs_text().to_string())
        })
        .unwrap_or_default();
        let expected = runs_said(STATE.with(|s| s.borrow().duration), 200);
        assert_eq!(said, expected, "the card runs at the stop the hand chose");

        unsafe {
            let mut msg = MSG::default();
            for _ in 0..10 {
                while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
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
                    shots.join("speed-buffer.png"),
                    &rgba,
                    host.width,
                    host.height,
                    image::ExtendedColorType::Rgba8,
                );
            }
        });

        crate::platform::windows::windowhost::screenshot::capture(
            window.window,
            &shots.join("speed-window.png"),
        );

        // The apply button asks for the run, at the stop that was chosen.
        let _ = unsafe {
            press(
                window.window,
                layout.run.x + layout.run.width / 2.0,
                layout.run.y + layout.run.height / 2.0,
            )
        };
        let event = rx.recv().expect("the run was asked");
        match event {
            ToolEvent::Run {
                operation,
                inputs,
                settings,
            } => {
                assert_eq!(operation, "tools.speed");
                assert_eq!(inputs, vec![video.clone()]);
                assert_eq!(settings.speed_percent, Some(200), "the stop chosen is the one run");
            }
            other => panic!("the run was asked, and something else said: {:?}", other),
        }

        window.hide();
        let _ = std::fs::remove_dir_all(&scratch);
    }
}
