//! The window normalize volume is asked in.
//!
//! Measures card dimensions via Layout::of(), manages Slint NormalizeCard,
//! handles interactive sliders (target loudness, loudness range, true peak limit),
//! real PCM waveform decoding, keyboard navigation, and execution dispatch.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc::{Receiver, Sender};

use slint::platform::software_renderer::MinimalSoftwareWindow;
use slint::{PhysicalSize, SharedPixelBuffer};
use windows::core::PCWSTR;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, EndPaint, InvalidateRect, PAINTSTRUCT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetClientRect, PeekMessageW,
    PostQuitMessage, RegisterClassW, SetTimer, SetWindowPos, ShowWindow, TranslateMessage,
    CS_HREDRAW, CS_VREDRAW, CW_USEDEFAULT, MSG, PM_REMOVE, SWP_NOACTIVATE, SWP_NOMOVE,
    SWP_NOZORDER, SW_HIDE, SW_SHOW, WM_DESTROY, WM_ERASEBKGND, WM_KEYDOWN, WM_LBUTTONDOWN,
    WM_LBUTTONUP, WM_MOUSEMOVE, WM_PAINT, WM_TIMER, WNDCLASSW, WS_EX_LAYERED,
};

use crate::media::locator;
use crate::platform::windows::toolwindow::ToolEvent;
use crate::platform::windows::windowhost::{fixed_style, scale_factor, to_wide};
use crate::radial::execution::ToolSettings;
use crate::ui::wheel::{window_for_next_component, Bgra8, NormalizeCard, Panel};

const FADE_TIMER_ID: usize = 55;

pub const CARD_W: i32 = 400;
pub const CARD_H: i32 = 560;

#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    pub client_width: i32,
    pub client_height: i32,
    pub header: Panel,
    pub detail: Panel,
    pub hairline_top: Panel,
    pub waveform: Panel,
    pub transport: Panel,
    pub target_slider: Panel,
    pub range_slider: Panel,
    pub peak_slider: Panel,
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
        let waveform = Panel {
            x: pad,
            y: 74.0 * scale,
            width: content_w,
            height: 100.0 * scale,
        };
        let transport = Panel {
            x: pad,
            y: 180.0 * scale,
            width: content_w,
            height: 28.0 * scale,
        };
        let target_slider = Panel {
            x: pad,
            y: 216.0 * scale,
            width: content_w,
            height: 48.0 * scale,
        };
        let range_slider = Panel {
            x: pad,
            y: 270.0 * scale,
            width: content_w,
            height: 48.0 * scale,
        };
        let peak_slider = Panel {
            x: pad,
            y: 324.0 * scale,
            width: content_w,
            height: 48.0 * scale,
        };
        let footnote = Panel {
            x: pad,
            y: 380.0 * scale,
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
            waveform,
            transport,
            target_slider,
            range_slider,
            peak_slider,
            footnote,
            hairline_bottom,
            helper,
            cancel,
            run,
        }
    }
}

pub fn format_clock(seconds: f64) -> String {
    let s = seconds.max(0.0);
    let mins = (s / 60.0).floor() as u32;
    let rem = s - (mins as f64 * 60.0);
    format!("{mins}:{rem:05.2}")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DragTarget {
    None,
    Target,
    Range,
    Peak,
    Scrub,
}

struct NormalizeState {
    file_path: PathBuf,
    file_name: String,
    detail: String,
    duration: f64,
    playhead: f64,
    is_playing: bool,
    target_lufs: f64,
    loudness_range: f64,
    true_peak: f64,
    slider_focus: usize,
    button_focus: usize,
    dragging: DragTarget,
    ffmpeg: Option<locator::Tools>,
}

impl Default for NormalizeState {
    fn default() -> Self {
        Self {
            file_path: PathBuf::new(),
            file_name: String::new(),
            detail: String::new(),
            duration: 0.0,
            playhead: 0.0,
            is_playing: false,
            target_lufs: -16.0,
            loudness_range: 11.0,
            true_peak: -1.5,
            slider_focus: 0,
            button_focus: 0,
            dragging: DragTarget::None,
            ffmpeg: None,
        }
    }
}

type StripPiece = (Vec<u8>, u32, u32);

thread_local! {
    static WINDOW: Cell<usize> = const { Cell::new(0) };
    static HOST: RefCell<Option<Host>> = const { RefCell::new(None) };
    static STATE: RefCell<NormalizeState> = RefCell::new(NormalizeState::default());
    static SENDER: RefCell<Option<Sender<ToolEvent>>> = const { RefCell::new(None) };
    static STRIP_INBOX: RefCell<Option<Receiver<StripPiece>>> = const { RefCell::new(None) };
    static STRIP_WORN: Cell<bool> = const { Cell::new(false) };
}

fn get_strip_worn() -> bool {
    STRIP_WORN.with(|s| s.get())
}

fn ask_for_strip(tools: locator::Tools, input: PathBuf, width: u32, height: u32) {
    let (tx, rx) = std::sync::mpsc::channel::<StripPiece>();
    std::thread::spawn(move || {
        let (samples, _rate) = crate::media::audio::pcm_samples(&tools, &input).unwrap_or_default();
        let buckets = ((width as f32 / 4.0).round() as usize).clamp(50, 200);
        let peaks = crate::media::audio::waveform_peaks(&samples, buckets);
        let strip = crate::media::audio::render_waveform_strip(&peaks, width, height);
        let (w, h) = (strip.width(), strip.height());
        let _ = tx.send((strip.into_raw(), w, h));
    });
    STRIP_INBOX.with(|slot| {
        *slot.borrow_mut() = Some(rx);
    });
    STRIP_WORN.with(|slot| slot.set(false));
}

fn strip_tick() -> Option<StripPiece> {
    STRIP_INBOX.with(|slot| {
        let mut buried = slot.borrow_mut();
        let receiver = buried.as_ref()?;
        match receiver.try_recv() {
            Ok(piece) => Some(piece),
            Err(std::sync::mpsc::TryRecvError::Empty) => None,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                *buried = None;
                None
            }
        }
    })
}

unsafe fn wear_strip(piece: StripPiece) {
    let (pixels, width, height) = piece;
    if width == 0 || height == 0 {
        return;
    }
    let buffer = SharedPixelBuffer::clone_from_slice(&pixels, width, height);
    let image = slint::Image::from_rgba8(buffer);
    HOST.with(|slot| {
        if let Some(host) = slot.borrow_mut().as_mut() {
            host.card.set_waveform_strip(image);
            host.software.request_redraw();
        }
    });
    let window = window_handle();
    if !window.0.is_null() {
        let _ = unsafe { InvalidateRect(Some(window), None, true) };
    }
    STRIP_WORN.with(|slot| slot.set(true));
}

fn window_handle() -> HWND {
    HWND(WINDOW.get() as *mut _)
}

struct Host {
    window: HWND,
    card: NormalizeCard,
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
        unsafe {
            let dc = windows::Win32::Graphics::Gdi::GetDC(Some(self.window));
            let mut info = windows::Win32::Graphics::Gdi::BITMAPINFO::default();
            info.bmiHeader.biSize = std::mem::size_of::<windows::Win32::Graphics::Gdi::BITMAPINFOHEADER>() as u32;
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
                self.buffer.as_ptr() as *const core::ffi::c_void,
                &info,
                windows::Win32::Graphics::Gdi::DIB_RGB_COLORS,
            );
            let _ = windows::Win32::Graphics::Gdi::ReleaseDC(Some(self.window), dc);
        }
    }
}

pub struct NormalizeWindow {
    pub window: HWND,
    #[allow(dead_code)]
    pub sender: Sender<ToolEvent>,
}

impl NormalizeWindow {
    pub fn new(sender: Sender<ToolEvent>) -> Result<Self, String> {
        let class_name = to_wide("OpenTangerineNormalizeCard");
        let hinstance = unsafe {
            windows::Win32::System::LibraryLoader::GetModuleHandleW(PCWSTR::null())
                .unwrap_or_default()
                .into()
        };
        let wc = WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(normalize_card_proc),
            hInstance: hinstance,
            lpszClassName: PCWSTR(class_name.as_ptr()),
            ..Default::default()
        };
        unsafe {
            let _ = RegisterClassW(&wc);
        }

        let software = window_for_next_component();
        let card = NormalizeCard::new().map_err(|e| e.to_string())?;

        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_LAYERED,
                PCWSTR(class_name.as_ptr()),
                PCWSTR(to_wide("Normalize Volume").as_ptr()),
                fixed_style(),
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                CARD_W,
                CARD_H,
                None,
                None,
                Some(wc.hInstance),
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
                std::mem::size_of_val(&preference) as u32,
            );
        }

        WINDOW.with(|slot| slot.set(hwnd.0 as usize));
        SENDER.with(|slot| *slot.borrow_mut() = Some(sender.clone()));

        // Wire Slint callbacks
        let snd_run = sender.clone();
        card.on_run_chosen(move || {
            let (path, settings) = STATE.with(|s| {
                let st = s.borrow();
                (
                    st.file_path.clone(),
                    ToolSettings {
                        normalize_lufs_x10: Some((st.target_lufs * 10.0).round() as i32),
                        normalize_range_x10: Some((st.loudness_range * 10.0).round() as i32),
                        normalize_true_peak_x10: Some((st.true_peak * 10.0).round() as i32),
                        ..Default::default()
                    },
                )
            });
            let _ = snd_run.send(ToolEvent::Run {
                operation: "tools.normalize".to_string(),
                inputs: vec![path],
                settings,
            });
            unsafe { ShowWindow(hwnd, SW_HIDE) };
        });

        let snd_dismiss = sender.clone();
        card.on_dismiss_chosen(move || {
            let _ = snd_dismiss.send(ToolEvent::Cancelled);
            unsafe { ShowWindow(hwnd, SW_HIDE) };
        });
        let snd_cancel = sender.clone();
        card.on_cancel_chosen(move || {
            let _ = snd_cancel.send(ToolEvent::Cancelled);
            unsafe { ShowWindow(hwnd, SW_HIDE) };
        });

        card.on_play_chosen(|| {
            STATE.with(|s| {
                let mut st = s.borrow_mut();
                st.is_playing = !st.is_playing;
            });
            refresh_card();
        });

        card.on_target_changed(|val| {
            STATE.with(|s| {
                let mut st = s.borrow_mut();
                st.target_lufs = (val as f64).clamp(-30.0, -8.0);
            });
            refresh_card();
        });

        card.on_range_changed(|val| {
            STATE.with(|s| {
                let mut st = s.borrow_mut();
                st.loudness_range = (val as f64).clamp(1.0, 20.0);
            });
            refresh_card();
        });

        card.on_peak_changed(|val| {
            STATE.with(|s| {
                let mut st = s.borrow_mut();
                st.true_peak = (val as f64).clamp(-9.0, 0.0);
            });
            refresh_card();
        });

        let host = Host {
            window: hwnd,
            card,
            software,
            buffer: Vec::new(),
            width: CARD_W as u32,
            height: CARD_H as u32,
        };
        HOST.with(|slot| *slot.borrow_mut() = Some(host));

        Ok(Self { window: hwnd, sender })
    }

    pub fn show_for(&self, _operation: &str, _label: &str, _detail: &str, files: &[PathBuf]) {
        let Some(first) = files.first() else { return };
        let file_path = first.clone();
        let file_name = file_path
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "audio.wav".to_string());
        let detail = if !_detail.is_empty() {
            _detail.to_string()
        } else {
            "Normalize audio volume".to_string()
        };

        let tools = locator::find();
        let duration = tools
            .as_ref()
            .and_then(|t| crate::media::frames::duration_of(t, &file_path))
            .unwrap_or(2.0);

        STATE.with(|slot| {
            let mut st = slot.borrow_mut();
            st.file_path = file_path.clone();
            st.file_name = file_name;
            st.detail = detail;
            st.duration = duration;
            st.playhead = 0.0;
            st.is_playing = false;
            st.target_lufs = -16.0;
            st.loudness_range = 11.0;
            st.true_peak = -1.5;
            st.slider_focus = 0;
            st.button_focus = 0;
            st.dragging = DragTarget::None;
            st.ffmpeg = tools.clone();
        });

        let scale = scale_factor(self.window);
        let phys_w = (CARD_W as f32 * scale).round() as i32;
        let phys_h = (CARD_H as f32 * scale).round() as i32;

        unsafe {
            SetWindowPos(
                self.window,
                None,
                0,
                0,
                phys_w,
                phys_h,
                SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }

        apply_layout(phys_w, phys_h);
        refresh_card();

        if let Some(t) = tools {
            let layout = Layout::of(phys_w, phys_h);
            ask_for_strip(
                t,
                file_path,
                layout.waveform.width.round() as u32,
                layout.waveform.height.round() as u32,
            );
        }

        unsafe {
            SetTimer(Some(self.window), FADE_TIMER_ID, 16, None);
            ShowWindow(self.window, SW_SHOW);
        }
    }

    pub fn hide(&self) {
        unsafe { ShowWindow(self.window, SW_HIDE) };
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
            host.card.set_waveform_box(layout.waveform);
            host.card.set_transport_box(layout.transport);
            host.card.set_target_box(layout.target_slider);
            host.card.set_range_box(layout.range_slider);
            host.card.set_peak_box(layout.peak_slider);
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
                host.card.set_detail(st.detail.as_str().into());
                let frac = if st.duration > 0.0 {
                    (st.playhead / st.duration).clamp(0.0, 1.0) as f32
                } else {
                    0.0
                };
                host.card.set_playhead_fraction(frac);
                host.card.set_is_playing(st.is_playing);
                host.card.set_timecode_text(
                    format!("{}/{}", format_clock(st.playhead), format_clock(st.duration)).as_str().into(),
                );
                host.card.set_target_lufs(st.target_lufs as f32);
                host.card.set_target_text(format!("{:.0} LUFS", st.target_lufs).as_str().into());
                host.card.set_loudness_range(st.loudness_range as f32);
                host.card.set_range_text(format!("{:.0} LU", st.loudness_range).as_str().into());
                host.card.set_true_peak(st.true_peak as f32);
                host.card.set_peak_text(format!("{:.1} dBTP", st.true_peak).as_str().into());
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

unsafe extern "system" fn normalize_card_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_TIMER => {
            if wparam.0 == FADE_TIMER_ID {
                if let Some(piece) = strip_tick() {
                    unsafe { wear_strip(piece) };
                }
            }
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
            unsafe { EndPaint(hwnd, &ps) };
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            let x = (lparam.0 & 0xffff) as i16 as f32;
            let y = ((lparam.0 >> 16) & 0xffff) as i16 as f32;
            let mut rect = RECT::default();
            let _ = unsafe { GetClientRect(hwnd, &mut rect) };
            let layout = Layout::of(rect.right, rect.bottom);

            // Check if clicking in waveform box
            if x >= layout.waveform.x
                && x <= layout.waveform.x + layout.waveform.width
                && y >= layout.waveform.y
                && y <= layout.waveform.y + layout.waveform.height
            {
                let ratio = ((x - layout.waveform.x) / layout.waveform.width).clamp(0.0, 1.0) as f64;
                STATE.with(|s| {
                    let mut st = s.borrow_mut();
                    st.playhead = ratio * st.duration;
                    st.dragging = DragTarget::Scrub;
                });
                refresh_card();
                return LRESULT(0);
            }

            // Check target slider
            if x >= layout.target_slider.x
                && x <= layout.target_slider.x + layout.target_slider.width
                && y >= layout.target_slider.y
                && y <= layout.target_slider.y + layout.target_slider.height
            {
                let ratio = ((x - layout.target_slider.x) / layout.target_slider.width).clamp(0.0, 1.0) as f64;
                STATE.with(|s| {
                    let mut st = s.borrow_mut();
                    st.target_lufs = (-30.0 + ratio * 22.0).round();
                    st.slider_focus = 1;
                    st.button_focus = 0;
                    st.dragging = DragTarget::Target;
                });
                refresh_card();
                return LRESULT(0);
            }

            // Check range slider
            if x >= layout.range_slider.x
                && x <= layout.range_slider.x + layout.range_slider.width
                && y >= layout.range_slider.y
                && y <= layout.range_slider.y + layout.range_slider.height
            {
                let ratio = ((x - layout.range_slider.x) / layout.range_slider.width).clamp(0.0, 1.0) as f64;
                STATE.with(|s| {
                    let mut st = s.borrow_mut();
                    st.loudness_range = (1.0 + ratio * 19.0).round();
                    st.slider_focus = 2;
                    st.button_focus = 0;
                    st.dragging = DragTarget::Range;
                });
                refresh_card();
                return LRESULT(0);
            }

            // Check peak slider
            if x >= layout.peak_slider.x
                && x <= layout.peak_slider.x + layout.peak_slider.width
                && y >= layout.peak_slider.y
                && y <= layout.peak_slider.y + layout.peak_slider.height
            {
                let ratio = ((x - layout.peak_slider.x) / layout.peak_slider.width).clamp(0.0, 1.0) as f64;
                STATE.with(|s| {
                    let mut st = s.borrow_mut();
                    st.true_peak = ((-9.0 + ratio * 9.0) * 2.0).round() / 2.0;
                    st.slider_focus = 3;
                    st.button_focus = 0;
                    st.dragging = DragTarget::Peak;
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
            if dragging != DragTarget::None {
                let x = (lparam.0 & 0xffff) as i16 as f32;
                let mut rect = RECT::default();
                let _ = unsafe { GetClientRect(hwnd, &mut rect) };
                let layout = Layout::of(rect.right, rect.bottom);

                match dragging {
                    DragTarget::Scrub => {
                        let ratio = ((x - layout.waveform.x) / layout.waveform.width).clamp(0.0, 1.0) as f64;
                        STATE.with(|s| {
                            let mut st = s.borrow_mut();
                            st.playhead = ratio * st.duration;
                        });
                        refresh_card();
                    }
                    DragTarget::Target => {
                        let ratio = ((x - layout.target_slider.x) / layout.target_slider.width).clamp(0.0, 1.0) as f64;
                        STATE.with(|s| {
                            let mut st = s.borrow_mut();
                            st.target_lufs = (-30.0 + ratio * 22.0).round();
                        });
                        refresh_card();
                    }
                    DragTarget::Range => {
                        let ratio = ((x - layout.range_slider.x) / layout.range_slider.width).clamp(0.0, 1.0) as f64;
                        STATE.with(|s| {
                            let mut st = s.borrow_mut();
                            st.loudness_range = (1.0 + ratio * 19.0).round();
                        });
                        refresh_card();
                    }
                    DragTarget::Peak => {
                        let ratio = ((x - layout.peak_slider.x) / layout.peak_slider.width).clamp(0.0, 1.0) as f64;
                        STATE.with(|s| {
                            let mut st = s.borrow_mut();
                            st.true_peak = ((-9.0 + ratio * 9.0) * 2.0).round() / 2.0;
                        });
                        refresh_card();
                    }
                    DragTarget::None => {}
                }
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
                    // ESC
                    HOST.with(|slot| {
                        if let Some(host) = slot.borrow().as_ref() {
                            host.card.invoke_dismiss_chosen();
                        }
                    });
                }
                0x09 => {
                    // TAB
                    STATE.with(|s| {
                        let mut st = s.borrow_mut();
                        if st.slider_focus == 0 && st.button_focus == 0 {
                            st.slider_focus = 1;
                        } else if st.slider_focus == 1 {
                            st.slider_focus = 2;
                        } else if st.slider_focus == 2 {
                            st.slider_focus = 3;
                        } else if st.slider_focus == 3 {
                            st.slider_focus = 0;
                            st.button_focus = 2;
                        } else {
                            st.slider_focus = 1;
                            st.button_focus = 0;
                        }
                    });
                    refresh_card();
                }
                0x25 => {
                    // Left Arrow
                    STATE.with(|s| {
                        let mut st = s.borrow_mut();
                        if st.slider_focus == 1 {
                            st.target_lufs = (st.target_lufs - 1.0).clamp(-30.0, -8.0);
                        } else if st.slider_focus == 2 {
                            st.loudness_range = (st.loudness_range - 1.0).clamp(1.0, 20.0);
                        } else if st.slider_focus == 3 {
                            st.true_peak = (st.true_peak - 0.5).clamp(-9.0, 0.0);
                        }
                    });
                    refresh_card();
                }
                0x27 => {
                    // Right Arrow
                    STATE.with(|s| {
                        let mut st = s.borrow_mut();
                        if st.slider_focus == 1 {
                            st.target_lufs = (st.target_lufs + 1.0).clamp(-30.0, -8.0);
                        } else if st.slider_focus == 2 {
                            st.loudness_range = (st.loudness_range + 1.0).clamp(1.0, 20.0);
                        } else if st.slider_focus == 3 {
                            st.true_peak = (st.true_peak + 0.5).clamp(-9.0, 0.0);
                        }
                    });
                    refresh_card();
                }
                0x0D | 0x20 => {
                    // Enter or Space
                    HOST.with(|slot| {
                        if let Some(host) = slot.borrow().as_ref() {
                            host.card.invoke_run_chosen();
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
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_normalize_layout_measures_all_boxes_proportionally() {
        let layout = Layout::of(400, 560);
        assert_eq!(layout.client_width, 400);
        assert_eq!(layout.client_height, 560);

        assert_eq!(layout.header.x, 18.0);
        assert_eq!(layout.header.y, 14.0);
        assert_eq!(layout.header.width, 364.0);
        assert_eq!(layout.header.height, 22.0);

        assert_eq!(layout.detail.x, 18.0);
        assert_eq!(layout.detail.y, 38.0);
        assert_eq!(layout.detail.width, 364.0);
        assert_eq!(layout.detail.height, 18.0);

        assert_eq!(layout.hairline_top.y, 62.0);
        assert_eq!(layout.hairline_top.height, 1.0);

        assert_eq!(layout.waveform.x, 18.0);
        assert_eq!(layout.waveform.width, 364.0);
        assert_eq!(layout.waveform.height, 100.0);

        assert_eq!(layout.target_slider.width, 364.0);
        assert_eq!(layout.range_slider.width, 364.0);
        assert_eq!(layout.peak_slider.width, 364.0);

        assert_eq!(layout.hairline_bottom.y, 495.0);
        assert_eq!(layout.run.width, 98.0);
        assert_eq!(layout.cancel.width, 76.0);
        assert!(layout.run.y > layout.footnote.y);
    }

    #[test]
    fn clock_format_matches_mm_ss_hundredths() {
        assert_eq!(format_clock(0.0), "0:00.00");
        assert_eq!(format_clock(2.0), "0:02.00");
        assert_eq!(format_clock(65.43), "1:05.43");
    }
}

#[cfg(test)]
pub mod show_tests {
    use super::*;

    #[test]
    fn showing_and_pumping_the_normalize_card_does_not_panic() {
        crate::ui::wheel::install_platform();
        let (tx, _rx) = std::sync::mpsc::channel();
        let window = NormalizeWindow::new(tx).expect("window created");
        let scratch = std::env::temp_dir().join(format!("ot-norm-show-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&scratch);
        let test_wav = scratch.join("voice.wav");
        let pcm = crate::media::audio::encode_wav_bytes(&[1000, -1000], 44100, 1);
        std::fs::write(&test_wav, pcm).expect("test wav written");

        window.show_for("tools.normalize", "Normalize", "Normalize audio volume", &[test_wav]);

        unsafe {
            let mut msg = MSG::default();
            for _ in 0..20 {
                while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        }

        window.hide();
        let _ = std::fs::remove_dir_all(&scratch);
    }
}

#[cfg(test)]
pub mod capture_tests {
    use super::*;

    #[test]
    #[ignore = "captures the normalize audio window"]
    fn capture_the_normalize_audio_window() {
        crate::ui::wheel::install_platform();
        let (tx, _rx) = std::sync::mpsc::channel();
        let window = NormalizeWindow::new(tx).expect("window created");
        let scratch = std::env::temp_dir().join(format!("ot-norm-cap-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&scratch);
        let test_wav = scratch.join("Voice with pauses.wav");

        // Make realistic audio with pauses matching normalizeAudio.webp
        let rate = 44100u32;
        let mut samples = vec![0i16; (rate * 2) as usize]; // 2 seconds
        // Two bursts: 0.35s..0.85s and 1.15s..1.65s
        for i in ((rate as f32 * 0.35) as usize)..((rate as f32 * 0.85) as usize) {
            samples[i] = if (i / 10) % 2 == 0 { 24000 } else { -24000 };
        }
        for i in ((rate as f32 * 1.15) as usize)..((rate as f32 * 1.65) as usize) {
            samples[i] = if (i / 10) % 2 == 0 { 24000 } else { -24000 };
        }
        let pcm = crate::media::audio::encode_wav_bytes(&samples, rate, 1);
        std::fs::write(&test_wav, pcm).expect("test wav written");

        window.show_for("tools.normalize", "Normalize", "Normalize audio volume", &[test_wav]);

        // Wait for waveform strip arrival
        let start = std::time::Instant::now();
        unsafe {
            let mut msg = MSG::default();
            while !get_strip_worn() && start.elapsed().as_secs() < 5 {
                while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            // Pump more messages to ensure full render
            for _ in 0..40 {
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
                    shots.join("normalize-buffer.png"),
                    &rgba,
                    host.width,
                    host.height,
                    image::ExtendedColorType::Rgba8,
                );
            }
        });

        crate::platform::windows::windowhost::screenshot::capture(window.window, &shots.join("normalize-window.png"));
        window.hide();
        let _ = std::fs::remove_dir_all(&scratch);
    }
}
