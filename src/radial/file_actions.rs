//! File actions: copy path, duplicate, rename, reveal in Explorer.
//!
//! These are handled entirely by the application with Win32 APIs, no
//! conversion backend needed.
//!
//! The work happens on a worker, never on the thread that draws.

use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

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
            "actions.terminal" => self.terminal(input),
            "actions.extract" => self.extract(input),
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

    /// Open a terminal at the file's folder.
    fn terminal(&self, input: &Path) -> Result<PathBuf, FileActionError> {
        let folder = if input.is_dir() {
            input.to_path_buf()
        } else {
            input.parent().unwrap_or(input).to_path_buf()
        };
        Command::new("cmd.exe")
            .args(["/k", "cd /d", folder.to_string_lossy().as_ref()])
            .spawn()
            .map_err(|_| FileActionError::ToolFailed)?;
        Ok(input.to_path_buf())
    }

    /// Extract archive contents to a folder beside the original.
    fn extract(&self, input: &Path) -> Result<PathBuf, FileActionError> {
        let ext = input.extension().map(|e| e.to_string_lossy().to_string()).unwrap_or_default();
        let target_dir = input.with_extension(format!("{}.extracted", ext));
        if target_dir.exists() {
            return Err(FileActionError::WriteError);
        }
        fs::create_dir_all(&target_dir).map_err(|_| FileActionError::WriteError)?;

        let data = fs::read(input).map_err(|_| FileActionError::ReadError)?;

        match ext.as_str() {
            "zip" => extract_zip(&data, &target_dir)?,
            "tar" => extract_tar(&data, &target_dir)?,
            "gz" => extract_gz(&data, &target_dir)?,
            _ => return Err(FileActionError::UnsupportedKind),
        }

        Ok(target_dir)
    }
}

/// Why a file action did not happen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileActionError {
    NotImplemented,
    ToolFailed,
    WriteError,
    UnsupportedKind,
    ReadError,
}

impl FileActionError {
    pub fn name(&self) -> &'static str {
        match self {
            FileActionError::NotImplemented => "this action is not implemented yet",
            FileActionError::ToolFailed => "the action did not finish",
            FileActionError::WriteError => "the result could not be written",
            FileActionError::UnsupportedKind => "this action does not work on this kind of file",
            FileActionError::ReadError => "the source file could not be read",
        }
    }
}

/// Extract a ZIP archive into a directory.
fn extract_zip(data: &[u8], target_dir: &Path) -> Result<(), FileActionError> {
    let reader = std::io::Cursor::new(data);
    let mut archive = zip::ZipArchive::new(reader).map_err(|_| FileActionError::UnsupportedKind)?;
    for i in 0..archive.len() {
        let mut file = archive.by_index(i).map_err(|_| FileActionError::UnsupportedKind)?;
        let name = file.name().to_string();
        if file.is_dir() {
            continue;
        }
        let out_path = target_dir.join(&name);
        if let Some(parent) = out_path.parent() {
            fs::create_dir_all(parent).map_err(|_| FileActionError::WriteError)?;
        }
        let mut out_file = fs::File::create(&out_path).map_err(|_| FileActionError::WriteError)?;
        std::io::copy(&mut file, &mut out_file).map_err(|_| FileActionError::WriteError)?;
    }
    Ok(())
}

/// Extract a TAR archive into a directory.
fn extract_tar(data: &[u8], target_dir: &Path) -> Result<(), FileActionError> {
    let mut archive = tar::Archive::new(std::io::Cursor::new(data));
    for entry in archive.entries().map_err(|_| FileActionError::UnsupportedKind)? {
        let mut entry = entry.map_err(|_| FileActionError::UnsupportedKind)?;
        if entry.header().entry_type().is_dir() {
            continue;
        }
        let out_path = target_dir.join(entry.path().map_err(|_| FileActionError::UnsupportedKind)?);
        if let Some(parent) = out_path.parent() {
            fs::create_dir_all(parent).map_err(|_| FileActionError::WriteError)?;
        }
        let mut out_file = fs::File::create(&out_path).map_err(|_| FileActionError::WriteError)?;
        std::io::copy(&mut entry, &mut out_file).map_err(|_| FileActionError::WriteError)?;
    }
    Ok(())
}

/// Extract a GZ archive into a directory (single file).
fn extract_gz(data: &[u8], target_dir: &Path) -> Result<(), FileActionError> {
    let mut decoder = flate2::read::GzDecoder::new(data);
    let mut bytes = Vec::new();
    decoder.read_to_end(&mut bytes).map_err(|_| FileActionError::UnsupportedKind)?;
    let out_path = target_dir.join("extracted");
    fs::write(&out_path, &bytes).map_err(|_| FileActionError::WriteError)?;
    Ok(())
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
