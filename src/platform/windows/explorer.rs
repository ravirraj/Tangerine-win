//! Recognising a file drag that comes out of Explorer.
//!
//! Two things have to be true before the wheel may open:
//!
//! 1. the press started on an Explorer window, and
//! 2. the pointer moved far enough to be a drag rather than a click.
//!
//! The file list is the hard part. Explorer's drag keeps its payload to
//! itself: it is not on the global clipboard, and it is not on the OLE
//! clipboard either, because Windows 8 and later leave the clipboard alone
//! during a drag. The only way to see what is being dragged is to be a
//! registered drop target, which is what `droptarget.rs` is for.
//!
//! Nothing is faked: if the payload cannot be read, the file list stays empty
//! and the interaction is reported as having no files.

use std::path::PathBuf;

use windows::Win32::Foundation::{HGLOBAL, HWND, POINT};
use windows::Win32::System::DataExchange::{
    CloseClipboard, GetClipboardData, GetClipboardSequenceNumber, OpenClipboard,
    RegisterClipboardFormatW,
};
use windows::Win32::System::Memory::{GlobalLock, GlobalUnlock};
use windows::Win32::System::Ole::CF_HDROP;
use windows::Win32::UI::Shell::{DROPFILES, DragQueryFileW, HDROP};
use windows::Win32::UI::WindowsAndMessaging::{GA_ROOT, GetAncestor, WindowFromPoint};

use crate::platform::windows::input;
use crate::radial::geometry::Point;

/// How far the pointer has to travel before a press counts as a drag.
pub const DRAG_THRESHOLD: f32 = 8.0;

/// The window classes Explorer uses for its file lists.
const EXPLORER_CLASSES: [&str; 2] = ["CabinetWClass", "ExploreWClass"];

/// Whether a window belongs to Explorer's file list.
pub fn is_explorer_window(hwnd: HWND) -> bool {
    match input::window_class(hwnd) {
        Some(class) => EXPLORER_CLASSES.iter().any(|known| known == &class),
        None => false,
    }
}

/// The Explorer window under a screen point, if there is one.
pub fn explorer_window_at_point(point: Point) -> Option<HWND> {
    let native = POINT {
        x: point.x as i32,
        y: point.y as i32,
    };
    // SAFETY: `WindowFromPoint` takes a POINT by value and never dereferences it.
    let hit = unsafe { WindowFromPoint(native) };
    if hit.0.is_null() {
        return None;
    }
    // SAFETY: `GetAncestor` only walks the window tree.
    let root = unsafe { GetAncestor(hit, GA_ROOT) };
    let root = if root.0.is_null() { hit } else { root };
    if is_explorer_window(root) {
        Some(root)
    } else {
        None
    }
}

/// The class name of the window under a screen point.
pub fn window_class_at_point(point: Point) -> Option<String> {
    let native = POINT {
        x: point.x as i32,
        y: point.y as i32,
    };
    // SAFETY: `WindowFromPoint` takes a POINT by value and never dereferences it.
    let hit = unsafe { WindowFromPoint(native) };
    if hit.0.is_null() {
        return None;
    }
    // SAFETY: `GetAncestor` only walks the window tree.
    let root = unsafe { GetAncestor(hit, GA_ROOT) };
    let root = if root.0.is_null() { hit } else { root };
    input::window_class(root)
}

/// A number that changes every time the clipboard changes.
pub fn clipboard_sequence() -> u32 {
    // SAFETY: the call takes no arguments and cannot fail.
    unsafe { GetClipboardSequenceNumber() }
}

/// The clipboard format Explorer puts a dragged file list under.
fn drag_drop_format() -> Option<u32> {
    // SAFETY: the name is a static wide string.
    let id = unsafe { RegisterClipboardFormatW(windows::core::w!("DragDrop")) };
    (id != 0).then_some(id)
}

/// The files that are being dragged right now.
///
/// This is the fallback for sources that do put their payload on the
/// clipboard. Explorer's own drag does not, so the real list comes from the
/// drop target instead.
///
/// Returns `None` when the clipboard could not be read at all, and an empty
/// list when it could be read but holds no file drag.
pub fn dragged_paths() -> Option<Vec<PathBuf>> {
    clipboard_dragged_paths()
}

/// The files in the ordinary clipboard formats.
fn clipboard_dragged_paths() -> Option<Vec<PathBuf>> {
    let format = drag_drop_format()?;
    // SAFETY: the clipboard is opened for reading and closed again below.
    unsafe {
        if OpenClipboard(None).is_err() {
            // Another window has the clipboard open. This happens during a
            // drag, so it is worth trying again a moment later rather than
            // giving up on the whole interaction.
            return None;
        }
        let payload = GetClipboardData(format)
            .ok()
            .and_then(|handle| parse_dropfiles(handle))
            .or_else(|| hdrop_paths());
        let _ = CloseClipboard();
        Some(payload.unwrap_or_default())
    }
}

/// Reads the files out of a `DROPFILES` structure.
unsafe fn parse_dropfiles(handle: windows::Win32::Foundation::HANDLE) -> Option<Vec<PathBuf>> {
    // SAFETY: the handle comes from the clipboard and is a global memory
    // object holding a DROPFILES header followed by a file list.
    let base = unsafe { GlobalLock(HGLOBAL(handle.0)) };
    if base.is_null() {
        return None;
    }
    let header = base.cast::<DROPFILES>();
    let offset = unsafe { (*header).pFiles } as usize;
    let wide = unsafe { (*header).fWide }.as_bool();
    let list = unsafe { base.cast::<u8>().add(offset) };

    let mut paths = Vec::new();
    if wide {
        let mut cursor = list.cast::<u16>();
        loop {
            let length = unsafe { utf16_length(cursor) };
            if length == 0 {
                break;
            }
            let text =
                String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(cursor, length) });
            if !text.is_empty() {
                paths.push(PathBuf::from(text));
            }
            cursor = unsafe { cursor.add(length + 1) };
        }
    } else {
        let mut cursor = list;
        loop {
            let length = unsafe { byte_length(cursor) };
            if length == 0 {
                break;
            }
            let bytes = unsafe { std::slice::from_raw_parts(cursor, length) };
            let text = String::from_utf8_lossy(bytes).to_string();
            if !text.is_empty() {
                paths.push(PathBuf::from(text));
            }
            cursor = unsafe { cursor.add(length + 1) };
        }
    }
    let _ = unsafe { GlobalUnlock(HGLOBAL(handle.0)) };
    Some(paths)
}

/// Reads the files out of a `CF_HDROP` clipboard entry.
unsafe fn hdrop_paths() -> Option<Vec<PathBuf>> {
    let handle = unsafe { GetClipboardData(CF_HDROP.0 as u32) }.ok()?;
    Some(unsafe { paths_from_hdrop(HGLOBAL(handle.0)) })
}

/// Reads the files out of a drop handle.
unsafe fn paths_from_hdrop(handle: HGLOBAL) -> Vec<PathBuf> {
    let drop = HDROP(handle.0);
    let count = unsafe { DragQueryFileW(drop, u32::MAX, None) };
    if count == 0 {
        return Vec::new();
    }
    let mut paths = Vec::with_capacity(count as usize);
    // The longest path Windows allows is a good upper bound.
    let mut buffer = vec![0u16; 32_768];
    for index in 0..count {
        let written = unsafe { DragQueryFileW(drop, index, Some(&mut buffer[..])) };
        if written == 0 {
            continue;
        }
        let text = String::from_utf16_lossy(&buffer[..written as usize]);
        if !text.is_empty() {
            paths.push(PathBuf::from(text));
        }
    }
    paths
}

/// The length of a wide string, up to the first null.
unsafe fn utf16_length(cursor: *const u16) -> usize {
    let mut length = 0usize;
    while unsafe { *cursor.add(length) } != 0 {
        length += 1;
        if length > 32_768 {
            break;
        }
    }
    length
}

/// The length of a byte string, up to the first null.
unsafe fn byte_length(cursor: *const u8) -> usize {
    let mut length = 0usize;
    while unsafe { *cursor.add(length) } != 0 {
        length += 1;
        if length > 32_768 {
            break;
        }
    }
    length
}

/// What the drag detector noticed this frame.
#[derive(Clone, Debug, PartialEq)]
pub enum DragEvent {
    /// Nothing changed.
    None,
    /// A drag started. The file list may still be empty: the shell publishes
    /// the payload a moment after the drag begins.
    Started { paths: Vec<PathBuf> },
    /// The drag ended.
    Ended,
}

/// Watches for an Explorer drag.
///
/// The detector only ever looks at the pointer, the button and the window
/// under the pointer. It does not install a hook and it does not change any
/// input.
pub struct DragDetector {
    armed: bool,
    reported: bool,
    origin: Option<Point>,
    press_sequence: u32,
    paths: Vec<PathBuf>,
    /// How many frames the drag has been running.
    frames: u32,
}

impl Default for DragDetector {
    fn default() -> Self {
        Self::new()
    }
}

impl DragDetector {
    pub fn new() -> Self {
        Self {
            armed: false,
            reported: false,
            origin: None,
            press_sequence: 0,
            paths: Vec::new(),
            frames: 0,
        }
    }

    /// Whether a drag is currently being watched.
    pub fn is_armed(&self) -> bool {
        self.armed
    }

    /// The file list of the drag in progress.
    pub fn paths(&self) -> &[PathBuf] {
        &self.paths
    }

    /// Looks at the pointer and the mouse button.
    ///
    /// `left_down` says whether the primary button is held.
    pub fn poll(&mut self, cursor: Point, left_down: bool) -> DragEvent {
        if !left_down {
            let was_armed = self.armed;
            self.armed = false;
            self.reported = false;
            self.origin = None;
            self.paths.clear();
            return if was_armed {
                DragEvent::Ended
            } else {
                DragEvent::None
            };
        }

        if !self.armed {
            // The button has just gone down. Only a press that starts on an
            // Explorer window can become a file drag.
            if explorer_window_at_point(cursor).is_none() {
                return DragEvent::None;
            }
            self.armed = true;
            self.reported = false;
            self.origin = Some(cursor);
            self.press_sequence = clipboard_sequence();
            self.paths.clear();
            self.frames = 0;
            return DragEvent::None;
        }

        if !self.reported {
            let origin = self.origin.unwrap_or(cursor);
            let travelled = ((cursor.x - origin.x).powi(2) + (cursor.y - origin.y).powi(2)).sqrt();
            if travelled < DRAG_THRESHOLD {
                return DragEvent::None;
            }
            // The drag has only just started, so this is the first moment the
            // shell has published its payload. Read it now, before it can go
            // stale.
            //
            // The clipboard is only trusted when it changed since the press:
            // the shell leaves the payload of the last drag on the clipboard,
            // and reading that would name the wrong files.
            if clipboard_sequence() != self.press_sequence
                && let Some(paths) = dragged_paths()
                && !paths.is_empty()
            {
                self.paths = paths;
            }
            self.reported = true;
            return DragEvent::Started {
                paths: self.paths.clone(),
            };
        }

        // Keep the payload fresh for the rest of the drag, but not every
        // frame: reading the clipboard is not free.
        self.frames += 1;
        if self.frames.is_multiple_of(8)
            && clipboard_sequence() != self.press_sequence
            && let Some(paths) = dragged_paths()
            && !paths.is_empty()
        {
            self.paths = paths;
        }
        DragEvent::None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_detector_starts_idle() {
        let detector = DragDetector::new();
        assert!(!detector.is_armed());
        assert!(detector.paths().is_empty());
    }

    #[test]
    fn a_detector_ignores_movement_while_the_button_is_up() {
        let mut detector = DragDetector::new();
        for step in 0..40 {
            let event = detector.poll(Point::new(step as f32 * 30.0, 100.0), false);
            assert_eq!(event, DragEvent::None);
        }
        assert!(!detector.is_armed());
    }

    #[test]
    fn the_drag_threshold_is_reasonable() {
        assert!(DRAG_THRESHOLD >= 4.0);
        assert!(DRAG_THRESHOLD <= 24.0);
    }

    #[test]
    fn reading_the_clipboard_never_panics() {
        // The clipboard may or may not be available; the call must not crash.
        let _ = dragged_paths();
    }
}
