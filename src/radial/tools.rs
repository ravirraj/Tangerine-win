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
}
