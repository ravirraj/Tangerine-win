//! Where the screens are.
//!
//! The wheel is placed on the monitor the cursor is on, and clamped so that it
//! never runs off the edge of that monitor.

use windows::Win32::Foundation::{HWND, POINT, RECT};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, HMONITOR, MONITOR_DEFAULTTONEAREST, MONITOR_FROM_FLAGS, MONITORINFO,
    MonitorFromPoint,
};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, GetDpiForWindow, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::WindowsAndMessaging::{
    GetSystemMetrics, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN,
};

/// The area of one monitor, in physical screen pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MonitorInfo {
    /// The whole monitor.
    pub bounds: (i32, i32, i32, i32),
    /// The monitor without the task bar and other app bars.
    pub work_area: (i32, i32, i32, i32),
    /// The scale factor, as a percentage (100 means no scaling).
    pub dpi: u32,
}

impl MonitorInfo {
    fn from_rect(monitor: RECT, work: RECT, dpi: u32) -> Self {
        Self {
            bounds: (monitor.left, monitor.top, monitor.right, monitor.bottom),
            work_area: (work.left, work.top, work.right, work.bottom),
            dpi,
        }
    }
}

/// The monitor a screen point sits on.
pub fn monitor_for_point(x: i32, y: i32) -> Option<MonitorInfo> {
    let handle = monitor_handle_at(x, y)?;
    let mut info = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    // SAFETY: `info` is a correctly sized MONITORINFO.
    if unsafe { GetMonitorInfoW(handle, &mut info) }.as_bool() {
        Some(MonitorInfo::from_rect(
            info.rcMonitor,
            info.rcWork,
            dpi_for_monitor(handle),
        ))
    } else {
        None
    }
}

/// The handle of the monitor a screen point sits on.
fn monitor_handle_at(x: i32, y: i32) -> Option<HMONITOR> {
    let point = POINT { x, y };
    // SAFETY: the call only reads the point.
    let handle: HMONITOR =
        unsafe { MonitorFromPoint(point, MONITOR_FROM_FLAGS(MONITOR_DEFAULTTONEAREST.0)) };
    if handle.0.is_null() {
        None
    } else {
        Some(handle)
    }
}

/// The scale factor of a monitor, as a percentage.
fn dpi_for_monitor(handle: HMONITOR) -> u32 {
    let mut x: u32 = 0;
    let mut y: u32 = 0;
    // SAFETY: the handle is a live monitor and the two numbers are writable.
    let result = unsafe { GetDpiForMonitor(handle, MDT_EFFECTIVE_DPI, &mut x, &mut y) };
    if result.is_err() || x == 0 { 96 } else { x }
}

/// The scale factor of the monitor a screen point sits on, as a percentage.
pub fn dpi_for_point(x: i32, y: i32) -> u32 {
    monitor_handle_at(x, y).map(dpi_for_monitor).unwrap_or(96)
}

/// The DPI of a window, as a percentage.
pub fn dpi_for_window(hwnd: HWND) -> u32 {
    // SAFETY: the call only reads the window handle.
    let dpi = unsafe { GetDpiForWindow(hwnd) };
    if dpi == 0 { 96 } else { dpi }
}

/// The area covered by every monitor, in physical screen pixels.
pub fn virtual_bounds() -> (i32, i32, i32, i32) {
    // SAFETY: the calls only read global metrics.
    unsafe {
        let x = GetSystemMetrics(SM_XVIRTUALSCREEN);
        let y = GetSystemMetrics(SM_YVIRTUALSCREEN);
        let width = GetSystemMetrics(SM_CXVIRTUALSCREEN);
        let height = GetSystemMetrics(SM_CYVIRTUALSCREEN);
        (x, y, x + width, y + height)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_virtual_area_is_sane() {
        let (left, top, right, bottom) = virtual_bounds();
        assert!(right > left);
        assert!(bottom > top);
    }

    #[test]
    fn a_monitor_can_be_found_for_the_primary_point() {
        let monitor = monitor_for_point(0, 0);
        assert!(monitor.is_some());
        if let Some(monitor) = monitor {
            assert!(monitor.bounds.2 > monitor.bounds.0);
            assert!(monitor.bounds.3 > monitor.bounds.1);
        }
    }
}
