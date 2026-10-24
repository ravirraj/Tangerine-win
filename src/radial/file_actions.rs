//! File actions: copy path, duplicate, rename, reveal in Explorer.
//!
//! These are handled entirely by the application with Win32 APIs, no
//! conversion backend needed.
//!
//! The work happens on a worker, never on the thread that draws.

use std::fs;
use std::path::{Path, PathBuf};

use crate::diagnostics::{self, Level};
use crate::media::publish::Publisher;
use crate::radial::execution::{Failure, Handler, Outcome, Request};
use crate::radial::registry::operation_by_id;
use crate::radial::selection::Selection;

/// The handler for file actions.
pub struct FileActions<P: Publisher> {
    /// Puts a finished file at its name (for duplicate).
    pub publisher: P,
}

impl<P: Publisher> FileActions<P> {
    /// Runs one file action on one input file.
    fn run_one(&self, operation_id: &str, input: &Path) -> Result<PathBuf, FileActionError> {
        match operation_id {
            "actions.copy-path" => self.copy_path(input),
            "actions.duplicate" => self.duplicate(input),
            "actions.rename" => self.rename(input),
            "actions.reveal" => self.reveal(input),
            _ => Err(FileActionError::NotImplemented),
        }
    }

    /// Copy the full path to the clipboard.
    fn copy_path(&self, input: &Path) -> Result<PathBuf, FileActionError> {
        crate::platform::windows::clipboard::set_text(input.to_string_lossy().as_ref())
            .map_err(|_| FileActionError::ToolFailed)?;
        Ok(input.to_path_buf())
    }

    /// Make a duplicate copy beside the original.
    fn duplicate(&self, input: &Path) -> Result<PathBuf, FileActionError> {
        let ext = input.extension().map(|e| e.to_string_lossy().to_string()).unwrap_or_default();
        let target = input.with_extension(format!("{}.copy{}", ext, ext));
        if target.exists() {
            return Err(FileActionError::WriteError);
        }
        fs::copy(input, &target).map_err(|_| FileActionError::WriteError)?;
        Ok(target)
    }

    /// Rename the file (requires user input for new name, handled by UI).
    fn rename(&self, _input: &Path) -> Result<PathBuf, FileActionError> {
        Err(FileActionError::NotImplemented)
    }

    /// Reveal the file in Explorer.
    fn reveal(&self, input: &Path) -> Result<PathBuf, FileActionError> {
        crate::platform::windows::shell::reveal_in_explorer(input)
            .map_err(|_| FileActionError::ToolFailed)?;
        Ok(input.to_path_buf())
    }
}

/// Why a file action did not happen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileActionError {
    NotImplemented,
    ToolFailed,
    WriteError,
}

impl FileActionError {
    pub fn name(&self) -> &'static str {
        match self {
            FileActionError::NotImplemented => "this action is not implemented yet",
            FileActionError::ToolFailed => "the action did not finish",
            FileActionError::WriteError => "the result could not be written",
        }
    }
}

impl<P: Publisher> Handler for FileActions<P> {
    fn run(&self, request: &Request, _selection: &Selection) -> Outcome {
        let Some(operation) = operation_by_id(&request.operation) else {
            return Outcome::Failed {
                reason: Failure::UnknownOperation,
            };
        };

        let mut outputs = Vec::new();
        let mut failed = 0usize;
        let mut first_reason: Option<String> = None;
        for input in &request.inputs {
            match self.run_one(&request.operation, input) {
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

    struct Nothing;

    impl Publisher for Nothing {
        fn publish(&self, _temporary: &Path, _destination: &Path) -> Result<(), crate::media::publish::PublishError> {
            Ok(())
        }
    }

    #[test]
    fn unknown_operation_is_rejected() {
        let actions = FileActions { publisher: Nothing };
        let environment = crate::radial::registry::Environment::none();
        let request = Request {
            operation: "actions.nonexistent".to_string(),
            inputs: Vec::new(),
        };
        let outcome = crate::radial::execution::run(&request, &Selection::inspect(&[]), &environment, &actions);
        match outcome {
            Outcome::Failed { reason } => {
                assert!(matches!(reason, Failure::UnknownOperation));
            }
            other => panic!("unexpected outcome: {other:?}"),
        }
    }
}
