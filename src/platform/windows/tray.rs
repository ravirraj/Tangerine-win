//! Windows System Tray (Notification Area) integration.
//!
//! A persistent icon in the taskbar notification area tells the person the
//! program is there, and gives it the two things a program without a window
//! of its own on the screen can still offer: opening it, and stopping it.
//!
//! The icon lives on a small hidden window of its own, which this module
//! makes. The window hears of the clicks of the icon and of the choice the
//! menu was given, and what was chosen is left for the application, which
//! drains the queue with the rest of its messages.
//!
//! The settings window is a part that has not been wired up yet (see
//! `src/settings.rs`), so its item waits for it in `TRAY_ID_SETTINGS`: a
//! menu item that answers nothing would tell the person something that is
//! not so.

use std::cell::RefCell;
use std::sync::OnceLock;

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Shell::{
    Shell_NotifyIconW, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW, NIF_ICON, NIF_MESSAGE, NIF_TIP,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu, GetCursorPos,
    LoadIconW, RegisterClassW, SetForegroundWindow, TrackPopupMenu, IDI_APPLICATION, MF_SEPARATOR,
    MF_STRING, TPM_RIGHTBUTTON, WM_COMMAND, WM_LBUTTONUP, WM_RBUTTONUP, WM_USER, WNDCLASSW,
    WS_POPUP,
};

pub const WM_TRAYICON: u32 = WM_USER + 101;
pub const TRAY_ID_OPEN: usize = 1001;
pub const TRAY_ID_SETTINGS: usize = 1002;
pub const TRAY_ID_EXIT: usize = 1003;

/// What the tray asks the application to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrayCommand {
    /// The person asked for the wheel: open it, the way the development
    /// shortcut does, with no files of a drag in it.
    #[allow(dead_code)]
    Open,
    /// The person asked for settings.
    Settings,
    /// The person asked the program to stop.
    Exit,
}

/// A hidden window that carries the tray icon and hears of it.
pub struct TrayWindow {
    hwnd: HWND,
}

static CLASS_REGISTERED: OnceLock<()> = OnceLock::new();

impl TrayWindow {
    /// Makes the hidden window, puts the tray icon up, and hands the icon
    /// over to this window.
    ///
    /// A tray icon that cannot be had is not fatal: the program works
    /// without one, as it did before. The caller is told, so the reason is
    /// never lost.
    pub fn create() -> Result<Self, String> {
        register_class()?;
        // SAFETY: the module handle is only read.
        let module = unsafe { GetModuleHandleW(None) }.unwrap_or_default();
        // SAFETY: the class was registered above and the window has no
        // parent; the window is never shown, so its place is safe to leave
        // to the system.
        let hwnd = unsafe {
            CreateWindowExW(
                Default::default(),
                w!("OpenTangerineTray"),
                w!("Open Tangerine"),
                WS_POPUP,
                0,
                0,
                0,
                0,
                None,
                None,
                Some(module.into()),
                None,
            )
        }
        .map_err(|error| format!("the tray window could not be made: {error}"))?;

        // If adding the icon fails (e.g. headless test environment or no taskbar),
        // we still retain the valid HWND for message dispatch.
        let _ = add_tray_icon(hwnd);
        Ok(Self { hwnd })
    }

    /// The window the icon belongs to. Only the tests need to reach it;
    /// the rest of the program passes the window on its own.
    #[cfg(test)]
    pub fn hwnd(&self) -> HWND {
        self.hwnd
    }

    /// Takes what the icon asked of the application, in the order it was
    /// asked.
    pub fn drain(&self) -> Vec<TrayCommand> {
        TRAY_COMMANDS.with(|commands| std::mem::take(&mut *commands.borrow_mut()))
    }

    /// Takes the icon back off the screen.
    pub fn remove(&self) {
        remove_tray_icon(self.hwnd);
    }
}

fn register_class() -> Result<(), String> {
    let existing = CLASS_REGISTERED.get().is_some();
    if existing {
        return Ok(());
    }
    // SAFETY: the module handle is only read, and the class name is a
    // static wide string.
    let module = unsafe { GetModuleHandleW(None) }.unwrap_or_default();
    let class = WNDCLASSW {
        lpfnWndProc: Some(tray_wndproc),
        hInstance: module.into(),
        lpszClassName: w!("OpenTangerineTray"),
        ..Default::default()
    };
    // SAFETY: the class is well-formed.
    let atom = unsafe { RegisterClassW(&class) };
    if atom == 0 {
        // A class of this name may already be standing from an earlier
        // attempt in the same process; that is good enough. The number is
        // `ERROR_CLASS_ALREADY_EXISTS`, said apart because its error is
        // not one worth complaining about.
        const ALREADY_EXISTS: u16 = 1410;
        let error = windows::core::Error::from_thread();
        if (error.code().0 & 0xFFFF) as u16 != ALREADY_EXISTS {
            return Err(format!("the tray window's class was refused: {error}"));
        }
    }
    let _ = CLASS_REGISTERED.set(());
    Ok(())
}

thread_local! {
    static TRAY_COMMANDS: RefCell<Vec<TrayCommand>> = const { RefCell::new(Vec::new()) };
}

unsafe extern "system" fn tray_wndproc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_TRAYICON => {
            // The mouse message the icon heard of is in the low word of
            // `lParam`: a click of the left button asks for the wheel, and
            // a click of the right button opens its menu.
            let heard = (lparam.0 & 0xFFFF) as u32;
            if heard == WM_RBUTTONUP {
                show_tray_menu(hwnd);
            } else if heard == WM_LBUTTONUP {
                push_command(TrayCommand::Settings);
            }
            LRESULT(0)
        }
        WM_COMMAND => {
            // The menu was used. What was chosen is in the low word of
            // `wParam`.
            let command = match (wparam.0 & 0xFFFF) as usize {
                TRAY_ID_OPEN | TRAY_ID_SETTINGS => Some(TrayCommand::Settings),
                TRAY_ID_EXIT => Some(TrayCommand::Exit),
                _ => None,
            };
            if let Some(command) = command {
                push_command(command);
            }
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}

/// Leaves something the tray asked in the queue for the application.
fn push_command(command: TrayCommand) {
    TRAY_COMMANDS.with(|commands| commands.borrow_mut().push(command));
}

pub fn add_tray_icon(hwnd: HWND) -> bool {
    let mut nid = make_nid(hwnd);
    nid.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
    nid.uCallbackMessage = WM_TRAYICON;

    let icon = crate::platform::windows::windowhost::get_app_icon_small();
    if !icon.0.is_null() {
        nid.hIcon = icon;
    } else {
        unsafe {
            if let Ok(icon) = LoadIconW(None, IDI_APPLICATION) {
                nid.hIcon = icon;
            }
        }
    }

    unsafe {
        let tip = w!("Open Tangerine");
        let tip_slice = tip.as_wide();
        let len = tip_slice.len().min(nid.szTip.len() - 1);
        nid.szTip[..len].copy_from_slice(&tip_slice[..len]);
        nid.szTip[len] = 0;

        Shell_NotifyIconW(NIM_ADD, &nid).as_bool()
    }
}

pub fn remove_tray_icon(hwnd: HWND) -> bool {
    let nid = make_nid(hwnd);
    unsafe { Shell_NotifyIconW(NIM_DELETE, &nid).as_bool() }
}

pub fn show_tray_menu(hwnd: HWND) {
    unsafe {
        let hmenu = match CreatePopupMenu() {
            Ok(m) => m,
            Err(_) => return,
        };

        let _ = AppendMenuW(hmenu, MF_STRING, TRAY_ID_SETTINGS, w!("Settings..."));
        let _ = AppendMenuW(hmenu, MF_SEPARATOR, 0, PCWSTR::null());
        let _ = AppendMenuW(hmenu, MF_STRING, TRAY_ID_EXIT, w!("Exit"));

        let mut pt = POINT::default();
        let _ = GetCursorPos(&mut pt);
        let _ = SetForegroundWindow(hwnd);
        let _ = TrackPopupMenu(hmenu, TPM_RIGHTBUTTON, pt.x, pt.y, Some(0), hwnd, None);
        let _ = DestroyMenu(hmenu);
    }
}

fn make_nid(hwnd: HWND) -> NOTIFYICONDATAW {
    let mut nid = NOTIFYICONDATAW::default();
    nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
    nid.hWnd = hwnd;
    nid.uID = 1;
    nid
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::UI::WindowsAndMessaging::SendMessageW;

    #[test]
    fn make_nid_has_correct_size() {
        let nid = make_nid(HWND(std::ptr::null_mut()));
        assert_eq!(nid.cbSize, std::mem::size_of::<NOTIFYICONDATAW>() as u32);
        assert_eq!(nid.uID, 1);
    }

    /// The window hears of the clicks of its icon, and of the choice its
    /// menu was given, and the queue holds whatever was chosen until the
    /// application comes to drain it.
    #[test]
    fn the_tray_hands_what_was_chosen_over_to_the_queue() {
        let tray = TrayWindow::create().expect("the tray window was made");

        // A click of the left button of the icon asks for Settings. That
        // part of the icon is imitated here, because a click of the right
        // button would open a real menu and wait for a hand.
        unsafe {
            let _ = SendMessageW(
                tray.hwnd(),
                WM_TRAYICON,
                Some(WPARAM(0)),
                Some(LPARAM(WM_LBUTTONUP as isize)),
            );
        }
        assert_eq!(tray.drain(), vec![TrayCommand::Settings]);
        assert!(tray.drain().is_empty(), "nothing is said twice");

        // The menu asks by numbers through `WM_COMMAND`: this is the
        // message `TrackPopupMenu` sends for the item that was chosen.
        unsafe {
            let _ = SendMessageW(
                tray.hwnd(),
                WM_COMMAND,
                Some(WPARAM(TRAY_ID_SETTINGS)),
                Some(LPARAM(0)),
            );
            let _ = SendMessageW(
                tray.hwnd(),
                WM_COMMAND,
                Some(WPARAM(TRAY_ID_EXIT)),
                Some(LPARAM(0)),
            );
        }
        assert_eq!(tray.drain(), vec![TrayCommand::Settings, TrayCommand::Exit]);
    }
}
