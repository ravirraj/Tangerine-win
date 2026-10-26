//! Windows System Tray (Notification Area) integration.
//!
//! Provides a persistent icon in the taskbar notification area, allowing the user
//! to open Tangerine, access Settings, or exit the application via a context menu.

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HWND, POINT};
use windows::Win32::UI::Shell::{
    Shell_NotifyIconW, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW, NIF_ICON, NIF_MESSAGE, NIF_TIP,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, DestroyMenu, GetCursorPos, LoadIconW, SetForegroundWindow,
    TrackPopupMenu, IDI_APPLICATION, MF_SEPARATOR, MF_STRING, TPM_RIGHTBUTTON, WM_USER,
};

#[allow(dead_code)]
pub const WM_TRAYICON: u32 = WM_USER + 101;
#[allow(dead_code)]
pub const TRAY_ID_OPEN: usize = 1001;
#[allow(dead_code)]
pub const TRAY_ID_SETTINGS: usize = 1002;
#[allow(dead_code)]
pub const TRAY_ID_EXIT: usize = 1003;

#[allow(dead_code)]
pub fn add_tray_icon(hwnd: HWND) -> bool {
    let mut nid = make_nid(hwnd);
    nid.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
    nid.uCallbackMessage = WM_TRAYICON;

    unsafe {
        if let Ok(icon) = LoadIconW(None, IDI_APPLICATION) {
            nid.hIcon = icon;
        }
        let tip = w!("Open Tangerine");
        let tip_slice = tip.as_wide();
        let len = tip_slice.len().min(nid.szTip.len() - 1);
        nid.szTip[..len].copy_from_slice(&tip_slice[..len]);
        nid.szTip[len] = 0;

        Shell_NotifyIconW(NIM_ADD, &nid).as_bool()
    }
}

#[allow(dead_code)]
pub fn remove_tray_icon(hwnd: HWND) -> bool {
    let nid = make_nid(hwnd);
    unsafe { Shell_NotifyIconW(NIM_DELETE, &nid).as_bool() }
}

#[allow(dead_code)]
pub fn show_tray_menu(hwnd: HWND) {
    unsafe {
        let hmenu = match CreatePopupMenu() {
            Ok(m) => m,
            Err(_) => return,
        };

        let _ = AppendMenuW(hmenu, MF_STRING, TRAY_ID_OPEN, w!("Open Tangerine"));
        let _ = AppendMenuW(hmenu, MF_STRING, TRAY_ID_SETTINGS, w!("Settings…"));
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

    #[test]
    fn make_nid_has_correct_size() {
        let nid = make_nid(HWND(std::ptr::null_mut()));
        assert_eq!(nid.cbSize, std::mem::size_of::<NOTIFYICONDATAW>() as u32);
        assert_eq!(nid.uID, 1);
    }
}
