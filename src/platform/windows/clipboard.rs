//! Clipboard access for copy-path file action using native Win32 APIs.

use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use windows::Win32::Foundation::HANDLE;
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData,
};
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};

const CF_UNICODETEXT: u32 = 13;

struct ClipboardGuard;
impl Drop for ClipboardGuard {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseClipboard();
        }
    }
}

/// Set the clipboard text to the given string using native Win32 clipboard API.
pub fn set_text(text: &str) -> Result<(), Box<dyn std::error::Error>> {
    let wide: Vec<u16> = OsStr::new(text).encode_wide().chain(std::iter::once(0)).collect();
    let bytes_len = wide.len() * std::mem::size_of::<u16>();

    unsafe {
        let mut opened = false;
        for _ in 0..5 {
            if OpenClipboard(None).is_ok() {
                opened = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        if !opened {
            return Err("failed to open Win32 clipboard".into());
        }

        let _guard = ClipboardGuard;

        EmptyClipboard().map_err(|e| format!("failed to empty clipboard: {e}"))?;

        let hmem = GlobalAlloc(GMEM_MOVEABLE, bytes_len)
            .map_err(|e| format!("failed to allocate global memory: {e}"))?;

        let ptr = GlobalLock(hmem);
        if ptr.is_null() {
            return Err("failed to lock global memory".into());
        }

        std::ptr::copy_nonoverlapping(wide.as_ptr() as *const u8, ptr as *mut u8, bytes_len);
        let _ = GlobalUnlock(hmem);

        if SetClipboardData(CF_UNICODETEXT, Some(HANDLE(hmem.0))).is_err() {
            return Err("failed to set clipboard data".into());
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_clipboard_set_text() {
        // Only run test if interactive/available; shouldn't panic
        let res = set_text("Tangerine test clipboard text");
        assert!(res.is_ok(), "Setting clipboard text should succeed: {:?}", res);
    }
}
