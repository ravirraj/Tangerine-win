//! The overlay window.
//!
//! The wheel floats above everything else, is see-through where nothing is
//! drawn, never takes the keyboard focus and, while a drag is running, lets
//! every mouse message through to Explorer so the drag is not disturbed.
//!
//! It is a plain Win32 layered window: `WS_EX_LAYERED` gives per-pixel
//! transparency, `UpdateLayeredWindow` puts the pixels on the screen,
//! `WS_EX_NOACTIVATE` keeps the focus where it was and `WS_EX_TRANSPARENT`
//! makes the window click-through.

use std::cell::{Cell, RefCell};
use std::ffi::c_void;
use std::fmt;
use std::rc::Rc;

use windows::Win32::Foundation::{
    COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM,
};
use windows::Win32::Graphics::Gdi::{
    AC_SRC_ALPHA, AC_SRC_OVER, BITMAPINFO, BITMAPINFOHEADER, BLENDFUNCTION, CreateCompatibleDC,
    CreateDIBSection, DIB_RGB_COLORS, DeleteDC, DeleteObject, HDC, HGDIOBJ, SelectObject,
};
use windows::Win32::Media::{timeBeginPeriod, timeEndPeriod};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Ole::{IDropTarget, RegisterDragDrop, RevokeDragDrop};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, GWL_EXSTYLE, GWLP_USERDATA, GetWindowLongPtrW, GetWindowRect,
    HWND_TOPMOST, IsWindowVisible, RegisterClassW, SW_HIDE, SW_SHOWNOACTIVATE, SWP_NOACTIVATE,
    SetWindowLongPtrW, SetWindowPos, ShowWindow, ULW_ALPHA, UpdateLayeredWindow, WM_ERASEBKGND,
    WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEWHEEL, WM_NCHITTEST, WNDCLASSW, WS_EX_LAYERED,
    WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TRANSPARENT, WS_POPUP,
};
use windows::core::w;

/// Something the overlay can report.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum OverlayEvent {
    /// The primary button went down, in screen coordinates.
    PointerDown { x: f32, y: f32 },
    /// The primary button came up, in screen coordinates.
    PointerUp { x: f32, y: f32 },
    /// The wheel was scrolled, in screen coordinates.
    Wheel { delta: f32, x: f32, y: f32 },
}

/// Why the overlay could not do something.
#[derive(Debug)]
pub enum OverlayError {
    Win32(windows::core::Error),
    BadPixels,
    /// The window has not been placed yet, so there is nowhere to draw.
    NotPlaced,
}

impl fmt::Display for OverlayError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OverlayError::Win32(error) => write!(f, "win32 error: {error}"),
            OverlayError::BadPixels => write!(f, "the pixel buffer does not match the window"),
            OverlayError::NotPlaced => {
                write!(f, "the overlay has not been placed on the screen yet")
            }
        }
    }
}

impl std::error::Error for OverlayError {}

impl From<windows::core::Error> for OverlayError {
    fn from(value: windows::core::Error) -> Self {
        OverlayError::Win32(value)
    }
}

/// The state the window procedure reaches through the window's user data.
struct OverlayState {
    input: RefCell<Vec<OverlayEvent>>,
}

const CLASS_NAME: windows::core::PCWSTR = w!("OpenTangerineOverlayWindow");

/// Where the window is made, before it has been placed.
///
/// Far enough away that no screen shows it.
const OFF_SCREEN_X: i32 = -32_000;
const OFF_SCREEN_Y: i32 = -32_000;

/// Registers the overlay window class, once per process.
fn register_class() -> windows::core::Result<()> {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    let mut result = Ok(());
    ONCE.call_once(|| {
        // SAFETY: the module handle is only read.
        let module = unsafe { GetModuleHandleW(None) }.unwrap_or_default();
        let class = WNDCLASSW {
            lpfnWndProc: Some(overlay_wndproc),
            hInstance: HINSTANCE(module.0),
            lpszClassName: CLASS_NAME,
            ..Default::default()
        };
        // SAFETY: the class name is a static wide string.
        let atom = unsafe { RegisterClassW(&class) };
        result = if atom == 0 {
            Err(windows::core::Error::from_thread())
        } else {
            Ok(())
        };
    });
    result
}

unsafe extern "system" fn overlay_wndproc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_LBUTTONDOWN => {
            let (x, y) = point_from_lparam(lparam);
            push_input(hwnd, OverlayEvent::PointerDown { x, y });
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            let (x, y) = point_from_lparam(lparam);
            push_input(hwnd, OverlayEvent::PointerUp { x, y });
            LRESULT(0)
        }
        WM_MOUSEWHEEL => {
            let delta = ((wparam.0 >> 16) as i16) as f32 / 120.0;
            let (x, y) = point_from_lparam(lparam);
            push_input(hwnd, OverlayEvent::Wheel { delta, x, y });
            LRESULT(0)
        }
        WM_NCHITTEST => {
            // The window is transparent to the mouse, so the system should
            // look for the window underneath.
            LRESULT(-1)
        }
        WM_ERASEBKGND => LRESULT(1),
        _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}

/// Reads the client coordinates out of an `LPARAM`.
fn point_from_lparam(lparam: LPARAM) -> (f32, f32) {
    let x = (lparam.0 & 0xFFFF) as i16 as f32;
    let y = ((lparam.0 >> 16) & 0xFFFF) as i16 as f32;
    (x, y)
}

/// Adds an input to the queue the application drains.
fn push_input(hwnd: HWND, input: OverlayEvent) {
    // SAFETY: the pointer was set when the window was created and the
    // `Overlay` that owns it outlives the window.
    unsafe {
        let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA);
        if pointer == 0 {
            return;
        }
        let state = &*(pointer as *const OverlayState);
        state.input.borrow_mut().push(input);
    }
}

/// The overlay window.
pub struct Overlay {
    hwnd: HWND,
    state: Rc<OverlayState>,
    memory_dc: HDC,
    dib: Option<DibSurface>,
    /// Whether the window has been placed at least once.
    placed: Cell<bool>,
    /// Whether the window is currently a drop target.
    drop_target_registered: Cell<bool>,
    /// Whether the application believes the window is on screen.
    visible: Cell<bool>,
}

/// A 32-bit premultiplied BGRA surface the pixels are drawn into.
struct DibSurface {
    bitmap: HGDIOBJ,
    previous: HGDIOBJ,
    bits: *mut c_void,
    width: u32,
    height: u32,
}

impl Overlay {
    /// Creates the overlay. It starts hidden, and off the screen.
    ///
    /// The window is made at the size it will be drawn at. That matters:
    /// `UpdateLayeredWindow` resizes a layered window to the size it is given
    /// and makes it visible, so a window that was never placed would put the
    /// wheel in the top left corner of the screen.
    pub fn new(width: u32, height: u32) -> windows::core::Result<Self> {
        register_class()?;
        // SAFETY: the module handle is only read.
        let module = unsafe { GetModuleHandleW(None) }.unwrap_or_default();
        // SAFETY: the class was registered above and the window has no parent.
        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_LAYERED | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW | WS_EX_TRANSPARENT,
                CLASS_NAME,
                w!("Open Tangerine"),
                WS_POPUP,
                // Far away from any screen, so the window can never be seen
                // before it has been placed.
                OFF_SCREEN_X,
                OFF_SCREEN_Y,
                width.max(1) as i32,
                height.max(1) as i32,
                None,
                None,
                Some(HINSTANCE(module.0)),
                None,
            )
        }?;

        let state = Rc::new(OverlayState {
            input: RefCell::new(Vec::new()),
        });
        // SAFETY: the pointer stays valid as long as the `Overlay` is alive,
        // and the window is destroyed with the `Overlay`.
        unsafe {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, Rc::as_ptr(&state) as isize);
        }

        // SAFETY: a memory device context for the source bitmap.
        let memory_dc = unsafe { CreateCompatibleDC(None) };

        Ok(Self {
            hwnd,
            state,
            memory_dc,
            dib: None,
            placed: Cell::new(false),
            drop_target_registered: Cell::new(false),
            visible: Cell::new(false),
        })
    }

    pub fn hwnd(&self) -> HWND {
        self.hwnd
    }

    /// Where the window actually is, in physical screen pixels.
    pub fn window_rect(&self) -> (i32, i32, i32, i32) {
        let mut rect = RECT::default();
        // SAFETY: the handle is ours and the rectangle is writable.
        unsafe {
            let _ = GetWindowRect(self.hwnd, &mut rect);
        }
        (rect.left, rect.top, rect.right, rect.bottom)
    }

    /// Moves the window to where the wheel belongs.
    ///
    /// The window is placed before it is ever drawn, so the wheel can never
    /// appear in the wrong place.
    pub fn place(&self, x: i32, y: i32, width: u32, height: u32) {
        // SAFETY: the window handle is ours.
        unsafe {
            let _ = SetWindowPos(
                self.hwnd,
                Some(HWND_TOPMOST),
                x,
                y,
                width as i32,
                height as i32,
                // The window is moved without being shown: it is shown once
                // there is something to show.
                SWP_NOACTIVATE,
            );
        }
        self.placed.set(true);
        crate::diagnostics::log(
            crate::diagnostics::Level::Info,
            "overlay",
            &format!(
                "the window was moved to ({x}, {y}), {width} by {height} pixels, and is now at {:?}",
                self.window_rect()
            ),
        );
    }

    /// Shows the window without giving it the focus.
    ///
    /// Showing a window that is already showing does nothing, so this is safe
    /// to call on every frame.
    pub fn show(&self) {
        if self.visible.get() {
            return;
        }
        self.visible.set(true);
        // SAFETY: the window handle is ours.
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_SHOWNOACTIVATE);
        }
        crate::diagnostics::log(
            crate::diagnostics::Level::Info,
            "overlay",
            &format!("the overlay was shown at {:?}", self.window_rect()),
        );
    }

    /// Hides the window.
    pub fn hide(&self) {
        if !self.visible.get() {
            return;
        }
        self.visible.set(false);
        // SAFETY: the window handle is ours.
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_HIDE);
        }
        crate::diagnostics::log(
            crate::diagnostics::Level::Info,
            "overlay",
            "the overlay was hidden",
        );
    }

    /// Whether the window is on screen.
    pub fn is_visible(&self) -> bool {
        // SAFETY: the window handle is ours.
        unsafe { IsWindowVisible(self.hwnd).as_bool() }
    }

    /// Turns mouse-through on or off.
    ///
    /// While a drag is running the window has to be see-through to the mouse,
    /// otherwise Explorer would lose the drag. Once the wheel is only on screen
    /// to be clicked, the window takes the mouse again.
    pub fn set_mouse_through(&self, through: bool) {
        // SAFETY: the window handle is ours.
        unsafe {
            let current = GetWindowLongPtrW(self.hwnd, GWL_EXSTYLE) as u32;
            let mut style = current;
            if through {
                style |= WS_EX_TRANSPARENT.0;
            } else {
                style &= !WS_EX_TRANSPARENT.0;
            }
            SetWindowLongPtrW(self.hwnd, GWL_EXSTYLE, style as isize);
        }
    }

    /// The inputs the window procedure collected since the last call.
    pub fn drain_input(&self) -> Vec<OverlayEvent> {
        let mut input = self.state.input.borrow_mut();
        std::mem::take(&mut *input)
    }

    /// Asks Windows for a finer timer while the wheel is on screen.
    ///
    /// The wheel animates, and a coarse timer would make the motion uneven.
    /// The request is undone as soon as the wheel is gone, so the rest of the
    /// system is left as it was found.
    pub fn set_fine_timer(&self, fine: bool) {
        // SAFETY: the call only changes the timer resolution of the system.
        unsafe {
            if fine {
                timeBeginPeriod(1);
            } else {
                timeEndPeriod(1);
            }
        }
    }

    /// Makes the window a target for the drag that passes over it.
    ///
    /// This is how the file list of a drag is found: Explorer's drag keeps its
    /// payload to itself, and only a drop target is told what it carries.
    /// The registration is undone again as soon as the interaction is over.
    ///
    /// Calling this again while the window is already a drop target does
    /// nothing: registering twice would fail and leave the window without a
    /// drop target at all.
    pub fn register_drop_target(&self, target: &IDropTarget) -> Result<(), OverlayError> {
        if self.drop_target_registered.get() {
            return Ok(());
        }
        // SAFETY: the window is ours and the target outlives the registration.
        unsafe { RegisterDragDrop(self.hwnd, target) }?;
        self.drop_target_registered.set(true);
        crate::diagnostics::log(
            crate::diagnostics::Level::Info,
            "overlay",
            "the overlay is now a drop target",
        );
        Ok(())
    }

    /// Stops being a drop target.
    pub fn revoke_drop_target(&self) {
        if !self.drop_target_registered.get() {
            return;
        }
        self.drop_target_registered.set(false);
        // SAFETY: the window is ours.
        let _ = unsafe { RevokeDragDrop(self.hwnd) };
        crate::diagnostics::log(
            crate::diagnostics::Level::Info,
            "overlay",
            "the overlay is no longer a drop target",
        );
    }

    /// Puts the pixels on the screen.
    ///
    /// `pixels` holds one 32-bit premultiplied BGRA value per pixel, in rows
    /// from the top.
    pub fn present(&mut self, pixels: &[u32], width: u32, height: u32) -> Result<(), OverlayError> {
        if width == 0 || height == 0 {
            return Err(OverlayError::BadPixels);
        }
        if pixels.len() < (width as usize) * (height as usize) {
            return Err(OverlayError::BadPixels);
        }
        if !self.placed.get() {
            // Drawing into a window that has never been placed would put the
            // wheel in the top left corner of the screen.
            return Err(OverlayError::NotPlaced);
        }
        self.ensure_surface(width, height)?;
        let surface = self.dib.as_mut().expect("surface was just created");

        // SAFETY: the surface was allocated for exactly this many pixels.
        unsafe {
            std::ptr::copy_nonoverlapping(
                pixels.as_ptr(),
                surface.bits.cast::<u32>(),
                (width as usize) * (height as usize),
            );
        }

        let size = SIZE {
            cx: width as i32,
            cy: height as i32,
        };
        let source = POINT { x: 0, y: 0 };
        let blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            BlendFlags: 0,
            SourceConstantAlpha: 255,
            AlphaFormat: AC_SRC_ALPHA as u8,
        };
        // SAFETY: the source device context holds the selected bitmap and the
        // window is a layered window of exactly this size.
        //
        // The destination device context is left out on purpose. When it is
        // given, the point below is the place the window is moved to, and a
        // point of zero means the top left corner of the screen. Without it
        // the window keeps the place `place` gave it.
        unsafe {
            UpdateLayeredWindow(
                self.hwnd,
                None,
                None,
                Some(&size),
                Some(self.memory_dc),
                Some(&source),
                COLORREF(0),
                Some(&blend),
                ULW_ALPHA,
            )?;
        }
        Ok(())
    }

    fn ensure_surface(&mut self, width: u32, height: u32) -> Result<(), OverlayError> {
        if let Some(surface) = &self.dib
            && surface.width == width
            && surface.height == height
        {
            return Ok(());
        }
        // A surface of the wrong size is thrown away and made again.
        if let Some(old) = self.dib.take() {
            // SAFETY: the bitmap was selected into our own device context.
            unsafe {
                SelectObject(self.memory_dc, old.previous);
                let _ = DeleteObject(old.bitmap);
            }
        }

        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width as i32,
                // A negative height puts the first row at the top.
                biHeight: -(height as i32),
                biPlanes: 1,
                biBitCount: 32,
                biCompression: 0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits: *mut c_void = std::ptr::null_mut();
        // SAFETY: the bitmap information is filled in above.
        let bitmap = unsafe {
            CreateDIBSection(
                Some(self.memory_dc),
                &info,
                DIB_RGB_COLORS,
                &mut bits,
                None,
                0,
            )?
        };
        // SAFETY: both device contexts are ours.
        let previous = unsafe { SelectObject(self.memory_dc, HGDIOBJ(bitmap.0)) };
        self.dib = Some(DibSurface {
            bitmap: HGDIOBJ(bitmap.0),
            previous,
            bits,
            width,
            height,
        });
        Ok(())
    }
}

impl Drop for Overlay {
    fn drop(&mut self) {
        if let Some(surface) = self.dib.take() {
            // SAFETY: the bitmap was selected into our own device context.
            unsafe {
                SelectObject(self.memory_dc, surface.previous);
                let _ = DeleteObject(surface.bitmap);
            }
        }
        // SAFETY: both device contexts were created by us.
        unsafe {
            let _ = DeleteDC(self.memory_dc);
        }
    }
}
