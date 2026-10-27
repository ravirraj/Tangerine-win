//! The window audio channels conversion is asked in.
//!
//! Measures card dimensions via Layout::of(), manages Slint ChannelsCard,
//! handles interactive channel modes (Stereo, Mono, Left, Right, Swap),
//! left/right volume sliders, real PCM waveform decoding, keyboard navigation,
//! and execution dispatch.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc::{Receiver, Sender};

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
    RegisterClassW, SetForegroundWindow, SetLayeredWindowAttributes, SetTimer, SetWindowPos,
    SetWindowTextW, ShowWindow, CW_USEDEFAULT, IDC_ARROW, LWA_ALPHA, SWP_NOACTIVATE,
    SWP_NOMOVE, SWP_NOZORDER, SW_HIDE, SW_SHOW, WM_DESTROY, WM_ERASEBKGND, WM_KEYDOWN,
    WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_PAINT, WM_TIMER, WNDCLASSW, WS_EX_LAYERED,
};

use crate::media::locator;
use crate::platform::windows::toolwindow::ToolEvent;
use crate::platform::windows::windowhost::{fixed_style, outer_size_for, scale_factor, to_wide};
use crate::radial::execution::ToolSettings;
use crate::ui::wheel::{window_for_next_component, Bgra8, ChannelsCard, Panel};

pub const CARD_W: i32 = 400;
pub const CARD_H: i32 = 560;

const FADE_TIMER_ID: usize = 1;

/// The boxes worked out for the card and its controls.
#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    pub client_width: i32,
    pub client_height: i32,
    pub header: Panel,
    pub detail: Panel,
    pub hairline_top: Panel,
    pub waveform: Panel,
    pub transport: Panel,
    pub modes: Panel,
    pub left_slider: Panel,
    pub right_slider: Panel,
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
        let modes = Panel {
            x: pad,
            y: 216.0 * scale,
            width: content_w,
            height: 48.0 * scale,
        };
        let left_slider = Panel {
            x: pad,
            y: 270.0 * scale,
            width: content_w,
            height: 48.0 * scale,
        };
        let right_slider = Panel {
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
            modes,
            left_slider,
            right_slider,
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
    Scrub,
    LeftGain,
    RightGain,
}

struct State {
    file_path: PathBuf,
    file_name: String,
    file_detail: String,
    duration: f64,
    playhead: f64,
    is_playing: bool,
    mode_index: i32, // 0=Stereo, 1=Mono, 2=Left, 3=Right, 4=Swap
    left_gain: f64,  // 0..200
    right_gain: f64, // 0..200
    slider_focus: usize, // 1 = left, 2 = right
    button_focus: usize, // 0 = none, 1 = cancel, 2 = run
    dragging: DragTarget,
}

impl Default for State {
    fn default() -> Self {
        Self {
            file_path: PathBuf::new(),
            file_name: String::new(),
            file_detail: "2 channels · 44100 Hz".to_string(),
            duration: 0.0,
            playhead: 0.0,
            is_playing: false,
            mode_index: 0,
            left_gain: 100.0,
            right_gain: 100.0,
            slider_focus: 0,
            button_focus: 0,
            dragging: DragTarget::None,
        }
    }
}

type StripPiece = (Vec<u8>, u32, u32);

thread_local! {
    static HOST: RefCell<Option<Host>> = const { RefCell::new(None) };
    static STATE: RefCell<State> = RefCell::new(State::default());
    static WINDOW: Cell<usize> = const { Cell::new(0) };
    static SENDER: RefCell<Option<Sender<ToolEvent>>> = const { RefCell::new(None) };
    static STRIP_INBOX: RefCell<Option<Receiver<StripPiece>>> = const { RefCell::new(None) };
    static STRIP_WORN: Cell<bool> = const { Cell::new(false) };
}

fn window_handle() -> HWND {
    HWND(WINDOW.with(|slot| slot.get()) as *mut _)
}

#[cfg(test)]
fn get_strip_worn() -> bool {
    STRIP_WORN.with(|slot| slot.get())
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

struct Host {
    window: HWND,
    card: ChannelsCard,
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

pub struct ChannelsWindow {
    pub window: HWND,
    #[allow(dead_code)]
    pub sender: Sender<ToolEvent>,
}

impl ChannelsWindow {
    pub fn new(sender: Sender<ToolEvent>) -> Result<Self, String> {
        let class_name = w!("OpenTangerineChannelsCard");
        let instance = unsafe { GetModuleHandleW(None).map_err(|e| e.to_string())? };
        let wc = WNDCLASSW {
            lpfnWndProc: Some(channels_card_proc),
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
        let card = ChannelsCard::new().map_err(|e| e.to_string())?;

        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_LAYERED,
                class_name,
                w!("Audio Channels"),
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
                &preference as *const _ as *const core::ffi::c_void,
                std::mem::size_of::<i32>() as u32,
            );
            let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), 0, LWA_ALPHA);
        }

        WINDOW.with(|slot| slot.set(hwnd.0 as usize));
        SENDER.with(|slot| *slot.borrow_mut() = Some(sender.clone()));

        // Wire Slint callbacks
        let snd_run = sender.clone();
        card.on_run_chosen(move || {
            let (path, settings) = STATE.with(|s| {
                let st = s.borrow();
                let mode_str = match st.mode_index {
                    1 => "mono",
                    2 => "left_only",
                    3 => "right_only",
                    4 => "swap",
                    _ => "stereo",
                };
                (
                    st.file_path.clone(),
                    ToolSettings {
                        channel_mode: Some(mode_str.to_string()),
                        channel_left_gain: Some(st.left_gain.round() as u32),
                        channel_right_gain: Some(st.right_gain.round() as u32),
                        ..Default::default()
                    },
                )
            });
            let _ = snd_run.send(ToolEvent::Run {
                operation: "tools.channels".to_string(),
                inputs: vec![path],
                settings,
            });
            unsafe { let _ = ShowWindow(hwnd, SW_HIDE); };
        });

        let snd_dismiss = sender.clone();
        card.on_dismiss_chosen(move || {
            let _ = snd_dismiss.send(ToolEvent::Cancelled);
            unsafe { let _ = ShowWindow(hwnd, SW_HIDE); };
        });
        let snd_cancel = sender.clone();
        card.on_cancel_chosen(move || {
            let _ = snd_cancel.send(ToolEvent::Cancelled);
            unsafe { let _ = ShowWindow(hwnd, SW_HIDE); };
        });

        card.on_play_chosen(|| {
            STATE.with(|s| {
                let mut st = s.borrow_mut();
                st.is_playing = !st.is_playing;
            });
            refresh_card();
        });

        card.on_mode_chosen(|idx| {
            STATE.with(|s| {
                let mut st = s.borrow_mut();
                st.mode_index = idx;
            });
            refresh_card();
        });

        card.on_left_changed(|val| {
            STATE.with(|s| {
                let mut st = s.borrow_mut();
                st.left_gain = val as f64;
            });
            refresh_card();
        });

        card.on_right_changed(|val| {
            STATE.with(|s| {
                let mut st = s.borrow_mut();
                st.right_gain = val as f64;
            });
            refresh_card();
        });

        card.on_reset_gains_chosen(|| {
            STATE.with(|s| {
                let mut st = s.borrow_mut();
                st.left_gain = 100.0;
                st.right_gain = 100.0;
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
            .unwrap_or("Audio")
            .to_string();

        let tools = locator::find();
        let duration = tools
            .as_ref()
            .and_then(|t| crate::media::frames::duration_of(t, &file_path))
            .unwrap_or(2.0);

        let file_detail = if let Some(ref t) = tools {
            if let Some(pcm) = crate::media::audio::decode_full_pcm(t, &file_path) {
                format!("{} channels \u{b7} {} Hz", pcm.channels, pcm.sample_rate)
            } else {
                "2 channels \u{b7} 44100 Hz".to_string()
            }
        } else {
            "2 channels \u{b7} 44100 Hz".to_string()
        };

        STATE.with(|slot| {
            let mut st = slot.borrow_mut();
            st.file_path = file_path.clone();
            st.file_name = file_name;
            st.file_detail = file_detail;
            st.duration = duration;
            st.playhead = 0.0;
            st.is_playing = false;
            st.mode_index = 0;
            st.left_gain = 100.0;
            st.right_gain = 100.0;
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

        if let Some(t) = tools {
            let layout = Layout::of(phys_w, phys_h);
            ask_for_strip(
                t,
                file_path,
                layout.waveform.width.round() as u32,
                layout.waveform.height.round() as u32,
            );
        }

        let label_text = if !title.is_empty() { title } else { "Audio Channels" };
        let label_wide = to_wide(label_text);
        unsafe {
            let _ = SetWindowTextW(self.window, PCWSTR(label_wide.as_ptr()));
            let _ = SetLayeredWindowAttributes(self.window, COLORREF(0), 255, LWA_ALPHA);
            let _ = SetForegroundWindow(self.window);
            SetTimer(Some(self.window), FADE_TIMER_ID, 16, None);
            let _ = ShowWindow(self.window, SW_SHOW);
        }
    }

    pub fn hide(&self) {
        unsafe { let _ = ShowWindow(self.window, SW_HIDE); };
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
            host.card.set_modes_box(layout.modes);
            host.card.set_left_box(layout.left_slider);
            host.card.set_right_box(layout.right_slider);
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
                host.card.set_selected_mode(st.mode_index);
                host.card.set_left_gain(st.left_gain as f32);
                host.card.set_left_text(format!("{:.0}%", st.left_gain).as_str().into());
                host.card.set_right_gain(st.right_gain as f32);
                host.card.set_right_text(format!("{:.0}%", st.right_gain).as_str().into());
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

pub fn format_clock(seconds: f64) -> String {
    let total_hundredths = (seconds.max(0.0) * 100.0).round() as u64;
    let hundredths = total_hundredths % 100;
    let total_secs = total_hundredths / 100;
    let secs = total_secs % 60;
    let mins = total_secs / 60;
    format!("{mins}:{secs:02}.{hundredths:02}")
}

unsafe extern "system" fn channels_card_proc(
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
            unsafe { let _ = EndPaint(hwnd, &ps); };
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

            // Check mode pills (row in layout.modes, y offset 18..46)
            let scale = layout.client_width as f32 / Layout::WIDTH_UNITS;
            let pill_top = layout.modes.y + 18.0 * scale;
            let pill_bottom = layout.modes.y + 46.0 * scale;
            if x >= layout.modes.x
                && x <= layout.modes.x + layout.modes.width
                && y >= pill_top
                && y <= pill_bottom
            {
                let pill_w = layout.modes.width / 5.0;
                let idx = (((x - layout.modes.x) / pill_w).floor() as i32).clamp(0, 4);
                STATE.with(|s| {
                    let mut st = s.borrow_mut();
                    st.mode_index = idx;
                });
                refresh_card();
                return LRESULT(0);
            }

            // Check left slider
            if x >= layout.left_slider.x
                && x <= layout.left_slider.x + layout.left_slider.width
                && y >= layout.left_slider.y
                && y <= layout.left_slider.y + layout.left_slider.height
            {
                let ratio = ((x - layout.left_slider.x) / layout.left_slider.width).clamp(0.0, 1.0) as f64;
                STATE.with(|s| {
                    let mut st = s.borrow_mut();
                    st.left_gain = (ratio * 200.0).round();
                    st.slider_focus = 1;
                    st.button_focus = 0;
                    st.dragging = DragTarget::LeftGain;
                });
                refresh_card();
                return LRESULT(0);
            }

            // Check right slider
            if x >= layout.right_slider.x
                && x <= layout.right_slider.x + layout.right_slider.width
                && y >= layout.right_slider.y
                && y <= layout.right_slider.y + layout.right_slider.height
            {
                let ratio = ((x - layout.right_slider.x) / layout.right_slider.width).clamp(0.0, 1.0) as f64;
                STATE.with(|s| {
                    let mut st = s.borrow_mut();
                    st.right_gain = (ratio * 200.0).round();
                    st.slider_focus = 2;
                    st.button_focus = 0;
                    st.dragging = DragTarget::RightGain;
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
                    DragTarget::LeftGain => {
                        let ratio = ((x - layout.left_slider.x) / layout.left_slider.width).clamp(0.0, 1.0) as f64;
                        STATE.with(|s| {
                            let mut st = s.borrow_mut();
                            st.left_gain = (ratio * 200.0).round();
                        });
                        refresh_card();
                    }
                    DragTarget::RightGain => {
                        let ratio = ((x - layout.right_slider.x) / layout.right_slider.width).clamp(0.0, 1.0) as f64;
                        STATE.with(|s| {
                            let mut st = s.borrow_mut();
                            st.right_gain = (ratio * 200.0).round();
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
                    // Tab
                    STATE.with(|s| {
                        let mut st = s.borrow_mut();
                        if st.slider_focus == 0 && st.button_focus == 0 {
                            st.slider_focus = 1;
                        } else if st.slider_focus == 1 {
                            st.slider_focus = 2;
                        } else if st.slider_focus == 2 {
                            st.slider_focus = 0;
                            st.button_focus = 1;
                        } else if st.button_focus == 1 {
                            st.button_focus = 2;
                        } else {
                            st.slider_focus = 0;
                            st.button_focus = 0;
                        }
                    });
                    refresh_card();
                }
                0x25 => {
                    // Left arrow
                    let moved = STATE.with(|s| {
                        let mut st = s.borrow_mut();
                        if st.slider_focus == 1 {
                            st.left_gain = (st.left_gain - 5.0).max(0.0);
                            true
                        } else if st.slider_focus == 2 {
                            st.right_gain = (st.right_gain - 5.0).max(0.0);
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
                    // Right arrow
                    let moved = STATE.with(|s| {
                        let mut st = s.borrow_mut();
                        if st.slider_focus == 1 {
                            st.left_gain = (st.left_gain + 5.0).min(200.0);
                            true
                        } else if st.slider_focus == 2 {
                            st.right_gain = (st.right_gain + 5.0).min(200.0);
                            true
                        } else {
                            false
                        }
                    });
                    if moved {
                        refresh_card();
                    }
                }
                0x20 => {
                    // Space -> toggle play/pause
                    STATE.with(|s| {
                        let mut st = s.borrow_mut();
                        st.is_playing = !st.is_playing;
                    });
                    refresh_card();
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
    fn a_channels_layout_measures_all_boxes_proportionally() {
        let layout = Layout::of(400, 560);
        assert_eq!(layout.client_width, 400);
        assert_eq!(layout.client_height, 560);
        assert_eq!(layout.header.x, 18.0);
        assert_eq!(layout.header.y, 14.0);
        assert_eq!(layout.header.height, 22.0);
        assert_eq!(layout.detail.x, 18.0);
        assert_eq!(layout.detail.y, 38.0);
        assert_eq!(layout.detail.height, 18.0);
        assert_eq!(layout.hairline_top.y, 62.0);
        assert_eq!(layout.waveform.width, 364.0);
        assert_eq!(layout.waveform.height, 100.0);
        assert_eq!(layout.modes.height, 48.0);
        assert_eq!(layout.left_slider.height, 48.0);
        assert_eq!(layout.right_slider.height, 48.0);
        assert_eq!(layout.run.width, 98.0);
        assert_eq!(layout.cancel.width, 76.0);
        assert!(layout.run.y > layout.footnote.y);
    }

    #[test]
    fn clock_format_matches_mm_ss_hundredths() {
        assert_eq!(format_clock(0.0), "0:00.00");
        assert_eq!(format_clock(2.05), "0:02.05");
        assert_eq!(format_clock(65.5), "1:05.50");
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
    fn showing_and_pumping_the_channels_card_does_not_panic() {
        crate::ui::wheel::install_platform();
        let (tx, _rx) = std::sync::mpsc::channel();
        let window = ChannelsWindow::new(tx).expect("channels window created");
        let scratch = std::env::temp_dir().join(format!("ot-chan-show-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&scratch);
        let test_wav = scratch.join("test.wav");
        let pcm = crate::media::audio::encode_wav_bytes(&[0i16; 8000], 8000, 2);
        std::fs::write(&test_wav, pcm).expect("test wav written");

        window.show_for("tools.channels", "Channels", "Convert audio channels", &[test_wav]);
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

    /// Tabs to the left slider and arrows it three steps: the arrows run
    /// their answer, the window keeps standing, and the slider has moved.
    /// An arrow ran its answer while the state was still held — and the
    /// answer reads the state again — so a Tab and one arrow used to take
    /// the whole window down.
    #[test]
    fn tabbing_to_a_slider_and_arrowing_it_keeps_the_window_up() {
        use windows::Win32::Foundation::{LPARAM, WPARAM};
        use windows::Win32::UI::Input::KeyboardAndMouse::{VK_LEFT, VK_RIGHT, VK_TAB};
        use windows::Win32::UI::WindowsAndMessaging::{
            IsWindowVisible, SendMessageW,
        };

        crate::ui::wheel::install_platform();
        let (tx, _rx) = std::sync::mpsc::channel();
        let window = ChannelsWindow::new(tx).expect("channels window created");
        let scratch = std::env::temp_dir().join(format!("ot-chan-arrow-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&scratch);
        let test_wav = scratch.join("test.wav");
        let pcm = crate::media::audio::encode_wav_bytes(&[0i16; 8000], 8000, 2);
        std::fs::write(&test_wav, pcm).expect("test wav written");

        window.show_for("tools.channels", "Channels", "Convert audio channels", &[test_wav]);
        unsafe {
            let mut msg = MSG::default();
            for _ in 0..10 {
                while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }
            assert!(
                IsWindowVisible(window.window).as_bool(),
                "the channels window is up before the keys"
            );

            let tell = |key: windows::Win32::UI::Input::KeyboardAndMouse::VIRTUAL_KEY| {
                let _ = SendMessageW(
                    window.window,
                    WM_KEYDOWN,
                    Some(WPARAM(key.0 as usize)),
                    Some(LPARAM(0x0000_0001)), // repeat 1, scan code 0
                );
            };
            // The slider is on: one tab, then the arrows.
            tell(VK_TAB);
            for _ in 0..3 {
                tell(VK_LEFT);
                for _ in 0..4 {
                    while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                        let _ = TranslateMessage(&msg);
                        DispatchMessageW(&msg);
                    }
                }
                assert!(
                    IsWindowVisible(window.window).as_bool(),
                    "the channels window stays up through the left arrow"
                );
            }
            tell(VK_RIGHT);
            tell(VK_RIGHT);
            for _ in 0..4 {
                while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }
        }
        // Three arrows left and two right: 100.0 - 15.0 + 10.0.
        let (left_gain, focused_on_slider) = STATE.with(|slot| {
            let st = slot.borrow();
            (st.left_gain, st.slider_focus == 1)
        });
        assert!(focused_on_slider, "the tab put the focus on the left slider");
        assert!(
            (left_gain - 95.0).abs() < 0.01,
            "the arrows moved the gain: got {left_gain}"
        );
        unsafe {
            assert!(
                IsWindowVisible(window.window).as_bool(),
                "the channels window stays up through the keys"
            )
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

    #[test]
    #[ignore = "captures the audio channels window"]
    fn capture_the_audio_channels_window() {
        crate::ui::wheel::install_platform();
        let (tx, _rx) = std::sync::mpsc::channel();
        let window = ChannelsWindow::new(tx).expect("window created");
        let scratch = std::env::temp_dir().join(format!("ot-chan-cap-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&scratch);
        let test_wav = scratch.join("Audio track.wav");

        let rate = 44100u32;
        let mut samples = vec![0i16; (rate * 2 * 2) as usize]; // 2 seconds stereo
        for i in 0..(rate * 2) as usize {
            let val = if (i / 15) % 2 == 0 { 20000 } else { -20000 };
            samples[i * 2] = val; // L
            samples[i * 2 + 1] = (val as f32 * 0.6) as i16; // R
        }
        let pcm = crate::media::audio::encode_wav_bytes(&samples, rate, 2);
        std::fs::write(&test_wav, pcm).expect("test wav written");

        window.show_for("tools.channels", "Channels", "Convert audio channels", &[test_wav]);

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
                    shots.join("channels-buffer.png"),
                    &rgba,
                    host.width,
                    host.height,
                    image::ExtendedColorType::Rgba8,
                );
            }
        });

        crate::platform::windows::windowhost::screenshot::capture(window.window, &shots.join("channels-window.png"));
        window.hide();
        let _ = std::fs::remove_dir_all(&scratch);
    }
}
