//! Tool operations: compress, crop, trim, merge, rotate, resize, metadata.
//!
//! These are handled with FFmpeg (image/video/audio), without needing a
//! separate heavyweight engine.
//!
//! The work happens on a worker, never on the thread that draws.

use std::ffi::OsString;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use crate::diagnostics::{self, Level};
use crate::media::command::convert as build_convert_args;
use crate::media::command::muxer_needs_file;
use crate::media::locator::Tools as FfmpegTools;
use crate::media::manifest::Manifest;
use crate::media::publish::{PublishError, Publisher};
use crate::media::runner::{ProcessRunner, SystemRunner};
use crate::radial::execution::{Failure, Handler, Outcome, Request};
use crate::radial::registry::{operation_by_id, Output};
use crate::radial::selection::Selection;

/// Why a tool operation did not happen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolError {
    /// The tool is not supported for this kind of file.
    UnsupportedKind,
    /// The tool is not implemented yet.
    NotImplemented,
    /// The tool failed to run.
    ToolFailed,
    /// The source file could not be read.
    ReadError,
    /// The result could not be written.
    WriteError,
    /// There was nothing the tool could be asked to do.
    ///
    /// The settings named no work: a split with no points inside the
    /// video, or no points at all.
    NothingToDo,
}

impl ToolError {
    pub fn name(&self) -> &'static str {
        match self {
            ToolError::UnsupportedKind => "the tool does not work on this kind of file",
            ToolError::NotImplemented => "this tool is not implemented yet",
            ToolError::ToolFailed => "the tool did not finish",
            ToolError::ReadError => "the source file could not be read",
            ToolError::WriteError => "the result could not be written",
            ToolError::NothingToDo => "there is nothing to do",
        }
    }
}

/// The handler for tool operations.
pub struct Tools<P: Publisher> {
    /// The FFmpeg programs.
    pub ffmpeg_tools: FfmpegTools,
    /// Puts a finished file at its name.
    pub publisher: P,
}

impl<P: Publisher> Tools<P> {
    /// Runs one tool operation on one input file.
    fn run_one(
        &self,
        operation_id: &str,
        input: &Path,
        source: &crate::media::manifest::FormatSupport,
        target: &crate::media::manifest::FormatSupport,
        temporary: &Path,
        runner: &dyn ProcessRunner,
        settings: &crate::radial::execution::ToolSettings,
    ) -> Result<PathBuf, ToolError> {
        // A crop takes two entirely different pairs of hands — a video is
        // cut by the conversion program, a picture by the program itself —
        // so it stands apart from the conversions below.
        if operation_id == "tools.crop" {
            return self.run_crop(input, target, temporary, runner, settings);
        }

        let resize = resize_filter(settings);
        let video_filter = match operation_id {
            "tools.rotate" => Some("transpose=1"),
            "tools.resize" => Some(resize.as_str()),
            _ => None,
        };

        // A trim keeps what it is given and copies it: the streams are
        // not decoded and made again, only lifted out between the two
        // moments. Everything else is a conversion.
        let arguments = if operation_id == "tools.trim" {
            trim_args(input, source, target, Some(temporary), settings)
        } else {
            build_convert_args(
                input,
                source,
                target,
                Some(temporary),
                video_filter,
            )
        };

        self.finish(
            &arguments,
            source,
            temporary,
            &final_names(input, target.format.as_str()),
            runner,
        )
    }

    /// Cuts one file down to the rectangle the window asked for.
    ///
    /// A video goes through the conversion program: the picture is made
    /// again at the rectangle, the sound is carried over as it is. A
    /// picture is cut with the program's own two hands — decoded,
    /// clipped, written back in the format it came in — and needs no
    /// program at all. A sound has no edges to take off.
    fn run_crop(
        &self,
        input: &Path,
        target: &crate::media::manifest::FormatSupport,
        temporary: &Path,
        runner: &dyn ProcessRunner,
        settings: &crate::radial::execution::ToolSettings,
    ) -> Result<PathBuf, ToolError> {
        let source_ext = input
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();
        match crate::media::probe::kind_of(&source_ext) {
            Some(crate::media::probe::Kind::Video) => {
                let Some((x, y, w, h)) = crop_rect(settings) else {
                    return Err(ToolError::NothingToDo);
                };
                let arguments = crop_args(
                    input,
                    target,
                    Some(temporary),
                    (x, y, w, h),
                    (settings.width, settings.height),
                );
                let finals = final_names(input, target.format.as_str());
                self.finish(&arguments, target, temporary, &finals, runner)
            }
            Some(crate::media::probe::Kind::Image) => {
                crop_image(input, temporary, settings)?;
                let finals = final_names(input, &source_ext);
                self.publish(&finals, temporary)
            }
            _ => Err(ToolError::UnsupportedKind),
        }
    }
    /// Puts what a command wrote where it belongs.
    ///
    /// A muxer that wants a file writes the temporary itself; one that
    /// pours its result over the pipe is given the temporary as its
    /// bucket. What was written is moved to the first of the names it
    /// may take that is free — the conversions name their results the
    /// same way — and it never takes a file that is already there.
    fn finish(
        &self,
        arguments: &[OsString],
        target: &crate::media::manifest::FormatSupport,
        temporary: &Path,
        finals: &[PathBuf],
        runner: &dyn ProcessRunner,
    ) -> Result<PathBuf, ToolError> {
        let needs_file = muxer_needs_file(&target.write.muxer);
        let run_result = if needs_file {
            runner
                .run(&self.ffmpeg_tools.ffmpeg, arguments, None)
                .map_err(|_| ToolError::ToolFailed)?
        } else {
            let mut file = fs::File::create(temporary).map_err(|_| ToolError::WriteError)?;
            runner
                .run(&self.ffmpeg_tools.ffmpeg, arguments, Some(&mut file))
                .map_err(|_| ToolError::ToolFailed)?
        };

        if !run_result.succeeded() {
            return Err(ToolError::ToolFailed);
        }

        self.publish(finals, temporary)
    }

    /// Puts a finished file at the first of its names that is free.
    ///
    /// When the name it asked for is taken, it takes the same name with a
    /// number after it — the way a file manager renames, and the way the
    /// conversions have always named what they copy. Nothing is ever
    /// written over.
    fn publish(&self, finals: &[PathBuf], temporary: &Path) -> Result<PathBuf, ToolError> {
        for final_name in finals {
            match self.publisher.publish(temporary, final_name) {
                Ok(()) => return Ok(final_name.clone()),
                Err(PublishError::DestinationExists) => continue,
                Err(PublishError::CouldNotMove) => return Err(ToolError::WriteError),
            }
        }
        Err(ToolError::WriteError)
    }

    /// Splits, at the points the window carried, into a folder of
    /// numbered clips.
    ///
    /// The video's own length is asked of it: the points stand inside that
    /// length, and the last part runs to wherever the video ends. The
    /// folder is made beside the original and named for it — "claw clips"
    /// for claw.mp4 — and gets its number when that name is taken, the way
    /// a folder is renamed when its twin is already there. Each clip is a
    /// stretch of the original, lifted out without decoding: the same
    /// copying a trim does.
    fn run_split(&self, request: &Request, runner: &dyn ProcessRunner) -> Outcome {
        if request.settings.points_ms.is_empty() {
            return Outcome::Failed {
                reason: Failure::InvalidInput("there are no split points".to_string()),
            };
        }

        let manifest = Manifest::get();
        let mut outputs = Vec::new();
        let mut failed = 0usize;
        let mut first_reason: Option<String> = None;
        for input in &request.inputs {
            let result = self.split_one(input, &manifest, runner, &request.settings);
            match result {
                Ok(paths) => outputs.extend(paths),
                Err(reason) => {
                    failed += 1;
                    let name = reason.name();
                    diagnostics::log(Level::Warn, "action", &format!("a file was not processed: {name}"));
                    if first_reason.is_none() {
                        first_reason = Some(name.to_string());
                    }
                }
            }
        }

        if failed == 0 {
            Outcome::Succeeded { outputs }
        } else {
            let because = match first_reason {
                Some(reason) => format!(": {reason}"),
                None => String::new(),
            };
            Outcome::Failed {
                reason: Failure::Error(format!(
                    "{failed} of the files could not be processed{because}"
                )),
            }
        }
    }

    /// Splits one video into its folder of numbered clips.
    fn split_one(
        &self,
        input: &Path,
        manifest: &Manifest,
        runner: &dyn ProcessRunner,
        settings: &crate::radial::execution::ToolSettings,
    ) -> Result<Vec<PathBuf>, ToolError> {
        let source_ext = input
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();
        let source = manifest
            .format(&source_ext)
            .ok_or(ToolError::UnsupportedKind)?;

        // The video says how long it stands: the points stand inside it,
        // and the last clip runs to its end.
        let duration = crate::media::frames::duration_of(&self.ffmpeg_tools, input)
            .ok_or(ToolError::ReadError)?;
        let segments = split_segments(&settings.points_ms, duration);
        if segments.is_empty() {
            return Err(ToolError::NothingToDo);
        }

        let stem = input
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_else(|| "clip".to_string());
        let under = input
            .parent()
            .map(|parent| parent.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."));
        let folder = unique_folder(&under, &format!("{stem} clips"))?;

        let mut outputs = Vec::new();
        for (index, (start, end)) in segments.iter().enumerate() {
            let asked = crate::radial::execution::ToolSettings {
                start_ms: Some(*start),
                end_ms: Some(*end),
                ..Default::default()
            };
            let leaf = format!("{} part {}.{source_ext}", stem, index + 1);
            let final_name = folder.join(&leaf);
            let work_path = folder.join(format!("{leaf}.part"));
            let arguments = trim_args(input, source, source, Some(&work_path), &asked);
            let made = self.finish(&arguments, source, &work_path, &[final_name], runner);
            match made {
                Ok(path) => outputs.push(path),
                Err(reason) => return Err(reason),
            }
        }
        Ok(outputs)
    }
}

/// The arguments that lift a stretch of a video out of its file.
///
/// The seek sits before the input, which makes it fast, and the streams
/// are copied as they are: no decoding, no making again. With no range
/// given, the whole file is lifted — which is a re-mux, and also what
/// the tool did before it had a window to be asked in (and did badly:
/// one frame of it).
fn trim_args(
    input: &Path,
    source: &crate::media::manifest::FormatSupport,
    target: &crate::media::manifest::FormatSupport,
    output_path: Option<&Path>,
    settings: &crate::radial::execution::ToolSettings,
) -> Vec<OsString> {
    let seconds = |milliseconds: u64| format!("{}.{:03}", milliseconds / 1000, milliseconds % 1000);

    let mut args = vec![
        OsString::from("-hide_banner"),
        OsString::from("-nostdin"),
        OsString::from("-y"),
    ];
    if let Some(start) = settings.start_ms.filter(|ms| *ms > 0) {
        args.push(OsString::from("-ss"));
        args.push(OsString::from(seconds(start)));
    }
    args.push(OsString::from("-f"));
    args.push(OsString::from(&source.read.demuxer));
    args.push(OsString::from("-i"));
    args.push(input.as_os_str().to_os_string());
    match (settings.start_ms, settings.end_ms) {
        (Some(start), Some(end)) if end > start => {
            args.push(OsString::from("-t"));
            args.push(OsString::from(seconds(end - start)));
        }
        (None, Some(end)) if end > 0 => {
            args.push(OsString::from("-t"));
            args.push(OsString::from(seconds(end)));
        }
        _ => {}
    }
    args.push(OsString::from("-map"));
    args.push(OsString::from("0"));
    args.push(OsString::from("-c"));
    args.push(OsString::from("copy"));
    args.push(OsString::from("-f"));
    args.push(OsString::from(&target.write.muxer));
    match output_path {
        Some(path) => args.push(path.as_os_str().to_os_string()),
        None => args.push(OsString::from("pipe:1")),
    }
    args
}

/// The folder a split's clips stand in, not the name that is taken.
///
/// The name asked for first, then the name with a number after it — the way
/// a file manager renames folders, and the way the original's
/// uniqueDirectory does. The folder is made as soon as a name is free.
fn unique_folder(under: &Path, name: &str) -> Result<PathBuf, ToolError> {
    let mut candidate = under.join(name);
    if !candidate.exists() {
        std::fs::create_dir_all(&candidate).map_err(|_| ToolError::WriteError)?;
        return Ok(candidate);
    }
    for number in 2..1000u32 {
        candidate = under.join(format!("{name} {number}"));
        if !candidate.exists() {
            std::fs::create_dir_all(&candidate).map_err(|_| ToolError::WriteError)?;
            return Ok(candidate);
        }
    }
    Err(ToolError::WriteError)
}

/// The stretches the video is cut into, from the points asked for.
///
/// The bounds are the beginning, the points that stand between, and the
/// end; a stretch shorter than 0.05 of a second is not a clip, it is a
/// blade still pressed against the one beside it — the original skips
/// those. Points at or beyond the video's ends are not points at all.
fn split_segments(points_ms: &[u64], duration: f64) -> Vec<(u64, u64)> {
    let end_ms = (duration * 1000.0).round() as u64;
    let mut bounded: Vec<u64> = points_ms
        .iter()
        .copied()
        .filter(|ms| *ms > 0 && *ms < end_ms)
        .collect();
    bounded.sort_unstable();
    let mut bounds: Vec<u64> = vec![0];
    bounds.extend(bounded);
    bounds.push(end_ms);
    let mut segments = Vec::new();
    for pair in bounds.windows(2) {
        let (start, end) = (pair[0], pair[1]);
        if end - start > 50 {
            segments.push((start, end));
        }
    }
    segments
}

/// The rectangle a crop keeps, if the settings carry a whole one.
fn crop_rect(settings: &crate::radial::execution::ToolSettings) -> Option<(u32, u32, u32, u32)> {
    Some((
        settings.crop_x?,
        settings.crop_y?,
        settings.crop_w?,
        settings.crop_h?,
    ))
}

/// The names a result that keeps the original's name may take.
///
/// The name itself, and the same one with a number after it — because the
/// name itself is taken: it is the very file the work came from. A cut
/// that keeps the format lands beside the original like the conversions
/// do, never over anything.
fn final_names(input: &Path, extension: &str) -> Vec<PathBuf> {
    crate::media::publish::names_for(input, extension)
}

/// A size the video encoders take: both sides even. An odd side is
/// brought down to the even one below it, the way the original rounds
/// its crop before it cuts.
fn even(size: u32) -> u32 {
    size - (size % 2)
}

/// Kept inside the picture: a crop that runs past an edge is pulled back
/// to it, and what is left is never without width or height.
fn clamp_rect(
    x: u32,
    y: u32,
    w: u32,
    h: u32,
    whole_w: u32,
    whole_h: u32,
) -> (u32, u32, u32, u32) {
    let x = x.min(whole_w.saturating_sub(1));
    let y = y.min(whole_h.saturating_sub(1));
    let w = w.min(whole_w - x).max(1);
    let h = h.min(whole_h - y).max(1);
    (x, y, w, h)
}

/// The arguments that cut a video down to the rectangle it was asked
/// for.
///
/// The picture is made again — a crop cannot be copied, the pixels
/// themselves move — and the sound is carried over as it is. When the
/// window also said a size to come out at, the cut is followed by a
/// scale; when it did not, the cut area is what comes out.
fn crop_args(
    input: &Path,
    target: &crate::media::manifest::FormatSupport,
    output_path: Option<&Path>,
    rect: (u32, u32, u32, u32),
    output: (Option<u32>, Option<u32>),
) -> Vec<OsString> {
    let (x, y, w, h) = rect;
    // The crop itself is said with colons — width, height, then where it
    // starts — and a scale that follows is chained on with a comma.
    let mut filter = format!("crop={w}:{h}:{x}:{y}", w = even(w), h = even(h));
    match (output.0, output.1) {
        (Some(ow), Some(oh)) => filter.push_str(&format!(",scale={}:{}", even(ow), even(oh))),
        (Some(ow), None) => filter.push_str(&format!(",scale={}:{}", even(ow), -1)),
        (None, Some(oh)) => filter.push_str(&format!(",scale=-1:{}", even(oh))),
        (None, None) => {}
    }
    let mut args = vec![
        OsString::from("-hide_banner"),
        OsString::from("-nostdin"),
        OsString::from("-y"),
        OsString::from("-f"),
        OsString::from(&target.read.demuxer),
        OsString::from("-i"),
        input.as_os_str().to_os_string(),
        OsString::from("-vf"),
        OsString::from(filter.replace('&', ",")),
        OsString::from("-c:v"),
        OsString::from(&target.write.encoder),
        OsString::from("-c:a"),
        OsString::from("copy"),
        OsString::from("-f"),
        OsString::from(&target.write.muxer),
    ];
    match output_path {
        Some(path) => args.push(path.as_os_str().to_os_string()),
        None => args.push(OsString::from("pipe:1")),
    }
    args
}

/// Cuts a picture down to the rectangle it was asked for, with the
/// program's own two hands, and writes it where the temporary waits —
/// in the format it came in.
fn crop_image(
    input: &Path,
    temporary: &Path,
    settings: &crate::radial::execution::ToolSettings,
) -> Result<(), ToolError> {
    let extension = input
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    let Some(format) = image_format_of(&extension) else {
        return Err(ToolError::UnsupportedKind);
    };
    let picture = image::open(input).map_err(|_| ToolError::ReadError)?;
    let Some((x, y, w, h)) = crop_rect(settings) else {
        return Err(ToolError::NothingToDo);
    };
    let (x, y, w, h) = clamp_rect(x, y, w, h, picture.width(), picture.height());
    let mut cut = picture.crop_imm(x, y, w, h);
    if let (Some(ow), Some(oh)) = (settings.width, settings.height) {
        if ow > 0 && oh > 0 {
            cut = cut.resize_exact(ow, oh, image::imageops::FilterType::Lanczos3);
        }
    }
    cut.save_with_format(temporary, format)
        .map_err(|_| ToolError::WriteError)
}

/// The picture format a file's ending names, as far as the picture
/// library writes them.
fn image_format_of(extension: &str) -> Option<image::ImageFormat> {
    match extension {
        "png" => Some(image::ImageFormat::Png),
        "jpeg" | "jpg" => Some(image::ImageFormat::Jpeg),
        "bmp" => Some(image::ImageFormat::Bmp),
        "tiff" | "tif" => Some(image::ImageFormat::Tiff),
        "gif" => Some(image::ImageFormat::Gif),
        "webp" => Some(image::ImageFormat::WebP),
        "ico" => Some(image::ImageFormat::Ico),
        _ => None,
    }
}

/// The filter that resizes a picture, from what was asked.
///
/// A side that was not said keeps its share: `-1` is the conversion
/// program's word for "the size that keeps the picture's shape". With
/// nothing said, the picture is made half the size, which is what the
/// wheel did before there was a window to ask in.
fn resize_filter(settings: &crate::radial::execution::ToolSettings) -> String {
    match (settings.width, settings.height) {
        (Some(width), Some(height)) => format!("scale={width}:{height}"),
        (Some(width), None) => format!("scale={width}:-1"),
        (None, Some(height)) => format!("scale=-1:{height}"),
        (None, None) => "scale=iw/2:ih/2".to_string(),
    }
}

impl<P: Publisher> Handler for Tools<P> {
    fn run(&self, request: &Request, _selection: &Selection) -> Outcome {
        let Some(operation) = operation_by_id(&request.operation) else {
            return Outcome::Failed {
                reason: Failure::UnknownOperation,
            };
        };

        // A split is run apart from the rest: it takes the input's own
        // format, and what it writes is a folder of numbered clips beside
        // the original, not one file under the Beside policy.
        if request.operation == "tools.split" {
            return self.run_split(request, &SystemRunner);
        }

        let manifest = Manifest::get();
        let runner = SystemRunner;

        let mut outputs = Vec::new();
        let mut failed = 0usize;
        let mut first_reason: Option<String> = None;
        for input in &request.inputs {
            let source_ext = input
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_lowercase();
            let Some(source) = manifest.format(&source_ext) else {
                failed += 1;
                let name = ToolError::UnsupportedKind.name();
                diagnostics::log(Level::Warn, "action", &format!("a file was not processed: {name}"));
                if first_reason.is_none() {
                    first_reason = Some(name.to_string());
                }
                continue;
            };

            // What the operation writes: the format it was told, or — when
            // it keeps what it was given — the format of this very file.
            // The per-input answer is what lets one operation take kinds
            // of different backs at all.
            let extension = match operation.output {
                Output::Beside(fixed) => fixed,
                Output::BesideAsInput => source_ext.as_str(),
                Output::InPlace | Output::Nothing => {
                    failed += 1;
                    let name = ToolError::UnsupportedKind.name();
                    diagnostics::log(Level::Warn, "action", &format!("a file was not processed: {name}"));
                    if first_reason.is_none() {
                        first_reason = Some(name.to_string());
                    }
                    continue;
                }
            };

            let Some(target) = manifest.format(extension) else {
                failed += 1;
                let name = ToolError::UnsupportedKind.name();
                diagnostics::log(Level::Warn, "action", &format!("a file was not processed: {name}"));
                if first_reason.is_none() {
                    first_reason = Some(name.to_string());
                }
                continue;
            };

            let target_ext = extension;
            let temporary = input.with_extension(format!("{target_ext}.part"));

            let result = self.run_one(
                &request.operation,
                input,
                source,
                target,
                &temporary,
                &runner,
                &request.settings,
            );

            match result {
                Ok(path) => outputs.push(path),
                Err(reason) => {
                    failed += 1;
                    let name = reason.name();
                    diagnostics::log(Level::Warn, "action", &format!("a file was not processed: {name}"));
                    if first_reason.is_none() {
                        first_reason = Some(name.to_string());
                    }
                }
            }
        }

        if failed == 0 {
            Outcome::Succeeded { outputs }
        } else {
            let because = match first_reason {
                Some(reason) => format!(": {reason}"),
                None => String::new(),
            };
            Outcome::Failed {
                reason: Failure::Error(format!(
                    "{failed} of the files could not be processed{because}"
                )),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::radial::execution::ToolSettings;

    #[test]
    fn the_resize_filter_takes_what_was_asked() {
        let both = ToolSettings {
            width: Some(800),
            height: Some(600),
            ..Default::default()
        };
        assert_eq!(resize_filter(&both), "scale=800:600");
        let only_width = ToolSettings {
            width: Some(800),
            height: None,
            ..Default::default()
        };
        assert_eq!(resize_filter(&only_width), "scale=800:-1");
        let only_height = ToolSettings {
            width: None,
            height: Some(600),
            ..Default::default()
        };
        assert_eq!(resize_filter(&only_height), "scale=-1:600");
    }

    #[test]
    fn the_resize_filter_without_a_window_makes_half() {
        assert_eq!(resize_filter(&ToolSettings::default()), "scale=iw/2:ih/2");
    }

    /// Where a word sits in the arguments, if it is there.
    fn at(args: &[OsString], word: &str) -> usize {
        args.iter()
            .position(|said| said.to_string_lossy() == word)
            .unwrap_or(usize::MAX)
    }

    #[test]
    fn the_trim_takes_the_range_it_was_given() {
        let mp4 = crate::media::manifest::Manifest::get()
            .format("mp4")
            .expect("mp4 is in the manifest");
        let asked = ToolSettings {
            start_ms: Some(1_000),
            end_ms: Some(3_500),
            ..Default::default()
        };
        let args = trim_args(
            Path::new("one.mp4"),
            mp4,
            mp4,
            Some(Path::new("one.mp4.part")),
            &asked,
        );
        // The seek comes before the input; the length after it.
        assert!(at(&args, "-ss") < at(&args, "-i"));
        assert!(at(&args, "-i") < at(&args, "-t"));
        assert_eq!(
            args[at(&args, "-ss") + 1].to_string_lossy(),
            "1.000",
            "the seek is said in seconds: {args:?}"
        );
        assert_eq!(
            args[at(&args, "-t") + 1].to_string_lossy(),
            "2.500",
            "the length is the end less the beginning: {args:?}"
        );
        assert!(at(&args, "-c") < args.len());
        assert_eq!(args[at(&args, "-c") + 1].to_string_lossy(), "copy");
        // The whole of every stream is carried, sound and all.
        assert!(args.iter().any(|said| said.to_string_lossy() == "-map"));
        assert_eq!(args[at(&args, "-map") + 1].to_string_lossy(), "0");
    }

    #[test]
    fn the_trim_without_a_range_copies_the_whole() {
        let mp4 = crate::media::manifest::Manifest::get()
            .format("mp4")
            .expect("mp4 is in the manifest");
        let args = trim_args(
            Path::new("one.mp4"),
            mp4,
            mp4,
            Some(Path::new("one.mp4.part")),
            &ToolSettings::default(),
        );
        assert_eq!(at(&args, "-ss"), usize::MAX, "no seek is said");
        assert_eq!(at(&args, "-t"), usize::MAX, "no length is said");
        assert_eq!(args[at(&args, "-c") + 1].to_string_lossy(), "copy");
    }

    #[test]
    fn a_split_cuts_the_video_into_the_stretches_its_points_make() {
        // Three seconds, one point at one: two clips, 0–1 and 1–3.
        assert_eq!(
            split_segments(&[1_000], 3.0),
            vec![(0, 1_000), (1_000, 3_000)]
        );
        // Equal parts is what the window says: three points at 1 s apart
        // cut four clips of the same length.
        assert_eq!(
            split_segments(&[1_000, 2_000], 3.0),
            vec![(0, 1_000), (1_000, 2_000), (2_000, 3_000)]
        );
        // The points stand on their own: an empty set asks for nothing.
        assert_eq!(split_segments(&[], 3.0), vec![(0, 3_000)]);
    }

    #[test]
    fn a_split_ignores_blades_and_points_beyond_the_video() {
        // A point at a boundary is not a clip; two at the same moment are
        // one; a stretch of 0.05 s or less is not a clip either.
        assert_eq!(split_segments(&[0, 3_000], 3.0), vec![(0, 3_000)]);
        assert_eq!(split_segments(&[1_000, 1_000], 3.0), vec![(0, 1_000), (1_000, 3_000)]);
        // 500–550 is 0.05 s exactly: the original skips it, so 550 keeps.
        assert_eq!(
            split_segments(&[500, 550, 2_000], 3.0),
            vec![(0, 500), (550, 2_000), (2_000, 3_000)]
        );
        // Points outside the length are not points: a shorter video cut at
        // the marks of a longer one still cuts what it has.
        assert_eq!(split_segments(&[1_000, 5_000], 3.0), vec![(0, 1_000), (1_000, 3_000)]);
        // and a video whose length nobody knows is not cut at all.
        assert_eq!(split_segments(&[1_000], 0.0), vec![]);
    }

    #[test]
    fn a_clips_folder_that_is_taken_gets_a_number() {
        let under = std::env::temp_dir().join("open-tangerine-unique-folder");
        let _ = std::fs::remove_dir_all(&under);
        std::fs::create_dir_all(&under).expect("the scratch folder was made");
        let first = unique_folder(&under, "claw clips").expect("the folder was made");
        assert_eq!(first.file_name().unwrap().to_string_lossy(), "claw clips");
        assert!(first.is_dir(), "the folder is there as soon as it is named");
        let second = unique_folder(&under, "claw clips").expect("the second folder was made");
        assert_eq!(
            second.file_name().unwrap().to_string_lossy(),
            "claw clips 2",
            "a taken name is numbered"
        );
        assert!(second.is_dir(), "the second folder is made when it is named");
        let _ = std::fs::remove_dir_all(&under);
    }

    #[test]
    #[ignore = "needs the conversion program"]
    fn a_video_is_split_into_a_folder_of_numbered_clips() {
        struct Renamer;
        impl crate::media::publish::Publisher for Renamer {
            fn publish(&self, temporary: &Path, destination: &Path) -> Result<(), crate::media::publish::PublishError> {
                if destination.is_file() {
                    return Err(crate::media::publish::PublishError::DestinationExists);
                }
                std::fs::rename(temporary, destination)
                    .map_err(|_| crate::media::publish::PublishError::CouldNotMove)
            }
        }
        let folder = Path::new("vendor").join("ffmpeg");
        let ffmpeg = folder.join("ffmpeg.exe");
        let ffprobe = folder.join("ffprobe.exe");
        if !ffmpeg.is_file() || !ffprobe.is_file() {
            eprintln!("the program is not there; this test needs it");
            return;
        }
        let tools = crate::media::locator::Tools {
            ffmpeg: ffmpeg.clone(),
            ffprobe: ffprobe.clone(),
        };

        let scratch = std::env::temp_dir().join("open-tangerine-split-test");
        let _ = std::fs::remove_dir_all(&scratch);
        std::fs::create_dir_all(&scratch).expect("the scratch folder was made");

        // The same three-second video the trim test makes.
        let raw = scratch.join("three.raw");
        let pixels = vec![255u8, 0, 0, 255].repeat(160 * 120 * 30);
        std::fs::write(&raw, pixels).expect("the pixels could be written");
        let input = scratch.join("three.mp4");
        let making: Vec<OsString> = [
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
            input.as_os_str().to_os_string(),
        ])
        .collect();
        let made = SystemRunner.run(&ffmpeg, &making, None).expect("the video was made");
        assert!(made.succeeded(), "the test video was not made: {}", made.stderr);

        // The split is run the way the wheel runs it: at the one-second
        // mark, which is what the equal-parts slider says for 2 parts.
        let probe = tools.clone();
        let splitter = Tools {
            ffmpeg_tools: tools,
            publisher: Renamer,
        };
        let asked = crate::radial::execution::Request {
            operation: "tools.split".to_string(),
            inputs: vec![input.clone()],
            settings: crate::radial::execution::ToolSettings {
                points_ms: vec![1_000],
                ..Default::default()
            },
        };
        let outcome = splitter.run(&asked, &Selection::inspect(&[]));
        let Outcome::Succeeded { outputs } = outcome else {
            panic!("the split did not succeed");
        };
        // Two clips in the folder beside the original, counted in order,
        // each whole and the length its stretch says.
        assert_eq!(outputs.len(), 2, "one point makes two clips");
        let clips = outputs[0].parent().expect("the clips have a folder");
        assert_eq!(
            clips.file_name().unwrap().to_string_lossy(),
            "three clips",
            "the folder is named for the original"
        );
        assert_eq!(outputs[0].file_name().unwrap().to_string_lossy(), "three part 1.mp4");
        assert_eq!(outputs[1].file_name().unwrap().to_string_lossy(), "three part 2.mp4");
        let first =
            crate::media::frames::duration_of(&probe, &outputs[0]).expect("the first clip has a length");
        assert!(
            (first - 1.0).abs() < 0.35,
            "the first clip is close to a second: {first}"
        );
        let second =
            crate::media::frames::duration_of(&probe, &outputs[1]).expect("the second clip has a length");
        assert!(
            (second - 2.0).abs() < 0.35,
            "the second clip carries the rest: {second}"
        );
        let _ = std::fs::remove_dir_all(&scratch);
    }

    #[test]
    #[ignore = "needs the conversion program"]
    fn a_video_is_trimmed_to_the_range_it_was_given() {
        let folder = Path::new("vendor").join("ffmpeg");
        let ffmpeg = folder.join("ffmpeg.exe");
        let ffprobe = folder.join("ffprobe.exe");
        if !ffmpeg.is_file() || !ffprobe.is_file() {
            eprintln!("the program is not there; this test needs it");
            return;
        }
        let tools = crate::media::locator::Tools {
            ffmpeg: ffmpeg.clone(),
            ffprobe: ffprobe.clone(),
        };
        let scratch = std::env::temp_dir().join("open-tangerine-trim-test");
        std::fs::create_dir_all(&scratch).expect("the scratch folder was made");

        // A video of three seconds, ten frames a second, made the way
        // the tests of the media module make their pictures: raw pixels
        // in a file, read by the rawvideo reader.
        let raw = scratch.join("three.raw");
        let pixels = vec![255u8, 0, 0, 255].repeat(160 * 120 * 30);
        std::fs::write(&raw, pixels).expect("the pixels could be written");
        let input = scratch.join("three.mp4");
        let making: Vec<OsString> = [
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
            input.as_os_str().to_os_string(),
        ])
        .collect();
        let made = SystemRunner.run(&ffmpeg, &making, None).expect("the video was made");
        assert!(made.succeeded(), "the test video was not made: {}", made.stderr);

        // The first second of it is lifted out, and the result says so.
        let mp4 = crate::media::manifest::Manifest::get()
            .format("mp4")
            .expect("mp4 is in the manifest");
        let trimmed = scratch.join("one.mp4");
        let asked = ToolSettings {
            start_ms: Some(0),
            end_ms: Some(1_000),
            ..Default::default()
        };
        let arguments = trim_args(&input, mp4, mp4, Some(&trimmed), &asked);
        let run = SystemRunner
            .run(&ffmpeg, &arguments, None)
            .expect("the trim ran");
        assert!(run.succeeded(), "the trim did not finish: {}", run.stderr);
        let kept = crate::media::frames::duration_of(&tools, &trimmed)
            .expect("the trimmed video has a length");
        assert!(
            (kept - 1.0).abs() < 0.35,
            "the trim kept {kept}s of a 3s video, wanted about 1s"
        );
        // The window's own question: what the video looks like at a
        // moment of it. Halfway into the original there is a frame.
        let frame = crate::media::frames::frame_at(&tools, &input, 1.5)
            .expect("a frame was taken from the middle of the video");
        assert!(frame.width() > 0, "the frame holds a picture");
        let _ = std::fs::remove_dir_all(&scratch);
    }

    struct Nothing;

    impl Publisher for Nothing {
        fn publish(&self, _temporary: &Path, _destination: &Path) -> Result<(), crate::media::publish::PublishError> {
            Ok(())
        }
    }

    #[test]
    fn unknown_operation_is_rejected() {
        let tools = Tools {
            ffmpeg_tools: crate::media::locator::Tools {
                ffmpeg: PathBuf::from("ffmpeg"),
                ffprobe: PathBuf::from("ffprobe"),
            },
            publisher: Nothing,
        };
        let environment = crate::radial::registry::Environment::none();
        let request = Request {
            operation: "tools.nonexistent".to_string(),
            inputs: Vec::new(),
            ..Default::default()
        };
        let outcome = crate::radial::execution::run(&request, &Selection::inspect(&[]), &environment, &tools);
        match outcome {
            Outcome::Failed { reason } => {
                assert!(matches!(reason, Failure::UnknownOperation));
            }
            other => panic!("unexpected outcome: {other:?}"),
        }
    }

    #[test]
    fn the_crop_is_said_the_way_the_program_reads_it() {
        let mp4 = crate::media::manifest::Manifest::get()
            .format("mp4")
            .expect("mp4 is in the manifest");
        let args = crop_args(
            Path::new("one.mp4"),
            mp4,
            Some(Path::new("one.mp4.part")),
            (10, 8, 81, 61),
            (None, None),
        );
        let filter = args
            .iter()
            .position(|said| said.to_string_lossy() == "-vf")
            .map(|at| args[at + 1].to_string_lossy().into_owned())
            .expect("the crop carries a filter");
        // The odd sides are brought down to even ones, and the rectangle
        // is said width, height, then where it starts.
        assert_eq!(filter, "crop=80:60:10:8", "{args:?}");
        // The sound is carried over as it is, not made again.
        let audio = args
            .iter()
            .position(|said| said.to_string_lossy() == "-c:a")
            .map(|at| args[at + 1].to_string_lossy().into_owned())
            .expect("the crop says what its sound does");
        assert_eq!(audio, "copy");
    }

    #[test]
    fn a_crop_that_is_also_resized_says_so() {
        let mp4 = crate::media::manifest::Manifest::get()
            .format("mp4")
            .expect("mp4 is in the manifest");
        let args = crop_args(
            Path::new("one.mp4"),
            mp4,
            Some(Path::new("one.mp4.part")),
            (0, 0, 100, 100),
            (Some(41), None),
        );
        let filter = args
            .iter()
            .position(|said| said.to_string_lossy() == "-vf")
            .map(|at| args[at + 1].to_string_lossy().into_owned())
            .expect("the crop carries a filter");
        assert_eq!(filter, "crop=100:100:0:0,scale=40:-1", "{args:?}");
    }

    #[test]
    fn a_crop_that_runs_past_the_edge_is_pulled_back() {
        // A rect that starts past the picture is brought to its edge.
        assert_eq!(clamp_rect(60, 60, 20, 20, 64, 64), (60, 60, 4, 4));
        // One that is bigger than the picture is cut down to it.
        assert_eq!(clamp_rect(0, 0, 100, 100, 64, 64), (0, 0, 64, 64));
        // And one inside stays exactly as it was.
        assert_eq!(clamp_rect(4, 4, 32, 24, 64, 64), (4, 4, 32, 24));
    }

    /// A publisher shaped like the real one: the move happens only when
    /// nothing stands at the name it is asked for.
    struct RealShaped;

    impl Publisher for RealShaped {
        fn publish(
            &self,
            temporary: &Path,
            destination: &Path,
        ) -> Result<(), crate::media::publish::PublishError> {
            if destination.is_file() {
                return Err(crate::media::publish::PublishError::DestinationExists);
            }
            std::fs::copy(temporary, destination)
                .map(|_| ())
                .map_err(|_| crate::media::publish::PublishError::CouldNotMove)
        }
    }

    #[test]
    fn a_result_that_keeps_the_name_lands_beside_the_original() {
        // A cut that keeps the format wants the original's own name, and
        // the original is standing there: the result takes the next free
        // name instead. Nothing is ever written over.
        let scratch = std::env::temp_dir().join("open-tangerine-publish-beside-test");
        let _ = std::fs::remove_dir_all(&scratch);
        std::fs::create_dir_all(&scratch).expect("the scratch folder was made");
        let input = scratch.join("one.mp4");
        std::fs::write(&input, b"original").expect("the original could be made");
        let temporary = scratch.join("one.mp4.part");
        std::fs::write(&temporary, b"made").expect("the work could be made");

        let tools = Tools {
            ffmpeg_tools: crate::media::locator::Tools {
                ffmpeg: PathBuf::from("ffmpeg"),
                ffprobe: PathBuf::from("ffprobe"),
            },
            publisher: RealShaped,
        };
        let made = tools
            .publish(&final_names(&input, "mp4"), &temporary)
            .expect("a free name was found");
        assert_eq!(made, scratch.join("one (2).mp4"));
        assert_eq!(
            std::fs::read(&made).expect("the result is there"),
            b"made",
            "what was made is what arrived"
        );
        let _ = std::fs::remove_dir_all(&scratch);
    }

    /// A publisher shaped like the real one: the work is copied to the
    /// name it is given, or refused when something is already standing
    /// there — the handler is the one that then tries the next name.
    struct CopyPublisher;

    impl Publisher for CopyPublisher {
        fn publish(
            &self,
            temporary: &Path,
            destination: &Path,
        ) -> Result<(), crate::media::publish::PublishError> {
            if destination.is_file() {
                return Err(crate::media::publish::PublishError::DestinationExists);
            }
            std::fs::copy(temporary, destination)
                .map(|_| ())
                .map_err(|_| crate::media::publish::PublishError::CouldNotMove)
        }
    }

    /// Makes the three-second test video the trim test makes, and says
    /// where it is.
    fn make_test_video(scratch: &Path) -> PathBuf {
        let ffmpeg = Path::new("vendor").join("ffmpeg").join("ffmpeg.exe");
        assert!(ffmpeg.is_file(), "the conversion program is not there");
        let raw = scratch.join("three.raw");
        std::fs::write(&raw, vec![255u8, 0, 0, 255].repeat(160 * 120 * 30))
            .expect("the pixels could be written");
        let input = scratch.join("clip.mp4");
        let making: Vec<OsString> = [
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
            input.as_os_str().to_os_string(),
        ])
        .collect();
        let made = SystemRunner
            .run(&ffmpeg, &making, None)
            .expect("the video was made");
        assert!(made.succeeded(), "the test video was not made: {}", made.stderr);
        input
    }

    /// How big the picture in a video is, as the probe says.
    fn video_size(ffprobe: &Path, video: &Path) -> (u32, u32) {
        let asking: Vec<OsString> = [
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=width,height",
            "-of",
            "csv=p=0:s=x",
        ]
        .iter()
        .map(|word| OsString::from(*word))
        .chain([video.as_os_str().to_os_string()])
        .collect();
        let run = SystemRunner
            .run(ffprobe, &asking, None)
            .expect("the probe ran");
        assert!(run.succeeded(), "the probe did not finish: {}", run.stderr);
        let said = String::from_utf8_lossy(&run.stdout);
        let (w, h) = said.trim().split_once('x').expect("the probe said a size");
        (w.parse().expect("a width"), h.parse().expect("a height"))
    }

    #[test]
    #[ignore = "needs the conversion program"]
    fn a_video_is_cropped_to_the_rectangle_it_was_asked_for() {
        let folder = Path::new("vendor").join("ffmpeg");
        let ffmpeg = folder.join("ffmpeg.exe");
        let ffprobe = folder.join("ffprobe.exe");
        if !ffmpeg.is_file() || !ffprobe.is_file() {
            eprintln!("the program is not there; this test needs it");
            return;
        }
        let scratch = std::env::temp_dir().join("open-tangerine-crop-video-test");
        std::fs::create_dir_all(&scratch).expect("the scratch folder was made");
        let input = make_test_video(&scratch);

        // The crop goes the whole way through the handler, the way the
        // wheel drives it: the rectangle is odd on purpose, and the even
        // sides are what come out.
        let tools = Tools {
            ffmpeg_tools: crate::media::locator::Tools {
                ffmpeg,
                ffprobe: ffprobe.clone(),
            },
            publisher: CopyPublisher,
        };
        let request = Request {
            operation: "tools.crop".to_string(),
            inputs: vec![input.clone()],
            settings: ToolSettings {
                crop_x: Some(10),
                crop_y: Some(8),
                crop_w: Some(81),
                crop_h: Some(61),
                ..Default::default()
            },
            ..Default::default()
        };
        let outcome = tools.run(&request, &Selection::inspect(&[input.clone()]));
        let outputs = match outcome {
            Outcome::Succeeded { outputs } => outputs,
            Outcome::Failed { reason } => panic!("the crop failed: {:?}", reason),
            Outcome::Cancelled => panic!("the crop was not asked to stop"),
        };
        assert_eq!(outputs.len(), 1, "one file in, one file out");
        let made = outputs.first().expect("the crop is there");
        assert!(made.is_file(), "the cropped video exists");
        let (w, h) = video_size(&ffprobe, made);
        assert_eq!((w, h), (80, 60), "the crop is the rectangle, evened");
        let _ = std::fs::remove_dir_all(&scratch);
    }

    #[test]
    #[ignore = "needs the picture library and nothing more"]
    fn a_picture_is_cropped_with_the_programs_own_hands() {
        let scratch = std::env::temp_dir().join("open-tangerine-crop-image-test");
        std::fs::create_dir_all(&scratch).expect("the scratch folder was made");
        let input = scratch.join("photo.png");
        image::DynamicImage::new_rgb8(64, 48)
            .save(&input)
            .expect("the picture was made");

        let tools = Tools {
            ffmpeg_tools: crate::media::locator::Tools {
                ffmpeg: PathBuf::from("ffmpeg"),
                ffprobe: PathBuf::from("ffprobe"),
            },
            publisher: CopyPublisher,
        };
        let request = Request {
            operation: "tools.crop".to_string(),
            inputs: vec![input.clone()],
            settings: ToolSettings {
                crop_x: Some(4),
                crop_y: Some(4),
                crop_w: Some(32),
                crop_h: Some(24),
                ..Default::default()
            },
            ..Default::default()
        };
        let outcome = tools.run(&request, &Selection::inspect(&[input.clone()]));
        let outputs = match outcome {
            Outcome::Succeeded { outputs } => outputs,
            Outcome::Failed { reason } => panic!("the crop failed: {:?}", reason),
            Outcome::Cancelled => panic!("the crop was not asked to stop"),
        };
        let made = outputs.first().expect("the crop is there");
        let picture = image::open(made).expect("the crop reads back");
        assert_eq!((picture.width(), picture.height()), (32, 24));
        let _ = std::fs::remove_dir_all(&scratch);
    }

    #[test]
    #[ignore = "needs the conversion program"]
    fn a_trim_runs_the_whole_way_from_the_wheel_path() {
        // A trim keeps the format it was given, and there was a time when
        // the handler only knew how to answer a named format â€” the trim
        // was refused before it began. This walks the very path the wheel
        // drives, so the refusal can never come back unnoticed.
        let folder = Path::new("vendor").join("ffmpeg");
        let ffmpeg = folder.join("ffmpeg.exe");
        if !ffmpeg.is_file() {
            eprintln!("the program is not there; this test needs it");
            return;
        }
        let scratch = std::env::temp_dir().join("open-tangerine-trim-wheel-test");
        std::fs::create_dir_all(&scratch).expect("the scratch folder was made");
        let input = make_test_video(&scratch);

        let tools = Tools {
            ffmpeg_tools: crate::media::locator::Tools {
                ffmpeg,
                ffprobe: Path::new("vendor").join("ffmpeg").join("ffprobe.exe"),
            },
            publisher: CopyPublisher,
        };
        let request = Request {
            operation: "tools.trim".to_string(),
            inputs: vec![input.clone()],
            settings: ToolSettings {
                start_ms: Some(0),
                end_ms: Some(1_000),
                ..Default::default()
            },
            ..Default::default()
        };
        let outcome = tools.run(&request, &Selection::inspect(&[input.clone()]));
        match outcome {
            Outcome::Succeeded { outputs } => {
                assert_eq!(outputs.len(), 1, "one file in, one file out");
                assert!(outputs.first().is_some_and(|made| made.is_file()));
            }
            Outcome::Failed { reason } => panic!("the trim failed: {:?}", reason),
            Outcome::Cancelled => panic!("the trim was not asked to stop"),
        }
        let _ = std::fs::remove_dir_all(&scratch);
    }
}
