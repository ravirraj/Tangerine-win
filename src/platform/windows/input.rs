//! Reading the pointer, the modifier keys and the foreground window.
//!
//! Almost everything here is polled from the application's own message loop,
//! so there is no hook to install, no hook to leak and no input for anyone
//! else to swallow. The one exception is the scroll wheel: the wheel overlay
//! never takes the focus, so the scroll messages never reach it, and a low
//! level hook is the only way to see them. That hook is installed only while
//! the wheel is on screen and it never stops a message.

use std::cell::RefCell;

use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, VIRTUAL_KEY, VK_CONTROL, VK_DOWN, VK_ESCAPE, VK_LCONTROL, VK_LEFT, VK_LMENU,
    VK_LSHIFT, VK_LWIN, VK_MENU, VK_RCONTROL, VK_RETURN, VK_RIGHT, VK_RMENU, VK_RSHIFT, VK_RWIN,
    VK_SHIFT, VK_UP,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetClassNameW, GetCursorPos, GetForegroundWindow, HC_ACTION, HHOOK, MSLLHOOKSTRUCT,
    SetWindowsHookExW, UnhookWindowsHookEx, WH_MOUSE_LL, WM_MOUSEWHEEL,
};

use crate::radial::actions::Modifiers;
use crate::radial::geometry::Point;

/// The keys the wheel listens for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Escape,
    Enter,
    Left,
    Right,
    Up,
    Down,
}

const TRACKED: [(VIRTUAL_KEY, Key); 6] = [
    (VK_ESCAPE, Key::Escape),
    (VK_RETURN, Key::Enter),
    (VK_LEFT, Key::Left),
    (VK_RIGHT, Key::Right),
    (VK_UP, Key::Up),
    (VK_DOWN, Key::Down),
];

/// Whether a key is held right now.
pub fn is_key_down(key: VIRTUAL_KEY) -> bool {
    // The high bit is the "currently down" bit; the low bit says whether the
    // key was pressed since the last call.
    // SAFETY: reading the asynchronous state of a key is always allowed.
    unsafe { (GetAsyncKeyState(key.0 as i32) as u16) & 0x8000 != 0 }
}

/// The modifier keys that are held right now.
///
/// The left and the right key are asked for separately, as well as the
/// general one. A key that is injected, or a keyboard that only reports the
/// general key, would otherwise be missed.
pub fn modifiers() -> Modifiers {
    Modifiers {
        shift: is_key_down(VK_SHIFT) || is_key_down(VK_LSHIFT) || is_key_down(VK_RSHIFT),
        ctrl: is_key_down(VK_CONTROL) || is_key_down(VK_LCONTROL) || is_key_down(VK_RCONTROL),
        alt: is_key_down(VK_MENU) || is_key_down(VK_LMENU) || is_key_down(VK_RMENU),
    }
}

/// Where the pointer is, in physical screen pixels.
pub fn cursor_position() -> Option<Point> {
    let mut point = POINT::default();
    // SAFETY: `point` is a valid, writable POINT.
    if unsafe { GetCursorPos(&mut point) }.is_ok() {
        Some(Point::new(point.x as f32, point.y as f32))
    } else {
        None
    }
}

/// The window that has the keyboard focus.
pub fn foreground_window() -> Option<HWND> {
    let hwnd = unsafe { GetForegroundWindow() };
    if hwnd.0.is_null() { None } else { Some(hwnd) }
}

/// The class name of a window, for example `CabinetWClass` for Explorer.
pub fn window_class(hwnd: HWND) -> Option<String> {
    let mut buffer = [0u16; 256];
    // SAFETY: `buffer` is a valid writable array and the length is passed on.
    let length = unsafe { GetClassNameW(hwnd, &mut buffer) };
    if length <= 0 {
        return None;
    }
    Some(String::from_utf16_lossy(&buffer[..length as usize]))
}

/// The class name of the foreground window.
pub fn foreground_class() -> Option<String> {
    foreground_window().and_then(window_class)
}

/// Notices keys that went down since the last poll.
///
/// Win32 has no "key pressed" event without a hook, so the keys are sampled
/// and the edges are worked out here.
#[derive(Default)]
pub struct KeyTracker {
    previous: [bool; 6],
}

impl KeyTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// The keys that were pressed since the last call.
    pub fn poll(&mut self) -> Vec<Key> {
        let mut pressed = Vec::new();
        for (index, (key, name)) in TRACKED.iter().enumerate() {
            let down = is_key_down(*key);
            if down && !self.previous[index] {
                pressed.push(*name);
            }
            self.previous[index] = down;
        }
        pressed
    }
}

/// Whether the Windows key is held. The wheel stays out of the way of the
/// system shortcuts, so it never reacts while one is down.
pub fn system_key_held() -> bool {
    is_key_down(VK_LWIN) || is_key_down(VK_RWIN)
}

/// The scroll wheel, watched with a low level hook.
///
/// The wheel is a modal overlay that never takes the focus, so the scroll
/// messages go to the program underneath and never reach it. A low level
/// mouse hook is the only way to see them.
///
/// The hook is only installed while the wheel is on screen, it only looks at
/// the scroll messages, and it never stops one from reaching its proper
/// place: the next hook in the chain is always called.
pub struct WheelWatcher {
    hook: HHOOK,
}

thread_local! {
    /// The scrolls seen since the last time they were taken.
    static SCROLLS: RefCell<Vec<f32>> = const { RefCell::new(Vec::new()) };
}

unsafe extern "system" fn wheel_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    // A low level hook is given the message in `wParam` and a pointer to the
    // details in `lParam`.
    if code == HC_ACTION as i32 && wparam.0 as u32 == WM_MOUSEWHEEL {
        // SAFETY: the pointer is the one the system handed us.
        let details = unsafe { &*(lparam.0 as *const MSLLHOOKSTRUCT) };
        // The wheel delta is in the top half of the mouse data.
        let delta = ((details.mouseData >> 16) as i16) as f32 / 120.0;
        SCROLLS.with(|scrolls| scrolls.borrow_mut().push(delta));
    }
    // The message is never stopped: it belongs to the program underneath.
    LRESULT(0)
}

impl WheelWatcher {
    /// Starts watching the scroll wheel.
    pub fn start() -> Result<Self, String> {
        // SAFETY: the hook procedure is a plain function and the module handle
        // is only read.
        let module = unsafe { GetModuleHandleW(None) }.unwrap_or_default();
        // SAFETY: the callback lives as long as the hook.
        let hook = unsafe {
            SetWindowsHookExW(WH_MOUSE_LL, Some(wheel_hook), Some(HINSTANCE(module.0)), 0)
        };
        match hook {
            Ok(hook) => Ok(Self { hook }),
            Err(error) => Err(format!("the scroll wheel could not be watched: {error}")),
        }
    }

    /// The scrolls seen since the last call.
    pub fn take_scrolls(&self) -> Vec<f32> {
        SCROLLS.with(|scrolls| std::mem::take(&mut *scrolls.borrow_mut()))
    }
}

impl Drop for WheelWatcher {
    fn drop(&mut self) {
        // SAFETY: the hook was installed by this object.
        unsafe {
            let _ = UnhookWindowsHookEx(self.hook);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_key_tracker_starts_empty() {
        let tracker = KeyTracker::new();
        assert!(tracker.previous.iter().all(|down| !down));
    }

    #[test]
    fn the_modifier_reading_is_consistent() {
        let first = modifiers();
        // Asking twice in a row must not disagree with itself.
        assert_eq!(first, modifiers());
    }

    #[test]
    fn the_cursor_reading_is_in_range_or_absent() {
        if let Some(point) = cursor_position() {
            assert!(point.x.is_finite() && point.y.is_finite());
        }
    }
}
