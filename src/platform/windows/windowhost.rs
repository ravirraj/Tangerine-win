//! The plumbing every tool window shares.
//!
//! The windows themselves keep their own geometry, their own card and
//! their own answers — what is shared is only the things that are the
//! same shape wherever a window is made: how much the display
//! stretches the pixels, how much a window is stretched, the words in
//! the wide letters the system reads, the frame the windows wear, and
//! the face where the times are said. The tests' way of taking a
//! screenshot is here too, so every window is checked the same way.

use std::sync::atomic::{AtomicIsize, Ordering};
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    GetDC, GetDeviceCaps, LOGPIXELSY, ReleaseDC,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, LoadImageW, RegisterClassW, SendMessageW,
    HICON, ICON_BIG, ICON_SMALL, IMAGE_ICON, LR_DEFAULTCOLOR, LR_LOADFROMFILE, LR_SHARED,
    WINDOW_STYLE, WM_SETICON, WNDCLASSW, WS_MAXIMIZEBOX, WS_OVERLAPPEDWINDOW, WS_POPUP,
    WS_THICKFRAME,
};

static APP_ICON_BIG: AtomicIsize = AtomicIsize::new(0);
static APP_ICON_SMALL: AtomicIsize = AtomicIsize::new(0);

pub(crate) fn get_app_icon() -> HICON {
    let val = APP_ICON_BIG.load(Ordering::Relaxed);
    if val != 0 {
        return HICON(val as *mut _);
    }
    let icon = load_icon_of_size(32);
    APP_ICON_BIG.store(icon.0 as isize, Ordering::Relaxed);
    icon
}

pub(crate) fn get_app_icon_small() -> HICON {
    let val = APP_ICON_SMALL.load(Ordering::Relaxed);
    if val != 0 {
        return HICON(val as *mut _);
    }
    let icon = load_icon_of_size(16);
    APP_ICON_SMALL.store(icon.0 as isize, Ordering::Relaxed);
    icon
}

fn load_icon_of_size(size: i32) -> HICON {
    unsafe {
        let hinstance = windows::Win32::System::LibraryLoader::GetModuleHandleW(None).unwrap_or_default();
        if let Ok(handle) = LoadImageW(
            Some(hinstance.into()),
            PCWSTR(1 as _),
            IMAGE_ICON,
            size,
            size,
            LR_DEFAULTCOLOR | LR_SHARED,
        ) {
            if !handle.0.is_null() {
                return HICON(handle.0);
            }
        }
        if let Ok(handle) = LoadImageW(
            None,
            w!("assets\\icon.ico"),
            IMAGE_ICON,
            size,
            size,
            LR_LOADFROMFILE | LR_SHARED,
        ) {
            if !handle.0.is_null() {
                return HICON(handle.0);
            }
        }
        HICON(std::ptr::null_mut())
    }
}

thread_local! {
    static ROOT_OWNER: std::cell::Cell<isize> = const { std::cell::Cell::new(0) };
}

unsafe extern "system" fn owner_wnd_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

pub(crate) fn root_owner_window() -> HWND {
    ROOT_OWNER.with(|cell| {
        let val = cell.get();
        if val != 0 {
            let hwnd = HWND(val as *mut _);
            unsafe {
                if windows::Win32::UI::WindowsAndMessaging::IsWindow(Some(hwnd)).as_bool() {
                    return hwnd;
                }
            }
        }
        unsafe {
            let hinstance = windows::Win32::System::LibraryLoader::GetModuleHandleW(None).unwrap_or_default();
            let class_name = w!("OpenTangerineOwner");
            let wc = WNDCLASSW {
                lpfnWndProc: Some(owner_wnd_proc),
                hInstance: hinstance.into(),
                lpszClassName: class_name,
                ..Default::default()
            };
            let _ = RegisterClassW(&wc);
            let hwnd = CreateWindowExW(
                Default::default(),
                class_name,
                w!("Open Tangerine"),
                WS_POPUP,
                0,
                0,
                0,
                0,
                None,
                None,
                Some(hinstance.into()),
                None,
            )
            .unwrap_or(HWND(std::ptr::null_mut()));

            cell.set(hwnd.0 as isize);
            hwnd
        }
    })
}

pub(crate) fn hide_from_taskbar(window: HWND) {
    if window.0.is_null() {
        return;
    }
    unsafe {
        use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER};
        use windows::Win32::UI::Shell::{ITaskbarList, TaskbarList};

        if let Ok(taskbar) = CoCreateInstance::<_, ITaskbarList>(&TaskbarList, None, CLSCTX_INPROC_SERVER) {
            let _ = taskbar.HrInit();
            let _ = taskbar.DeleteTab(window);
        }
    }
}

pub(crate) fn prepare_tool_window(window: HWND) {
    if window.0.is_null() {
        return;
    }
    unsafe {
        let icon_big = get_app_icon();
        let icon_sm = get_app_icon_small();
        if !icon_big.0.is_null() {
            let _ = SendMessageW(
                window,
                WM_SETICON,
                Some(WPARAM(ICON_BIG as _)),
                Some(LPARAM(icon_big.0 as _)),
            );
        }
        if !icon_sm.0.is_null() {
            let _ = SendMessageW(
                window,
                WM_SETICON,
                Some(WPARAM(ICON_SMALL as _)),
                Some(LPARAM(icon_sm.0 as _)),
            );
        }
    }
    hide_from_taskbar(window);
}

/// How much the display stretches a named window: the dots it paints
/// with for every 96 the design asks, taken from the window's own DC.
pub(crate) fn scale_factor(window: HWND) -> f32 {
    unsafe {
        let context = GetDC(Some(window));
        let dots = GetDeviceCaps(Some(context), LOGPIXELSY);
        let _ = ReleaseDC(Some(window), context);
        dots as f32 / 96.0
    }
}

/// How much the display stretches everything, before any window is made
/// to ask it of.
pub(crate) fn screen_scale() -> f32 {
    unsafe {
        let context = GetDC(None);
        let dots = GetDeviceCaps(Some(context), LOGPIXELSY);
        let _ = ReleaseDC(None, context);
        dots as f32 / 96.0
    }
}

/// Words in the wide letters the system reads.
pub(crate) fn to_wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain([0]).collect()
}

/// The frame every window wears: an ordinary window, but not one the
/// person may stretch — the card is sized by what it holds.
pub(crate) fn fixed_style() -> WINDOW_STYLE {
    WS_OVERLAPPEDWINDOW & !(WS_THICKFRAME | WS_MAXIMIZEBOX)
}

/// A share of the whole, from a moment of it.
pub(crate) fn fraction(moment: f64, duration: f64) -> f32 {
    if duration > 0.0 {
        (moment / duration).clamp(0.0, 1.0) as f32
    } else {
        0.0
    }
}

/// How a moment is said: minutes, seconds and the hundredths, in the
/// face where every digit takes the same room — the way the reference
/// says its times.
pub(crate) fn timecode(moment: f64) -> String {
    let moment = moment.max(0.0);
    let minutes = (moment / 60.0) as u64;
    let seconds = moment - (minutes as f64) * 60.0;
    format!("{minutes}:{seconds:05.2}")
}

/// The outer window dimensions needed to contain a client area of `client_w` x `client_h`.
pub(crate) fn outer_size_for(window: HWND, client_w: i32, client_h: i32) -> (i32, i32) {
    let mut rect = windows::Win32::Foundation::RECT {
        left: 0,
        top: 0,
        right: client_w,
        bottom: client_h,
    };
    let style = fixed_style();
    let dpi = (scale_factor(window) * 96.0).round() as u32;
    let _ = unsafe {
        windows::Win32::UI::HiDpi::AdjustWindowRectExForDpi(
            &mut rect,
            style,
            false,
            windows::Win32::UI::WindowsAndMessaging::WINDOW_EX_STYLE::default(),
            if dpi == 0 { 96 } else { dpi },
        )
    };
    (rect.right - rect.left, rect.bottom - rect.top)
}

/// Blits a BGRA8 pixel buffer directly to the window's device context using top-down DIB.
pub(crate) fn blit_card_buffer(window: HWND, buffer: &[crate::ui::wheel::Bgra8], width: u32, height: u32) {
    if buffer.is_empty() || width == 0 || height == 0 {
        return;
    }
    unsafe {
        let dc = windows::Win32::Graphics::Gdi::GetDC(Some(window));
        let mut info = windows::Win32::Graphics::Gdi::BITMAPINFO::default();
        info.bmiHeader.biSize =
            std::mem::size_of::<windows::Win32::Graphics::Gdi::BITMAPINFOHEADER>() as u32;
        info.bmiHeader.biWidth = width as i32;
        info.bmiHeader.biHeight = -(height as i32); // Negative for top-down
        info.bmiHeader.biPlanes = 1;
        info.bmiHeader.biBitCount = 32;
        info.bmiHeader.biCompression = windows::Win32::Graphics::Gdi::BI_RGB.0 as u32;
        windows::Win32::Graphics::Gdi::SetDIBitsToDevice(
            dc,
            0,
            0,
            width,
            height,
            0,
            0,
            0,
            height,
            buffer.as_ptr() as *const core::ffi::c_void,
            &info,
            windows::Win32::Graphics::Gdi::DIB_RGB_COLORS,
        );
        let _ = windows::Win32::Graphics::Gdi::ReleaseDC(Some(window), dc);
    }
}

/// Creates and registers a standard Win32 layered card window.
pub(crate) fn create_layered_window(
    class_name: &str,
    title: &str,
    wnd_proc: windows::Win32::UI::WindowsAndMessaging::WNDPROC,
    client_w: i32,
    client_h: i32,
) -> Result<HWND, String> {
    unsafe {
        let wide_class = to_wide(class_name);
        let hinstance = windows::Win32::System::LibraryLoader::GetModuleHandleW(windows::core::PCWSTR::null())
            .map_err(|e| e.to_string())?;
        let wc = windows::Win32::UI::WindowsAndMessaging::WNDCLASSW {
            lpfnWndProc: wnd_proc,
            hInstance: hinstance.into(),
            lpszClassName: windows::core::PCWSTR(wide_class.as_ptr()),
            hCursor: windows::Win32::UI::WindowsAndMessaging::LoadCursorW(
                None,
                windows::Win32::UI::WindowsAndMessaging::IDC_ARROW,
            )
            .unwrap_or_default(),
            hIcon: get_app_icon(),
            hbrBackground: windows::Win32::Graphics::Gdi::HBRUSH(std::ptr::null_mut()),
            ..Default::default()
        };
        let _ = windows::Win32::UI::WindowsAndMessaging::RegisterClassW(&wc);

        let (outer_w, outer_h) = outer_size_for(HWND(std::ptr::null_mut()), client_w, client_h);
        let wide_title = to_wide(title);
        let owner = root_owner_window();
        let window = windows::Win32::UI::WindowsAndMessaging::CreateWindowExW(
            windows::Win32::UI::WindowsAndMessaging::WS_EX_LAYERED,
            windows::core::PCWSTR(wide_class.as_ptr()),
            windows::core::PCWSTR(wide_title.as_ptr()),
            fixed_style(),
            windows::Win32::UI::WindowsAndMessaging::CW_USEDEFAULT,
            windows::Win32::UI::WindowsAndMessaging::CW_USEDEFAULT,
            outer_w,
            outer_h,
            if owner.0.is_null() { None } else { Some(owner) },
            None,
            Some(hinstance.into()),
            None,
        )
        .map_err(|e| e.to_string())?;

        prepare_tool_window(window);

        Ok(window)
    }
}

/// Saves a BGRA8 pixel buffer to a PNG file, creating parent directories if needed.
#[allow(dead_code)]
pub(crate) fn save_buffer_as_png(
    buffer: &[crate::ui::wheel::Bgra8],
    width: u32,
    height: u32,
    path: &std::path::Path,
) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let mut rgba = image::RgbaImage::new(width, height);
    for y in 0..height {
        for x in 0..width {
            let pixel = buffer[(y * width + x) as usize];
            let (a, r, g, b) = pixel.parts();
            rgba.put_pixel(x, y, image::Rgba([r, g, b, a]));
        }
    }
    rgba.save(path).map_err(|e| e.to_string())
}

#[cfg(test)]
pub(crate) mod screenshot {
    //! The ways a test gives proof: a small video made of a sure red,
    //! and the window put on paper as a picture.

    use std::ffi::OsString;
    use std::path::PathBuf;

    use windows::Win32::Foundation::{HWND, RECT};
    use windows::Win32::Graphics::Gdi::{
        CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDIBits,
        GetWindowDC, ReleaseDC, SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB,
        DIB_RGB_COLORS,
    };
    use windows::Win32::Storage::Xps::{PrintWindow, PRINT_WINDOW_FLAGS};
    use windows::Win32::UI::WindowsAndMessaging::GetWindowRect;

    use crate::media::runner::{ProcessRunner, SystemRunner};

    /// Makes a small test video the way the tests do, so the capture
    /// has a real frame and a real length to show.
    pub(crate) fn make_test_video(folder: &std::path::Path) -> PathBuf {
        let ffmpeg = std::path::Path::new("vendor")
            .join("ffmpeg")
            .join("ffmpeg.exe");
        assert!(ffmpeg.is_file(), "the conversion program is not there");
        let raw = folder.join("capture.raw");
        std::fs::write(&raw, vec![255u8, 0, 0, 255].repeat(160 * 120 * 30))
            .expect("the pixels could be written");
        let video = folder.join("capture.mp4");
        let arguments: Vec<OsString> = [
            "-hide_banner",
            "-nostdin",
            "-y",
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgba",
            "-s",
            "160x120",
            "-r",
            "10",
            "-i",
        ]
        .iter()
        .map(|word| OsString::from(*word))
        .chain([
            raw.as_os_str().to_os_string(),
            OsString::from("-c:v"),
            OsString::from("mpeg4"),
            OsString::from("-pix_fmt"),
            OsString::from("yuv420p"),
            OsString::from("-f"),
            OsString::from("mov"),
            video.as_os_str().to_os_string(),
        ])
        .collect();
        let run = SystemRunner.run(&ffmpeg, &arguments, None).expect("the video was made");
        assert!(run.succeeded(), "the capture video was not made: {}", run.stderr);
        video
    }

    /// Puts what the window looks like into a picture, so the design can
    /// be checked without someone sitting at the screen.
    pub(crate) fn capture(window: HWND, path: &std::path::Path) {
        unsafe {
            let mut rect = RECT::default();
            let _ = GetWindowRect(window, &mut rect);
            let width = rect.right - rect.left;
            let height = rect.bottom - rect.top;
            let screen = GetWindowDC(Some(window));
            let memory = CreateCompatibleDC(Some(screen));
            let bitmap = CreateCompatibleBitmap(screen, width, height);
            let old = SelectObject(memory, bitmap.into());
            // PW_RENDERFULLCONTENT: the window is layered and rounded;
            // plain PrintWindow would miss what the system composes.
            const PW_RENDERFULLCONTENT: PRINT_WINDOW_FLAGS = PRINT_WINDOW_FLAGS(2);
            let _ = PrintWindow(window, memory, PW_RENDERFULLCONTENT);
            let mut info = BITMAPINFO::default();
            info.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
            info.bmiHeader.biWidth = width;
            info.bmiHeader.biHeight = -height;
            info.bmiHeader.biPlanes = 1;
            info.bmiHeader.biBitCount = 32;
            info.bmiHeader.biCompression = BI_RGB.0 as u32;
            let mut pixels = vec![0u8; (width.max(0) * height.max(0)) as usize * 4];
            let _ = GetDIBits(
                memory,
                bitmap,
                0,
                height as u32,
                Some(pixels.as_mut_ptr() as *mut core::ffi::c_void),
                &mut info,
                DIB_RGB_COLORS,
            );
            let _ = SelectObject(memory, old);
            let _ = DeleteObject(bitmap.into());
            let _ = DeleteDC(memory);
            let _ = ReleaseDC(Some(window), screen);
            // The bytes come back blue-first; the picture wants red-first.
            for pixel in pixels.chunks_exact_mut(4) {
                pixel.swap(0, 2);
            }
            image::save_buffer(
                path,
                &pixels,
                width as u32,
                height as u32,
                image::ExtendedColorType::Rgba8,
            )
            .expect("the capture was written");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_screen_says_how_much_it_stretches() {
        // A sane display is at least the design's own stretch, and no
        // display asks more than a few of them.
        let stretch = screen_scale();
        assert!(stretch >= 1.0, "the screen said {stretch}");
        assert!(stretch < 3.0, "the screen said {stretch}");
    }

    #[test]
    fn a_moment_is_said_in_minutes_and_hundredths() {
        assert_eq!(timecode(0.0), "0:00.00");
        assert_eq!(timecode(83.456), "1:23.46");
        assert_eq!(timecode(4.0), "0:04.00");
        assert_eq!(timecode(600.0), "10:00.00");
    }

    #[test]
    fn a_share_of_the_whole_is_kept_between_the_ends() {
        assert_eq!(fraction(5.0, 10.0), 0.5);
        assert_eq!(fraction(-1.0, 10.0), 0.0);
        assert_eq!(fraction(20.0, 10.0), 1.0);
        // No duration known: everything stands at the beginning.
        assert_eq!(fraction(5.0, 0.0), 0.0);
    }
}
