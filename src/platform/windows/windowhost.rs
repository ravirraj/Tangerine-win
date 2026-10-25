//! The plumbing every tool window shares.
//!
//! The windows themselves keep their own geometry, their own card and
//! their own answers — what is shared is only the things that are the
//! same shape wherever a window is made: how much the display
//! stretches the pixels, how much a window is stretched, the words in
//! the wide letters the system reads, the frame the windows wear, and
//! the face where the times are said. The tests' way of taking a
//! screenshot is here too, so every window is checked the same way.

use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Gdi::{
    GetDC, GetDeviceCaps, LOGPIXELSY, ReleaseDC,
};
use windows::Win32::UI::WindowsAndMessaging::{
    WINDOW_STYLE, WS_MAXIMIZEBOX, WS_OVERLAPPEDWINDOW, WS_THICKFRAME,
};

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
