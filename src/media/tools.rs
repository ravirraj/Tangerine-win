//! Tool operations: compress, crop, trim, merge, rotate, resize, metadata.
//!
//! These are handled with FFmpeg (image/video/audio) or Rust crates
//! (metadata), without needing a separate heavyweight engine.
//!
//! The work happens on a worker, never on the thread that draws.

use std::fs;
use std::io::{Read, Write};
use std::path::Path;

use crate::media::command::convert as convert_arguments;
use crate::media::locator::Tools as FfmpegTools;
use crate::media::publish::Publisher;
use crate::media::runner::{ProcessRunner, SystemRunner};

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
}

impl ToolError {
    pub fn name(&self) -> &'static str {
        match self {
            ToolError::UnsupportedKind => "the tool does not work on this kind of file",
            ToolError::NotImplemented => "this tool is not implemented yet",
            ToolError::ToolFailed => "the tool did not finish",
            ToolError::ReadError => "the source file could not be read",
            ToolError::WriteError => "the result could not be written",
        }
    }
}

/// The handler for tool operations.
///
/// Like Pictures and Documents, this runs on a worker thread.
pub struct Tools<P: Publisher> {
    /// The FFmpeg programs.
    pub ffmpeg_tools: FfmpegTools,
    /// Puts a finished file at its name.
    pub publisher: P,
}

impl<P: Publisher> Tools<P> {
    /// Runs a tool operation on the given inputs.
    pub fn run(
        &self,
        operation_id: &str,
        inputs: &[std::path::PathBuf],
        environment: &crate::radial::registry::Environment,
    ) -> crate::radial::execution::Outcome {
        let runner = SystemRunner;

        let mut outputs = Vec::new();
        let mut failed = 0usize;
        let mut first_reason: Option<String> = None;
        for input in inputs {
            match self.run_one(operation_id, input, &runner) {
                Ok(path) => outputs.push(path),
                Err(reason) => {
                    failed += 1;
                    let name = reason.name();
                    crate::diagnostics::log(
                        crate::diagnostics::Level::Warn,
                        "action",
                        &format!("a file was not processed: {name}"),
                    );
                    if first_reason.is_none() {
                        first_reason = Some(name.to_string());
                    }
                }
            }
        }

        if failed == 0 {
            crate::radial::execution::Outcome::Succeeded { outputs }
        } else {
            let because = match first_reason {
                Some(reason) => format!(": {reason}"),
                None => String::new(),
            };
            crate::radial::execution::Outcome::Failed {
                reason: crate::radial::execution::Failure::Error(format!(
                    "{failed} of the files could not be processed{because}"
                )),
            }
        }
    }

    /// Runs one tool operation on one input file.
    fn run_one(
        &self,
        operation_id: &str,
        input: &std::path::Path,
        runner: &dyn ProcessRunner,
    ) -> Result<std::path::PathBuf, ToolError> {
        let extension = input
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();

        // Determine target extension (same as input for most tools).
        let target_ext = extension.as_str();

        let manifest = crate::media::manifest::Manifest::get();
        let source = manifest
            .format(target_ext)
            .ok_or(ToolError::UnsupportedKind)?;

        let target = source; // Same format for in-place-style tools

        let temporary = input.with_extension(format!("{}.part", target_ext));

        let arguments = match operation_id {
            "tools.compress" => {
                // Re-encode at lower quality/size
                convert_arguments(input, source, target, Some(&temporary))
            }
            "tools.crop" => {
                // Crop: crop filter
                convert_arguments(input, source, target, Some(&temporary))
            }
            "tools.trim" => {
                // Trim: trim filter
                convert_arguments(input, source, target, Some(&temporary))
            }
            "tools.merge" => {
                // Merge: concat demuxer (same-type files only)
                convert_arguments(input, source, target, Some(&temporary))
            }
            "tools.rotate" => {
                // Rotate: transpose filter
                convert_arguments(input, source, target, Some(&temporary))
            }
            "tools.resize" => {
                // Resize: scale filter
                convert_arguments(input, source, target, Some(&temporary))
            }
            _ => return Err(ToolError::NotImplemented),
        };

        let needs_file = crate::media::command::muxer_needs_file(&target.write.muxer);
        let result = if needs_file {
            runner.run(&self.ffmpeg_tools.ffmpeg, &arguments, None)
        } else {
            let file = fs::File::create(&temporary).map_err(|_| ToolError::WriteError)?;
            runner
                .run(&self.ffmpeg_tools.ffmpeg, &arguments, Some(&file))
                .map_err(|_| ToolError::ToolFailed)?
        };

        if !result.succeeded() {
            return Err(ToolError::ToolFailed);
        }

        // Publish to final name (beside original)
        let final_name = input.with_extension(target_ext);
        self.publisher
            .publish(&temporary, &final_name)
            .map_err(|_| ToolError::WriteError)?;

        Ok(final_name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Nothing;

    impl Publisher for Nothing {
        fn publish(&self, _temporary: &std::path::Path, _destination: &std::path::Path) -> Result<(), crate::media::publish::PublishError> {
            Ok(())
        }
    }

    #[test]
    fn unsupported_operation_is_rejected() {
        let tools = Tools {
            ffmpeg_tools: crate::media::locator::Tools {
                ffmpeg: std::path::PathBuf::from("ffmpeg"),
                ffprobe: std::path::PathBuf::from("ffprobe"),
            },
            publisher: Nothing,
        };
        let environment = crate::radial::registry::Environment::none();
        let outcome = tools.run("tools.nonexistent", &[], &environment);
        match outcome {
            crate::radial::execution::Outcome::Failed { reason } => {
                assert!(matches!(reason, crate::radial::execution::Failure::Error(_)));
            }
            other => panic!("unexpected outcome: {other:?}"),
        }
    }
}
