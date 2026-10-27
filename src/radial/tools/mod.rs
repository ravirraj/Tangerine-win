//! Tool implementations for Tangerine.
//!
//! Submodules organize tools by domain:
//! - `audio`: audio visualizer, loudness normalization, audio channels conversion
//! - `video`: video crop, split, snapshots, speed/retime, merge
//! - `pictures`: photo frame/background, collage, annotate, edit adjustments
//! - `document`: PDF merge and page organize
//! - `compress`: image/video/audio compression and metadata stripping/editing
//! - `redact`: image, video, and audio bleep redaction

pub mod audio;
pub mod compress;
pub mod document;
pub mod pictures;
pub mod redact;
pub mod video;

#[cfg(test)]
mod tests;

pub use compress::*;
pub use redact::*;
pub use video::*;

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

use crate::diagnostics::{self, Level};
use crate::media::command::convert as build_convert_args;
use crate::media::locator::Tools as FfmpegTools;
use crate::media::manifest::{FormatSupport, Manifest};
use crate::media::publish::{PublishError, Publisher};
use crate::media::runner::{ProcessRunner, SystemRunner};
use crate::radial::execution::{Failure, Handler, Outcome, Request};
use crate::radial::registry::{operation_by_id, Output};
use crate::radial::selection::Selection;

/// The temporary file name used while a tool runs.
pub(crate) fn staging_name(original: &Path, words: &str) -> PathBuf {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static COUNTER: AtomicUsize = AtomicUsize::new(1);
    let stem = original
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "media".to_string());
    let nth = COUNTER.fetch_add(1, Ordering::Relaxed);
    original.with_file_name(format!("{stem}.{}.{}.{}", std::process::id(), nth, words))
}

/// A temporary file whose existence ends with the scope that made it.
pub(crate) struct SweepOf(pub(crate) PathBuf);

impl Drop for SweepOf {
    fn drop(&mut self) {
        if self.0.exists() {
            let _ = fs::remove_file(&self.0);
        }
    }
}

/// The ways a tool run may fail.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ToolError {
    NothingToDo,
    UnsupportedKind,
    ReadError,
    WriteError,
    ToolFailed,
    NotImplemented,
    TooSmall,
}

impl ToolError {
    pub fn name(&self) -> &'static str {
        match self {
            ToolError::NothingToDo => "nothing to do",
            ToolError::UnsupportedKind => "unsupported file format",
            ToolError::ReadError => "could not read file",
            ToolError::WriteError => "could not write file",
            ToolError::ToolFailed => "tool process failed",
            ToolError::NotImplemented => "feature not implemented",
            ToolError::TooSmall => "target file size is too small",
        }
    }
}

/// Returns true if the muxer requires a real file rather than a stdout pipe.
pub fn muxer_needs_file(muxer: &str) -> bool {
    matches!(muxer, "mp4" | "mov" | "m4a" | "ipod" | "3gp" | "3g2")
}

/// The names a result that keeps the original's name may take.
pub(crate) fn final_names(input: &Path, extension: &str) -> Vec<PathBuf> {
    crate::media::publish::names_for(input, extension)
}

/// The candidate file names for metadata output: `{stem} {suffix}.{ext}`,
/// `{stem} {suffix} (2).{ext}`, etc.
pub(crate) fn metadata_final_names(original: &Path, suffix: &str, extension: &str) -> Vec<PathBuf> {
    let stem = original
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "file".to_string());
    let parent = original.parent().map(|p| p.to_path_buf());
    let mut names = Vec::new();

    let first = format!("{stem} {suffix}.{extension}");
    names.push(match &parent {
        Some(p) => p.join(first),
        None => PathBuf::from(first),
    });

    for index in 2..1000u32 {
        let name = format!("{stem} {suffix} ({index}).{extension}");
        names.push(match &parent {
            Some(p) => p.join(name),
            None => PathBuf::from(name),
        });
    }

    names
}

/// Dispatches radial wheel tool operations.
pub struct Tools<P: Publisher> {
    pub ffmpeg_tools: FfmpegTools,
    pub publisher: P,
}

impl<P: Publisher> Tools<P> {
    #[allow(dead_code)]
    pub fn name(&self) -> &'static str {
        "tools"
    }

    /// Runs one tool operation on one input file.
    pub(crate) fn run_one(
        &self,
        operation_id: &str,
        input: &Path,
        source: &FormatSupport,
        target: &FormatSupport,
        temporary: &Path,
        runner: &dyn ProcessRunner,
        settings: &crate::radial::execution::ToolSettings,
    ) -> Result<PathBuf, ToolError> {
        if operation_id == "tools.crop" {
            return self.run_crop(input, target, temporary, runner, settings);
        }

        let resize = resize_filter(settings);
        let video_filter = match operation_id {
            "tools.rotate" => Some("transpose=1"),
            "tools.resize" => Some(resize.as_str()),
            _ => None,
        };

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

    /// Puts what a command wrote where it belongs.
    pub(crate) fn finish(
        &self,
        arguments: &[OsString],
        target: &FormatSupport,
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
    pub(crate) fn publish(&self, finals: &[PathBuf], temporary: &Path) -> Result<PathBuf, ToolError> {
        for final_name in finals {
            match self.publisher.publish(temporary, final_name) {
                Ok(()) => return Ok(final_name.clone()),
                Err(PublishError::DestinationExists) => continue,
                Err(PublishError::CouldNotMove) => return Err(ToolError::WriteError),
            }
        }
        Err(ToolError::WriteError)
    }
}

impl<P: Publisher> Handler for Tools<P> {
    fn run(&self, request: &Request, _selection: &Selection) -> Outcome {
        let Some(operation) = operation_by_id(&request.operation) else {
            return Outcome::Failed {
                reason: Failure::UnknownOperation,
            };
        };

        if request.operation == "tools.split" {
            return self.run_split(request, &SystemRunner);
        }

        if request.operation == "tools.snapshots" {
            return self.run_snapshots(request, &SystemRunner);
        }

        if request.operation == "tools.speed" {
            return self.run_speed(request, &SystemRunner);
        }

        if request.operation == "tools.compress" {
            return self.run_compress(request, &SystemRunner);
        }

        if request.operation == "tools.metadata" {
            return self.run_metadata(request, &SystemRunner);
        }

        if request.operation == "tools.normalize" {
            return self.run_normalize(request, &SystemRunner);
        }

        if request.operation == "tools.channels" {
            return self.run_channels(request, &SystemRunner);
        }

        if request.operation == "tools.redact_image" {
            return self.run_redact_image(request);
        }

        if request.operation == "tools.redact_video" {
            return self.run_redact_video(request, &SystemRunner);
        }

        if request.operation == "tools.bleep" {
            return self.run_bleep(request, &SystemRunner);
        }

        if request.operation == "tools.merge" {
            return self.run_merge(request, &SystemRunner);
        }

        if request.operation == "tools.organize_pdf" {
            return self.run_organize_pdf(request);
        }

        if request.operation == "tools.frame_image" {
            return self.run_frame_image(request);
        }

        if request.operation == "tools.audio_to_video" {
            return self.run_audio_to_video(request);
        }

        if request.operation == "tools.collage" {
            return self.run_collage(request);
        }

        if request.operation == "tools.annotate" {
            return self.run_annotate(request);
        }

        if request.operation == "tools.edit_image" {
            return self.run_edit_image(request);
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
            let Some(source) = manifest
                .format_of_extension(&source_ext)
                .or_else(|| manifest.format(&source_ext))
            else {
                failed += 1;
                let name = ToolError::UnsupportedKind.name();
                diagnostics::log(Level::Warn, "action", &format!("a file was not processed: {name}"));
                if first_reason.is_none() {
                    first_reason = Some(name.to_string());
                }
                continue;
            };

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

            let Some(target) = manifest
                .format_of_extension(extension)
                .or_else(|| manifest.format(extension))
            else {
                failed += 1;
                let name = ToolError::UnsupportedKind.name();
                diagnostics::log(Level::Warn, "action", &format!("a file was not processed: {name}"));
                if first_reason.is_none() {
                    first_reason = Some(name.to_string());
                }
                continue;
            };

            let target_ext = extension;
            let temporary = staging_name(input, &format!("{target_ext}.part"));
            let _sweep = SweepOf(temporary.clone());

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
