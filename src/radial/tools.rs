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
use crate::media::publish::Publisher;
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
        let resize = resize_filter(settings);
        let video_filter = match operation_id {
            "tools.crop" => Some("crop=iw/2:ih/2:(iw-iw/2)/2:(ih-iw/2)/2"),
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
            &input.with_extension(target.format.as_str()),
            runner,
        )
    }

    /// Puts what a command wrote where it belongs.
    ///
    /// A muxer that wants a file writes the temporary itself; one that
    /// pours its result over the pipe is given the temporary as its
    /// bucket. What was written is moved to the final name, which never
    /// takes a file that is already there.
    fn finish(
        &self,
        arguments: &[OsString],
        target: &crate::media::manifest::FormatSupport,
        temporary: &Path,
        final_name: &Path,
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

        self.publisher
            .publish(temporary, final_name)
            .map_err(|_| ToolError::WriteError)?;

        Ok(final_name.to_path_buf())
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
            let made = self.finish(&arguments, source, &work_path, &final_name, runner);
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

        let Output::Beside(extension) = operation.output else {
            return Outcome::Failed {
                reason: Failure::NotImplemented,
            };
        };

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

            let target = match manifest.format(extension) {
                Some(t) => t,
                None => {
                    failed += 1;
                    let name = ToolError::UnsupportedKind.name();
                    diagnostics::log(Level::Warn, "action", &format!("a file was not processed: {name}"));
                    if first_reason.is_none() {
                        first_reason = Some(name.to_string());
                    }
                    continue;
                }
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
}
