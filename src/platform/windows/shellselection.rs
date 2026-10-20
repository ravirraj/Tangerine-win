//! The files that are being dragged, read from the shell's own view of the
//! Explorer window the drag started in.
//!
//! Explorer's drag keeps its payload to itself. It is not on the global
//! clipboard, it is not on the OLE clipboard, and the shell does not offer it
//! to drop targets outside its own. What the shell *does* publish is the
//! selection of the window the drag started in, and that selection is exactly
//! what is being dragged.
//!
//! So the files are asked for from the shell itself, through the automation
//! interface the shell has always offered: the same `Shell.Application` object a
//! script would use. The call only reads; it never captures the mouse, it
//! never touches the drag, and it cannot change what Explorer does with it.
//!
//! The thread has to be able to talk to COM. The application already starts
//! OLE on its main thread, which is an apartment thread, so the calls can be
//! made there.

use std::path::PathBuf;

use windows::Win32::Foundation::HWND;
use windows::Win32::System::Com::{
    CLSCTX_ALL, CoCreateInstance, DISPATCH_FLAGS, DISPATCH_METHOD, DISPATCH_PROPERTYGET,
    DISPPARAMS, IDispatch,
};
use windows::Win32::System::Variant::{
    VARIANT, VARIANT_0_0, VARIANT_0_0_0, VT_BSTR, VT_DISPATCH, VT_I2, VT_I4, VT_I8, VT_INT, VT_UI2,
    VT_UI4, VT_UI8, VariantClear,
};
use windows::Win32::UI::Shell::IShellDispatch;
use windows::core::{BSTR, GUID, HRESULT, Interface, PCWSTR};

use crate::platform::windows::explorer::is_explorer_window;
use crate::session::Resolution;

/// The language the shell's names are asked for in.
const LOCALE_USER_DEFAULT: u32 = 1024;

/// The shell's object for one Explorer window.
///
/// Finding it takes a moment, and it takes a good deal longer while a drag
/// is going, because then Explorer is busy and the shell has to wait for it.
/// The same goes for the document of the window, which the shell has to build
/// before it can say what is selected. So both are made while Explorer is
/// still idle, and the selection is read from them a moment later, when the
/// drag has begun.
#[derive(Clone)]
pub struct Window {
    document: IDispatch,
}

impl Window {
    /// The shell's object for an Explorer window, if the shell knows it.
    pub fn find(window: HWND) -> Option<Self> {
        if window.0.is_null() {
            return None;
        }
        if !is_explorer_window(window) {
            return None;
        }
        let dispatch = explorer_window(window).ok()?;
        let document = document_of(&dispatch).ok()?;
        Some(Self { document })
    }

    /// Reads the files that are selected in the window.
    ///
    /// When the shell cannot be asked, the answer says so and holds no path
    /// at all.
    pub fn selected_paths(&self) -> Resolution {
        let started = std::time::Instant::now();
        let resolution = match selected_items(&self.document) {
            Ok(paths) => Resolution::Resolved(paths),
            Err(reason) => Resolution::Unresolved(reason),
        };
        crate::diagnostics::log(
            crate::diagnostics::Level::Debug,
            "shell",
            &format!(
                "the selection was read in {:?} and holds {} file(s)",
                started.elapsed(),
                resolution.paths().len()
            ),
        );
        resolution
    }
}

/// Reads the files that are selected in an Explorer window.
///
/// `window` is the Explorer window the drag started in. The result is the
/// selection of that window, with the full path of every file. When the shell
/// cannot be asked, the result says so and holds no path at all.
pub fn selected_paths(window: HWND) -> Resolution {
    let started = std::time::Instant::now();
    let resolution = match Window::find(window) {
        Some(found) => found.selected_paths(),
        None => Resolution::Unresolved("the shell does not know that window"),
    };
    crate::diagnostics::log(
        crate::diagnostics::Level::Debug,
        "shell",
        &format!(
            "the shell was asked in {:?} and answered with {} file(s)",
            started.elapsed(),
            resolution.paths().len()
        ),
    );
    resolution
}

/// A description of what the shell can be asked, for the log.
///
/// It says how many windows the shell knows about and what their handles are,
/// so that a window the shell does not know can be told apart from a shell
/// that could not be asked at all.
pub fn probe() -> String {
    let shell = match shell_object() {
        Ok(shell) => shell,
        Err(error) => return format!("the shell object could not be made: {error}"),
    };
    // SAFETY: the shell object is alive for the length of the call.
    let windows = match unsafe { shell.Windows() } {
        Ok(windows) => windows,
        Err(error) => return format!("the shell gave no list of windows: {error}"),
    };
    // SAFETY: the collection is alive for the length of the call.
    let count = match unsafe { count_of(&windows) } {
        Ok(count) => count,
        Err(error) => return format!("the shell's list could not be counted: {error}"),
    };
    let mut text = format!("the shell knows {count} window(s)");
    for index in 0..count {
        // SAFETY: the collection is alive for the length of the call.
        match unsafe { item_of(&windows, index) } {
            Ok(item) => {
                // SAFETY: the item is a shell object.
                match unsafe { property_number(&item, "HWND") } {
                    Ok(handle) => text.push_str(&format!(" {handle:#x}")),
                    Err(error) => text.push_str(&format!(" (the handle of {index}: {error})")),
                }
            }
            Err(error) => text.push_str(&format!(" (the window {index}: {error})")),
        }
    }
    text
}

thread_local! {
    /// The shell's automation object, made once and then kept.
    ///
    /// Making it takes a moment, and the wheel cannot wait for that while a
    /// drag is going. The object is made when the program starts, and then it
    /// is only asked questions.
    static SHELL: std::cell::RefCell<Option<IShellDispatch>> = const { std::cell::RefCell::new(None) };
}

/// The shell's own automation object.
fn shell_object() -> Result<windows::Win32::UI::Shell::IShellDispatch, windows::core::Error> {
    SHELL.with(|cell| {
        if let Some(shell) = cell.borrow().as_ref() {
            return Ok(shell.clone());
        }
        // SAFETY: the shell object is created on the apartment thread the
        // application already prepared.
        let shell: IShellDispatch = unsafe {
            let class = windows::Win32::System::Com::CLSIDFromProgID(windows::core::w!(
                "Shell.Application"
            ))?;
            CoCreateInstance(&class, None, CLSCTX_ALL)?
        };
        *cell.borrow_mut() = Some(shell.clone());
        Ok(shell)
    })
}

/// Makes the shell's automation object and asks it a question.
///
/// The shell takes its time the first time it is asked anything, and the
/// wheel cannot wait for that while a drag is going. So the shell is asked
/// now, while nothing is happening. The answer describes what the shell can
/// be asked, and is only used for the log.
pub fn warm_up() -> String {
    let _ = shell_object();
    probe()
}

/// The shell's object for one Explorer window.
fn explorer_window(window: HWND) -> Result<IDispatch, &'static str> {
    let shell = shell_object().map_err(|_| "the shell object could not be made")?;
    // SAFETY: the shell object is alive for the length of the call.
    let windows = unsafe { shell.Windows() }.map_err(|_| "the shell gave no list of windows")?;
    // SAFETY: the collection is alive for the length of the call.
    let count =
        unsafe { count_of(&windows) }.map_err(|_| "the shell's list could not be counted")?;
    for index in 0..count {
        // SAFETY: the collection is alive for the length of the call.
        let item = match unsafe { item_of(&windows, index) } {
            Ok(item) => item,
            Err(_) => continue,
        };
        // SAFETY: the item is a shell object.
        let handle = match unsafe { property_number(&item, "HWND") } {
            Ok(handle) => handle,
            Err(_) => continue,
        };
        if handle as isize == window.0 as isize {
            return Ok(item);
        }
    }
    Err("the shell does not know that window")
}

/// The document of an Explorer window.
///
/// The shell has to build this before it can say what is selected, and it
/// takes a good deal longer while a drag is going, because then Explorer is
/// busy. So it is built while Explorer is still idle.
fn document_of(window: &IDispatch) -> Result<IDispatch, &'static str> {
    // SAFETY: the window object is alive for the length of the call.
    let document =
        unsafe { member(window, "Document", &[]) }.map_err(|_| "the window has no document")?;
    // The document is a shell object.
    document
        .as_dispatch()
        .map_err(|_| "the document is not a shell object")
}

/// The full paths of the items selected in an Explorer window.
fn selected_items(document: &IDispatch) -> Result<Vec<PathBuf>, &'static str> {
    // SAFETY: the document is alive for the length of the call.
    let items = unsafe { member(document, "SelectedItems", &[]) }
        .map_err(|_| "the selection could not be asked for")?;
    // The collection is a shell object.
    let items = items
        .as_dispatch()
        .map_err(|_| "the selection is not a shell object")?;
    // SAFETY: the collection is alive for the length of the call.
    let count = unsafe { count_of(&items) }.map_err(|_| "the selection could not be counted")?;
    let mut paths = Vec::new();
    for index in 0..count {
        // SAFETY: the collection is alive for the length of the call.
        let item = match unsafe { item_of(&items, index) } {
            Ok(item) => item,
            Err(_) => continue,
        };
        // SAFETY: the item is a shell object.
        match unsafe { property_text(&item, "Path") } {
            Ok(path) if !path.is_empty() => paths.push(PathBuf::from(path)),
            _ => {
                // A virtual item has no path of its own. It is left out
                // rather than guessed at.
            }
        }
    }
    Ok(paths)
}

/// How many things a shell collection holds.
unsafe fn count_of(collection: &IDispatch) -> windows::core::Result<i32> {
    let count = unsafe { call(collection, "Count", &[])? };
    count.as_number()
}

/// Calls a member of a shell object.
///
/// The shell exposes some members as a property and some as a method, and it
/// does not always say which. Both ways of asking are tried, the property way
/// first.
unsafe fn member(
    object: &IDispatch,
    name: &str,
    arguments: &[VARIANT],
) -> windows::core::Result<Variant> {
    match unsafe { call_with(object, name, arguments, DISPATCH_PROPERTYGET) } {
        Ok(value) => Ok(value),
        Err(first) => {
            let second = unsafe { call_with(object, name, arguments, DISPATCH_METHOD) };
            second.map_err(|_| first)
        }
    }
}

/// One thing out of a shell collection.
///
/// Some collections hand out their items through a parameterised property,
/// others through a plain method, so both are tried.
unsafe fn item_of(collection: &IDispatch, index: i32) -> windows::core::Result<IDispatch> {
    let item = unsafe { member(collection, "Item", &[number_argument(index)]) }?;
    item.as_dispatch()
}

/// A variant that holds a number.
fn number_argument(value: i32) -> VARIANT {
    let mut argument = VARIANT::default();
    // SAFETY: the variant is empty and is given a number.
    unsafe {
        let header = (&raw mut argument.Anonymous).cast::<VARIANT_0_0>();
        (*header).vt = VT_I4;
        let data = (&raw mut (*header).Anonymous).cast::<VARIANT_0_0_0>();
        (*data).lVal = value;
    }
    argument
}

/// A number property of a shell object.
unsafe fn property_number(object: &IDispatch, name: &str) -> windows::core::Result<i32> {
    unsafe { call(object, name, &[])? }.as_number()
}

/// A text property of a shell object.
unsafe fn property_text(object: &IDispatch, name: &str) -> windows::core::Result<String> {
    unsafe { call(object, name, &[])? }.as_text()
}

/// Calls a method or reads a property of a shell object.
unsafe fn call(
    object: &IDispatch,
    name: &str,
    arguments: &[VARIANT],
) -> windows::core::Result<Variant> {
    unsafe { call_with(object, name, arguments, DISPATCH_PROPERTYGET) }
}

/// Calls a method or reads a property of a shell object, with the way the
/// member wants to be called.
unsafe fn call_with(
    object: &IDispatch,
    name: &str,
    arguments: &[VARIANT],
    flags: DISPATCH_FLAGS,
) -> windows::core::Result<Variant> {
    let mut id = 0i32;
    let wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
    let identity = GUID::zeroed();
    let name_pointer = PCWSTR(wide.as_ptr());
    unsafe { object.GetIDsOfNames(&identity, &name_pointer, 1, LOCALE_USER_DEFAULT, &mut id)? };
    let mut held = arguments.to_vec();
    let parameters = DISPPARAMS {
        rgvarg: if held.is_empty() {
            std::ptr::null_mut()
        } else {
            held.as_mut_ptr()
        },
        rgdispidNamedArgs: std::ptr::null_mut(),
        cArgs: held.len() as u32,
        cNamedArgs: 0,
    };
    let mut result = VARIANT::default();
    unsafe {
        object.Invoke(
            id,
            &identity,
            LOCALE_USER_DEFAULT,
            flags,
            &parameters,
            Some(&mut result),
            None,
            None,
        )?;
    }
    Ok(Variant { value: result })
}

/// A value the shell gave back.
struct Variant {
    value: VARIANT,
}

impl Variant {
    /// The header of the variant: what kind of value it holds.
    fn inner(&self) -> &VARIANT_0_0 {
        // SAFETY: the variant was filled in by the shell, and the first
        // member of the union is the header.
        unsafe { &*(&raw const self.value.Anonymous).cast::<VARIANT_0_0>() }
    }

    /// The value the variant holds.
    fn data(&self) -> &VARIANT_0_0_0 {
        // SAFETY: the variant was filled in by the shell.
        unsafe { &*(&raw const self.inner().Anonymous).cast::<VARIANT_0_0_0>() }
    }

    /// The kind of value.
    fn kind(&self) -> u16 {
        self.inner().vt.0
    }

    /// The value as a number.
    ///
    /// The shell does not always answer with the same kind of number, so
    /// every kind of whole number is understood.
    fn as_number(&self) -> windows::core::Result<i32> {
        let kind = self.kind();
        if kind == VT_I4.0 {
            // SAFETY: the variant holds a number.
            return Ok(unsafe { self.data().lVal });
        }
        if kind == VT_I8.0 {
            // SAFETY: the variant holds a number.
            return Ok(unsafe { self.data().llVal } as i32);
        }
        if kind == VT_INT.0 {
            // SAFETY: the variant holds a number.
            return Ok(unsafe { self.data().intVal });
        }
        if kind == VT_I2.0 {
            // SAFETY: the variant holds a number.
            return Ok(unsafe { self.data().iVal } as i32);
        }
        if kind == VT_UI4.0 {
            // SAFETY: the variant holds a number.
            return Ok(unsafe { self.data().uintVal } as i32);
        }
        if kind == VT_UI2.0 {
            // SAFETY: the variant holds a number.
            return Ok(unsafe { self.data().uiVal } as i32);
        }
        if kind == VT_UI8.0 {
            // SAFETY: the variant holds a number.
            return Ok(unsafe { self.data().ullVal } as i32);
        }
        if kind == VT_BSTR.0 {
            let text = self.as_text()?;
            return text.trim().parse::<i32>().map_err(|_| wrong_shape(kind));
        }
        Err(wrong_shape(kind))
    }

    /// The value as a shell object.
    fn as_dispatch(&self) -> windows::core::Result<IDispatch> {
        let kind = self.kind();
        if kind != VT_DISPATCH.0 {
            return Err(wrong_shape(kind));
        }
        // SAFETY: the variant holds a dispatch. It is only read, never taken,
        // so that clearing the variant does not throw it away.
        let unknown = unsafe { self.data().punkVal.as_ref() };
        let unknown = unknown.ok_or_else(|| wrong_shape(kind))?;
        unknown.cast::<IDispatch>().map_err(|_| wrong_shape(kind))
    }

    /// The value as text.
    fn as_text(&self) -> windows::core::Result<String> {
        let kind = self.kind();
        if kind != VT_BSTR.0 {
            return Err(wrong_shape(kind));
        }
        // SAFETY: the variant holds a string, which is read before the variant
        // is cleared.
        let text: &BSTR = unsafe { &*(&raw const self.data().bstrVal).cast::<BSTR>() };
        Ok(String::from_utf16_lossy(text))
    }
}

impl Drop for Variant {
    fn drop(&mut self) {
        // SAFETY: the variant was filled in by the shell.
        unsafe {
            let _ = VariantClear(&mut self.value);
        }
    }
}

/// The error used when the shell answers with a kind of value that was not
/// asked for.
fn wrong_shape(kind: u16) -> windows::core::Error {
    windows::core::Error::new(
        HRESULT(0x8000_4005u32 as i32),
        format!("the shell answered with the kind of value {kind}"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::UI::WindowsAndMessaging::GetDesktopWindow;

    #[test]
    fn a_selection_that_was_not_read_has_no_paths() {
        let selection = Resolution::Unresolved("the shell does not know that window");
        assert!(selection.paths().is_empty());
        assert!(!selection.is_resolved());
    }

    #[test]
    fn a_selection_that_was_read_may_still_be_empty() {
        let selection = Resolution::Resolved(Vec::new());
        assert!(selection.paths().is_empty());
        assert!(selection.is_resolved());
    }

    #[test]
    fn a_window_that_is_not_known_has_no_paths() {
        let selection = selected_paths(HWND(std::ptr::null_mut()));
        assert!(!selection.is_resolved());
        assert!(selection.paths().is_empty());
    }

    #[test]
    fn a_window_that_is_not_an_explorer_window_has_no_paths() {
        // SAFETY: the desktop window is a real window, so the call is sound.
        let desktop = unsafe { GetDesktopWindow() };
        let selection = selected_paths(desktop);
        assert!(!selection.is_resolved());
        assert!(selection.paths().is_empty());
    }
}
