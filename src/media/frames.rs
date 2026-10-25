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
}
