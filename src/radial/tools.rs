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
use crate::radial::execution::{Failure, Handler, Outcome, RedactStyle, Request};
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
    /// The size that was asked is smaller than the file can be made.
    ///
    /// A picture asked to fit a target size keeps being made less until
    /// there is almost nothing left of it; a target under that is not a
    /// size, it is a wish.
    TooSmall,
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
            ToolError::TooSmall => "the asked size is smaller than the file can be made",
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

    /// Covers the parts of pictures the window drew on.
    ///
    /// The look is laid on by the program's own hands: the picture is
    /// read, every rectangle takes the style that was asked of it, and
    /// the readied copy stands beside under its own name. Nothing of the
    /// run needs the conversion program — so a picture the picture
    /// library cannot read is refused politely, and nothing more.
    ///
    /// A redaction without areas is not a redaction: the handler says so
    /// rather than covering nothing and calling the file changed.
    fn run_redact_image(&self, request: &Request) -> Outcome {
        if request.settings.redact_areas.is_empty() {
            return Outcome::Failed {
                reason: Failure::InvalidInput("there are no redaction areas".to_string()),
            };
        }

        let manifest = Manifest::get();
        let mut outputs = Vec::new();
        let mut failed = 0usize;
        let mut first_reason: Option<String> = None;
        for input in &request.inputs {
            let result = self.redact_image_one(input, &manifest, &request.settings);
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

    /// Covers the drawn rectangles of one picture.
    fn redact_image_one(
        &self,
        input: &Path,
        manifest: &Manifest,
        settings: &crate::radial::execution::ToolSettings,
    ) -> Result<PathBuf, ToolError> {
        let source_ext = input
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();
        let source = manifest
            .format_of_extension(&source_ext)
            .or_else(|| manifest.format(&source_ext))
            .ok_or(ToolError::UnsupportedKind)?;
        if crate::media::probe::kind_of(&source.format) != Some(crate::media::probe::Kind::Image) {
            return Err(ToolError::UnsupportedKind);
        }
        let stem = input
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "file".to_string());
        let temporary = input.with_file_name(format!("{stem}.redact-part.{source_ext}"));

        let picture = image::open(input).map_err(|_| ToolError::ReadError)?;
        let covered = redact_picture(picture, settings);
        covered.save(&temporary).map_err(|_| ToolError::WriteError)?;

        let finals = metadata_final_names(input, "redacted", &source_ext);
        self.publish(&finals, &temporary)
    }

    /// Covers the parts of videos the window drew on.
    ///
    /// The rectangles go to the conversion program as a filter graph:
    /// a solid box is drawn, a smeared part is the crop made small and
    /// laid back large (the program carries no blur of its own — this
    /// build has none), a pixelated part is the crop made small and laid
    /// back large in squares. Every rectangle shows for its own stretch
    /// of time, told with `between(t, start, end)`; a rectangle given no
    /// stretch covers the whole way through. The copy is made again in
    /// the container it came in, at a constant picture quality — a
    /// redaction is not about the size.
    fn run_redact_video(&self, request: &Request, runner: &dyn ProcessRunner) -> Outcome {
        if request.settings.redact_areas.is_empty() {
            return Outcome::Failed {
                reason: Failure::InvalidInput("there are no redaction areas".to_string()),
            };
        }

        let manifest = Manifest::get();
        let mut outputs = Vec::new();
        let mut failed = 0usize;
        let mut first_reason: Option<String> = None;
        for input in &request.inputs {
            let result = self.redact_video_one(
                input,
                &manifest,
                runner,
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

    /// Covers the drawn rectangles of one video.
    fn redact_video_one(
        &self,
        input: &Path,
        manifest: &Manifest,
        runner: &dyn ProcessRunner,
        settings: &crate::radial::execution::ToolSettings,
    ) -> Result<PathBuf, ToolError> {
        let source_ext = input
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();
        let source = manifest
            .format_of_extension(&source_ext)
            .or_else(|| manifest.format(&source_ext))
            .ok_or(ToolError::UnsupportedKind)?;
        if crate::media::probe::kind_of(&source.format) != Some(crate::media::probe::Kind::Video) {
            return Err(ToolError::UnsupportedKind);
        }

        let (width, height, _fps) =
            crate::media::frames::shape_of(&self.ffmpeg_tools, input).ok_or(ToolError::ReadError)?;
        let graph = redact_video_filter(settings, width, height)?;
        let stem = input
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "file".to_string());
        let temporary = input.with_file_name(format!("{stem}.redact-part.{source_ext}"));
        let arguments = redact_video_args(input, source, &temporary, &graph);

        let finals = metadata_final_names(input, "redacted", &source_ext);
        self.finish(&arguments, source, &temporary, &finals, runner)
    }

    /// Bleeps over the stretches of sounds the window marked.
    ///
    /// The marks are said in milliseconds: every one of them becomes a
    /// span of samples where the original is dropped and a tone takes
    /// its place — the bleep does not hide under the sound, it stands
    /// over it. An overlap shows the last bleep laid, the way covers
    /// stand on a picture. The tone sings at the frequency the settings
    /// ask, and everything outside the marks is carried through for
    /// nothing to hear the difference.
    fn run_bleep(&self, request: &Request, runner: &dyn ProcessRunner) -> Outcome {
        if request.settings.bleep_ranges.is_empty() {
            return Outcome::Failed {
                reason: Failure::InvalidInput("there are no bleep ranges".to_string()),
            };
        }

        let manifest = Manifest::get();
        let mut outputs = Vec::new();
        let mut failed = 0usize;
        let mut first_reason: Option<String> = None;
        for input in &request.inputs {
            let result = self.bleep_one(input, &manifest, runner, &request.settings);
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

    /// Bleeps over the marked stretches of one sound, the way the
    /// volume tool works: read as numbers, laid over here, written back
    /// into the container it came in.
    fn bleep_one(
        &self,
        input: &Path,
        manifest: &Manifest,
        runner: &dyn ProcessRunner,
        settings: &crate::radial::execution::ToolSettings,
    ) -> Result<PathBuf, ToolError> {
        let source_ext = input
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();
        let source = manifest
            .format_of_extension(&source_ext)
            .or_else(|| manifest.format(&source_ext))
            .ok_or(ToolError::UnsupportedKind)?;

        if crate::media::probe::kind_of(&source.format) != Some(crate::media::probe::Kind::Audio) {
            return Err(ToolError::UnsupportedKind);
        }

        let pcm = crate::media::audio::decode_full_pcm(&self.ffmpeg_tools, input)
            .ok_or(ToolError::ReadError)?;

        let mut samples = pcm.samples;
        let channels = pcm.channels.max(1) as usize;
        let frames = samples.len() / channels;
        for range in &settings.bleep_ranges {
            let (start, end) = range.frame_span(frames, pcm.sample_rate);
            bleep_over(
                &mut samples,
                pcm.channels,
                start,
                end,
                pcm.sample_rate,
                settings.bleep_frequency_hz(),
            );
        }

        let temporary = input.with_extension(format!("{source_ext}.part"));

        if source_ext == "wav" {
            let wav_bytes = crate::media::audio::encode_wav_bytes(
                &samples,
                pcm.sample_rate,
                pcm.channels,
            );
            fs::write(&temporary, wav_bytes).map_err(|_| ToolError::WriteError)?;
        } else {
            let staging_wav = input.with_extension("staging_bleep.wav");
            let wav_bytes = crate::media::audio::encode_wav_bytes(
                &samples,
                pcm.sample_rate,
                pcm.channels,
            );
            fs::write(&staging_wav, wav_bytes).map_err(|_| ToolError::WriteError)?;

            let mut args = vec![
                OsString::from("-hide_banner"),
                OsString::from("-nostdin"),
                OsString::from("-y"),
                OsString::from("-i"),
                staging_wav.as_os_str().to_os_string(),
                OsString::from("-vn"),
                OsString::from("-c:a"),
                OsString::from(&source.write.encoder),
            ];
            if !source.write.muxer.is_empty() {
                args.push(OsString::from("-f"));
                args.push(OsString::from(&source.write.muxer));
            }
            args.push(temporary.as_os_str().to_os_string());

            let run_res = runner.run(&self.ffmpeg_tools.ffmpeg, &args, None);
            let _ = fs::remove_file(&staging_wav);

            match run_res {
                Ok(status) if status.succeeded() => {}
                _ => {
                    let _ = fs::remove_file(&temporary);
                    return Err(ToolError::ToolFailed);
                }
            }
        }

        let finals = metadata_final_names(input, "bleeped", &source_ext);
        self.publish(&finals, &temporary)
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
            .format_of_extension(&source_ext)
            .or_else(|| manifest.format(&source_ext))
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

    /// Takes the moments the window asked for as pictures, in a folder
    /// of its own beside the original, the way a split's clips stand.
    ///
    /// The shots are said in thousandths of a second and taken by the
    /// same seeking the crop's stage uses: whatever a moment says is
    /// what stands there. A moment past the video's end is not taken;
    /// the run does not fail for one lost frame, but a video whose
    /// frames all refuse to come out is refused.
    fn run_snapshots(&self, request: &Request, _runner: &dyn ProcessRunner) -> Outcome {
        if request.settings.snapshots_ms.is_empty() {
            return Outcome::Failed {
                reason: Failure::InvalidInput("there are no snapshots".to_string()),
            };
        }

        let manifest = Manifest::get();
        let mut outputs = Vec::new();
        let mut failed = 0usize;
        let mut first_reason: Option<String> = None;
        for input in &request.inputs {
            let result = self.snapshots_one(input, &manifest, &request.settings);
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

    /// Takes one video's snapshots, each a picture of its own.
    fn snapshots_one(
        &self,
        input: &Path,
        manifest: &Manifest,
        settings: &crate::radial::execution::ToolSettings,
    ) -> Result<Vec<PathBuf>, ToolError> {
        let source_ext = input
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();
        let source = manifest
            .format_of_extension(&source_ext)
            .or_else(|| manifest.format(&source_ext))
            .ok_or(ToolError::UnsupportedKind)?;
        if crate::media::probe::kind_of(&source.format) != Some(crate::media::probe::Kind::Video) {
            return Err(ToolError::UnsupportedKind);
        }

        // The video says how long it stands, so a moment asked past its
        // end can be set down rather than sought for nothing.
        let duration = crate::media::frames::duration_of(&self.ffmpeg_tools, input)
            .ok_or(ToolError::ReadError)?;
        let mut shots: Vec<u64> = settings
            .snapshots_ms
            .iter()
            .copied()
            .filter(|&ms| (ms as f64) <= duration * 1000.0)
            .collect();
        if shots.is_empty() {
            return Err(ToolError::NothingToDo);
        }
        shots.sort_unstable();

        let stem = input
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_else(|| "clip".to_string());
        let under = input
            .parent()
            .map(|parent| parent.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."));
        let folder = unique_folder(&under, &format!("{stem} frames"))?;

        let mut outputs = Vec::new();
        for (index, ms) in shots.iter().enumerate() {
            // The picture at the moment, said by the name it stands
            // under: the first frame taken is frame 1.
            let Some(picture) =
                crate::media::frames::frame_at(&self.ffmpeg_tools, input, *ms as f64 / 1000.0)
            else {
                // The video does not stand where it was asked to; the
                // frame is set down, not the whole taking.
                diagnostics::log(
                    Level::Warn,
                    "action",
                    &format!("a frame was not taken at {} ms", ms),
                );
                continue;
            };
            let target = folder.join(format!("{stem} frame {}.png", index + 1));
            picture
                .save(&target)
                .map_err(|_| ToolError::WriteError)?;
            outputs.push(target);
        }
        if outputs.is_empty() {
            return Err(ToolError::ReadError);
        }
        Ok(outputs)
    }

    /// Lays the speed the window asked on the files in the request.
    ///
    /// The video's own moments are counted by the frame, re-timed, and
    /// said again into one copy the video's own container wears — the
    /// picture keeps its size, and the sound runs with it. The sound's
    /// pitch is not kept: a faster copy sings higher, the way a tape
    /// runs. Each result is published beside the original under the
    /// shared naming, and nothing is written over.
    fn run_speed(&self, request: &Request, _runner: &dyn ProcessRunner) -> Outcome {
        let Some(percent) = request.settings.speed_percent else {
            return Outcome::Failed {
                reason: Failure::InvalidInput("there is no speed to run at".to_string()),
            };
        };

        let manifest = Manifest::get();
        let mut outputs = Vec::new();
        let mut failed = 0usize;
        let mut first_reason: Option<String> = None;
        for input in &request.inputs {
            let result = self.speed_one(input, &manifest, percent);
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

    /// Retimes one video, whole frames taken and said again.
    fn speed_one(
        &self,
        input: &Path,
        manifest: &Manifest,
        percent: u32,
    ) -> Result<PathBuf, ToolError> {
        let source_ext = input
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();
        let source = manifest
            .format_of_extension(&source_ext)
            .or_else(|| manifest.format(&source_ext))
            .ok_or(ToolError::UnsupportedKind)?;
        if crate::media::probe::kind_of(&source.format) != Some(crate::media::probe::Kind::Video) {
            return Err(ToolError::UnsupportedKind);
        }

        // The video says what it is: its sides and the frames a second it
        // stands at. The seconds say nothing here; the frames themselves
        // are what is counted, and the rate says how long the copy runs.
        let (width, height, rate) = crate::media::frames::shape_of(&self.ffmpeg_tools, input)
            .ok_or(ToolError::ReadError)?;
        if width == 0 || height == 0 || rate <= 0.0 {
            return Err(ToolError::ReadError);
        }

        let stem = input
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_else(|| "video".to_string());
        let temporary = input.with_file_name(format!("{stem}.speed-part.{source_ext}"));
        let raw = input.with_file_name(format!("{stem}.speed-raw.bin"));
        let retime = input.with_file_name(format!("{stem}.speed-frames.bin"));
        let sound = input.with_file_name(format!("{stem}.speed-sound.wav"));

        // The frames are decoded to the picture's own numbers, all of
        // them, and gathered in a scratch beside the original.
        let frame_bytes = (width as usize) * (height as usize) * 4;
        let decode_args: Vec<OsString> = [
            "-hide_banner",
            "-nostdin",
            "-y",
            "-i",
        ]
        .iter()
        .map(|word| OsString::from(*word))
        .chain([input.as_os_str().to_os_string()])
        .chain(
            [
                "-pix_fmt",
                "rgba",
                "-f",
                "rawvideo",
                "pipe:1",
            ]
            .iter()
            .map(|word| OsString::from(*word)),
        )
        .collect();
        let raw_file = std::fs::File::create(&raw).map_err(|_| ToolError::WriteError)?;
        let decode = SystemRunner
            .run(&self.ffmpeg_tools.ffmpeg, &decode_args, Some(&raw_file))
            .map_err(|_| ToolError::ToolFailed)?;
        if !decode.succeeded() {
            let _ = std::fs::remove_file(&raw);
            return Err(ToolError::ToolFailed);
        }
        let gathered = std::fs::read(&raw).map_err(|_| ToolError::ReadError)?;
        let _ = std::fs::remove_file(&raw);
        let held = gathered.len() / frame_bytes;
        if held == 0 {
            return Err(ToolError::ReadError);
        }

        // The moments are said again: the copy's own rate stays, so a
        // faster run is fewer frames said at the same rate, and a slower
        // one repeats what stands. Whatever a moment asks for is what
        // stands there, the last frame never going past its edge.
        let pace = percent as f64 / 100.0;
        let said = ((held as f64) / pace).round().max(1.0) as usize;
        let mut retimed = vec![0u8; said * frame_bytes];
        for moment in 0..said {
            let stand = (((moment as f64) * pace).floor() as usize).min(held - 1);
            retimed[moment * frame_bytes..(moment + 1) * frame_bytes]
                .copy_from_slice(&gathered[stand * frame_bytes..(stand + 1) * frame_bytes]);
        }
        let _ = std::fs::remove_file(&retime);
        std::fs::write(&retime, &retimed).map_err(|_| ToolError::WriteError)?;

        // The sound runs with the copy, said sample for sample the same
        // way the pictures are: whatever moment it stands at is what is
        // heard. Its pitch is not kept, deliberately: the tape runs.
        let mut audio_args: Vec<OsString> = Vec::new();
        if let Some(voice) = crate::media::audio::decode_full_pcm(&self.ffmpeg_tools, input) {
            let channels = (voice.channels as usize).max(1);
            let held_notes = voice.samples.len() / channels;
            if held_notes > 0 {
                let said_notes = ((held_notes as f64) / pace).round().max(1.0) as usize;
                let mut heard = Vec::with_capacity(said_notes * channels);
                for moment in 0..said_notes {
                    let stand = (((moment as f64) * pace).floor() as usize).min(held_notes - 1);
                    heard.extend_from_within(stand * channels..(stand + 1) * channels);
                }
                let sung = crate::media::audio::encode_wav_bytes(
                    &heard,
                    voice.sample_rate,
                    voice.channels,
                );
                let _ = std::fs::remove_file(&sound);
                std::fs::write(&sound, &sung).map_err(|_| ToolError::WriteError)?;
                audio_args = [
                    "-i",
                ]
                .iter()
                .map(|word| OsString::from(*word))
                .chain([sound.as_os_str().to_os_string()])
                .chain(
                    [
                        "-c:a",
                        "aac",
                    ]
                    .iter()
                    .map(|word| OsString::from(*word)),
                )
                .collect();
            }
        }

        let mut arguments: Vec<OsString> = [
            "-hide_banner",
            "-nostdin",
            "-y",
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgba",
            "-s",
        ]
        .iter()
        .map(|word| OsString::from(*word))
        .chain([OsString::from(format!("{width}x{height}"))])
        .chain(
            [
                "-framerate",
            ]
            .iter()
            .map(|word| OsString::from(*word))
            .chain([OsString::from(format!("{rate:.6}"))]),
        )
        .chain(["-i"].iter().map(|word| OsString::from(*word)))
        .chain([retime.as_os_str().to_os_string()])
        .collect();
        let silent = audio_args.is_empty();
        arguments.extend(audio_args);
        if silent {
            arguments.push(OsString::from("-an"));
        }
        arguments.extend(
            [
                "-c:v",
                "mpeg4",
                "-pix_fmt",
                "yuv420p",
                "-f",
            ]
            .iter()
            .map(|word| OsString::from(*word)),
        );
        arguments.push(OsString::from(source.write.muxer.clone()));
        arguments.push(temporary.as_os_str().to_os_string());
        let made = SystemRunner
            .run(&self.ffmpeg_tools.ffmpeg, &arguments, None)
            .map_err(|_| ToolError::ToolFailed)?;
        let _ = std::fs::remove_file(&retime);
        let _ = std::fs::remove_file(&sound);
        if !made.succeeded() {
            let _ = std::fs::remove_file(&temporary);
            return Err(ToolError::ToolFailed);
        }

        let finals = speed_final_names(input, &speed_saying(percent), &source_ext);
        self.publish(&finals, &temporary)
    }

    /// Joins several alike files into one: videos or PDFs.
    fn run_merge(&self, request: &Request, _runner: &dyn ProcessRunner) -> Outcome {
        if request.inputs.len() < 2 {
            return Outcome::Failed {
                reason: Failure::InvalidInput("at least two files are needed to merge".to_string()),
            };
        }
        let first = &request.inputs[0];
        let ext = first
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();

        let result = if ext == "pdf" {
            self.merge_pdfs(&request.inputs)
        } else {
            self.merge_videos(&request.inputs)
        };

        match result {
            Ok(output) => Outcome::Succeeded {
                outputs: vec![output],
            },
            Err(reason) => {
                let name = reason.name();
                Outcome::Failed {
                    reason: Failure::Error(format!("the files could not be merged: {name}")),
                }
            }
        }
    }

    /// Merges multiple PDF files into one, all pages copied in sequence.
    fn merge_pdfs(&self, inputs: &[PathBuf]) -> Result<PathBuf, ToolError> {
        let first = &inputs[0];
        let stem = first
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "document".to_string());
        let temporary = first.with_file_name(format!("{stem}.merge-part.pdf"));

        crate::media::pdf::merge(inputs, &temporary).map_err(|err| match err {
            crate::media::pdf::PdfError::LibraryMissing => ToolError::ToolFailed,
            crate::media::pdf::PdfError::NotAPdf => ToolError::ReadError,
            crate::media::pdf::PdfError::RenderFailed => ToolError::ReadError,
            crate::media::pdf::PdfError::TextFailed => ToolError::ReadError,
        })?;

        let dest_names = metadata_final_names(first, "merged", "pdf");
        self.publish(&dest_names, &temporary)
    }

    /// Assembles an organized PDF from pages and rotations specified in settings,
    /// or copies all pages if none were specified.
    fn run_organize_pdf(&self, request: &Request) -> Outcome {
        if request.inputs.is_empty() {
            return Outcome::Failed {
                reason: Failure::InvalidInput("at least one file is needed to organize".to_string()),
            };
        }
        let first = &request.inputs[0];
        let stem = first
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "document".to_string());
        let temporary = first.with_file_name(format!("{stem}.organize-part.pdf"));

        let specs: Vec<crate::media::pdf::PageSpec> = if request.settings.pdf_pages.is_empty() {
            // Default: all pages from all inputs in order, 0 rotation
            let mut all_specs = Vec::new();
            for (doc_idx, input) in request.inputs.iter().enumerate() {
                match crate::media::pdf::render_pages(input, 72, crate::media::pdf::Encoding::Png) {
                    Ok(rendered) => {
                        for page_idx in 0..rendered.len() {
                            all_specs.push(crate::media::pdf::PageSpec {
                                source_doc: doc_idx,
                                source_index: page_idx,
                                rotation_degrees: 0,
                            });
                        }
                    }
                    Err(err) => {
                        let name = match err {
                            crate::media::pdf::PdfError::LibraryMissing => ToolError::ToolFailed.name(),
                            _ => ToolError::ReadError.name(),
                        };
                        return Outcome::Failed {
                            reason: Failure::Error(format!("the file could not be read: {name}")),
                        };
                    }
                }
            }
            all_specs
        } else {
            request
                .settings
                .pdf_pages
                .iter()
                .map(|&(source_doc, source_index, rotation_degrees)| {
                    crate::media::pdf::PageSpec {
                        source_doc,
                        source_index,
                        rotation_degrees,
                    }
                })
                .collect()
        };

        if specs.is_empty() {
            return Outcome::Failed {
                reason: Failure::InvalidInput("the document would have no pages".to_string()),
            };
        }

        if let Err(err) = crate::media::pdf::assemble(&request.inputs, &specs, &temporary) {
            let name = match err {
                crate::media::pdf::PdfError::LibraryMissing => ToolError::ToolFailed.name(),
                _ => ToolError::ReadError.name(),
            };
            return Outcome::Failed {
                reason: Failure::Error(format!("the pages could not be organized: {name}")),
            };
        }

        let dest_names = metadata_final_names(first, "organized", "pdf");
        match self.publish(&dest_names, &temporary) {
            Ok(output) => Outcome::Succeeded {
                outputs: vec![output],
            },
            Err(reason) => {
                let name = reason.name();
                Outcome::Failed {
                    reason: Failure::Error(format!("the file could not be saved: {name}")),
                }
            }
        }
    }

    /// Joins multiple videos into one MP4 movie, clips scaled to fit the first clip.
    fn merge_videos(&self, inputs: &[PathBuf]) -> Result<PathBuf, ToolError> {
        let first = &inputs[0];
        let stem = first
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "video".to_string());

        let (raw_w, raw_h, raw_rate) = crate::media::frames::shape_of(&self.ffmpeg_tools, first)
            .ok_or(ToolError::ReadError)?;
        if raw_w == 0 || raw_h == 0 {
            return Err(ToolError::ReadError);
        }
        let target_w = (raw_w / 2) * 2;
        let target_h = (raw_h / 2) * 2;
        let target_fps = if raw_rate >= 10.0 && raw_rate <= 60.0 {
            raw_rate
        } else {
            30.0
        };

        let temp_video_raw = first.with_file_name(format!("{stem}.merge-video.bin"));
        let temp_audio_wav = first.with_file_name(format!("{stem}.merge-sound.wav"));
        let temp_output = first.with_file_name(format!("{stem}.merge-part.mp4"));

        let _ = std::fs::remove_file(&temp_video_raw);
        let _ = std::fs::remove_file(&temp_audio_wav);
        let _ = std::fs::remove_file(&temp_output);

        let mut raw_video_out = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&temp_video_raw)
            .map_err(|_| ToolError::WriteError)?;

        let mut all_audio_samples: Vec<i16> = Vec::new();

        for input in inputs {
            let (clip_w, clip_h, _) = crate::media::frames::shape_of(&self.ffmpeg_tools, input)
                .ok_or(ToolError::ReadError)?;
            if clip_w == 0 || clip_h == 0 {
                let _ = std::fs::remove_file(&temp_video_raw);
                return Err(ToolError::ReadError);
            }
            let clip_frame_bytes = (clip_w as usize) * (clip_h as usize) * 4;

            let clip_raw_path = input.with_file_name(format!("{stem}.clip-decode.bin"));
            let _ = std::fs::remove_file(&clip_raw_path);
            let clip_file = std::fs::File::create(&clip_raw_path).map_err(|_| ToolError::WriteError)?;

            let decode_args: Vec<OsString> = [
                "-hide_banner",
                "-nostdin",
                "-y",
                "-i",
            ]
            .iter()
            .map(|word| OsString::from(*word))
            .chain([input.as_os_str().to_os_string()])
            .chain(
                [
                    "-pix_fmt",
                    "rgba",
                    "-f",
                    "rawvideo",
                    "pipe:1",
                ]
                .iter()
                .map(|word| OsString::from(*word)),
            )
            .collect();

            let decode = SystemRunner
                .run(&self.ffmpeg_tools.ffmpeg, &decode_args, Some(&clip_file))
                .map_err(|_| ToolError::ToolFailed)?;
            if !decode.succeeded() {
                let _ = std::fs::remove_file(&clip_raw_path);
                let _ = std::fs::remove_file(&temp_video_raw);
                return Err(ToolError::ToolFailed);
            }

            let gathered = std::fs::read(&clip_raw_path).map_err(|_| ToolError::ReadError)?;
            let _ = std::fs::remove_file(&clip_raw_path);
            let clip_frames = gathered.len() / clip_frame_bytes;
            if clip_frames == 0 {
                let _ = std::fs::remove_file(&temp_video_raw);
                return Err(ToolError::ReadError);
            }

            if clip_w == target_w && clip_h == target_h {
                use std::io::Write;
                raw_video_out.write_all(&gathered).map_err(|_| ToolError::WriteError)?;
            } else {
                use std::io::Write;
                let scale_x = target_w as f64 / clip_w as f64;
                let scale_y = target_h as f64 / clip_h as f64;
                let scale = scale_x.min(scale_y);
                let fit_w = ((clip_w as f64 * scale).round() as u32).max(1).min(target_w);
                let fit_h = ((clip_h as f64 * scale).round() as u32).max(1).min(target_h);
                let offset_x = (target_w - fit_w) / 2;
                let offset_y = (target_h - fit_h) / 2;

                for f in 0..clip_frames {
                    let frame_slice = &gathered[f * clip_frame_bytes..(f + 1) * clip_frame_bytes];
                    let src_img = image::RgbaImage::from_raw(clip_w, clip_h, frame_slice.to_vec())
                        .ok_or(ToolError::ReadError)?;
                    let resized = image::imageops::resize(&src_img, fit_w, fit_h, image::imageops::FilterType::Nearest);
                    let mut canvas = image::RgbaImage::from_pixel(target_w, target_h, image::Rgba([0, 0, 0, 255]));
                    image::imageops::overlay(&mut canvas, &resized, offset_x as i64, offset_y as i64);
                    raw_video_out.write_all(canvas.as_raw()).map_err(|_| ToolError::WriteError)?;
                }
            }

            let clip_duration = crate::media::frames::duration_of(&self.ffmpeg_tools, input)
                .unwrap_or(clip_frames as f64 / target_fps);
            let target_audio_frames = (clip_duration * 48000.0).round() as usize;

            if let Some(voice) = crate::media::audio::decode_full_pcm(&self.ffmpeg_tools, input) {
                let channels = (voice.channels as usize).max(1);
                let src_audio_frames = voice.samples.len() / channels;
                if src_audio_frames > 0 {
                    let mut resampled_stereo = Vec::with_capacity(target_audio_frames * 2);
                    for i in 0..target_audio_frames {
                        let src_idx = (((i as f64) * (src_audio_frames as f64) / (target_audio_frames as f64)).floor() as usize).min(src_audio_frames - 1);
                        if channels == 1 {
                            let s = voice.samples[src_idx];
                            resampled_stereo.push(s);
                            resampled_stereo.push(s);
                        } else {
                            let l = voice.samples[src_idx * channels];
                            let r = voice.samples[src_idx * channels + 1];
                            resampled_stereo.push(l);
                            resampled_stereo.push(r);
                        }
                    }
                    all_audio_samples.extend(resampled_stereo);
                } else {
                    all_audio_samples.extend(vec![0i16; target_audio_frames * 2]);
                }
            } else {
                all_audio_samples.extend(vec![0i16; target_audio_frames * 2]);
            }
        }
        drop(raw_video_out);

        let mut encode_args: Vec<OsString> = [
            "-hide_banner",
            "-nostdin",
            "-y",
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgba",
            "-s",
        ]
        .iter()
        .map(|w| OsString::from(*w))
        .collect();

        encode_args.push(OsString::from(format!("{target_w}x{target_h}")));
        encode_args.push(OsString::from("-r"));
        encode_args.push(OsString::from(format!("{target_fps}")));
        encode_args.push(OsString::from("-i"));
        encode_args.push(temp_video_raw.as_os_str().to_os_string());

        let has_audio = !all_audio_samples.is_empty();
        if has_audio {
            let wav_bytes = crate::media::audio::encode_wav_bytes(&all_audio_samples, 48000, 2);
            std::fs::write(&temp_audio_wav, &wav_bytes).map_err(|_| ToolError::WriteError)?;
            encode_args.push(OsString::from("-i"));
            encode_args.push(temp_audio_wav.as_os_str().to_os_string());
        }

        encode_args.extend(
            [
                "-c:v",
                "mpeg4",
                "-q:v",
                "5",
            ]
            .iter()
            .map(|w| OsString::from(*w)),
        );

        if has_audio {
            encode_args.extend(
                [
                    "-c:a",
                    "aac",
                    "-b:a",
                    "192k",
                ]
                .iter()
                .map(|w| OsString::from(*w)),
            );
        }

        encode_args.push(OsString::from("-pix_fmt"));
        encode_args.push(OsString::from("yuv420p"));
        encode_args.push(OsString::from("-f"));
        encode_args.push(OsString::from("mov"));
        encode_args.push(temp_output.as_os_str().to_os_string());

        let encode = SystemRunner
            .run(&self.ffmpeg_tools.ffmpeg, &encode_args, None)
            .map_err(|_| ToolError::ToolFailed)?;

        let _ = std::fs::remove_file(&temp_video_raw);
        let _ = std::fs::remove_file(&temp_audio_wav);

        if !encode.succeeded() || !temp_output.is_file() {
            let _ = std::fs::remove_file(&temp_output);
            return Err(ToolError::ToolFailed);
        }

        let dest_names = metadata_final_names(first, "joined", "mp4");
        self.publish(&dest_names, &temp_output)
    }

    /// Makes files lighter, at the preset and the target size the window
    /// carried.
    ///
    /// Every kind answers with its own pair of hands, so the compress is
    /// run apart from the conversions the way a split is: a picture is
    /// made lighter here, by the program itself; a video and a sound go
    /// to the conversion program with a bitrate; a WebP goes to the
    /// conversion program too, because its quality is a dial only the
    /// program has. Each result is published beside the original, under
    /// the shared naming, and nothing is written over.
    fn run_compress(&self, request: &Request, runner: &dyn ProcessRunner) -> Outcome {
        let manifest = Manifest::get();
        let mut outputs = Vec::new();
        let mut failed = 0usize;
        let mut first_reason: Option<String> = None;
        for input in &request.inputs {
            let result = self.compress_one(input, &manifest, runner, &request.settings);
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

    /// Compresses one file, whichever kind it is.
    fn compress_one(
        &self,
        input: &Path,
        manifest: &Manifest,
        runner: &dyn ProcessRunner,
        settings: &crate::radial::execution::ToolSettings,
    ) -> Result<PathBuf, ToolError> {
        let source_ext = input
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();
        let source = manifest
            .format_of_extension(&source_ext)
            .or_else(|| manifest.format(&source_ext))
            .ok_or(ToolError::UnsupportedKind)?;
        let temporary = input.with_extension(format!("{source_ext}.part"));

        match crate::media::probe::kind_of(&source.format) {
            Some(crate::media::probe::Kind::Image) => match source.format.as_str() {
                // A JPEG is written again at a quality; a PNG is written
                // again as itself, made less only when the preset or a
                // target size asks.
                "jpeg" | "png" => {
                    compress_picture(input, &temporary, settings)?;
                    let finals = final_names(input, &source_ext);
                    self.publish(&finals, &temporary)
                }
                // A WebP is squeezed by the conversion program: its
                // encoder carries a quality the picture library does not.
                "webp" => {
                    match settings.target_bytes {
                        Some(bytes) if bytes > 0 => {
                            search_webp_quality(
                                input,
                                source,
                                &temporary,
                                &self.ffmpeg_tools,
                                runner,
                                bytes,
                            )?;
                            let finals = final_names(input, &source_ext);
                            self.publish(&finals, &temporary)
                        }
                        _ => {
                            let arguments = webp_args(
                                input,
                                source,
                                Some(&temporary),
                                webp_quality(settings.compress_strong),
                            );
                            let finals = final_names(input, &source_ext);
                            self.finish(&arguments, source, &temporary, &finals, runner)
                        }
                    }
                }
                _ => Err(ToolError::UnsupportedKind),
            },
            Some(crate::media::probe::Kind::Video) => {
                let duration = crate::media::frames::duration_of(&self.ffmpeg_tools, input)
                    .ok_or(ToolError::ReadError)?;
                let (width, height, fps) =
                    crate::media::frames::shape_of(&self.ffmpeg_tools, input)
                        .ok_or(ToolError::ReadError)?;
                let weight = fs::metadata(input).map(|meta| meta.len()).unwrap_or(0);
                let picture_bits = video_bitrate(
                    weight,
                    duration,
                    settings.compress_strong,
                    settings.target_bytes,
                );
                let sound_bits = audio_bitrate(settings.compress_strong);
                let filter = compress_video_filter(width, height, fps, settings);
                let arguments = video_compress_args(
                    input,
                    source,
                    Some(&temporary),
                    filter,
                    picture_bits,
                    sound_bits,
                );
                let finals = final_names(input, &source_ext);
                self.finish(&arguments, source, &temporary, &finals, runner)
            }
            Some(crate::media::probe::Kind::Audio) => {
                let duration = crate::media::frames::duration_of(&self.ffmpeg_tools, input)
                    .ok_or(ToolError::ReadError)?;
                let bits = match settings.target_bytes {
                    Some(bytes) if bytes > 0 => audio_bitrate_for_target(bytes, duration),
                    _ => audio_bitrate(settings.compress_strong),
                };
                let arguments = audio_compress_args(input, source, Some(&temporary), bits);
                let finals = final_names(input, &source_ext);
                self.finish(&arguments, source, &temporary, &finals, runner)
            }
            _ => Err(ToolError::UnsupportedKind),
        }
    }

    /// Edits or strips metadata on the files in the request.
    ///
    /// Video and audio copy their streams untouched with the changed tags
    /// set or stripped; pictures are written clean without EXIF/GPS/IPTC
    /// metadata. Each result is published beside the original, named
    /// "{base} clean.{ext}" when all tags were removed or "{base} edited.{ext}"
    /// when tags were written.
    fn run_metadata(&self, request: &Request, runner: &dyn ProcessRunner) -> Outcome {
        let manifest = Manifest::get();
        let mut outputs = Vec::new();
        let mut failed = 0usize;
        let mut first_reason: Option<String> = None;
        for input in &request.inputs {
            let result = self.metadata_one(input, &manifest, runner, &request.settings);
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

    /// Edits or strips metadata on one file.
    fn metadata_one(
        &self,
        input: &Path,
        manifest: &Manifest,
        runner: &dyn ProcessRunner,
        settings: &crate::radial::execution::ToolSettings,
    ) -> Result<PathBuf, ToolError> {
        let source_ext = input
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();
        let source = manifest
            .format_of_extension(&source_ext)
            .or_else(|| manifest.format(&source_ext))
            .ok_or(ToolError::UnsupportedKind)?;
        let stem = input
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "file".to_string());
        let temporary = input.with_file_name(format!("{stem}.meta-part.{source_ext}"));

        let suffix = if settings.metadata_remove_all {
            "clean"
        } else {
            "edited"
        };

        if source.format == "pdf" {
            let content = std::fs::read(input).map_err(|_| ToolError::ReadError)?;
            std::fs::write(&temporary, &content).map_err(|_| ToolError::WriteError)?;
            let finals = metadata_final_names(input, suffix, &source_ext);
            return self.publish(&finals, &temporary);
        }

        match crate::media::probe::kind_of(&source.format) {
            Some(crate::media::probe::Kind::Image) => {
                let img = image::open(input).map_err(|_| ToolError::ReadError)?;
                img.save(&temporary).map_err(|_| ToolError::WriteError)?;
                let finals = metadata_final_names(input, suffix, &source_ext);
                self.publish(&finals, &temporary)
            }
            Some(crate::media::probe::Kind::Video | crate::media::probe::Kind::Audio) => {
                let arguments = metadata_args(input, &temporary, Some(&source.write.muxer), settings);
                let run = runner
                    .run(&self.ffmpeg_tools.ffmpeg, &arguments, None)
                    .map_err(|_| ToolError::ToolFailed)?;
                if !run.succeeded() {
                    let _ = std::fs::remove_file(&temporary);
                    return Err(ToolError::ToolFailed);
                }
                let finals = metadata_final_names(input, suffix, &source_ext);
                self.publish(&finals, &temporary)
            }
            _ => Err(ToolError::UnsupportedKind),
        }
    }

    /// Normalizes loudness/volume on the files in the request.
    ///
    /// Reads PCM audio samples, computes peak amplitude and applies linear gain
    /// to meet the target true peak limit, writing a normalized file beside the original.
    fn run_normalize(&self, request: &Request, runner: &dyn ProcessRunner) -> Outcome {
        let manifest = Manifest::get();
        let mut outputs = Vec::new();
        let mut failed = 0usize;
        let mut first_reason: Option<String> = None;
        for input in &request.inputs {
            let result = self.normalize_one(input, &manifest, runner, &request.settings);
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

    /// Normalizes loudness on one audio file.
    fn normalize_one(
        &self,
        input: &Path,
        manifest: &Manifest,
        runner: &dyn ProcessRunner,
        settings: &crate::radial::execution::ToolSettings,
    ) -> Result<PathBuf, ToolError> {
        let source_ext = input
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();
        let source = manifest
            .format_of_extension(&source_ext)
            .or_else(|| manifest.format(&source_ext))
            .ok_or(ToolError::UnsupportedKind)?;

        if crate::media::probe::kind_of(&source.format) != Some(crate::media::probe::Kind::Audio) {
            return Err(ToolError::UnsupportedKind);
        }

        let pcm = crate::media::audio::decode_full_pcm(&self.ffmpeg_tools, input)
            .ok_or(ToolError::ReadError)?;

        let (normalized_samples, _gain) =
            crate::media::audio::normalize_samples(&pcm.samples, settings.true_peak());

        let temporary = input.with_extension(format!("{source_ext}.part"));

        if source_ext == "wav" {
            let wav_bytes = crate::media::audio::encode_wav_bytes(
                &normalized_samples,
                pcm.sample_rate,
                pcm.channels,
            );
            fs::write(&temporary, wav_bytes).map_err(|_| ToolError::WriteError)?;
        } else {
            let staging_wav = input.with_extension("staging_norm.wav");
            let wav_bytes = crate::media::audio::encode_wav_bytes(
                &normalized_samples,
                pcm.sample_rate,
                pcm.channels,
            );
            fs::write(&staging_wav, wav_bytes).map_err(|_| ToolError::WriteError)?;

            let mut args = vec![
                OsString::from("-hide_banner"),
                OsString::from("-nostdin"),
                OsString::from("-y"),
                OsString::from("-i"),
                staging_wav.as_os_str().to_os_string(),
                OsString::from("-vn"),
                OsString::from("-c:a"),
                OsString::from(&source.write.encoder),
            ];
            if !source.write.muxer.is_empty() {
                args.push(OsString::from("-f"));
                args.push(OsString::from(&source.write.muxer));
            }
            args.push(temporary.as_os_str().to_os_string());

            let run_res = runner.run(&self.ffmpeg_tools.ffmpeg, &args, None);
            let _ = fs::remove_file(&staging_wav);

            match run_res {
                Ok(status) if status.succeeded() => {}
                _ => {
                    let _ = fs::remove_file(&temporary);
                    return Err(ToolError::ToolFailed);
                }
            }
        }

        let finals = metadata_final_names(input, "normalized", &source_ext);
        self.publish(&finals, &temporary)
    }

    /// Converts audio channels on the files in the request.
    ///
    /// Reads PCM audio samples, converts channels according to the chosen mode
    /// (stereo, mono mixdown, left only, right only, swap) and gains, writing
    /// the converted file beside the original.
    fn run_channels(&self, request: &Request, runner: &dyn ProcessRunner) -> Outcome {
        let manifest = Manifest::get();
        let mut outputs = Vec::new();
        let mut failed = 0usize;
        let mut first_reason: Option<String> = None;
        for input in &request.inputs {
            let result = self.channels_one(input, &manifest, runner, &request.settings);
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

    /// Converts audio channels on one audio file.
    fn channels_one(
        &self,
        input: &Path,
        manifest: &Manifest,
        runner: &dyn ProcessRunner,
        settings: &crate::radial::execution::ToolSettings,
    ) -> Result<PathBuf, ToolError> {
        let source_ext = input
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();
        let source = manifest
            .format_of_extension(&source_ext)
            .or_else(|| manifest.format(&source_ext))
            .ok_or(ToolError::UnsupportedKind)?;

        if crate::media::probe::kind_of(&source.format) != Some(crate::media::probe::Kind::Audio) {
            return Err(ToolError::UnsupportedKind);
        }

        let pcm = crate::media::audio::decode_full_pcm(&self.ffmpeg_tools, input)
            .ok_or(ToolError::ReadError)?;

        let mode = settings.channel_mode();
        let left_gain = settings.left_gain_factor();
        let right_gain = settings.right_gain_factor();

        let converted = crate::media::audio::convert_audio_channels(
            &pcm,
            mode,
            left_gain,
            right_gain,
        );

        let temporary = input.with_extension(format!("{source_ext}.part"));

        if source_ext == "wav" {
            let wav_bytes = crate::media::audio::encode_wav_bytes(
                &converted.samples,
                converted.sample_rate,
                converted.channels,
            );
            fs::write(&temporary, wav_bytes).map_err(|_| ToolError::WriteError)?;
        } else {
            let staging_wav = input.with_extension("staging_chan.wav");
            let wav_bytes = crate::media::audio::encode_wav_bytes(
                &converted.samples,
                converted.sample_rate,
                converted.channels,
            );
            fs::write(&staging_wav, wav_bytes).map_err(|_| ToolError::WriteError)?;

            let mut args = vec![
                OsString::from("-hide_banner"),
                OsString::from("-nostdin"),
                OsString::from("-y"),
                OsString::from("-i"),
                staging_wav.as_os_str().to_os_string(),
                OsString::from("-vn"),
                OsString::from("-c:a"),
                OsString::from(&source.write.encoder),
            ];
            if !source.write.muxer.is_empty() {
                args.push(OsString::from("-f"));
                args.push(OsString::from(&source.write.muxer));
            }
            args.push(temporary.as_os_str().to_os_string());

            let run_res = runner.run(&self.ffmpeg_tools.ffmpeg, &args, None);
            let _ = fs::remove_file(&staging_wav);

            match run_res {
                Ok(status) if status.succeeded() => {}
                _ => {
                    let _ = fs::remove_file(&temporary);
                    return Err(ToolError::ToolFailed);
                }
            }
        }

        let suffix = if mode == "mono" { "mono" } else { "channels" };
        let finals = metadata_final_names(input, suffix, &source_ext);
        self.publish(&finals, &temporary)
    }
}

/// The arguments built for FFmpeg to edit or strip metadata.
pub(crate) fn metadata_args(
    input: &Path,
    output: &Path,
    muxer: Option<&str>,
    settings: &crate::radial::execution::ToolSettings,
) -> Vec<OsString> {
    let mut args = vec![
        OsString::from("-hide_banner"),
        OsString::from("-nostdin"),
        OsString::from("-y"),
        OsString::from("-i"),
        input.as_os_str().to_os_string(),
        OsString::from("-map"),
        OsString::from("0"),
        OsString::from("-c"),
        OsString::from("copy"),
    ];

    if let Some(muxer) = muxer {
        args.push(OsString::from("-f"));
        args.push(OsString::from(muxer));
    }

    if settings.metadata_remove_all {
        args.push(OsString::from("-map_metadata"));
        args.push(OsString::from("-1"));
        args.push(OsString::from("-map_chapters"));
        args.push(OsString::from("-1"));
    } else {
        args.push(OsString::from("-map_metadata"));
        args.push(OsString::from("0"));
        for (key, val) in &settings.metadata_tags {
            let trimmed = val.trim();
            args.push(OsString::from("-metadata"));
            args.push(OsString::from(format!("{key}={trimmed}")));
        }
        if settings.metadata_remove_sensitive {
            for tag in &["location", "location-eng", "author", "comment", "copyright"] {
                args.push(OsString::from("-metadata"));
                args.push(OsString::from(format!("{tag}=")));
            }
        }
    }

    args.push(output.as_os_str().to_os_string());
    args
}

/// The candidate file names for metadata output: `{stem} {suffix}.{ext}`,
/// `{stem} {suffix} (2).{ext}`, etc.
/// The names a result that wears the speed's own saying may take.
///
/// The name carries the speed it runs at: "clip at 2x.mp4" — and the
/// same one with a number after it when the name is taken, the way the
/// metadata's own names do.
fn speed_final_names(original: &Path, saying: &str, extension: &str) -> Vec<PathBuf> {
    let stem = original
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "file".to_string());
    let parent = original.parent().map(|p| p.to_path_buf());
    let first = match &parent {
        Some(p) => p.join(format!("{stem} at {saying}.{extension}")),
        None => PathBuf::from(format!("{stem} at {saying}.{extension}")),
    };
    let mut names = vec![first];
    for number in 2..1000 {
        let candidate = match &parent {
            Some(p) => p.join(format!("{stem} at {saying} ({number}).{extension}")),
            None => PathBuf::from(format!("{stem} at {saying} ({number}).{extension}")),
        };
        names.push(candidate);
    }
    names
}

/// The speed said the way a name says it: "2x" for twice as fast, and
/// the fractions said with a point, never a spare nothing after it.
fn speed_saying(percent: u32) -> String {
    let said = percent as f64 / 100.0;
    if (said - said.round()).abs() < 0.001 {
        format!("{}x", said.round() as u32)
    } else {
        format!("{said}x")
    }
}

fn metadata_final_names(original: &Path, suffix: &str, extension: &str) -> Vec<PathBuf> {
    let stem = original
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "file".to_string());
    let parent = original.parent().map(|p| p.to_path_buf());
    let first = match &parent {
        Some(p) => p.join(format!("{stem} {suffix}.{extension}")),
        None => PathBuf::from(format!("{stem} {suffix}.{extension}")),
    };
    let mut names = vec![first];
    for number in 2..1000 {
        let candidate = match &parent {
            Some(p) => p.join(format!("{stem} {suffix} ({number}).{extension}")),
            None => PathBuf::from(format!("{stem} {suffix} ({number}).{extension}")),
        };
        names.push(candidate);
    }
    names
}

/// Lays every covered rectangle of the settings on a picture's own pixels.
///
/// The rectangles land through their fractions of the picture's size, and
/// what hangs off an edge is cut away inside the picture. Areas paint in
/// the order they stand in the settings, so an overlap shows the last one
/// that was laid — the order the reference's list paints in, too.
fn redact_picture(
    picture: image::DynamicImage,
    settings: &crate::radial::execution::ToolSettings,
) -> image::DynamicImage {
    if settings.redact_areas.is_empty() {
        return picture;
    }
    let mut buf = picture.to_rgba8();
    let (width, height) = (buf.width(), buf.height());
    let effect = settings.redaction_effect(width.min(height));
    let color = settings.redaction_color();
    for area in &settings.redact_areas {
        let (x, y, w, h) = area.rect_on(width, height);
        if w == 0 || h == 0 {
            continue;
        }
        match settings.redaction_style() {
            RedactStyle::Solid => {
                let box_ = image::ImageBuffer::from_pixel(
                    w,
                    h,
                    image::Rgba([color[0], color[1], color[2], 255]),
                );
                paste(&mut buf, &box_, x, y);
            }
            RedactStyle::Blur => {
                let part = image::imageops::crop(&mut buf, x, y, w, h).to_image();
                let smeared = image::imageops::blur(&part, effect as f32);
                paste(&mut buf, &smeared, x, y);
            }
            RedactStyle::Pixelate => {
                let part = image::imageops::crop(&mut buf, x, y, w, h).to_image();
                let block = effect.max(2.0) as u32;
                let small_w = ((w as f64 / block as f64).ceil() as u32).max(1);
                let small_h = ((h as f64 / block as f64).ceil() as u32).max(1);
                let small = image::imageops::resize(
                    &part,
                    small_w,
                    small_h,
                    image::imageops::FilterType::Triangle,
                );
                let squares =
                    image::imageops::resize(&small, w, h, image::imageops::FilterType::Nearest);
                paste(&mut buf, &squares, x, y);
            }
        }
    }
    image::DynamicImage::ImageRgba8(buf)
}

/// Writes a patch over part of a buffer, pixels exactly as they are.
fn paste(
    buf: &mut image::ImageBuffer<image::Rgba<u8>, Vec<u8>>,
    patch: &image::ImageBuffer<image::Rgba<u8>, Vec<u8>>,
    at_x: u32,
    at_y: u32,
) {
    for dy in 0..patch.height() {
        for dx in 0..patch.width() {
            buf.put_pixel(at_x + dx, at_y + dy, *patch.get_pixel(dx, dy));
        }
    }
}

/// How loud the bleep tone sings, against the top of the sound: half.
///
/// The reference mixed its tone at no level that was written down; half
/// of full scale is heard plainly over speech without clipping against
/// a loud master, and the choice stands recorded with the notes.
const BLEEP_TONE_AMPLITUDE: f64 = 0.5;

/// Lays the bleep over one stretch of a sound.
///
/// Everything the stretch held is dropped first — nothing of the
/// original is left under the tone — and a sine at the asked frequency
/// is written in its place, the same tone in every channel the sound
/// carries. The stretch is said in frames: a frame is one instant of
/// the sound, whatever number of channels it arrives in.
pub(crate) fn bleep_over(
    samples: &mut [i16],
    channels: u16,
    start_frame: usize,
    end_frame: usize,
    sample_rate: u32,
    frequency: u32,
) {
    let stride = channels.max(1) as usize;
    for frame in start_frame..end_frame {
        let time = frame as f64 / sample_rate as f64;
        let tone = (BLEEP_TONE_AMPLITUDE
            * (std::f64::consts::TAU * frequency as f64 * time).sin()
            * i16::MAX as f64)
            .round()
            .clamp(i16::MIN as f64, i16::MAX as f64) as i16;
        for channel in 0..stride {
            samples[frame * stride + channel] = tone;
        }
    }
}

/// A stretch of time long enough for anything one video covers: a day.
///
/// A rectangle given no end covers until the picture stops, and the
/// program's `between` wants a number to stop at — this one is a day of
/// seconds, longer than a file a person redacts ever stands.
const ALWAYS_UNTIL_MS: u64 = 86_400_000;

/// Says milliseconds the way the program reads seconds: always three
/// decimals, so `1.5` can never be read as `1.05`.
fn seconds_said(ms: u64) -> String {
    format!("{}.{:03}", ms / 1000, ms % 1000)
}

/// The time rule one rectangle keeps to.
///
/// Nothing at all when the rectangle covers the whole way through;
/// otherwise the program's own between, told where the covering starts
/// and where it stops.
fn redact_window(area: &crate::radial::execution::RedactArea) -> String {
    match (area.start_ms, area.end_ms) {
        (None, None) => String::new(),
        _ => {
            let start = seconds_said(area.start_ms.unwrap_or(0));
            let end = seconds_said(area.end_ms.unwrap_or(ALWAYS_UNTIL_MS));
            format!(":enable='between(t,{start},{end})'")
        }
    }
}

/// The filter graph that lays every covered rectangle on a video.
///
/// The rectangles land in pixels through their fractions, in the order
/// they stand in the settings — an overlap shows the last one laid, the
/// way the picture tool paints them. The program carries no blur of its
/// own in this build, so a smeared part is the crop made small and laid
/// back large, and a pixelated part is the same thing back in squares:
/// laid large with the nearest-neighbor flag, which keeps hard edges.
///
/// The graph ends with `[vout]`, and only stages that cover something
/// of the picture take part. With nothing left, it refuses — the same
/// honesty the picture tool has.
fn redact_video_filter(
    settings: &crate::radial::execution::ToolSettings,
    width: u32,
    height: u32,
) -> Result<String, ToolError> {
    let mut parts: Vec<(
        &crate::radial::execution::RedactArea,
        u32,
        u32,
        u32,
        u32,
    )> = Vec::new();
    for area in &settings.redact_areas {
        let (x, y, w, h) = area.rect_on(width, height);
        if w > 0 && h > 0 {
            parts.push((area, x, y, w, h));
        }
    }
    if parts.is_empty() {
        return Err(ToolError::NothingToDo);
    }

    let color = settings.redaction_color();
    let hex = format!("0x{:02x}{:02x}{:02x}@1", color[0], color[1], color[2]);
    let effect = settings.redaction_effect(width.min(height));
    let mut graph = String::new();
    let mut flowing = String::from("0:v");
    for (stage, part) in parts.iter().enumerate() {
        let (area, x, y, w, h) = *part;
        let window = redact_window(area);
        let last = stage == parts.len() - 1;
        let out = if last { "vout".to_string() } else { format!("r{stage}") };
        match settings.redaction_style() {
            RedactStyle::Solid => {
                graph.push_str(&format!(
                    "[{flowing}]drawbox=x={x}:y={y}:w={w}:h={h}:color={hex}:t=fill{window}[{out}];",
                    flowing = flowing,
                    out = out,
                ));
                if !last {
                    flowing = format!("r{stage}");
                }
            }
            RedactStyle::Blur => {
                // the crop made small and laid back large: a smear by
                // the program's scale, painted over for its own time
                let small_w = (w as f64 / effect).round().max(1.0) as u32;
                let small_h = (h as f64 / effect).round().max(1.0) as u32;
                graph.push_str(&format!(
                    "[{flowing}]split=2[m{stage}][b{stage}];",
                    flowing = flowing,
                ));
                graph.push_str(&format!(
                    "[b{stage}]crop=x={x}:y={y}:w={w}:h={h},scale={small_w}:{small_h},scale={w}:{h}[sm{stage}];"
                ));
                graph.push_str(&format!(
                    "[m{stage}][sm{stage}]overlay=x={x}:y={y}{window}[{out}];",
                    out = out,
                ));
                if !last {
                    flowing = format!("r{stage}");
                }
            }
            RedactStyle::Pixelate => {
                let block = effect.max(2.0);
                let small_w = ((w as f64 / block).ceil() as u32).max(1);
                let small_h = ((h as f64 / block).ceil() as u32).max(1);
                graph.push_str(&format!(
                    "[{flowing}]split=2[m{stage}][b{stage}];",
                    flowing = flowing,
                ));
                graph.push_str(&format!(
                    "[b{stage}]crop=x={x}:y={y}:w={w}:h={h},scale={small_w}:{small_h},scale={w}:{h}:flags=neighbor[px{stage}];"
                ));
                graph.push_str(&format!(
                    "[m{stage}][px{stage}]overlay=x={x}:y={y}{window}[{out}];",
                    out = out,
                ));
                if !last {
                    flowing = format!("r{stage}");
                }
            }
        }
    }
    graph.pop(); // the last statement wants no semicolon after it
    Ok(graph)
}

/// The arguments that make a covered video: the picture made again in
/// its own container at a constant quality, sound carried as sound is
/// carried when a conversion makes a video file.
fn redact_video_args(
    input: &Path,
    source: &crate::media::manifest::FormatSupport,
    output_path: &Path,
    graph: &str,
) -> Vec<OsString> {
    let mut args = vec![
        OsString::from("-hide_banner"),
        OsString::from("-nostdin"),
        OsString::from("-y"),
        OsString::from("-f"),
        OsString::from(&source.read.demuxer),
        OsString::from("-i"),
        input.as_os_str().to_os_string(),
        OsString::from("-filter_complex"),
        OsString::from(graph),
        OsString::from("-map"),
        OsString::from("[vout]"),
        OsString::from("-map"),
        OsString::from("0:a?"),
        OsString::from("-c:v"),
        OsString::from(&source.write.encoder),
        OsString::from("-q:v"),
        OsString::from("5"),
        OsString::from("-c:a"),
        OsString::from("aac"),
        OsString::from("-b:a"),
        OsString::from("128000"),
        OsString::from("-f"),
        OsString::from(&source.write.muxer),
    ];
    args.push(output_path.as_os_str().to_os_string());
    args
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

/// The quality the two presets write a JPEG with, in the hundredths the
/// picture library takes: balanced keeps most of the look, strong takes
/// half of it away.
fn jpeg_quality(strong: bool) -> u8 {
    if strong {
        50
    } else {
        72
    }
}

/// The quality the two presets ask of the WebP encoder — the same two
/// answers, said the program's way.
fn webp_quality(strong: bool) -> u32 {
    if strong {
        50
    } else {
        72
    }
}

/// The bits a second a compressed sound is written with: the balanced
/// preset keeps twice what the strong one keeps.
fn audio_bitrate(strong: bool) -> u64 {
    if strong {
        64_000
    } else {
        128_000
    }
}

/// The bits a second a compressed video's picture is written with.
///
/// With no target size, the video's own weight is spread over its
/// seconds to estimate the bits it carries now, and the preset keeps a
/// share of that — the strong one a smaller share, never fewer than
/// three hundred thousand. With a target size, the sound takes its
/// share of the bits first, and the picture divides what is left over
/// the seconds the video stands, never fewer than a hundred and fifty
/// thousand. A video whose length could not be said takes the floor.
fn video_bitrate(weight: u64, duration: f64, strong: bool, target: Option<u64>) -> u64 {
    match (target, duration > 0.0) {
        (Some(bytes), true) if bytes > 0 => {
            let total_bits = bytes as f64 * 8.0 * 0.94;
            let sound_bits = audio_bitrate(strong) as f64;
            (total_bits / duration - sound_bits).max(150_000.0) as u64
        }
        (_, true) => {
            let estimated = weight as f64 * 8.0 / duration;
            let share = if strong { 0.35 } else { 0.6 };
            (estimated * share).max(300_000.0) as u64
        }
        _ => 300_000,
    }
}

/// The bits a second a compressed sound takes when a target size was
/// asked: the size is spread over the seconds, kept between thirty-two
/// and three hundred and twenty thousand. A sound whose length could not
/// be said keeps the balanced rate.
fn audio_bitrate_for_target(target: u64, duration: f64) -> u64 {
    if duration <= 0.0 {
        return 128_000;
    }
    let bits = target as f64 * 8.0 * 0.96 / duration;
    (bits.max(32_000.0).min(320_000.0)) as u64
}

/// The picture filter a compressed video carries, if it carries one.
///
/// The strong preset squeezes the long edge to three quarters, and a
/// resolution cap squeezes it further; whichever limit is smaller wins,
/// and a video is never made larger than it stood. A frame-rate cap
/// drops the rate only when the video shows more than it was asked to.
/// Both sides of a scale are even — the encoders take nothing else.
fn compress_video_filter(
    width: u32,
    height: u32,
    fps: f64,
    settings: &crate::radial::execution::ToolSettings,
) -> Option<String> {
    let mut filters: Vec<String> = Vec::new();
    let long_now = width.max(height) as f64;
    if long_now > 0.0 {
        let share = if settings.compress_strong {
            0.75
        } else {
            1.0
        };
        let mut limit = long_now * share;
        if let Some(cap) = settings.max_long_edge {
            if cap > 0 {
                limit = limit.min(cap as f64);
            }
        }
        if limit < long_now - 1.0 {
            let even = |edge: f64| ((edge / 2.0).round() as u32).max(1) * 2;
            if width >= height {
                filters.push(format!("scale={}:{}", even(limit), "-2"));
            } else {
                filters.push(format!("scale=-2:{}", even(limit)));
            }
        }
    }
    if let Some(cap) = settings.max_fps {
        if cap > 0 && fps > cap as f64 + 0.5 {
            filters.push(format!("fps={cap}"));
        }
    }
    if filters.is_empty() {
        None
    } else {
        Some(filters.join(","))
    }
}

/// The arguments that make a video lighter: the picture made again at
/// the bitrate the preset and the target decided, the sound with it at
/// the preset's rate — the sound is written as AAC, the encoder this
/// program always has. The streams keep the file's own container, the
/// way every tool here writes beside the original.
fn video_compress_args(
    input: &Path,
    source: &crate::media::manifest::FormatSupport,
    output_path: Option<&Path>,
    filter: Option<String>,
    picture_bits: u64,
    sound_bits: u64,
) -> Vec<OsString> {
    let mut args = vec![
        OsString::from("-hide_banner"),
        OsString::from("-nostdin"),
        OsString::from("-y"),
        OsString::from("-f"),
        OsString::from(&source.read.demuxer),
        OsString::from("-i"),
        input.as_os_str().to_os_string(),
    ];
    if let Some(filter) = filter {
        args.push(OsString::from("-vf"));
        args.push(OsString::from(filter));
    }
    args.push(OsString::from("-c:v"));
    args.push(OsString::from(&source.write.encoder));
    args.push(OsString::from("-b:v"));
    args.push(OsString::from(picture_bits.to_string()));
    args.push(OsString::from("-c:a"));
    args.push(OsString::from("aac"));
    args.push(OsString::from("-b:a"));
    args.push(OsString::from(sound_bits.to_string()));
    args.push(OsString::from("-f"));
    args.push(OsString::from(&source.write.muxer));
    match output_path {
        Some(path) => args.push(path.as_os_str().to_os_string()),
        None => args.push(OsString::from("pipe:1")),
    }
    args
}

/// The arguments that make a sound lighter: the sound made again at the
/// bitrate that was decided, in the container it came in.
fn audio_compress_args(
    input: &Path,
    source: &crate::media::manifest::FormatSupport,
    output_path: Option<&Path>,
    bits: u64,
) -> Vec<OsString> {
    let mut args = vec![
        OsString::from("-hide_banner"),
        OsString::from("-nostdin"),
        OsString::from("-y"),
        OsString::from("-f"),
        OsString::from(&source.read.demuxer),
        OsString::from("-i"),
        input.as_os_str().to_os_string(),
        OsString::from("-c:a"),
        OsString::from(&source.write.encoder),
        OsString::from("-b:a"),
        OsString::from(bits.to_string()),
        OsString::from("-f"),
        OsString::from(&source.write.muxer),
    ];
    match output_path {
        Some(path) => args.push(path.as_os_str().to_os_string()),
        None => args.push(OsString::from("pipe:1")),
    }
    args
}

/// The arguments that write a WebP at a quality: the picture made again
/// by the program's WebP encoder, which takes its quality out of a
/// hundred.
fn webp_args(
    input: &Path,
    source: &crate::media::manifest::FormatSupport,
    output_path: Option<&Path>,
    quality: u32,
) -> Vec<OsString> {
    let mut args = vec![
        OsString::from("-hide_banner"),
        OsString::from("-nostdin"),
        OsString::from("-y"),
        OsString::from("-f"),
        OsString::from(&source.read.demuxer),
        OsString::from("-i"),
        input.as_os_str().to_os_string(),
        OsString::from("-c:v"),
        OsString::from(&source.write.encoder),
        OsString::from("-q:v"),
        OsString::from(quality.to_string()),
        OsString::from("-frames:v"),
        OsString::from("1"),
        OsString::from("-f"),
        OsString::from(&source.write.muxer),
    ];
    match output_path {
        Some(path) => args.push(path.as_os_str().to_os_string()),
        None => args.push(OsString::from("pipe:1")),
    }
    args
}

/// Writes a JPEG into the temporary at a quality, in the hundredths the
/// picture library takes.
fn write_jpeg(
    picture: &image::DynamicImage,
    path: &Path,
    quality: u8,
) -> Result<(), ToolError> {
    let file = fs::File::create(path).map_err(|_| ToolError::WriteError)?;
    let encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(file, quality);
    picture
        .write_with_encoder(encoder)
        .map_err(|_| ToolError::WriteError)
}

/// A picture made a share smaller, its shape kept, never nothing.
fn scale_by(picture: &image::DynamicImage, share: f64) -> image::DynamicImage {
    let width = ((picture.width() as f64) * share).round().max(1.0) as u32;
    let height = ((picture.height() as f64) * share).round().max(1.0) as u32;
    picture.resize_exact(width, height, image::imageops::FilterType::Lanczos3)
}

/// A picture whose long edge stands at most where it is asked, its shape
/// kept. A picture that already fits is given back as it is.
fn shrink_long_edge(picture: &image::DynamicImage, limit: u32) -> image::DynamicImage {
    let long = picture.width().max(picture.height());
    if long == 0 || limit == 0 || long <= limit {
        return picture.clone();
    }
    picture.resize(limit, limit, image::imageops::FilterType::Lanczos3)
}

/// Makes a picture lighter with the program's own hands.
///
/// A JPEG is written again at the preset's quality, or at the quality a
/// target size searched out. A PNG is written again as itself — the
/// format keeps every colour it was given, so lighter means less
/// picture: the strong preset takes a quarter off the long edge, and a
/// target size keeps shrinking the picture by a fifth until it fits or
/// until there is almost nothing left to shrink. The picture stays the
/// kind of file it was — a format change is a conversion, not a
/// compressing.
fn compress_picture(
    input: &Path,
    temporary: &Path,
    settings: &crate::radial::execution::ToolSettings,
) -> Result<(), ToolError> {
    let extension = input
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    match extension.as_str() {
        "jpeg" | "jpg" => {
            let picture = image::open(input).map_err(|_| ToolError::ReadError)?;
            match settings.target_bytes {
                Some(bytes) if bytes > 0 => {
                    search_jpeg_quality(picture, temporary, bytes)
                }
                _ => write_jpeg(&picture, temporary, jpeg_quality(settings.compress_strong)),
            }
        }
        "png" => {
            let mut picture = image::open(input).map_err(|_| ToolError::ReadError)?;
            if settings.compress_strong && settings.target_bytes.is_none() {
                let long = picture.width().max(picture.height());
                picture = shrink_long_edge(&picture, (long as f64 * 0.7) as u32);
            }
            match settings.target_bytes {
                Some(bytes) if bytes > 0 => {
                    let mut candidate = picture;
                    let mut fits = false;
                    for _ in 0..12 {
                        candidate
                            .save_with_format(temporary, image::ImageFormat::Png)
                            .map_err(|_| ToolError::WriteError)?;
                        let size = fs::metadata(temporary)
                            .map(|meta| meta.len())
                            .unwrap_or(u64::MAX);
                        if size <= bytes {
                            fits = true;
                            break;
                        }
                        if candidate.width() < 2 || candidate.height() < 2 {
                            break;
                        }
                        candidate = scale_by(&candidate, 0.8);
                    }
                    if fits {
                        Ok(())
                    } else {
                        Err(ToolError::TooSmall)
                    }
                }
                _ => picture
                    .save_with_format(temporary, image::ImageFormat::Png)
                    .map_err(|_| ToolError::WriteError),
            }
        }
        _ => Err(ToolError::UnsupportedKind),
    }
}

/// Searches for the highest JPEG quality whose file fits the size that
/// was asked, the way the original searches: twelve writes at most,
/// closing in from both sides, and when the search closes without a
/// fit, the picture is made a fifth smaller and the search starts over.
/// The best quality that fitted is written last; when nothing fitted,
/// the last thing written stands.
fn search_jpeg_quality(
    mut candidate: image::DynamicImage,
    temporary: &Path,
    target: u64,
) -> Result<(), ToolError> {
    let mut low = 0.15f64;
    let mut high = 0.95f64;
    let mut best: Option<u8> = None;
    for _ in 0..12 {
        let quality = (low + high) / 2.0;
        let said = (quality * 100.0).round() as u8;
        write_jpeg(&candidate, temporary, said)?;
        let size = fs::metadata(temporary)
            .map(|meta| meta.len())
            .unwrap_or(u64::MAX);
        if size <= target {
            best = Some(said);
            low = quality;
            if (size as f64) > target as f64 * 0.9 {
                break;
            }
        } else {
            high = quality;
        }
        if high - low < 0.03 {
            if best.is_some() {
                break;
            }
            candidate = scale_by(&candidate, 0.8);
            low = 0.15;
            high = 0.95;
            if candidate.width() <= 64 {
                break;
            }
        }
    }
    if let Some(said) = best {
        write_jpeg(&candidate, temporary, said)?;
    }
    Ok(())
}

/// Runs the program once to write a WebP into the temporary at a
/// quality, and says how heavy the file came out.
fn run_webp_once(
    input: &Path,
    source: &crate::media::manifest::FormatSupport,
    temporary: &Path,
    tools: &FfmpegTools,
    runner: &dyn ProcessRunner,
    quality: u32,
) -> Result<u64, ToolError> {
    let arguments = webp_args(input, source, Some(temporary), quality);
    let size = if muxer_needs_file(&source.write.muxer) {
        let result = runner
            .run(&tools.ffmpeg, &arguments, None)
            .map_err(|_| ToolError::ToolFailed)?;
        if !result.succeeded() {
            return Err(ToolError::ToolFailed);
        }
        fs::metadata(temporary).map(|meta| meta.len()).unwrap_or(0)
    } else {
        let mut file = fs::File::create(temporary).map_err(|_| ToolError::WriteError)?;
        let result = runner
            .run(&tools.ffmpeg, &arguments, Some(&mut file))
            .map_err(|_| ToolError::ToolFailed)?;
        drop(file);
        if !result.succeeded() {
            return Err(ToolError::ToolFailed);
        }
        fs::metadata(temporary).map(|meta| meta.len()).unwrap_or(0)
    };
    Ok(size)
}

/// Searches for the WebP quality whose file fits the size that was
/// asked — twelve whole runs of the program at most, closing in from
/// both sides, the best quality that fitted kept — and leaves the
/// winner written in the temporary. When no quality fits, the smallest
/// file seen stands.
fn search_webp_quality(
    input: &Path,
    source: &crate::media::manifest::FormatSupport,
    temporary: &Path,
    tools: &FfmpegTools,
    runner: &dyn ProcessRunner,
    target: u64,
) -> Result<(), ToolError> {
    let mut low = 15u32;
    let mut high = 95u32;
    let mut best: Option<u32> = None;
    let mut smallest: Option<u32> = None;
    let mut smallest_size = u64::MAX;
    for _ in 0..12 {
        let quality = (low + high) / 2;
        let size = run_webp_once(input, source, temporary, tools, runner, quality)?;
        if size < smallest_size {
            smallest_size = size;
            smallest = Some(quality);
        }
        if size <= target {
            best = Some(quality);
            low = quality;
            if size > target / 10 * 9 {
                break;
            }
        } else {
            high = quality;
        }
        if high.saturating_sub(low) < 3 {
            break;
        }
    }
    let quality = best.or(smallest).ok_or(ToolError::ToolFailed)?;
    run_webp_once(input, source, temporary, tools, runner, quality)?;
    Ok(())
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

        // A snapshot is run apart from the rest: what it writes is a
        // folder of pictures beside the original, not one file under the
        // Beside policy, the way a split writes a folder of clips.
        if request.operation == "tools.snapshots" {
            return self.run_snapshots(request, &SystemRunner);
        }

        // A speed is run apart from the rest: the video's own moments are
        // counted, retimed, and said again under the video's own name.
        if request.operation == "tools.speed" {
            return self.run_speed(request, &SystemRunner);
        }

        // A compress is run apart from the rest: every kind answers with
        // its own pair of hands — a picture is made lighter by the
        // program itself, a video and a sound at a bitrate the preset and
        // the target size decide — and a PDF's pages are the reader's to
        // redraw, so nothing here pretends otherwise.
        if request.operation == "tools.compress" {
            return self.run_compress(request, &SystemRunner);
        }

        // A metadata edit copies streams or re-encodes clean, saving copies beside.
        if request.operation == "tools.metadata" {
            return self.run_metadata(request, &SystemRunner);
        }

        // A normalize decodes audio to PCM, scales samples to meet true peak, and publishes beside.
        if request.operation == "tools.normalize" {
            return self.run_normalize(request, &SystemRunner);
        }

        // A channels conversion decodes audio to PCM, converts channel matrix/gain, and publishes beside.
        if request.operation == "tools.channels" {
            return self.run_channels(request, &SystemRunner);
        }

        // A redaction covers the rectangles the window drew on with the
        // program's own hands — no conversion program takes part.
        if request.operation == "tools.redact_image" {
            return self.run_redact_image(request);
        }

        // A video covered the same way, only through the conversion
        // program: its filters draw the boxes and smear the parts on its
        // own, and the copy is made again in the container it came in.
        if request.operation == "tools.redact_video" {
            return self.run_redact_video(request, &SystemRunner);
        }

        // A sound covered the same way, closer to home: the sound is
        // read as numbers through the program, and the bleep is laid in
        // these hands — this build of the program carries no tone of its
        // own to mix in.
        if request.operation == "tools.bleep" {
            return self.run_bleep(request, &SystemRunner);
        }

        // A merge joins several alike files together into one: videos
        // concatenated into a single movie, or PDFs merged page by page.
        if request.operation == "tools.merge" {
            return self.run_merge(request, &SystemRunner);
        }

        // An organize reorders, duplicates, removes, or rotates pages of
        // one or more PDF documents into a newly assembled PDF.
        if request.operation == "tools.organize_pdf" {
            return self.run_organize_pdf(request);
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
    use crate::radial::execution::{BleepRange, RedactArea, RedactStyle, ToolSettings};

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

    #[test]
    fn the_preset_numbers_are_the_two_the_originals_keep() {
        // Balanced keeps most of the look; strong takes half of it away.
        // The same two answers, said each writer's way.
        assert_eq!(jpeg_quality(false), 72);
        assert_eq!(jpeg_quality(true), 50);
        assert_eq!(webp_quality(false), 72);
        assert_eq!(webp_quality(true), 50);
        assert_eq!(audio_bitrate(false), 128_000);
        assert_eq!(audio_bitrate(true), 64_000);
    }

    #[test]
    fn the_video_bits_come_from_the_weight_and_the_length() {
        // A megabyte over ten seconds carries eight hundred thousand bits
        // a second; balanced keeps six tenths of that.
        let estimated = video_bitrate(1_000_000, 10.0, false, None);
        assert_eq!(estimated, 480_000);
        // Strong keeps a smaller share — here, the floor.
        assert_eq!(video_bitrate(1_000_000, 10.0, true, None), 300_000);
        // A target size takes the sound's share first and divides the
        // rest over the seconds.
        let asked = video_bitrate(0, 10.0, false, Some(1_000_000));
        assert_eq!(asked, 1_000_000 * 8 * 94 / 100 / 10 - 128_000);
        // Both floors hold, and a video whose length could not be said
        // takes the floor too.
        assert_eq!(video_bitrate(0, 10.0, false, Some(1)), 150_000);
        assert_eq!(video_bitrate(1_000_000, 0.0, false, None), 300_000);
        assert_eq!(video_bitrate(0, 0.0, false, Some(100_000)), 300_000);
    }

    #[test]
    fn the_sound_bits_for_a_target_stay_between_the_two_walls() {
        assert_eq!(audio_bitrate_for_target(1_000_000, 10.0), 320_000);
        assert_eq!(audio_bitrate_for_target(1_000, 10.0), 32_000);
        assert_eq!(audio_bitrate_for_target(100_000, 10.0), 76_800);
        // A sound whose length could not be said keeps the balanced rate.
        assert_eq!(audio_bitrate_for_target(100_000, 0.0), 128_000);
    }

    #[test]
    fn the_compress_filter_squeezes_only_what_was_asked() {
        // Nothing asked, nothing carried.
        assert_eq!(
            compress_video_filter(1920, 1080, 30.0, &ToolSettings::default()),
            None
        );
        // The strong preset squeezes the long edge to three quarters,
        // evened, on whichever side the long edge stands.
        let strong = ToolSettings {
            compress_strong: true,
            ..Default::default()
        };
        assert_eq!(
            compress_video_filter(1920, 1080, 30.0, &strong).as_deref(),
            Some("scale=1440:-2")
        );
        assert_eq!(
            compress_video_filter(1080, 1920, 30.0, &strong).as_deref(),
            Some("scale=-2:1440")
        );
        // A resolution cap squeezes further when it is the smaller limit,
        // and a video is never made larger than it stood.
        let capped = ToolSettings {
            max_long_edge: Some(720),
            ..Default::default()
        };
        assert_eq!(
            compress_video_filter(1920, 1080, 30.0, &capped).as_deref(),
            Some("scale=720:-2")
        );
        assert_eq!(
            compress_video_filter(640, 360, 30.0, &capped),
            None
        );
        // A frame-rate cap drops the rate only when the video shows more
        // than it was asked to, and it rides behind the scale.
        let fast = ToolSettings {
            max_fps: Some(30),
            compress_strong: true,
            ..Default::default()
        };
        assert_eq!(
            compress_video_filter(1920, 1080, 60.0, &fast).as_deref(),
            Some("scale=1440:-2,fps=30")
        );
        assert_eq!(compress_video_filter(1920, 1080, 30.0, &fast).as_deref(), Some("scale=1440:-2"));
        assert_eq!(compress_video_filter(1920, 1080, 24.0, &fast).as_deref(), Some("scale=1440:-2"));
    }

    #[test]
    fn the_compress_args_say_what_the_program_should_make() {
        let mp4 = Manifest::get().format("mp4").expect("mp4 is in the manifest");
        let strong = ToolSettings {
            compress_strong: true,
            ..Default::default()
        };
        let filter = compress_video_filter(1920, 1080, 60.0, &strong);
        let args = video_compress_args(
            Path::new("one.mp4"),
            mp4,
            Some(Path::new("one.mp4.part")),
            filter,
            480_000,
            128_000,
        );
        assert_eq!(at(&args, "-i"), 5);
        assert_eq!(at(&args, "-vf"), 7);
        assert_eq!(args[8], OsString::from("scale=1440:-2"));
        let picture = at(&args, "-c:v");
        assert_eq!(args[picture + 1], OsString::from("mpeg4"), "the container's own encoder");
        let bits_at = at(&args, "-b:v");
        assert_eq!(args[bits_at + 1], OsString::from("480000"));
        let sound = at(&args, "-c:a");
        assert_eq!(args[sound + 1], OsString::from("aac"), "the sound is written as AAC");
        let tail = args.last().expect("somewhere for the result to go");
        assert_eq!(tail, &OsString::from("one.mp4.part"));
    }

    #[test]
    fn the_sound_args_carry_the_rate_they_were_given() {
        let mp3 = Manifest::get().format("mp3").expect("mp3 is in the manifest");
        let args = audio_compress_args(
            Path::new("song.mp3"),
            mp3,
            None,
            96_000,
        );
        assert_eq!(at(&args, "-c:a"), 7);
        assert_eq!(args[8], OsString::from("libmp3lame"));
        assert_eq!(at(&args, "-b:a"), 9);
        assert_eq!(args[10], OsString::from("96000"));
        assert_eq!(args.last(), Some(&OsString::from("pipe:1")));
    }

    #[test]
    fn the_webp_args_ask_the_programs_encoder_for_a_quality() {
        let webp = Manifest::get().format("webp").expect("webp is in the manifest");
        let args = webp_args(Path::new("one.webp"), webp, None, 72);
        assert_eq!(at(&args, "-c:v"), 7);
        assert_eq!(args[8], OsString::from("libwebp"));
        assert_eq!(at(&args, "-q:v"), 9);
        assert_eq!(args[10], OsString::from("72"));
        // One frame: a webp that carries a still is the one to make
        // lighter, and the encoder stops at the first.
        assert_eq!(at(&args, "-frames:v"), 11);
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
    fn trim_args_for_audio_stream_copies_the_range() {
        let mp3 = crate::media::manifest::Manifest::get()
            .format("mp3")
            .expect("mp3 is in manifest");
        let asked = ToolSettings {
            start_ms: Some(2_000),
            end_ms: Some(5_500),
            ..Default::default()
        };
        let args = trim_args(
            Path::new("track.mp3"),
            mp3,
            mp3,
            Some(Path::new("track.mp3.part")),
            &asked,
        );
        assert!(at(&args, "-ss") < at(&args, "-i"));
        assert!(at(&args, "-i") < at(&args, "-t"));
        assert_eq!(args[at(&args, "-ss") + 1].to_string_lossy(), "2.000");
        assert_eq!(args[at(&args, "-t") + 1].to_string_lossy(), "3.500");
        assert_eq!(args[at(&args, "-c") + 1].to_string_lossy(), "copy");
        assert_eq!(args[at(&args, "-map") + 1].to_string_lossy(), "0");
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

    #[test]
    #[ignore = "needs the conversion program"]
    fn the_moments_asked_for_stand_as_pictures_of_the_own_size() {
        let folder = Path::new("vendor").join("ffmpeg");
        let ffmpeg = folder.join("ffmpeg.exe");
        let ffprobe = folder.join("ffprobe.exe");
        if !ffmpeg.is_file() || !ffprobe.is_file() {
            eprintln!("the program is not there; this test needs it");
            return;
        }
        let scratch = std::env::temp_dir().join("open-tangerine-snapshots-test");
        std::fs::create_dir_all(&scratch).expect("the scratch folder was made");
        let input = make_test_video(&scratch);

        // The snapshots go the whole way through the handler, the way
        // the wheel drives it: the moments are said however the window
        // took them, out of order on purpose, and the pictures stand in
        // the video's own order.
        let tools = Tools {
            ffmpeg_tools: crate::media::locator::Tools {
                ffmpeg,
                ffprobe: ffprobe.clone(),
            },
            publisher: CopyPublisher,
        };
        let request = Request {
            operation: "tools.snapshots".to_string(),
            inputs: vec![input.clone()],
            settings: ToolSettings {
                snapshots_ms: vec![2_500, 0, 1_000],
                ..Default::default()
            },
            ..Default::default()
        };
        let outcome = tools.run(&request, &Selection::inspect(&[input.clone()]));
        let outputs = match outcome {
            Outcome::Succeeded { outputs } => outputs,
            Outcome::Failed { reason } => panic!("the snapshots failed: {:?}", reason),
            Outcome::Cancelled => panic!("the snapshots were not asked to stop"),
        };
        assert_eq!(outputs.len(), 3, "three moments asked, three pictures");
        let shots = outputs[0].parent().expect("the pictures have a folder");
        assert_eq!(
            shots.file_name().unwrap().to_string_lossy(),
            "clip frames",
            "the folder is named for the original"
        );
        assert_eq!(outputs[0].file_name().unwrap().to_string_lossy(), "clip frame 1.png");
        assert_eq!(outputs[1].file_name().unwrap().to_string_lossy(), "clip frame 2.png");
        assert_eq!(outputs[2].file_name().unwrap().to_string_lossy(), "clip frame 3.png");
        // Every picture is a picture: it carries the PNG's own opening
        // words, and it is as wide and as high as the video itself.
        for made in &outputs {
            let bytes = std::fs::read(made).expect("the picture was written");
            assert_eq!(
                &bytes[..8],
                &[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a],
                "the picture is a PNG"
            );
            let opened = image::load_from_memory(&bytes).expect("the picture opened");
            assert_eq!((opened.width(), opened.height()), (160, 120));
        }

        // A moment past the video's end is set down, and the taking of
        // the rest still stands. The second taking's folder does not
        // step on the first's.
        let request = Request {
            operation: "tools.snapshots".to_string(),
            inputs: vec![input.clone()],
            settings: ToolSettings {
                snapshots_ms: vec![0, 6_000],
                ..Default::default()
            },
            ..Default::default()
        };
        let outcome = tools.run(&request, &Selection::inspect(&[input.clone()]));
        let outputs = match outcome {
            Outcome::Succeeded { outputs } => outputs,
            Outcome::Failed { reason } => panic!("the later snapshots failed: {:?}", reason),
            Outcome::Cancelled => panic!("the snapshots were not asked to stop"),
        };
        assert_eq!(
            outputs.len(),
            1,
            "one frame of the two asked for was inside the video"
        );
        assert_eq!(
            outputs[0]
                .parent()
                .expect("the pictures have a folder")
                .file_name()
                .unwrap()
                .to_string_lossy(),
            "clip frames 2",
            "the second taking stands beside the first"
        );
        let _ = std::fs::remove_dir_all(&scratch);
    }

    #[test]
    fn a_snapshot_run_without_moments_refuses() {
        let tools = Tools {
            ffmpeg_tools: crate::media::locator::Tools {
                ffmpeg: PathBuf::from("nowhere.exe"),
                ffprobe: PathBuf::from("nowhere-probe.exe"),
            },
            publisher: CopyPublisher,
        };
        let request = Request {
            operation: "tools.snapshots".to_string(),
            settings: ToolSettings::default(),
            ..Default::default()
        };
        let outcome = tools.run(&request, &Selection::inspect(&[]));
        let Outcome::Failed { reason } = outcome else {
            panic!("an empty asking should not succeed");
        };
        assert_eq!(
            reason,
            Failure::InvalidInput("there are no snapshots".to_string()),
            "the refusal says what was not asked"
        );
    }

    #[test]
    #[ignore = "needs the conversion program"]
    fn a_video_runs_at_the_speed_it_was_asked_for() {
        let folder = Path::new("vendor").join("ffmpeg");
        let ffmpeg = folder.join("ffmpeg.exe");
        let ffprobe = folder.join("ffprobe.exe");
        if !ffmpeg.is_file() || !ffprobe.is_file() {
            eprintln!("the program is not there; this test needs it");
            return;
        }
        let scratch = std::env::temp_dir().join("open-tangerine-speed-test");
        std::fs::create_dir_all(&scratch).expect("the scratch folder was made");
        let input = make_test_video(&scratch);

        // The copy goes the whole way through the handler, the way the
        // wheel drives it: twice as fast runs half as long, at the same
        // rate of frames, and the sides are what they were.
        let tools = Tools {
            ffmpeg_tools: crate::media::locator::Tools {
                ffmpeg,
                ffprobe: ffprobe.clone(),
            },
            publisher: CopyPublisher,
        };
        let request = Request {
            operation: "tools.speed".to_string(),
            inputs: vec![input.clone()],
            settings: ToolSettings {
                speed_percent: Some(200),
                ..Default::default()
            },
            ..Default::default()
        };
        let outcome = tools.run(&request, &Selection::inspect(&[input.clone()]));
        let outputs = match outcome {
            Outcome::Succeeded { outputs } => outputs,
            Outcome::Failed { reason } => panic!("the fast copy failed: {:?}", reason),
            Outcome::Cancelled => panic!("the run was not asked to stop"),
        };
        assert_eq!(outputs.len(), 1, "one video in, one copy out");
        assert_eq!(
            outputs[0].file_name().unwrap().to_string_lossy(),
            "clip at 2x.mp4",
            "the copy carries the speed it runs at"
        );
        let runs = crate::media::frames::duration_of(&tools.ffmpeg_tools, &outputs[0])
            .expect("the fast copy said its own length");
        assert!(
            (runs - 1.5).abs() < 0.15,
            "the fast copy runs half the time: {runs}"
        );
        let (width, height, rate) =
            crate::media::frames::shape_of(&tools.ffmpeg_tools, &outputs[0])
                .expect("the fast copy said its own shape");
        assert_eq!((width, height), (160, 120), "the sides stand as they were");
        assert!(
            (rate - 10.0).abs() < 0.5,
            "the frames stand at the video's own rate"
        );

        // Half speed stands the other way: the copy runs twice as long.
        let request = Request {
            operation: "tools.speed".to_string(),
            inputs: vec![input.clone()],
            settings: ToolSettings {
                speed_percent: Some(50),
                ..Default::default()
            },
            ..Default::default()
        };
        let outcome = tools.run(&request, &Selection::inspect(&[input.clone()]));
        let outputs = match outcome {
            Outcome::Succeeded { outputs } => outputs,
            Outcome::Failed { reason } => panic!("the slow copy failed: {:?}", reason),
            Outcome::Cancelled => panic!("the run was not asked to stop"),
        };
        assert_eq!(
            outputs[0].file_name().unwrap().to_string_lossy(),
            "clip at 0.5x.mp4",
            "the fraction is said with a point"
        );
        let runs = crate::media::frames::duration_of(&tools.ffmpeg_tools, &outputs[0])
            .expect("the slow copy said its own length");
        assert!(
            (runs - 6.0).abs() < 0.15,
            "the slow copy runs twice the time: {runs}"
        );
        let _ = std::fs::remove_dir_all(&scratch);
    }

    #[test]
    fn a_speed_without_a_speed_refuses() {
        let tools = Tools {
            ffmpeg_tools: crate::media::locator::Tools {
                ffmpeg: PathBuf::from("nowhere.exe"),
                ffprobe: PathBuf::from("nowhere-probe.exe"),
            },
            publisher: CopyPublisher,
        };
        let request = Request {
            operation: "tools.speed".to_string(),
            settings: ToolSettings::default(),
            ..Default::default()
        };
        let outcome = tools.run(&request, &Selection::inspect(&[]));
        let Outcome::Failed { reason } = outcome else {
            panic!("an asking without a speed should not succeed");
        };
        assert_eq!(
            reason,
            Failure::InvalidInput("there is no speed to run at".to_string()),
            "the refusal says what was not said"
        );
    }

    #[test]
    #[ignore = "needs the conversion program to run a real audio trim"]
    fn real_audio_trim_with_the_program() {
        let Some(tools) = crate::media::locator::find() else {
            return;
        };
        let ffmpeg = &tools.ffmpeg;
        let scratch = std::env::temp_dir().join(format!("ot-audio-trim-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&scratch);

        // Generate a 3-second test WAV file
        let input = scratch.join("three.wav");
        let pcm = make_wav_bytes(3.0, 44100);
        std::fs::write(&input, pcm).expect("three.wav written");

        let wav = crate::media::manifest::Manifest::get()
            .format("wav")
            .expect("wav is in the manifest");
        let trimmed = scratch.join("one.wav");
        let asked = ToolSettings {
            start_ms: Some(1_000),
            end_ms: Some(2_500),
            ..Default::default()
        };
        let arguments = trim_args(&input, wav, wav, Some(&trimmed), &asked);
        let run = SystemRunner
            .run(ffmpeg, &arguments, None)
            .expect("the audio trim ran");
        assert!(run.succeeded(), "the audio trim did not finish: {}", run.stderr);
        let kept = crate::media::frames::duration_of(&tools, &trimmed)
            .expect("the trimmed audio has a length");
        assert!(
            (kept - 1.5).abs() < 0.25,
            "the audio trim kept {kept}s of audio, wanted about 1.5s"
        );
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
    fn an_area_lands_on_the_pixels_of_its_rectangle() {
        let area = RedactArea {
            x_mille: 250,
            y_mille: 500,
            w_mille: 500,
            h_mille: 500,
            start_ms: None,
            end_ms: None,
        };
        assert_eq!(area.rect_on(80, 60), (20, 30, 40, 30));
        // what hangs off the right edge is cut away
        let wide = RedactArea {
            x_mille: 900,
            y_mille: 500,
            w_mille: 500,
            h_mille: 500,
            start_ms: None,
            end_ms: None,
        };
        assert_eq!(wide.rect_on(80, 60), (72, 30, 8, 30));
        // at the very edge there is nothing left of the rectangle
        let edge = RedactArea { x_mille: 1000, ..wide };
        assert_eq!(edge.rect_on(80, 60), (80, 30, 0, 30));
    }

    #[test]
    fn the_redact_look_answers_the_reference_defaults() {
        let settings = ToolSettings::default();
        assert_eq!(settings.redaction_style(), RedactStyle::Solid);
        assert_eq!(settings.redaction_strength_fraction(), 0.5);
        assert_eq!(settings.redaction_color(), [0, 0, 0]);
        // the reference's own rule: the strength takes the look between
        // 2% and 30% of the smaller side
        let quiet = ToolSettings {
            redact_strength: Some(0),
            ..ToolSettings::default()
        };
        let loud = ToolSettings {
            redact_strength: Some(100),
            ..ToolSettings::default()
        };
        assert!((quiet.redaction_effect(1_000) - 20.0).abs() < 1e-9);
        assert!((loud.redaction_effect(1_000) - 300.0).abs() < 1e-9);
        // the hex is read as three bytes
        let colored = ToolSettings {
            redact_color: Some("ff5a1f".to_string()),
            ..ToolSettings::default()
        };
        assert_eq!(colored.redaction_color(), [255, 90, 31]);
    }

    #[test]
    fn a_redaction_without_areas_is_refused_and_said_so() {
        let tools = Tools {
            ffmpeg_tools: crate::media::locator::Tools {
                ffmpeg: PathBuf::from("ffmpeg"),
                ffprobe: PathBuf::from("ffprobe"),
            },
            publisher: CopyPublisher,
        };
        let request = Request {
            operation: "tools.redact_image".to_string(),
            inputs: vec![PathBuf::from("photo.png")],
            settings: ToolSettings::default(),
            ..Default::default()
        };
        let outcome = tools.run(&request, &Selection::inspect(&[PathBuf::from("photo.png")]));
        match outcome {
            Outcome::Failed {
                reason: Failure::InvalidInput(message),
            } => {
                assert_eq!(message, "there are no redaction areas");
            }
            other => panic!("the redaction was not refused for saying nothing: {other:?}"),
        }
    }

    #[test]
    #[ignore = "needs the picture library and nothing more"]
    fn a_picture_is_redacted_with_the_programs_own_hands() {
        let scratch = std::env::temp_dir().join("open-tangerine-redact-image-test");
        std::fs::create_dir_all(&scratch).expect("the scratch folder was made");
        let input = scratch.join("photo.png");
        let mut picture = image::DynamicImage::new_rgb8(64, 48);
        for (_, _, tone) in picture.as_mut_rgb8().expect("rgb8").enumerate_pixels_mut() {
            *tone = image::Rgb([255, 255, 255]);
        }
        picture.save(&input).expect("the picture was made");

        let tools = Tools {
            ffmpeg_tools: crate::media::locator::Tools {
                ffmpeg: PathBuf::from("ffmpeg"),
                ffprobe: PathBuf::from("ffprobe"),
            },
            publisher: CopyPublisher,
        };
        let request = Request {
            operation: "tools.redact_image".to_string(),
            inputs: vec![input.clone()],
            settings: ToolSettings {
                redact_areas: vec![RedactArea {
                    x_mille: 250,
                    y_mille: 250,
                    w_mille: 500,
                    h_mille: 500,
                    start_ms: None,
                    end_ms: None,
                }],
                ..Default::default()
            },
            ..Default::default()
        };
        let outcome = tools.run(&request, &Selection::inspect(&[input.clone()]));
        let outputs = match outcome {
            Outcome::Succeeded { outputs } => outputs,
            Outcome::Failed { reason } => panic!("the redaction failed: {reason:?}"),
            Outcome::Cancelled => panic!("the redaction was not asked to stop"),
        };
        let made = outputs.first().expect("the redaction is there");
        assert!(made.is_file(), "the covered copy exists");
        assert_eq!(
            made.file_name().and_then(|n| n.to_str()),
            Some("photo redacted.png")
        );
        let read = image::open(made).expect("the copy reads back").to_rgba8();
        assert_eq!(
            read.get_pixel(32, 24),
            &image::Rgba([0, 0, 0, 255]),
            "the middle of the box wears the color it was asked"
        );
        assert_eq!(
            read.get_pixel(2, 2),
            &image::Rgba([255, 255, 255, 255]),
            "what was not covered is as it was"
        );
        let _ = std::fs::remove_dir_all(&scratch);
    }

    #[test]
    #[ignore = "needs the picture library and nothing more"]
    fn a_blurred_redaction_smears_what_it_covers_and_nothing_else() {
        let scratch = std::env::temp_dir().join("open-tangerine-redact-blur-test");
        std::fs::create_dir_all(&scratch).expect("the scratch folder was made");
        let input = scratch.join("photo.png");
        let mut picture = image::DynamicImage::new_rgb8(64, 48);
        for (_, _, tone) in picture.as_mut_rgb8().expect("rgb8").enumerate_pixels_mut() {
            *tone = image::Rgb([255, 255, 255]);
        }
        // a black square of the picture's own middle: the thing to smear
        for y in 18..30 {
            for x in 24..40 {
                picture.as_mut_rgb8().expect("rgb8").put_pixel(
                    x,
                    y,
                    image::Rgb([0, 0, 0]),
                );
            }
        }
        picture.save(&input).expect("the picture was made");

        let tools = Tools {
            ffmpeg_tools: crate::media::locator::Tools {
                ffmpeg: PathBuf::from("ffmpeg"),
                ffprobe: PathBuf::from("ffprobe"),
            },
            publisher: CopyPublisher,
        };
        let request = Request {
            operation: "tools.redact_image".to_string(),
            inputs: vec![input.clone()],
            settings: ToolSettings {
                redact_style: Some(RedactStyle::Blur),
                redact_areas: vec![RedactArea {
                    x_mille: 250,
                    y_mille: 250,
                    w_mille: 500,
                    h_mille: 500,
                    start_ms: None,
                    end_ms: None,
                }],
                ..Default::default()
            },
            ..Default::default()
        };
        let outcome = tools.run(&request, &Selection::inspect(&[input.clone()]));
        let outputs = match outcome {
            Outcome::Succeeded { outputs } => outputs,
            Outcome::Failed { reason } => panic!("the redaction failed: {reason:?}"),
            Outcome::Cancelled => panic!("the redaction was not asked to stop"),
        };
        let made = outputs.first().expect("the redaction is there");
        let read = image::open(made).expect("the copy reads back").to_rgba8();
        let middle = read.get_pixel(32, 24).0;
        assert_ne!(middle[0], 0, "the smeared middle is not the black it was");
        assert_ne!(middle[0], 255, "the smeared middle is not white either");
        assert_eq!(
            read.get_pixel(2, 2),
            &image::Rgba([255, 255, 255, 255]),
            "far outside the area nothing changed"
        );
        assert_eq!(
            read.get_pixel(10, 40),
            &image::Rgba([255, 255, 255, 255]),
            "just outside the area nothing changed"
        );
        let _ = std::fs::remove_dir_all(&scratch);
    }

    #[test]
    #[ignore = "needs the picture library and nothing more"]
    fn a_pixelated_redaction_blocks_what_it_covers_and_nothing_else() {
        let scratch = std::env::temp_dir().join("open-tangerine-redact-block-test");
        std::fs::create_dir_all(&scratch).expect("the scratch folder was made");
        let input = scratch.join("photo.png");
        let mut picture = image::DynamicImage::new_rgb8(64, 48);
        for (_, _, tone) in picture.as_mut_rgb8().expect("rgb8").enumerate_pixels_mut() {
            *tone = image::Rgb([255, 255, 255]);
        }
        // a black square inside the area, its edge crossing a block: the
        // block that straddles it comes out mixed, the middle keeps the
        // secret's dark
        for y in 18..30 {
            for x in 24..40 {
                picture.as_mut_rgb8().expect("rgb8").put_pixel(
                    x,
                    y,
                    image::Rgb([0, 0, 0]),
                );
            }
        }
        picture.save(&input).expect("the picture was made");

        let tools = Tools {
            ffmpeg_tools: crate::media::locator::Tools {
                ffmpeg: PathBuf::from("ffmpeg"),
                ffprobe: PathBuf::from("ffprobe"),
            },
            publisher: CopyPublisher,
        };
        let request = Request {
            operation: "tools.redact_image".to_string(),
            inputs: vec![input.clone()],
            settings: ToolSettings {
                redact_style: Some(RedactStyle::Pixelate),
                redact_areas: vec![RedactArea {
                    x_mille: 250,
                    y_mille: 250,
                    w_mille: 500,
                    h_mille: 500,
                    start_ms: None,
                    end_ms: None,
                }],
                ..Default::default()
            },
            ..Default::default()
        };
        let outcome = tools.run(&request, &Selection::inspect(&[input.clone()]));
        let outputs = match outcome {
            Outcome::Succeeded { outputs } => outputs,
            Outcome::Failed { reason } => panic!("the redaction failed: {reason:?}"),
            Outcome::Cancelled => panic!("the redaction was not asked to stop"),
        };
        let made = outputs.first().expect("the redaction is there");
        let read = image::open(made).expect("the copy reads back").to_rgba8();
        // the middle sits inside a block that is still the secret's
        // black: the downsampling lets a little of the page's white
        // bleed in, but it stays far from the page
        let middle = read.get_pixel(32, 24).0;
        assert!(
            middle[0] < 128,
            "the middle of the secret stays dark, saw {}", middle[0]
        );
        // the straddling block mixes the secret with its page
        let straddling = read.get_pixel(27, 18).0;
        assert_ne!(straddling[0], 0, "the straddling block is not all black");
        assert_ne!(
            straddling[0], 255,
            "the straddling block is not all page"
        );
        assert_eq!(
            read.get_pixel(2, 2),
            &image::Rgba([255, 255, 255, 255]),
            "outside the area nothing changed"
        );
        let _ = std::fs::remove_dir_all(&scratch);
    }

    #[test]
    fn the_redact_video_graph_draws_what_time_says() {
        // Two solid boxes: one for its own stretch of time, one for the
        // whole way through.
        let settings = ToolSettings {
            redact_style: Some(RedactStyle::Solid),
            redact_color: Some("ff0000".to_string()),
            redact_areas: vec![
                RedactArea {
                    x_mille: 0,
                    y_mille: 0,
                    w_mille: 500,
                    h_mille: 500,
                    start_ms: Some(1_000),
                    end_ms: Some(2_000),
                },
                RedactArea {
                    x_mille: 500,
                    y_mille: 500,
                    w_mille: 500,
                    h_mille: 500,
                    start_ms: None,
                    end_ms: None,
                },
            ],
            ..ToolSettings::default()
        };
        let graph = redact_video_filter(&settings, 80, 60).expect("the graph built");
        // the first box stands from one second to two
        assert!(
            graph.contains(
                "drawbox=x=0:y=0:w=40:h=30:color=0xff0000@1:t=fill:enable='between(t,1.000,2.000)'[r0];"
            ),
            "the ranged box is not drawn the way the program reads it: {graph}"
        );
        // the second covers the whole way through, so no time rule, and
        // its stream is the one the graph hands out
        assert!(
            graph.contains("[r0]drawbox=x=40:y=30:w=40:h=30:color=0xff0000@1:t=fill[vout]"),
            "the whole-way box is not drawn: {graph}"
        );
        assert!(!graph.contains("boxblur"), "no blur filter is borrowed: {graph}");
    }

    #[test]
    fn the_redact_video_graph_smears_and_blocks_with_what_the_program_has() {
        // This build of the program carries no blur filter at all (the
        // real test of the program proves the parts), so the smear is
        // the crop made small and laid back large, and the squares are
        // the same thing lifted with the nearest-neighbor flag.
        let settings = ToolSettings {
            redact_style: Some(RedactStyle::Blur),
            redact_areas: vec![RedactArea {
                x_mille: 250,
                y_mille: 250,
                w_mille: 500,
                h_mille: 500,
                start_ms: None,
                end_ms: None,
            }],
            ..ToolSettings::default()
        };
        let graph = redact_video_filter(&settings, 80, 60).expect("the graph built");
        // the crop is the rectangle; the effect is 16% of the smaller
        // side (9.6 of 60), so the small copy is four by three
        assert!(
            graph.contains(
                "crop=x=20:y=15:w=40:h=30,scale=4:3,scale=40:30[sm0];"
            ),
            "the smear is not made the way it was decided: {graph}"
        );
        assert!(
            graph.contains("[m0][sm0]overlay=x=20:y=15[vout]"),
            "the smear is not laid back where it came from: {graph}"
        );
        assert!(!graph.contains("boxblur"), "no missing filter is asked for: {graph}");

        let settings = ToolSettings {
            redact_style: Some(RedactStyle::Pixelate),
            ..settings
        };
        let graph = redact_video_filter(&settings, 80, 60).expect("the graph built");
        assert!(
            graph.contains(
                "crop=x=20:y=15:w=40:h=30,scale=5:4,scale=40:30:flags=neighbor[px0];"
            ),
            "the squares are not cut: {graph}"
        );
        assert!(!graph.contains("pixelize"), "no missing filter is asked for: {graph}");
    }

    #[test]
    fn a_video_covered_by_nothing_says_something() {
        // A rectangle at the very edge covers no pixel of the picture.
        let settings = ToolSettings {
            redact_areas: vec![RedactArea {
                x_mille: 1000,
                y_mille: 1000,
                w_mille: 500,
                h_mille: 500,
                start_ms: None,
                end_ms: None,
            }],
            ..ToolSettings::default()
        };
        let outcome = redact_video_filter(&settings, 80, 60);
        assert_eq!(outcome, Err(ToolError::NothingToDo));
    }

    #[test]
    fn the_redact_video_args_carry_the_graph_and_the_maps() {
        let source = crate::media::manifest::Manifest::get()
            .format("mp4")
            .expect("mp4 is in the manifest");
        let arguments = redact_video_args(
            Path::new("in.mp4"),
            &source,
            Path::new("in.redact-part.mp4"),
            "[0:v]drawbox=x=0:y=0:w=1:h=1:color=0x000000@1:t=fill[vout]",
        );
        let words: Vec<String> = arguments
            .iter()
            .map(|word| word.to_string_lossy().into_owned())
            .collect();
        let graph_at = words
            .iter()
            .position(|w| w == "-filter_complex")
            .expect("the graph was asked for");
        assert!(
            words[graph_at + 1].contains("drawbox"),
            "the graph rides in one word: {}", words[graph_at + 1]
        );
        let out_at = words
            .iter()
            .position(|word| word == "[vout]")
            .expect("the covered stream is mapped");
        assert_eq!(words[out_at - 1], "-map", "the map names the stream");
        assert!(
            words.iter().any(|word| word == "0:a?"),
            "the sound is mapped when it is there"
        );
        let quality_at = words
            .iter()
            .position(|word| word == "-q:v")
            .expect("the picture quality was set");
        assert_eq!(words[quality_at + 1], "5");
    }

    #[test]
    fn the_bleep_tone_sings_at_the_asked_pitch() {
        // A thousand samples a second and two hundred and fifty hertz:
        // one tone wave every four frames, at half of full scale.
        let mut samples = vec![0i16; 4];
        bleep_over(&mut samples, 1, 0, 4, 1000, 250);
        assert_eq!(
            samples,
            vec![0, 16384, 0, -16384],
            "the tone is a half-scale sine at the asked pitch"
        );
    }

    #[test]
    fn the_bleep_replaces_what_it_stands_over_and_nothing_else() {
        let mut samples = vec![7i16; 12];
        bleep_over(&mut samples, 1, 1, 3, 1000, 250);
        assert_eq!(
            samples,
            vec![
                7, 16384, 0, //
                7, 7, 7, 7, 7, 7, 7, 7, 7,
            ],
            "the tone stands over its stretch and nothing else"
        );
    }

    #[test]
    fn the_bleep_tone_sings_the_same_in_every_channel() {
        // Two channels of one frame at a quarter of the tone's wave:
        // both ears hear the same instant.
        let mut samples = vec![0i16; 2];
        bleep_over(&mut samples, 2, 0, 1, 1000, 250);
        assert_eq!(
            samples,
            vec![0, 0],
            "frame nought stands at the wave's own nought"
        );

        let mut samples = vec![0i16; 4];
        bleep_over(&mut samples, 2, 1, 2, 1000, 250);
        assert_eq!(samples, vec![0, 0, 16384, 16384]);
    }

    #[test]
    fn the_bleep_frequency_is_clamped_where_the_slider_stands() {
        assert_eq!(ToolSettings::default().bleep_frequency_hz(), 1000);
        assert_eq!(
            ToolSettings {
                bleep_frequency: Some(200),
                ..ToolSettings::default()
            }
            .bleep_frequency_hz(),
            300
        );
        assert_eq!(
            ToolSettings {
                bleep_frequency: Some(5000),
                ..ToolSettings::default()
            }
            .bleep_frequency_hz(),
            2000
        );
    }

    #[test]
    fn a_bleep_off_the_end_of_the_sound_is_no_bleep_at_all() {
        let range = BleepRange {
            start_ms: 3000,
            end_ms: 9000,
        };
        // what hangs off the end is cut away, nothing more
        assert_eq!(range.frame_span(8000, 1000), (3000, 8000));
        // a range past the end covers nothing
        let range = BleepRange {
            start_ms: 9000,
            end_ms: 9500,
        };
        assert_eq!(range.frame_span(8000, 1000), (8000, 8000));
    }

    #[test]
    fn a_sound_with_no_bleeps_is_refused_and_said_so() {
        let tools = Tools {
            ffmpeg_tools: crate::media::locator::Tools {
                ffmpeg: PathBuf::from("ffmpeg"),
                ffprobe: PathBuf::from("ffprobe"),
            },
            publisher: CopyPublisher,
        };
        let request = Request {
            operation: "tools.bleep".to_string(),
            inputs: vec![PathBuf::from("tone.wav")],
            settings: ToolSettings::default(),
            ..Default::default()
        };
        let outcome = tools.run(&request, &Selection::inspect(&[PathBuf::from("tone.wav")]));
        match outcome {
            Outcome::Failed {
                reason: Failure::InvalidInput(message),
            } => {
                assert_eq!(message, "there are no bleep ranges");
            }
            other => panic!("the bleep was not refused for saying nothing: {other:?}"),
        }
    }

    #[test]
    #[ignore = "needs the conversion program"]
    fn a_sound_is_bleeped_the_way_the_ranges_mark() {
        let ffmpeg = Path::new("vendor").join("ffmpeg").join("ffmpeg.exe");
        if !ffmpeg.is_file() {
            eprintln!("the program is not there; this test needs it");
            return;
        }
        let scratch = std::env::temp_dir().join("open-tangerine-bleep-test");
        std::fs::create_dir_all(&scratch).expect("the scratch folder was made");
        let input = scratch.join("tone.wav");

        // One second of a dull constant tone in both ears: eight
        // thousand frames of five thousands.
        let flat = vec![5000i16; 16000];
        let wav = crate::media::audio::encode_wav_bytes(&flat, 8000, 2);
        std::fs::write(&input, wav).expect("tone.wav written");

        let tools = Tools {
            ffmpeg_tools: crate::media::locator::Tools {
                ffmpeg,
                ffprobe: Path::new("vendor").join("ffmpeg").join("ffprobe.exe"),
            },
            publisher: CopyPublisher,
        };
        let request = Request {
            operation: "tools.bleep".to_string(),
            inputs: vec![input.clone()],
            settings: ToolSettings {
                // a fifth of a second to half a second: the tone sings
                // there, at the frequency nobody moved (a thousand)
                bleep_ranges: vec![BleepRange {
                    start_ms: 200,
                    end_ms: 500,
                }],
                ..ToolSettings::default()
            },
            ..Default::default()
        };
        let outcome = tools.run(&request, &Selection::inspect(&[input.clone()]));
        let outputs = match outcome {
            Outcome::Succeeded { outputs } => outputs,
            Outcome::Failed { reason } => panic!("the bleep failed: {reason:?}"),
            Outcome::Cancelled => panic!("the bleep was not asked to stop"),
        };
        let made = outputs.first().expect("the bleeped copy is there");
        assert!(made.is_file(), "the bleeped copy exists");
        assert_eq!(
            made.file_name().and_then(|n| n.to_str()),
            Some("tone bleeped.wav")
        );

        // read it back: what the bleep stood over sings at half scale,
        // and what it stood away from is exactly what was there
        let bytes = std::fs::read(made).expect("the bleeped copy reads");
        let (samples, rate) = crate::media::audio::parse_wav_samples(&bytes).expect("parse wav");
        assert_eq!(rate, 8000, "the sound keeps its rate");
        assert_eq!(samples.len(), 16000, "the sound keeps its length");
        assert_eq!(samples[0], 5000, "before the bleep nothing changed");
        assert_eq!(samples[15999], 5000, "after the bleep nothing changed");
        let start = 2 * 1600; // two channels, two hundred milliseconds in
        let end = 2 * 4000; // two channels, half a second in
        let peak = samples[start..end]
            .iter()
            .map(|&s| (s as i32).abs())
            .max()
            .unwrap_or(0);
        assert!(
            (peak - 16384).abs() <= 1,
            "the tone stands half of full scale, saw {peak}"
        );
        assert_ne!(
            samples[start],
            5000,
            "the start of the bleep is no longer the sound it was"
        );
        // both ears hear the same instant of the tone
        assert_eq!(samples[start], samples[start + 1]);

        let _ = std::fs::remove_dir_all(&scratch);
    }

    #[test]
    fn a_video_redaction_without_areas_is_refused_and_said_so() {
        let tools = Tools {
            ffmpeg_tools: crate::media::locator::Tools {
                ffmpeg: PathBuf::from("ffmpeg"),
                ffprobe: PathBuf::from("ffprobe"),
            },
            publisher: CopyPublisher,
        };
        let request = Request {
            operation: "tools.redact_video".to_string(),
            inputs: vec![PathBuf::from("clip.mp4")],
            settings: ToolSettings::default(),
            ..Default::default()
        };
        let outcome = tools.run(&request, &Selection::inspect(&[PathBuf::from("clip.mp4")]));
        match outcome {
            Outcome::Failed {
                reason: Failure::InvalidInput(message),
            } => {
                assert_eq!(message, "there are no redaction areas");
            }
            other => panic!("the redaction was not refused for saying nothing: {other:?}"),
        }
    }

    #[test]
    #[ignore = "needs the conversion program"]
    fn a_video_is_redacted_the_way_the_styles_ask() {
        let folder = Path::new("vendor").join("ffmpeg");
        let ffmpeg = folder.join("ffmpeg.exe");
        let ffprobe = folder.join("ffprobe.exe");
        if !ffmpeg.is_file() || !ffprobe.is_file() {
            eprintln!("the program is not there; this test needs it");
            return;
        }
        let scratch = std::env::temp_dir().join("open-tangerine-redact-video-test");
        std::fs::create_dir_all(&scratch).expect("the scratch folder was made");
        let input = make_test_video(&scratch);

        // A smear over the left half the whole way, over the right half
        // the whole way, and over the middle strip for one second only —
        // three covers in one graph, one style asked (the way the
        // reference asks a style of the whole batch, not of each box).
        let tools = Tools {
            ffmpeg_tools: crate::media::locator::Tools {
                ffmpeg: ffmpeg.clone(),
                ffprobe: ffprobe.clone(),
            },
            publisher: CopyPublisher,
        };
        let request = Request {
            operation: "tools.redact_video".to_string(),
            inputs: vec![input.clone()],
            settings: ToolSettings {
                redact_style: Some(RedactStyle::Blur),
                redact_areas: vec![
                    RedactArea {
                        x_mille: 0,
                        y_mille: 0,
                        w_mille: 500,
                        h_mille: 1000,
                        start_ms: None,
                        end_ms: None,
                    },
                    RedactArea {
                        x_mille: 500,
                        y_mille: 0,
                        w_mille: 500,
                        h_mille: 1000,
                        start_ms: None,
                        end_ms: None,
                    },
                    RedactArea {
                        x_mille: 300,
                        y_mille: 400,
                        w_mille: 400,
                        h_mille: 200,
                        start_ms: Some(1_000),
                        end_ms: Some(2_000),
                    },
                ],
                ..ToolSettings::default()
            },
            ..Default::default()
        };
        let outcome = tools.run(&request, &Selection::inspect(&[input.clone()]));
        let outputs = match outcome {
            Outcome::Succeeded { outputs } => outputs,
            Outcome::Failed { reason } => panic!("the redaction failed: {reason:?}"),
            Outcome::Cancelled => panic!("the redaction was not asked to stop"),
        };
        let made = outputs.first().expect("the redaction is there");
        assert!(made.is_file(), "the covered copy exists");
        assert_eq!(
            made.file_name().and_then(|n| n.to_str()),
            Some("clip redacted.mp4")
        );
        let size = video_size(&ffprobe, made);
        assert_eq!(size, (160, 120), "the covered video keeps its shape");
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

    /// A picture with detail in it — noise, so no encoder gets it for
    /// free — written as a JPEG at the quality the test asks for.
    fn make_noisy_jpeg(scratch: &Path, name: &str, quality: u8) -> PathBuf {
        let mut picture = image::DynamicImage::new_rgb8(320, 240);
        let mut seed = 0x2545F491u32;
        for (x, y, pixel) in picture.as_mut_rgb8().expect("rgb").enumerate_pixels_mut() {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let (x, y) = (x as u8, y as u8);
            *pixel = image::Rgb([seed as u8, (seed >> 8) as u8 ^ x, (seed >> 16) as u8 ^ y]);
        }
        let input = scratch.join(name);
        write_jpeg(&picture, &input, quality).expect("the picture was made");
        input
    }

    #[test]
    #[ignore = "needs the picture library and nothing more"]
    fn a_jpeg_is_made_lighter_at_the_preset() {
        let scratch = std::env::temp_dir().join("open-tangerine-compress-jpeg-test");
        let _ = std::fs::remove_dir_all(&scratch);
        std::fs::create_dir_all(&scratch).expect("the scratch folder was made");
        // The original is written at the top quality, so the balanced
        // preset has room to make it lighter.
        let input = make_noisy_jpeg(&scratch, "photo.jpg", 100);
        let weight = std::fs::metadata(&input).expect("the original weighs").len();

        let tools = Tools {
            ffmpeg_tools: crate::media::locator::Tools {
                ffmpeg: PathBuf::from("ffmpeg"),
                ffprobe: PathBuf::from("ffprobe"),
            },
            publisher: CopyPublisher,
        };
        let request = Request {
            operation: "tools.compress".to_string(),
            inputs: vec![input.clone()],
            settings: ToolSettings::default(),
            ..Default::default()
        };
        let outcome = tools.run(&request, &Selection::inspect(&[input.clone()]));
        let outputs = match outcome {
            Outcome::Succeeded { outputs } => outputs,
            Outcome::Failed { reason } => panic!("the compress failed: {:?}", reason),
            Outcome::Cancelled => panic!("the compress was not asked to stop"),
        };
        assert_eq!(outputs.len(), 1, "one file in, one file out");
        let made = outputs.first().expect("the compress is there");
        assert!(made.is_file(), "the lighter file exists");
        assert_ne!(made, &input, "the original stands untouched");
        let again = image::open(made).expect("the result is a picture");
        assert_eq!((again.width(), again.height()), (320, 240), "the shape is kept");
        let lighter = std::fs::metadata(made).expect("the result weighs").len();
        assert!(lighter < weight, "{lighter} should be under {weight}");
        let _ = std::fs::remove_dir_all(&scratch);
    }

    #[test]
    #[ignore = "needs the picture library and nothing more"]
    fn a_jpeg_searches_for_the_quality_that_fits() {
        let scratch = std::env::temp_dir().join("open-tangerine-compress-target-test");
        let _ = std::fs::remove_dir_all(&scratch);
        std::fs::create_dir_all(&scratch).expect("the scratch folder was made");
        let input = make_noisy_jpeg(&scratch, "photo.jpg", 100);

        let tools = Tools {
            ffmpeg_tools: crate::media::locator::Tools {
                ffmpeg: PathBuf::from("ffmpeg"),
                ffprobe: PathBuf::from("ffprobe"),
            },
            publisher: CopyPublisher,
        };
        let target = std::fs::metadata(&input).expect("the original weighs").len() / 2;
        let request = Request {
            operation: "tools.compress".to_string(),
            inputs: vec![input.clone()],
            settings: ToolSettings {
                target_bytes: Some(target),
                ..Default::default()
            },
            ..Default::default()
        };
        let outcome = tools.run(&request, &Selection::inspect(&[input.clone()]));
        let outputs = match outcome {
            Outcome::Succeeded { outputs } => outputs,
            Outcome::Failed { reason } => panic!("the search failed: {:?}", reason),
            Outcome::Cancelled => panic!("the search was not asked to stop"),
        };
        let made = outputs.first().expect("the compress is there");
        let weight = std::fs::metadata(made).expect("the result weighs").len();
        assert!(
            weight <= target,
            "{weight} should fit under the asked {target}"
        );
        let _ = std::fs::remove_dir_all(&scratch);
    }

    #[test]
    #[ignore = "needs the picture library and nothing more"]
    fn a_picture_asked_for_an_impossible_size_says_so() {
        let scratch = std::env::temp_dir().join("open-tangerine-compress-small-test");
        let _ = std::fs::remove_dir_all(&scratch);
        std::fs::create_dir_all(&scratch).expect("the scratch folder was made");
        let input = scratch.join("photo.png");
        image::DynamicImage::new_rgb8(400, 300)
            .save(&input)
            .expect("the picture was made");

        let tools = Tools {
            ffmpeg_tools: crate::media::locator::Tools {
                ffmpeg: PathBuf::from("ffmpeg"),
                ffprobe: PathBuf::from("ffprobe"),
            },
            publisher: CopyPublisher,
        };
        // A picture keeps being made less until there is almost nothing
        // left of it; sixty-four bytes is not a size, it is a wish.
        let request = Request {
            operation: "tools.compress".to_string(),
            inputs: vec![input.clone()],
            settings: ToolSettings {
                target_bytes: Some(64),
                ..Default::default()
            },
            ..Default::default()
        };
        let outcome = tools.run(&request, &Selection::inspect(&[input.clone()]));
        match outcome {
            Outcome::Failed { reason } => {
                assert!(
                    reason.message().contains("smaller than the file can be made"),
                    "the failure says what went wrong: {:?}",
                    reason
                );
            }
            Outcome::Succeeded { .. } => panic!("an impossible size was honoured"),
            Outcome::Cancelled => panic!("the compress was not asked to stop"),
        }
        let _ = std::fs::remove_dir_all(&scratch);
    }

    #[test]
    #[ignore = "needs the conversion program"]
    fn a_video_is_made_lighter_at_a_target_size() {
        let folder = Path::new("vendor").join("ffmpeg");
        let ffmpeg = folder.join("ffmpeg.exe");
        let ffprobe = folder.join("ffprobe.exe");
        if !ffmpeg.is_file() || !ffprobe.is_file() {
            eprintln!("the program is not there; this test needs it");
            return;
        }
        let scratch = std::env::temp_dir().join("open-tangerine-compress-video-test");
        let _ = std::fs::remove_dir_all(&scratch);
        std::fs::create_dir_all(&scratch).expect("the scratch folder was made");
        // A video with detail in it — noise, so the encoder has to spend
        // real bits on it, and a target of half its weight stands above
        // the floors the bitrate keeps.
        let raw = scratch.join("noise.raw");
        let mut seed = 0x2545F491u32;
        let mut pixels = Vec::with_capacity(160 * 120 * 30 * 3);
        for _ in 0..160 * 120 * 30 {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            pixels.extend_from_slice(&[seed as u8, (seed >> 8) as u8, (seed >> 16) as u8]);
        }
        std::fs::write(&raw, &pixels).expect("the pixels could be written");
        let input = scratch.join("clip.mp4");
        let making: Vec<OsString> = [
            "-hide_banner",
            "-nostdin",
            "-y",
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgb24",
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
        let made_video = SystemRunner
            .run(&ffmpeg, &making, None)
            .expect("the video was made");
        assert!(made_video.succeeded(), "the test video was not made: {}", made_video.stderr);
        let input = input;
        let weight = std::fs::metadata(&input).expect("the original weighs").len();

        let tools = Tools {
            ffmpeg_tools: crate::media::locator::Tools {
                ffmpeg,
                ffprobe: ffprobe.clone(),
            },
            publisher: CopyPublisher,
        };
        let request = Request {
            operation: "tools.compress".to_string(),
            inputs: vec![input.clone()],
            settings: ToolSettings {
                target_bytes: Some(weight / 2),
                ..Default::default()
            },
            ..Default::default()
        };
        let outcome = tools.run(&request, &Selection::inspect(&[input.clone()]));
        let outputs = match outcome {
            Outcome::Succeeded { outputs } => outputs,
            Outcome::Failed { reason } => panic!("the compress failed: {:?}", reason),
            Outcome::Cancelled => panic!("the compress was not asked to stop"),
        };
        let made = outputs.first().expect("the compress is there").clone();
        assert!(made.is_file(), "the lighter video exists");
        let (w, h) = video_size(&ffprobe, &made);
        assert_eq!((w, h), (160, 120), "the shape is kept");
        let lighter = std::fs::metadata(&made).expect("the result weighs").len();
        assert!(
            lighter < weight,
            "{lighter} should be under the original's {weight}"
        );
        let _ = std::fs::remove_dir_all(&scratch);
    }

    #[test]
    #[ignore = "needs the conversion program"]
    fn a_sound_is_made_lighter_at_the_strong_preset() {
        let folder = Path::new("vendor").join("ffmpeg");
        let ffmpeg = folder.join("ffmpeg.exe");
        if !ffmpeg.is_file() {
            eprintln!("the program is not there; this test needs it");
            return;
        }
        let scratch = std::env::temp_dir().join("open-tangerine-compress-audio-test");
        let _ = std::fs::remove_dir_all(&scratch);
        std::fs::create_dir_all(&scratch).expect("the scratch folder was made");
        // A second of a tone, written as an MP3 at the balanced rate.
        let wav = scratch.join("tone.wav");
        let rate: u32 = 44100;
        let data_len: u32 = rate * 2;
        let mut wav_bytes = Vec::with_capacity(44 + data_len as usize);
        wav_bytes.extend_from_slice(b"RIFF");
        wav_bytes.extend_from_slice(&(36 + data_len).to_le_bytes());
        wav_bytes.extend_from_slice(b"WAVEfmt ");
        wav_bytes.extend_from_slice(&16u32.to_le_bytes());
        wav_bytes.extend_from_slice(&1u16.to_le_bytes());
        wav_bytes.extend_from_slice(&1u16.to_le_bytes());
        wav_bytes.extend_from_slice(&rate.to_le_bytes());
        wav_bytes.extend_from_slice(&(rate * 2).to_le_bytes());
        wav_bytes.extend_from_slice(&2u16.to_le_bytes());
        wav_bytes.extend_from_slice(&16u16.to_le_bytes());
        wav_bytes.extend_from_slice(b"data");
        wav_bytes.extend_from_slice(&data_len.to_le_bytes());
        for i in 0..rate {
            let sample: i16 = if (i / 50) % 2 == 0 { 10000 } else { -10000 };
            wav_bytes.extend_from_slice(&sample.to_le_bytes());
        }
        std::fs::write(&wav, wav_bytes).expect("the tone could be written");
        let input = scratch.join("tone.mp3");
        let making: Vec<OsString> = [
            "-hide_banner",
            "-nostdin",
            "-y",
            "-f",
            "wav",
            "-i",
        ]
        .iter()
        .map(|word| OsString::from(*word))
        .chain([
            wav.into_os_string(),
            OsString::from("-c:a"),
            OsString::from("libmp3lame"),
            OsString::from("-b:a"),
            OsString::from("128k"),
            OsString::from("-f"),
            OsString::from("mp3"),
            input.as_os_str().to_os_string(),
        ])
        .collect();
        let made = SystemRunner.run(&ffmpeg, &making, None).expect("the tone was made");
        assert!(made.succeeded(), "the test sound was not made: {}", made.stderr);
        let weight = std::fs::metadata(&input).expect("the original weighs").len();

        let tools = Tools {
            ffmpeg_tools: crate::media::locator::Tools {
                ffmpeg,
                ffprobe: Path::new("vendor").join("ffmpeg").join("ffprobe.exe"),
            },
            publisher: CopyPublisher,
        };
        let request = Request {
            operation: "tools.compress".to_string(),
            inputs: vec![input.clone()],
            settings: ToolSettings {
                compress_strong: true,
                ..Default::default()
            },
            ..Default::default()
        };
        let outcome = tools.run(&request, &Selection::inspect(&[input.clone()]));
        let outputs = match outcome {
            Outcome::Succeeded { outputs } => outputs,
            Outcome::Failed { reason } => panic!("the compress failed: {:?}", reason),
            Outcome::Cancelled => panic!("the compress was not asked to stop"),
        };
        let made = outputs.first().expect("the compress is there").clone();
        assert!(made.is_file(), "the lighter sound exists");
        let lighter = std::fs::metadata(&made).expect("the result weighs").len();
        assert!(
            lighter < weight,
            "{lighter} should be under the original's {weight}"
        );
        let _ = std::fs::remove_dir_all(&scratch);
    }

    #[test]
    fn metadata_args_with_remove_all_clears_metadata_and_chapters() {
        let input = Path::new("song.mp3");
        let output = Path::new("song.meta-part");
        let settings = ToolSettings {
            metadata_remove_all: true,
            ..Default::default()
        };
        let args = metadata_args(input, output, Some("mp3"), &settings);
        let joined = args
            .iter()
            .map(|s| s.to_string_lossy())
            .collect::<Vec<_>>()
            .join(" ");
        assert!(joined.contains("-map 0 -c copy"), "args must stream copy: {joined}");
        assert!(joined.contains("-map_metadata -1"), "args must clear metadata: {joined}");
        assert!(joined.contains("-map_chapters -1"), "args must clear chapters: {joined}");
    }

    #[test]
    fn metadata_args_with_tags_sets_metadata_and_copies_streams() {
        let input = Path::new("clip.mp4");
        let output = Path::new("clip.meta-part");
        let settings = ToolSettings {
            metadata_tags: vec![
                ("title".to_string(), "Sample Video".to_string()),
                ("artist".to_string(), "Open Tangerine".to_string()),
            ],
            ..Default::default()
        };
        let args = metadata_args(input, output, Some("mp3"), &settings);
        let joined = args
            .iter()
            .map(|s| s.to_string_lossy())
            .collect::<Vec<_>>()
            .join(" ");
        assert!(joined.contains("-map 0 -c copy"), "args must stream copy: {joined}");
        assert!(joined.contains("-map_metadata 0"), "args must keep stream metadata: {joined}");
        assert!(joined.contains("-metadata title=Sample Video"), "title must be set: {joined}");
        assert!(joined.contains("-metadata artist=Open Tangerine"), "artist must be set: {joined}");
    }

    #[test]
    fn metadata_final_names_adds_suffix_and_numbers() {
        let original = Path::new("C:\\music\\track.mp3");
        let names = metadata_final_names(original, "clean", "mp3");
        assert_eq!(names[0], PathBuf::from("C:\\music\\track clean.mp3"));
        assert_eq!(names[1], PathBuf::from("C:\\music\\track clean (2).mp3"));
        assert_eq!(names[2], PathBuf::from("C:\\music\\track clean (3).mp3"));

        let edited_names = metadata_final_names(original, "edited", "mp3");
        assert_eq!(edited_names[0], PathBuf::from("C:\\music\\track edited.mp3"));
        assert_eq!(edited_names[1], PathBuf::from("C:\\music\\track edited (2).mp3"));
    }

    fn make_wav_bytes(duration_secs: f32, rate: u32) -> Vec<u8> {
        let sample_count = (duration_secs * rate as f32) as u32;
        let data_len = sample_count * 2;
        let mut wav_bytes = Vec::with_capacity(44 + data_len as usize);
        wav_bytes.extend_from_slice(b"RIFF");
        wav_bytes.extend_from_slice(&(36 + data_len).to_le_bytes());
        wav_bytes.extend_from_slice(b"WAVEfmt ");
        wav_bytes.extend_from_slice(&16u32.to_le_bytes());
        wav_bytes.extend_from_slice(&1u16.to_le_bytes());
        wav_bytes.extend_from_slice(&1u16.to_le_bytes());
        wav_bytes.extend_from_slice(&rate.to_le_bytes());
        wav_bytes.extend_from_slice(&(rate * 2).to_le_bytes());
        wav_bytes.extend_from_slice(&2u16.to_le_bytes());
        wav_bytes.extend_from_slice(&16u16.to_le_bytes());
        wav_bytes.extend_from_slice(b"data");
        wav_bytes.extend_from_slice(&data_len.to_le_bytes());
        for i in 0..sample_count {
            let sample: i16 = if (i / 50) % 2 == 0 { 10000 } else { -10000 };
            wav_bytes.extend_from_slice(&sample.to_le_bytes());
        }
        wav_bytes
    }

    #[test]
    #[ignore = "runs the real program to edit tags on audio"]
    fn real_metadata_edit_on_audio_sets_tags() {
        let ffmpeg = Path::new("vendor").join("ffmpeg").join("ffmpeg.exe");
        if !ffmpeg.is_file() {
            return;
        }
        let scratch = std::env::temp_dir().join(format!("ot-meta-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&scratch);
        let input = scratch.join("tone.wav");

        let pcm = make_wav_bytes(1.0, 44100);
        std::fs::write(&input, pcm).expect("tone.wav written");

        let tools = Tools {
            ffmpeg_tools: crate::media::locator::Tools {
                ffmpeg,
                ffprobe: Path::new("vendor").join("ffmpeg").join("ffprobe.exe"),
            },
            publisher: CopyPublisher,
        };
        let request = Request {
            operation: "tools.metadata".to_string(),
            inputs: vec![input.clone()],
            settings: ToolSettings {
                metadata_tags: vec![("title".to_string(), "Tone Test".to_string())],
                ..Default::default()
            },
            ..Default::default()
        };
        let outcome = tools.run(&request, &Selection::inspect(&[input.clone()]));
        let outputs = match outcome {
            Outcome::Succeeded { outputs } => outputs,
            Outcome::Failed { reason } => panic!("metadata edit failed: {:?}", reason),
            Outcome::Cancelled => panic!("cancelled"),
        };
        let made = outputs.first().expect("output exists");
        assert!(made.is_file(), "edited file exists: {:?}", made);
        assert!(made.to_string_lossy().contains("tone edited.wav"));
        let _ = std::fs::remove_dir_all(&scratch);
    }

    #[test]
    fn normalize_settings_defaults() {
        let settings = ToolSettings::default();
        assert_eq!(settings.target_lufs(), -16.0);
        assert_eq!(settings.loudness_range(), 11.0);
        assert_eq!(settings.true_peak(), -1.5);

        let custom = ToolSettings {
            normalize_lufs_x10: Some(-140),
            normalize_range_x10: Some(90),
            normalize_true_peak_x10: Some(-10),
            ..Default::default()
        };
        assert_eq!(custom.target_lufs(), -14.0);
        assert_eq!(custom.loudness_range(), 9.0);
        assert_eq!(custom.true_peak(), -1.0);
    }

    #[test]
    #[ignore = "runs the real program to normalize audio volume"]
    fn real_audio_normalize_with_the_program() {
        let ffmpeg = Path::new("vendor").join("ffmpeg").join("ffmpeg.exe");
        if !ffmpeg.is_file() {
            return;
        }
        let scratch = std::env::temp_dir().join(format!("ot-norm-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&scratch);
        let input = scratch.join("quiet.wav");

        // Make 1-second WAV with amplitude 10000 (~0.305 peak)
        let pcm = make_wav_bytes(1.0, 44100);
        std::fs::write(&input, pcm).expect("quiet.wav written");

        let tools = Tools {
            ffmpeg_tools: crate::media::locator::Tools {
                ffmpeg,
                ffprobe: Path::new("vendor").join("ffmpeg").join("ffprobe.exe"),
            },
            publisher: CopyPublisher,
        };
        let request = Request {
            operation: "tools.normalize".to_string(),
            inputs: vec![input.clone()],
            settings: ToolSettings {
                normalize_true_peak_x10: Some(-15),
                ..Default::default()
            },
            ..Default::default()
        };
        let outcome = tools.run(&request, &Selection::inspect(&[input.clone()]));
        let outputs = match outcome {
            Outcome::Succeeded { outputs } => outputs,
            Outcome::Failed { reason } => panic!("normalize failed: {:?}", reason),
            Outcome::Cancelled => panic!("cancelled"),
        };
        let made = outputs.first().expect("output exists");
        assert!(made.is_file(), "normalized file exists: {:?}", made);
        assert!(made.to_string_lossy().contains("quiet normalized.wav"));

        // Check that the output amplitude was increased towards target peak ~0.841 (-1.5 dBTP)
        let bytes = std::fs::read(made).expect("read normalized wav");
        let (samples, _) = crate::media::audio::parse_wav_samples(&bytes).expect("parse wav");
        let peak = samples.iter().map(|&s| (s as i32).abs()).max().unwrap_or(0) as f64 / 32767.0;
        assert!(
            (peak - 0.8414).abs() < 0.05,
            "peak amplitude after normalization was {peak}, expected ~0.8414"
        );

        let _ = std::fs::remove_dir_all(&scratch);
    }

    #[test]
    fn channels_settings_defaults() {
        let settings = ToolSettings::default();
        assert_eq!(settings.channel_mode(), "stereo");
        assert_eq!(settings.left_gain_factor(), 1.0);
        assert_eq!(settings.right_gain_factor(), 1.0);

        let custom = ToolSettings {
            channel_mode: Some("mono".to_string()),
            channel_left_gain: Some(150),
            channel_right_gain: Some(50),
            ..Default::default()
        };
        assert_eq!(custom.channel_mode(), "mono");
        assert_eq!(custom.left_gain_factor(), 1.5);
        assert_eq!(custom.right_gain_factor(), 0.5);
    }

    #[test]
    #[ignore = "runs the real program to convert audio channels"]
    fn real_audio_channels_with_the_program() {
        let ffmpeg = Path::new("vendor").join("ffmpeg").join("ffmpeg.exe");
        if !ffmpeg.is_file() {
            return;
        }

        let scratch = std::env::temp_dir().join(format!("ot-chan-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&scratch);
        let input = scratch.join("stereo_test.wav");

        // Stereo audio: Left channel has 8000 amplitude, Right has 16000 amplitude
        let mut samples = Vec::new();
        for _ in 0..4000 {
            samples.push(8000i16);
            samples.push(16000i16);
        }
        let pcm = crate::media::audio::encode_wav_bytes(&samples, 44100, 2);
        std::fs::write(&input, pcm).expect("stereo_test.wav written");

        let tools = Tools {
            ffmpeg_tools: crate::media::locator::Tools {
                ffmpeg,
                ffprobe: Path::new("vendor").join("ffmpeg").join("ffprobe.exe"),
            },
            publisher: CopyPublisher,
        };
        let request = Request {
            operation: "tools.channels".to_string(),
            inputs: vec![input.clone()],
            settings: ToolSettings {
                channel_mode: Some("swap".to_string()),
                channel_left_gain: Some(100),
                channel_right_gain: Some(100),
                ..Default::default()
            },
            ..Default::default()
        };
        let outcome = tools.run(&request, &Selection::inspect(&[input.clone()]));
        let outputs = match outcome {
            Outcome::Succeeded { outputs } => outputs,
            Outcome::Failed { reason } => panic!("channels conversion failed: {:?}", reason),
            Outcome::Cancelled => panic!("cancelled"),
        };
        let made = outputs.first().expect("output exists");
        assert!(made.is_file(), "converted file exists: {:?}", made);
        assert!(made.to_string_lossy().contains("stereo_test channels.wav"));

        // Verify channels swapped: L should now be 16000, R should be 8000
        let bytes = std::fs::read(made).expect("read swapped wav");
        let parsed = crate::media::audio::parse_wav_audio(&bytes).expect("parse wav");
        assert_eq!(parsed.channels, 2);
        assert_eq!(parsed.samples[0], 16000);
        assert_eq!(parsed.samples[1], 8000);

        let _ = std::fs::remove_dir_all(&scratch);
    }

    #[test]
    fn merge_requires_at_least_two_inputs() {
        let tools = Tools {
            ffmpeg_tools: crate::media::locator::Tools {
                ffmpeg: PathBuf::from("ffmpeg"),
                ffprobe: PathBuf::from("ffprobe"),
            },
            publisher: CopyPublisher,
        };
        let request = Request {
            operation: "tools.merge".to_string(),
            inputs: vec![PathBuf::from("one.mp4")],
            ..Default::default()
        };
        let outcome = tools.run(&request, &Selection::inspect(&[PathBuf::from("one.mp4")]));
        assert!(matches!(outcome, Outcome::Failed { .. }));
    }

    #[test]
    #[ignore = "needs pdfium.dll"]
    fn merge_two_pdfs_produces_merged_pdf() {
        if crate::media::pdf::library().is_none() {
            eprintln!("pdfium.dll is not there; this test needs it");
            return;
        }
        let scratch1 = std::env::temp_dir().join(format!("ot-pdf-m1-{}", std::process::id()));
        let scratch2 = std::env::temp_dir().join(format!("ot-pdf-m2-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&scratch1);
        let _ = std::fs::create_dir_all(&scratch2);
        let pdf1 = scratch1.join("doc_a.pdf");
        let pdf2 = scratch2.join("doc_b.pdf");

        // Make two valid PDFs
        let doc_bytes = {
            let content = b"BT /F1 12 Tf 20 50 Td (Page) Tj ET";
            format!(
                "%PDF-1.4\n\
                 1 0 obj << /Type /Catalog /Pages 2 0 R >> endobj\n\
                 2 0 obj << /Type /Pages /Kids [3 0 R] /Count 1 >> endobj\n\
                 3 0 obj << /Type /Page /Parent 2 0 R /MediaBox [0 0 200 100] \
                 /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >> endobj\n\
                 4 0 obj << /Length {} >> stream\n{}\nendstream endobj\n\
                 5 0 obj << /Type /Font /Subtype /Type1 /BaseFont /Helvetica >> endobj\n\
                 trailer << /Root 1 0 R /Size 6 >>",
                content.len(),
                String::from_utf8_lossy(content)
            )
        };
        std::fs::write(&pdf1, &doc_bytes).expect("pdf1 written");
        std::fs::write(&pdf2, &doc_bytes).expect("pdf2 written");

        let tools = Tools {
            ffmpeg_tools: crate::media::locator::Tools {
                ffmpeg: PathBuf::from("ffmpeg"),
                ffprobe: PathBuf::from("ffprobe"),
            },
            publisher: CopyPublisher,
        };
        let request = Request {
            operation: "tools.merge".to_string(),
            inputs: vec![pdf1.clone(), pdf2.clone()],
            ..Default::default()
        };
        let outcome = tools.run(&request, &Selection::inspect(&[pdf1.clone(), pdf2.clone()]));
        let outputs = match outcome {
            Outcome::Succeeded { outputs } => outputs,
            Outcome::Failed { reason } => panic!("pdf merge failed: {:?}", reason),
            Outcome::Cancelled => panic!("cancelled"),
        };
        let made = outputs.first().expect("output exists");
        assert!(made.is_file(), "merged file exists: {:?}", made);
        assert!(made.to_string_lossy().contains("doc_a merged.pdf"));

        let pages = crate::media::pdf::render_pages(made, 72, crate::media::pdf::Encoding::Png)
            .expect("rendered pages");
        assert_eq!(pages.len(), 2, "merged document has 2 pages");

        let _ = std::fs::remove_dir_all(&scratch1);
        let _ = std::fs::remove_dir_all(&scratch2);
    }

    #[test]
    fn organize_pdf_empty_inputs_fails() {
        let tools = Tools {
            ffmpeg_tools: crate::media::locator::Tools {
                ffmpeg: PathBuf::from("ffmpeg"),
                ffprobe: PathBuf::from("ffprobe"),
            },
            publisher: CopyPublisher,
        };
        let request = Request {
            operation: "tools.organize_pdf".to_string(),
            inputs: vec![],
            ..Default::default()
        };
        let outcome = tools.run(&request, &Selection::inspect(&[]));
        assert!(matches!(outcome, Outcome::Failed { .. }));
    }

    #[test]
    #[ignore = "needs pdfium.dll"]
    fn organize_pdf_assembles_selected_pages_with_rotation() {
        if crate::media::pdf::library().is_none() {
            eprintln!("pdfium.dll is not there; this test needs it");
            return;
        }
        let scratch = std::env::temp_dir().join(format!("ot-pdf-org-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&scratch);
        let pdf = scratch.join("my_doc.pdf");

        let doc_bytes = {
            let content = b"BT /F1 12 Tf 20 50 Td (Page) Tj ET";
            format!(
                "%PDF-1.4\n\
                 1 0 obj << /Type /Catalog /Pages 2 0 R >> endobj\n\
                 2 0 obj << /Type /Pages /Kids [3 0 R] /Count 1 >> endobj\n\
                 3 0 obj << /Type /Page /Parent 2 0 R /MediaBox [0 0 200 100] \
                 /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >> endobj\n\
                 4 0 obj << /Length {} >> stream\n{}\nendstream endobj\n\
                 5 0 obj << /Type /Font /Subtype /Type1 /BaseFont /Helvetica >> endobj\n\
                 trailer << /Root 1 0 R /Size 6 >>",
                content.len(),
                String::from_utf8_lossy(content)
            )
        };
        std::fs::write(&pdf, &doc_bytes).expect("pdf written");

        let tools = Tools {
            ffmpeg_tools: crate::media::locator::Tools {
                ffmpeg: PathBuf::from("ffmpeg"),
                ffprobe: PathBuf::from("ffprobe"),
            },
            publisher: CopyPublisher,
        };
        let request = Request {
            operation: "tools.organize_pdf".to_string(),
            inputs: vec![pdf.clone()],
            settings: ToolSettings {
                pdf_pages: vec![(0, 0, 90), (0, 0, 0)],
                ..Default::default()
            },
        };
        let outcome = tools.run(&request, &Selection::inspect(&[pdf.clone()]));
        let outputs = match outcome {
            Outcome::Succeeded { outputs } => outputs,
            Outcome::Failed { reason } => panic!("organize pdf failed: {:?}", reason),
            Outcome::Cancelled => panic!("cancelled"),
        };
        let made = outputs.first().expect("output exists");
        assert!(made.is_file(), "organized file exists: {:?}", made);
        assert!(made.to_string_lossy().contains("my_doc organized.pdf"));

        let pages = crate::media::pdf::render_pages(made, 72, crate::media::pdf::Encoding::Png)
            .expect("rendered pages");
        assert_eq!(pages.len(), 2, "assembled document has 2 pages");

        let _ = std::fs::remove_dir_all(&scratch);
    }

    #[test]
    #[ignore = "needs the conversion program to re-time the video"]
    fn merge_two_videos_produces_joined_video() {
        let folder = Path::new("vendor").join("ffmpeg");
        let ffmpeg = folder.join("ffmpeg.exe");
        let ffprobe = folder.join("ffprobe.exe");
        if !ffmpeg.is_file() {
            eprintln!("the program is not there; this test needs it");
            return;
        }

        let scratch = std::env::temp_dir().join(format!("ot-vid-merge-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&scratch);

        let vid1 = scratch.join("clip_a.mp4");
        let vid2 = scratch.join("clip_b.mp4");

        // Make two 160x120 15-frame videos: one red, one blue
        for (vid, color) in [(&vid1, [255u8, 0, 0, 255]), (&vid2, [0u8, 0, 255, 255])] {
            let raw = scratch.join(format!("{}.raw", vid.file_stem().unwrap().to_str().unwrap()));
            std::fs::write(&raw, color.repeat(160 * 120 * 15)).expect("raw video written");
            let args: Vec<OsString> = [
                "-hide_banner",
                "-nostdin",
                "-y",
                "-f", "rawvideo",
                "-pix_fmt", "rgba",
                "-s", "160x120",
                "-r", "15",
                "-i", raw.as_os_str().to_str().unwrap(),
                "-c:v", "mpeg4",
                "-pix_fmt", "yuv420p",
                "-f", "mov",
                "-q:v", "5",
                vid.as_os_str().to_str().unwrap(),
            ]
            .iter()
            .map(|w| OsString::from(*w))
            .collect();
            let made = SystemRunner.run(&ffmpeg, &args, None).expect("video created");
            assert!(made.succeeded(), "FFmpeg failed: status={:?}, stderr={}", made.status, made.stderr);
            let _ = std::fs::remove_file(&raw);
        }

        let tools = Tools {
            ffmpeg_tools: crate::media::locator::Tools {
                ffmpeg: ffmpeg.clone(),
                ffprobe: ffprobe.clone(),
            },
            publisher: CopyPublisher,
        };
        let request = Request {
            operation: "tools.merge".to_string(),
            inputs: vec![vid1.clone(), vid2.clone()],
            ..Default::default()
        };
        let outcome = tools.run(&request, &Selection::inspect(&[vid1.clone(), vid2.clone()]));
        let outputs = match outcome {
            Outcome::Succeeded { outputs } => outputs,
            Outcome::Failed { reason } => panic!("video merge failed: {:?}", reason),
            Outcome::Cancelled => panic!("cancelled"),
        };
        let made = outputs.first().expect("output exists");
        assert!(made.is_file(), "joined file exists: {:?}", made);
        assert!(made.to_string_lossy().contains("clip_a joined.mp4"));

        let (w, h, _) = crate::media::frames::shape_of(&tools.ffmpeg_tools, made).expect("shape");
        assert_eq!((w, h), (160, 120));
        let duration = crate::media::frames::duration_of(&tools.ffmpeg_tools, made).expect("duration");
        // Two 1-second clips should make a ~2-second joined video
        assert!(duration >= 1.5, "duration should be ~2s: {duration}");

        let _ = std::fs::remove_dir_all(&scratch);
    }
}
