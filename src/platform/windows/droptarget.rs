//! Being the target of the drag.
//!
//! Explorer's file drag keeps its payload to itself: the file list is not on
//! the global clipboard, and it is not on the OLE clipboard either. The only
//! way to see it is to be a registered drop target, which is what this is.
//!
//! Registering happens only while an interaction is running, so a drag that
//! has nothing to do with the wheel is never touched.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;

use windows::Win32::Foundation::POINTL;
use windows::Win32::System::Com::{
    DVASPECT_CONTENT, FORMATETC, IDataObject, STGMEDIUM, TYMED_HGLOBAL,
};
use windows::Win32::System::Ole::{
    CF_HDROP, DROPEFFECT, DROPEFFECT_COPY, IDropTarget, IDropTarget_Impl, ReleaseStgMedium,
};
use windows::Win32::System::SystemServices::MODIFIERKEYS_FLAGS;
use windows::Win32::UI::Shell::{DragQueryFileW, HDROP};
use windows::core::{Ref, implement};

/// What the drop target tells the application.
#[derive(Default)]
pub struct DropState {
    /// The files of the drag that is over the window.
    pub paths: RefCell<Vec<PathBuf>>,
    /// Whether a drag is over the window.
    pub over: Cell<bool>,
    /// Whether the drag was dropped on the window.
    pub dropped: Cell<bool>,
    /// Where the pointer is, in **screen** pixels.
    ///
    /// The points that the drag hands over are in screen coordinates, the
    /// same coordinates the pointer is read in, so both can be used without
    /// being converted.
    pub point: Cell<(i32, i32)>,
    /// The drag the files belong to.
    ///
    /// The files are told to the application while the drag is over the
    /// window, which is after the drag has begun. The number of the drag is
    /// put here when the window becomes a drop target, so that files that
    /// arrive late are not taken for the files of a drag that came after.
    pub session: Cell<u64>,
}

impl DropState {
    /// Takes the "a drag was dropped" flag, so that it is only seen once.
    pub fn take_drop(&self) -> bool {
        self.dropped.replace(false)
    }

    /// Where the pointer is, in screen pixels.
    pub fn screen_point(&self) -> Option<crate::radial::geometry::Point> {
        let (x, y) = self.point.get();
        if x == 0 && y == 0 {
            None
        } else {
            Some(crate::radial::geometry::Point::new(x as f32, y as f32))
        }
    }
}

/// The COM object Windows talks to when a drag passes over the wheel.
#[implement(IDropTarget)]
struct DropTargetImpl {
    state: Rc<DropState>,
}

impl IDropTarget_Impl for DropTargetImpl_Impl {
    fn DragEnter(
        &self,
        data: Ref<IDataObject>,
        _keys: MODIFIERKEYS_FLAGS,
        point: &POINTL,
        effect: *mut DROPEFFECT,
    ) -> windows::core::Result<()> {
        crate::diagnostics::log(
            crate::diagnostics::Level::Debug,
            "drag",
            &format!("the drag entered the wheel at ({}, {})", point.x, point.y),
        );
        *self.state.paths.borrow_mut() = match data.as_ref() {
            Some(object) => read_file_paths(object),
            None => Vec::new(),
        };
        self.state.over.set(true);
        self.state.point.set((point.x, point.y));
        // SAFETY: the pointer is given by the system and is valid.
        unsafe { *effect = DROPEFFECT_COPY };
        Ok(())
    }

    fn DragOver(
        &self,
        _keys: MODIFIERKEYS_FLAGS,
        point: &POINTL,
        effect: *mut DROPEFFECT,
    ) -> windows::core::Result<()> {
        self.state.point.set((point.x, point.y));
        // SAFETY: the pointer is given by the system and is valid.
        unsafe { *effect = DROPEFFECT_COPY };
        Ok(())
    }

    fn DragLeave(&self) -> windows::core::Result<()> {
        self.state.over.set(false);
        Ok(())
    }

    fn Drop(
        &self,
        data: Ref<IDataObject>,
        _keys: MODIFIERKEYS_FLAGS,
        point: &POINTL,
        effect: *mut DROPEFFECT,
    ) -> windows::core::Result<()> {
        crate::diagnostics::log(
            crate::diagnostics::Level::Info,
            "drag",
            &format!("the drag was dropped at ({}, {})", point.x, point.y),
        );
        *self.state.paths.borrow_mut() = match data.as_ref() {
            Some(object) => read_file_paths(object),
            None => Vec::new(),
        };
        self.state.over.set(false);
        self.state.dropped.set(true);
        self.state.point.set((point.x, point.y));
        // SAFETY: the pointer is given by the system and is valid.
        unsafe { *effect = DROPEFFECT_COPY };
        Ok(())
    }
}

/// Makes a drop target for the given state.
pub fn make_drop_target(state: Rc<DropState>) -> IDropTarget {
    DropTargetImpl { state }.into()
}

/// The files in the data object of a drag.
fn read_file_paths(data: &IDataObject) -> Vec<PathBuf> {
    // The shell offers the file list as CF_HDROP.
    if let Some(paths) = read_hdrop(data) {
        return paths;
    }
    // Some sources use the shell's own "DragDrop" format instead, which
    // carries a DROPFILES structure.
    read_dropfiles(data).unwrap_or_default()
}

/// Reads `CF_HDROP` from a data object.
fn read_hdrop(data: &IDataObject) -> Option<Vec<PathBuf>> {
    let format = FORMATETC {
        ptd: std::ptr::null_mut(),
        dwAspect: DVASPECT_CONTENT.0,
        lindex: -1,
        tymed: TYMED_HGLOBAL.0 as u32,
        cfFormat: CF_HDROP.0,
    };
    // SAFETY: the format is a plain clipboard format with no extra context.
    unsafe {
        if data.QueryGetData(&format).is_err() {
            return None;
        }
        let mut medium = data.GetData(&format).ok()?;
        let handle = medium.u.hGlobal;
        let paths = if handle.0.is_null() {
            Vec::new()
        } else {
            paths_from_hdrop(handle)
        };
        ReleaseStgMedium(&mut medium);
        Some(paths)
    }
}

/// Reads the shell's `DragDrop` format from a data object.
fn read_dropfiles(data: &IDataObject) -> Option<Vec<PathBuf>> {
    // SAFETY: the name is a static wide string.
    let format_id = unsafe {
        windows::Win32::System::DataExchange::RegisterClipboardFormatW(windows::core::w!(
            "DragDrop"
        ))
    };
    if format_id == 0 {
        return None;
    }
    let format = FORMATETC {
        ptd: std::ptr::null_mut(),
        dwAspect: DVASPECT_CONTENT.0,
        lindex: -1,
        tymed: TYMED_HGLOBAL.0 as u32,
        cfFormat: format_id as u16,
    };
    // SAFETY: the format is a plain clipboard format with no extra context.
    unsafe {
        if data.QueryGetData(&format).is_err() {
            return None;
        }
        let mut medium: STGMEDIUM = data.GetData(&format).ok()?;
        let handle = medium.u.hGlobal;
        if handle.0.is_null() {
            ReleaseStgMedium(&mut medium);
            return Some(Vec::new());
        }
        let paths = parse_dropfiles(handle);
        ReleaseStgMedium(&mut medium);
        Some(paths)
    }
}

/// The files in a drop handle.
fn paths_from_hdrop(handle: windows::Win32::Foundation::HGLOBAL) -> Vec<PathBuf> {
    let drop = HDROP(handle.0);
    // SAFETY: the handle is a drop handle.
    let count = unsafe { DragQueryFileW(drop, u32::MAX, None) };
    if count == 0 {
        return Vec::new();
    }
    let mut paths = Vec::with_capacity(count as usize);
    let mut buffer = vec![0u16; 32_768];
    for index in 0..count {
        // SAFETY: the buffer is long enough for any path.
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

/// The files in a `DROPFILES` structure.
fn parse_dropfiles(handle: windows::Win32::Foundation::HGLOBAL) -> Vec<PathBuf> {
    use windows::Win32::System::Memory::{GlobalLock, GlobalUnlock};
    use windows::Win32::UI::Shell::DROPFILES;
    // SAFETY: the handle is a global memory object with a DROPFILES header.
    unsafe {
        let base = GlobalLock(handle);
        if base.is_null() {
            return Vec::new();
        }
        let header = base.cast::<DROPFILES>();
        let offset = (*header).pFiles as usize;
        let wide = (*header).fWide.as_bool();
        let list = base.cast::<u8>().add(offset);
        let mut paths = Vec::new();
        if wide {
            let mut cursor = list.cast::<u16>();
            loop {
                let mut length = 0usize;
                while *cursor.add(length) != 0 {
                    length += 1;
                    if length > 32_768 {
                        break;
                    }
                }
                if length == 0 {
                    break;
                }
                let text = String::from_utf16_lossy(std::slice::from_raw_parts(cursor, length));
                if !text.is_empty() {
                    paths.push(PathBuf::from(text));
                }
                cursor = cursor.add(length + 1);
            }
        }
        let _ = GlobalUnlock(handle);
        paths
    }
}
