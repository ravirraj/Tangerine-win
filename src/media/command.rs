//! Building the argument lists.
//!
//! Every argument is a word of its own. Nothing is put together into a
//! single string for a shell to take apart again, so a path that holds a
//! space, a percent sign or a quote is just a path.

use std::ffi::OsString;
use std::path::Path;

use crate::media::manifest::{Alpha, FormatSupport};

/// The filters the composite graph needs.
pub const COMPOSITE_FILTERS: &[&str] = &["split", "drawbox", "overlay"];

/// The colour a transparent picture is painted onto.
///
/// A picture that cannot hold a transparent area has to be given one, and
/// it is given the same one every time.
pub const BACKGROUND: &str = "white";

/// The graph that paints a transparent picture onto the background.
///
/// The picture is split in two: one half is the background, painted over
/// completely, and the other is laid on top of it. The `replace` is what
/// makes the background opaque; without it the background would keep the
/// transparency of the original and the encoder would drop it, which is
/// the mistake this graph exists to avoid. The overlay is kept in RGB,
/// because a conversion to YUV before the transparency has been dealt with
/// would throw it away.
pub fn composite_graph() -> String {
    format!(
        "[0:v]format=rgba,split=2[fg][bg];\
         [bg]drawbox=color={BACKGROUND}:t=fill:replace=1[bgs];\
         [bgs][fg]overlay=shortest=1:format=rgb"
    )
}

/// Whether a muxer needs a seekable output file (cannot write to pipe).
///
/// Some muxers (mov, matroska, avi, asf, ipod) need to seek back to write
/// the header, so they cannot write to a pipe. Image muxers (image2pipe,
/// gif, webp) and audio muxers (adts, flac, mp3, ogg, wav, aiff) can write
/// to a pipe.
pub fn muxer_needs_file(muxer: &str) -> bool {
    matches!(
        muxer,
        "mov" | "matroska" | "avi" | "asf" | "mp4" | "mkv" | "ipod"
    )
}

/// Whether an encoder writes sound, not pictures.
///
/// The encoder says what a conversion makes, not the container: the same
/// container holds a film and a sound (asf holds wmv and wma), and it is
/// the encoder that tells the two apart.
pub fn is_audio_encoder(encoder: &str) -> bool {
    matches!(
        encoder,
        "pcm_s16le" | "libmp3lame" | "libvorbis" | "libopus" | "wmav2" | "aac" | "flac"
    )
}

/// The arguments that convert one file into another.
///
/// The demuxer is always named, so that the program never has to guess
/// what a file is from its name — a name that holds a percent sign is not
/// a picture sequence to it. By default the result is written to the
/// program's standard output; when `output_path` is given, the result is
/// written to that file instead (needed for muxers that require seekable
/// output). When `video_filter` is given, it is inserted as a `-vf`
/// filter before the encoder (used by tools like crop, trim, resize,
/// rotate).
pub fn convert(
    input: &Path,
    source: &FormatSupport,
    target: &FormatSupport,
    output_path: Option<&Path>,
    video_filter: Option<&str>,
) -> Vec<OsString> {
    let mut args = vec![
        OsString::from("-hide_banner"),
        // The program must not wait for anything typed at it.
        OsString::from("-nostdin"),
        // The result goes where the caller says.
        OsString::from("-y"),
        OsString::from("-f"),
        OsString::from(&source.read.demuxer),
        OsString::from("-i"),
        input.as_os_str().to_os_string(),
    ];

    if let Some(filter) = video_filter {
        args.push(OsString::from("-vf"));
        args.push(OsString::from(filter));
    }

    let audio = is_audio_encoder(&target.write.encoder);

    if target.alpha == Alpha::Matte && !audio {
        args.push(OsString::from("-filter_complex"));
        args.push(OsString::from(composite_graph()));
    }

    if audio {
        // A sound has no frames to limit and no picture to encode.
        args.push(OsString::from("-c:a"));
        args.push(OsString::from(&target.write.encoder));
    } else {
        // Video or image output: one frame, encode video.
        args.push(OsString::from("-frames:v"));
        args.push(OsString::from("1"));
        args.push(OsString::from("-c:v"));
        args.push(OsString::from(&target.write.encoder));
    }

    args.push(OsString::from("-f"));
    args.push(OsString::from(&target.write.muxer));

    match output_path {
        Some(path) => args.push(path.as_os_str().to_os_string()),
        None => args.push(OsString::from("-")),
    }
    args
}

/// The arguments that count the frames of a file.
///
/// The demuxer is named, for the same reason it is named in a conversion.
/// The count is of the frames that were really decoded, which is the only
/// count that says whether a file holds more than one picture.
pub fn count_frames(input: &Path, demuxer: &str) -> Vec<OsString> {
    vec![
        OsString::from("-v"),
        OsString::from("error"),
        OsString::from("-f"),
        OsString::from(demuxer),
        OsString::from("-count_frames"),
        OsString::from("-select_streams"),
        OsString::from("v:0"),
        OsString::from("-show_entries"),
        OsString::from("stream=nb_read_frames"),
        OsString::from("-of"),
        OsString::from("csv=p=0"),
        input.as_os_str().to_os_string(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media::manifest::Manifest;

    fn words(args: &[OsString]) -> Vec<String> {
        args.iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn the_conversion_names_the_demuxer_and_the_muxer() {
        let manifest = Manifest::get();
        let source = manifest.format("png").expect("png is in the manifest");
        let target = manifest.format("jpeg").expect("jpeg is in the manifest");
        let args = words(&convert(Path::new("photo.png"), source, target, None, None));
        let demuxer = args
            .iter()
            .position(|word| word == "-f")
            .expect("a format is named");
        assert_eq!(args[demuxer + 1], "png_pipe");
        let muxer = args
            .iter()
            .rposition(|word| word == "-f")
            .expect("a format is named");
        assert_eq!(args[muxer + 1], "image2pipe");
        assert_eq!(args.last().map(String::as_str), Some("-"));
    }

    #[test]
    fn a_format_that_cannot_hold_transparency_gets_the_graph() {
        let manifest = Manifest::get();
        let source = manifest.format("png").expect("png is in the manifest");
        let target = manifest.format("jpeg").expect("jpeg is in the manifest");
        let args = words(&convert(Path::new("photo.png"), source, target, None, None));
        let graph = args
            .iter()
            .position(|word| word == "-filter_complex")
            .map(|at| args[at + 1].clone());
        let graph = graph.expect("there is a graph");
        assert!(graph.contains("drawbox"), "the graph is {graph}");
        assert!(graph.contains("replace=1"), "the graph is {graph}");
        assert!(graph.contains("format=rgb"), "the graph is {graph}");
    }

    #[test]
    fn a_format_that_keeps_transparency_gets_no_graph() {
        let manifest = Manifest::get();
        let source = manifest.format("png").expect("png is in the manifest");
        let target = manifest.format("png").expect("png is in the manifest");
        let args = words(&convert(Path::new("photo.png"), source, target, None, None));
        assert!(!args.iter().any(|word| word == "-filter_complex"));
    }

    #[test]
    fn a_path_is_one_word_however_awkward_it_is() {
        let manifest = Manifest::get();
        let source = manifest.format("png").expect("png is in the manifest");
        let target = manifest.format("png").expect("png is in the manifest");
        let path = Path::new("C:\\odd folder\\100% photo.png");
        let args = convert(path, source, target, None, None);
        let input = args
            .iter()
            .position(|word| word == "-i")
            .expect("an input is named");
        assert_eq!(args[input + 1], path.as_os_str());
    }

    #[test]
    fn the_frame_count_names_the_demuxer() {
        let args = words(&count_frames(Path::new("photo.gif"), "gif"));
        let demuxer = args
            .iter()
            .position(|word| word == "-f")
            .expect("a format is named");
        assert_eq!(args[demuxer + 1], "gif");
        assert!(args.iter().any(|word| word == "-count_frames"));
        assert!(args.iter().any(|word| word == "stream=nb_read_frames"));
        assert_eq!(args.last().map(String::as_str), Some("photo.gif"));
    }
}
