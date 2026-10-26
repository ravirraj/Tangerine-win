//! The settings window for Open Tangerine.
//!
//! Provides interactive UI for user preferences:
//! - Theme: System, Dark, Light
//! - Accent color palette
//! - Output location (beside original or custom directory)
//! - System preferences: chime sound, startup launch, system tray, shell menu

use std::cell::RefCell;
use std::rc::Rc;

use slint::platform::software_renderer::MinimalSoftwareWindow;
use slint::PhysicalSize;
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, EndPaint, PAINTSTRUCT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    DefWindowProcW, PostQuitMessage,
    SetForegroundWindow, SetLayeredWindowAttributes, SetWindowPos, ShowWindow,
    LWA_ALPHA, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOZORDER, SW_HIDE,
    SW_SHOW, WM_DESTROY, WM_DPICHANGED, WM_ERASEBKGND, WM_KEYDOWN, WM_PAINT,
};

use crate::platform::windows::windowhost::scale_factor;
use crate::settings::{AppSettings, ThemeMode};
use crate::ui::wheel::{window_for_next_component, Bgra8, Panel, SettingsCard};

pub const CARD_W: i32 = 440;
pub const CARD_H: i32 = 580;

pub const ACCENT_PRESETS: [&str; 6] = [
    "#FF5C00", // Tangerine
    "#007AFF", // Blue
    "#34C759", // Green
    "#FF2D55", // Pink
    "#FFCC00", // Yellow
    "#5856D6", // Purple
];

#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    pub client_width: i32,
    pub client_height: i32,
    pub header: Panel,
    pub hairline_top: Panel,
    pub theme_label: Panel,
    pub theme_chips: [Panel; 3],
    pub accent_label: Panel,
    pub accent_swatches: [Panel; 6],
    pub hairline_mid1: Panel,
    pub output_label: Panel,
    pub output_chips: [Panel; 2],
    pub output_path: Panel,
    pub hairline_mid2: Panel,
    pub sound_toggle: Panel,
    pub startup_toggle: Panel,
    pub tray_toggle: Panel,
    pub shell_toggle: Panel,
    pub hairline_bot: Panel,
    pub helper: Panel,
    pub close_button: Panel,
}

impl Layout {
    pub const WIDTH_UNITS: f32 = 440.0;
    #[allow(dead_code)]
    pub const HEIGHT_UNITS: f32 = 580.0;

    pub fn of(client_width: i32, client_height: i32) -> Self {
        let scale = (client_width as f32 / Self::WIDTH_UNITS).max(0.5);
        let w = client_width as f32;
        let pad = 24.0 * scale;
        let content_w = w - pad * 2.0;

        let header = Panel {
            x: pad,
            y: 16.0 * scale,
            width: content_w,
            height: 24.0 * scale,
        };
        let hairline_top = Panel {
            x: 0.0,
            y: 52.0 * scale,
            width: w,
            height: 1.0,
        };

        // Theme row
        let theme_label = Panel {
            x: pad,
            y: 68.0 * scale,
            width: 90.0 * scale,
            height: 28.0 * scale,
        };
        let chip_w = (content_w - 100.0 * scale - 16.0 * scale) / 3.0;
        let mut theme_chips: [Panel; 3] = std::array::from_fn(|_| Panel::default());
        for i in 0..3 {
            theme_chips[i] = Panel {
                x: pad + 100.0 * scale + (i as f32) * (chip_w + 8.0 * scale),
                y: 68.0 * scale,
                width: chip_w,
                height: 28.0 * scale,
            };
        }

        // Accent row
        let accent_label = Panel {
            x: pad,
            y: 110.0 * scale,
            width: 90.0 * scale,
            height: 28.0 * scale,
        };
        let swatch_d = 26.0 * scale;
        let swatch_gap = 10.0 * scale;
        let mut accent_swatches: [Panel; 6] = std::array::from_fn(|_| Panel::default());
        for i in 0..6 {
            accent_swatches[i] = Panel {
                x: pad + 100.0 * scale + (i as f32) * (swatch_d + swatch_gap),
                y: 110.0 * scale,
                width: swatch_d,
                height: swatch_d,
            };
        }

        let hairline_mid1 = Panel {
            x: 0.0,
            y: 154.0 * scale,
            width: w,
            height: 1.0,
        };

        // Output destination row
        let output_label = Panel {
            x: pad,
            y: 168.0 * scale,
            width: 90.0 * scale,
            height: 28.0 * scale,
        };
        let out_chip_w = (content_w - 100.0 * scale - 8.0 * scale) / 2.0;
        let output_chips = [
            Panel {
                x: pad + 100.0 * scale,
                y: 168.0 * scale,
                width: out_chip_w,
                height: 28.0 * scale,
            },
            Panel {
                x: pad + 100.0 * scale + out_chip_w + 8.0 * scale,
                y: 168.0 * scale,
                width: out_chip_w,
                height: 28.0 * scale,
            },
        ];
        let output_path = Panel {
            x: pad,
            y: 206.0 * scale,
            width: content_w,
            height: 32.0 * scale,
        };

        let hairline_mid2 = Panel {
            x: 0.0,
            y: 252.0 * scale,
            width: w,
            height: 1.0,
        };

        // Toggles
        let toggle_h = 36.0 * scale;
        let sound_toggle = Panel {
            x: pad,
            y: 266.0 * scale,
            width: content_w,
            height: toggle_h,
        };
        let startup_toggle = Panel {
            x: pad,
            y: 266.0 * scale + toggle_h,
            width: content_w,
            height: toggle_h,
        };
        let tray_toggle = Panel {
            x: pad,
            y: 266.0 * scale + toggle_h * 2.0,
            width: content_w,
            height: toggle_h,
        };
        let shell_toggle = Panel {
            x: pad,
            y: 266.0 * scale + toggle_h * 3.0,
            width: content_w,
            height: toggle_h,
        };

        let hairline_bot = Panel {
            x: 0.0,
            y: client_height as f32 - 60.0 * scale,
            width: w,
            height: 1.0,
        };
        let helper = Panel {
            x: pad,
            y: client_height as f32 - 46.0 * scale,
            width: 200.0 * scale,
            height: 32.0 * scale,
        };
        let close_button = Panel {
            x: w - pad - 90.0 * scale,
            y: client_height as f32 - 46.0 * scale,
            width: 90.0 * scale,
            height: 32.0 * scale,
        };

        Self {
            client_width,
            client_height,
            header,
            hairline_top,
            theme_label,
            theme_chips,
            accent_label,
            accent_swatches,
            hairline_mid1,
            output_label,
            output_chips,
            output_path,
            hairline_mid2,
            sound_toggle,
            startup_toggle,
            tray_toggle,
            shell_toggle,
            hairline_bot,
            helper,
            close_button,
        }
    }
}

pub struct SettingsWindow {
    window: HWND,
    renderer_window: Rc<MinimalSoftwareWindow>,
    card: SettingsCard,
    settings: Rc<RefCell<AppSettings>>,
    layout: Layout,
    scale: f32,
    buffer: Vec<Bgra8>,
}

impl SettingsWindow {
    pub fn new() -> Result<Self, String> {
        let (window, renderer_window, card, settings, layout, scale, buffer) =
            Self::create_window(CARD_W, CARD_H)?;
        let mut sw = Self {
            window,
            renderer_window,
            card,
            settings,
            layout,
            scale,
            buffer,
        };
        sw.sync_card();
        Ok(sw)
    }

    fn create_window(
        width_units: i32,
        height_units: i32,
    ) -> Result<
        (
            HWND,
            Rc<MinimalSoftwareWindow>,
            SettingsCard,
            Rc<RefCell<AppSettings>>,
            Layout,
            f32,
            Vec<Bgra8>,
        ),
        String,
    > {
        let scale = scale_factor(HWND(std::ptr::null_mut()));
        let client_w = ((width_units as f32) * scale).round() as i32;
        let client_h = ((height_units as f32) * scale).round() as i32;
        let layout = Layout::of(client_w, client_h);

        let hwnd = crate::platform::windows::windowhost::create_layered_window(
            "OpenTangerineSettingsWindow",
            "Settings — Open Tangerine",
            Some(Self::wnd_proc),
            client_w,
            client_h,
        )?;

        let renderer_window = window_for_next_component();
        let card = SettingsCard::new().map_err(|e| e.to_string())?;
        renderer_window.set_size(PhysicalSize::new(client_w as u32, client_h as u32));

        let settings = Rc::new(RefCell::new(AppSettings::load()));
        let buffer = vec![Bgra8::default(); (client_w * client_h) as usize];

        Ok((
            hwnd,
            renderer_window,
            card,
            settings,
            layout,
            scale,
            buffer,
        ))
    }

    pub fn show(&self) {
        unsafe {
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

    pub fn hide(&self) {
        unsafe {
            let _ = ShowWindow(self.window, SW_HIDE);
        }
    }

    pub fn sync_card(&mut self) {
        let s = self.settings.borrow();
        let layout = Layout::of(self.layout.client_width, self.layout.client_height);

        self.card.set_card_width(layout.client_width as f32 / self.scale);
        self.card.set_card_height(layout.client_height as f32 / self.scale);
        self.card.set_ui_scale(self.scale);

        self.card.set_header_box(layout.header);
        self.card.set_hairline_top(layout.hairline_top);
        self.card.set_theme_label_box(layout.theme_label);

        let theme_chips = std::rc::Rc::new(slint::VecModel::from(layout.theme_chips.to_vec()));
        self.card.set_theme_chips(theme_chips.into());

        self.card.set_accent_label_box(layout.accent_label);
        let accent_swatches = std::rc::Rc::new(slint::VecModel::from(layout.accent_swatches.to_vec()));
        self.card.set_accent_swatches(accent_swatches.into());

        self.card.set_hairline_mid1(layout.hairline_mid1);
        self.card.set_output_label_box(layout.output_label);
        let output_chips = std::rc::Rc::new(slint::VecModel::from(layout.output_chips.to_vec()));
        self.card.set_output_chips(output_chips.into());
        self.card.set_output_path_box(layout.output_path);

        self.card.set_hairline_mid2(layout.hairline_mid2);
        self.card.set_sound_toggle_box(layout.sound_toggle);
        self.card.set_startup_toggle_box(layout.startup_toggle);
        self.card.set_tray_toggle_box(layout.tray_toggle);
        self.card.set_shell_toggle_box(layout.shell_toggle);

        self.card.set_hairline_bot(layout.hairline_bot);
        self.card.set_helper_box(layout.helper);
        self.card.set_close_button_box(layout.close_button);

        let theme_idx = match s.theme {
            ThemeMode::System => 0,
            ThemeMode::Dark => 1,
            ThemeMode::Light => 2,
        };
        self.card.set_selected_theme(theme_idx);

        let accent_idx = ACCENT_PRESETS
            .iter()
            .position(|&a| a.eq_ignore_ascii_case(&s.accent))
            .unwrap_or(0) as i32;
        self.card.set_selected_accent(accent_idx);

        self.card.set_selected_output(if s.output_beside_original { 0 } else { 1 });
        if let Some(path) = &s.custom_output_dir {
            self.card.set_custom_path(slint::SharedString::from(path.to_string_lossy().as_ref()));
        }

        self.card.set_sound_enabled(s.sound_enabled);
        self.card.set_startup_enabled(s.launch_at_startup);
        self.card.set_tray_enabled(s.show_tray_icon);
        self.card.set_shell_enabled(s.shell_menu_enabled);
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

    pub fn save_capture(&mut self, path: &std::path::Path) -> Result<(), String> {
        let (w, h) = self.render_to_buffer();
        crate::platform::windows::windowhost::save_buffer_as_png(&self.buffer, w as u32, h as u32, path)
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
                let _ = unsafe { windows::Win32::Graphics::Gdi::InvalidateRect(Some(hwnd), None, true) };
                LRESULT(0)
            }
            _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
        }
    }
}

#[cfg(test)]
pub mod show_tests {
    use super::*;

    #[test]
    fn layout_computes_sane_panels() {
        let layout = Layout::of(440, 580);
        assert!(layout.theme_chips.len() == 3);
        assert!(layout.accent_swatches.len() == 6);
        assert!(layout.output_chips.len() == 2);
        assert!(layout.close_button.width > 50.0);
    }

    #[test]
    fn showing_and_pumping_the_settings_card_does_not_panic() {
        crate::ui::wheel::install_platform();
        let window = SettingsWindow::new().expect("settings window can be made");
        window.show();
        window.hide();
    }

    #[test]
    fn capture_the_settings_window() {
        crate::ui::wheel::install_platform();
        let mut window = SettingsWindow::new().expect("settings window can be made");
        let shot_path = std::path::Path::new("target").join("shots").join("settings-window.png");
        window.save_capture(&shot_path).expect("settings window captured");
        assert!(shot_path.is_file(), "settings-window.png was written");
    }
}
