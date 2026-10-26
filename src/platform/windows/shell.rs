//! Shell integration for reveal-in-Explorer and share file actions.

use std::path::Path;
use std::process::Command;
use windows::core::PCWSTR;
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Shell::{ShellExecuteExW, SHELLEXECUTEINFOW, SEE_MASK_INVOKEIDLIST};
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
use crate::platform::windows::windowhost::to_wide;

/// Reveal the given path in Windows Explorer.
pub fn reveal_in_explorer(path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let path_str = path.to_string_lossy();
    Command::new("explorer.exe")
        .args(["/select,", path_str.as_ref()])
        .spawn()
        .map_err(|_| "failed to start explorer.exe")?;
    Ok(())
}

/// Invoke the Windows Share contract on the given path.
pub fn share_file(path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let wide_path = to_wide(&path.to_string_lossy());
    let wide_verb = to_wide("share");

    let mut info = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_INVOKEIDLIST,
        hwnd: HWND(std::ptr::null_mut()),
        lpVerb: PCWSTR(wide_verb.as_ptr()),
        lpFile: PCWSTR(wide_path.as_ptr()),
        lpParameters: PCWSTR::null(),
        lpDirectory: PCWSTR::null(),
        nShow: SW_SHOWNORMAL.0,
        ..Default::default()
    };

    let result = unsafe { ShellExecuteExW(&mut info) };
    if result.is_ok() {
        Ok(())
    } else {
        // Fallback to reveal in Explorer if the share verb is unavailable
        reveal_in_explorer(path)
    }
}
