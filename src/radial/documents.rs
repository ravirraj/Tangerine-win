//! The operations that turn one document into another.
//!
//! Subtitles (SRT, VTT, TXT) and archives (ZIP, TAR, GZ) are converted in
//! Rust without FFmpeg.
//!
//! The work happens on a worker, never on the thread that draws.

use crate::diagnostics::{self, Level};
use crate::media::documents::convert as convert_document;
use crate::media::publish::Publisher;
use crate::radial::execution::{Failure, Handler, Outcome, Request};
use crate::radial::registry::{operation_by_id, Output};
use crate::radial::selection::Selection;
use std::path::Path;

/// The handler for the document operations.
pub struct Documents<P: Publisher> {
    /// Puts a finished file at its name, and never overwrites what is there.
    pub publisher: P,
}

impl<P: Publisher> Handler for Documents<P> {
    fn run(&self, request: &Request, _selection: &Selection) -> Outcome {
        let Some(operation) = operation_by_id(&request.operation) else {
            return Outcome::Failed {
                reason: Failure::UnknownOperation,
            };
        };

        let Output::Beside(extension) = operation.output else {
            return Outcome::Failed {
                reason: Failure::NotImplemented,
            };
        };

        // Many pictures put on PDF become one PDF, a page each. This is
        // the one conversion that puts its inputs together, and the one
        // PDF is named after the first picture.
        if request.operation == "convert.pdf" {
            let Some(first) = request.inputs.first() else {
                return Outcome::Failed {
                    reason: Failure::Error("no picture was given".to_string()),
                };
            };
            let target = first.with_extension(extension);
            return match crate::media::documents::images_to_pdf(&request.inputs, &target) {
                Ok(()) => Outcome::Succeeded {
                    outputs: vec![target],
                },
                Err(reason) => {
                    let name = reason.name();
                    diagnostics::log(
                        Level::Warn,
                        "action",
                        &format!("the pictures were not put in one PDF: {name}"),
                    );
                    Outcome::Failed {
                        reason: Failure::Error(format!(
                            "the pictures could not be put in one PDF: {name}"
                        )),
                    }
                }
            };
        }

        let mut outputs = Vec::new();
        let mut failed = 0usize;
        let mut first_reason: Option<String> = None;
        for input in &request.inputs {
            let from = input
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("");
            let target_path = input.with_extension(extension);
            match convert_document(input, &target_path, from, extension) {
                Ok(()) => outputs.push(target_path),
                Err(reason) => {
                    failed += 1;
                    let name = reason.name();
                    diagnostics::log(
                        Level::Warn,
                        "action",
                        &format!("a file was not converted: {name}"),
                    );
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
                    "{failed} of the files could not be converted{because}"
                )),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media::publish::PublishError;
    use std::path::PathBuf;

    struct Nothing;

    impl Publisher for Nothing {
        fn publish(&self, _temporary: &Path, _destination: &Path) -> Result<(), PublishError> {
            Ok(())
        }
    }

    #[test]
    fn a_document_that_cannot_be_converted_reports_error() {
        // A source format that is not supported for conversion.
        let folder = std::env::temp_dir().join(format!(
            "open-tangerine-documents-{}-{}",
            std::process::id(),
            "unsupported"
        ));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).expect("the test folder could be made");
        let path = folder.join("subtitle.xyz");
        std::fs::write(&path, "hello").expect("the file could be made");

        let selection = Selection::inspect(std::slice::from_ref(&path));
        let request = Request {
            operation: "convert.srt".to_string(),
            inputs: vec![path.clone()],
            ..Default::default()
        };
        let documents = Documents {
            publisher: Nothing,
        };
        let outcome = documents.run(&request, &selection);
        match outcome {
            Outcome::Failed {
                reason: Failure::Error(_),
            } => {}
            other => panic!("the wrong outcome came of it: {other:?}"),
        }
        let _ = std::fs::remove_dir_all(&folder);
    }
}
