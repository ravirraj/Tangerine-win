//! The moments of a moving picture.
//!
//! How long a video is, and what it looks like at any moment of it. Both
//! are asked of the conversion program itself: the length from the
//! probe, the look of a moment from a fast seek that decodes one frame
//! and hands it back as a picture.

use std::ffi::OsString;
use std::path::Path;

use crate::media::locator::Tools;
use crate::media::runner::{ProcessRunner, SystemRunner};

/// How long a video is, in seconds, if the probe can say.
pub fn duration_of(tools: &Tools, input: &Path) -> Option<f64> {
    let arguments: Vec<OsString> = [
        "-v",
        "error",
        "-show_entries",
        "format=duration",
        "-of",
        "default=noprint_wrappers=1:nokey=1",
    ]
    .iter()
    .map(|word| OsString::from(*word))
    .chain([input.as_os_str().to_os_string()])
    .collect();
    let run = SystemRunner
        .run(&tools.ffprobe, &arguments, None)
        .ok()?;
    if !run.succeeded() {
        return None;
    }
    parse_duration(&String::from_utf8_lossy(&run.stdout))
}

/// What the probe says a length is, if it says one.
fn parse_duration(said: &str) -> Option<f64> {
    said.trim()
        .parse::<f64>()
        .ok()
        .filter(|seconds| seconds.is_finite() && *seconds > 0.0)
}

/// What the video looks like at a moment of it.
///
/// The seek is put before the input, which makes it fast: the program
/// jumps near the moment and decodes what it finds, instead of decoding
/// everything up to it.
pub fn frame_at(tools: &Tools, input: &Path, at_seconds: f64) -> Option<image::DynamicImage> {
    let extension = input
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or("")
        .to_lowercase();
    let demuxer = crate::media::manifest::Manifest::get()
        .format(&extension)?
        .read
        .demuxer
        .clone();
    let at = format!("{:.3}", at_seconds.max(0.0));
    let mut arguments: Vec<OsString> = [
        "-hide_banner",
        "-nostdin",
        "-y",
        "-ss",
        at.as_str(),
        "-f",
        demuxer.as_str(),
        "-i",
    ]
    .iter()
    .map(|word| OsString::from(*word))
    .collect();
    arguments.push(input.as_os_str().to_os_string());
    for word in [
        "-frames:v",
        "1",
        "-f",
        // The image pipe muxer, the one the build carries: the frame
        // goes to the standard output as one picture.
        "image2pipe",
        "-c:v",
        "png",
        "pipe:1",
    ] {
        arguments.push(OsString::from(word));
    }
    let run = SystemRunner
        .run(&tools.ffmpeg, &arguments, None)
        .ok()?;
    if !run.succeeded() {
        return None;
    }
    image::load_from_memory(&run.stdout).ok()
}

/// One narrow pillar of a picture: the middle of it, squeezed to the
/// tile a filmstrip wants. The strip is made of these standing side by
/// side, the way the reference runs its range over the video's own
/// moments.
pub fn pillar_of(
    frame: &image::DynamicImage,
    tile_width: u32,
    tile_height: u32,
) -> image::RgbaImage {
    // The middle slice that shares the tile's own shape, wide enough to
    // fill it: taller than the frame's own opening ratio.
    let slice_width = ((tile_width as f64 / tile_height as f64) * frame.height() as f64)
        .ceil() as u32;
    let slice_width = slice_width.max(1).min(frame.width());
    let slice = frame.crop_imm((frame.width() - slice_width) / 2, 0, slice_width, frame.height());
    slice
        .resize_exact(tile_width, tile_height, image::imageops::FilterType::Triangle)
        .to_rgba8()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_duration_is_read_from_what_the_probe_says() {
        assert_eq!(parse_duration("83.456\n"), Some(83.456));
        assert_eq!(parse_duration("12"), Some(12.0));
        assert_eq!(parse_duration(""), None);
        assert_eq!(parse_duration("N/A\n"), None);
        assert_eq!(parse_duration("0"), None);
        assert_eq!(parse_duration("-4.0"), None);
    }

    #[test]
    fn a_pillar_is_the_middle_of_the_frame_at_the_tiles_shape() {
        let frame = image::RgbImage::from_fn(100, 50, |x, _| {
            image::Rgb([(x * 2) as u8, 0, 0])
        })
        .into();
        let pillar = pillar_of(&frame, 16, 64);
        assert_eq!(pillar.dimensions(), (16, 64));
        // The middle is kept: the middle of the tile looks at the
        // middle of the frame's own middle.
        let last = *pillar.get_pixel(8, 32);
        let middle = frame.to_rgb8().get_pixel(50, 25).0[0];
        // The squeeze is a soft one: the middle of the tile is the
        // middle of the frame, within a degree of it.
        assert!((last.0[0] as i32 - middle as i32).abs() <= 3, "{}", last.0[0]);
    }

    #[test]
    fn a_narrow_frame_is_pillarised_without_panicking() {
        let frame = image::RgbImage::from_fn(10, 90, |_, _| image::Rgb([0, 255, 0])).into();
        let pillar = pillar_of(&frame, 16, 64);
        assert_eq!(pillar.dimensions(), (16, 64));
    }
}
