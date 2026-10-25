//! The operations that turn one picture into another.
//!
//! The operation says what the result should be — the format it is written
//! in, and where it is written — and this module does that with the
//! machinery of the media layer.
//!
//! The work happens on a worker, never on the thread that draws: a
//! conversion can take a while, and the wheel has to keep moving.

use crate::diagnostics::{self, Level};
use crate::media::convert::Converter;
use crate::media::locator::Tools;
use crate::media::manifest::Manifest;
use crate::media::publish::Publisher;
use crate::media::runner::SystemRunner;
use crate::radial::execution::{Failure, Handler, Outcome, Request};
use crate::radial::registry::{Output, operation_by_id};
use crate::radial::selection::Selection;

/// The handler for the picture operations.
///
/// The publisher is given to it rather than made by it, so that this module
/// knows nothing about the platform: how a finished file is put at its name
/// is the platform's business, and the rule that the name is never
/// overwritten belongs to both.
pub struct Pictures<P: Publisher> {
    /// The programs that do the work.
    pub tools: Tools,
    /// Puts a finished file at its name, and never overwrites what is there.
    pub publisher: P,
}

impl<P: Publisher> Handler for Pictures<P> {
    fn run(&self, request: &Request, _selection: &Selection) -> Outcome {
        let Some(operation) = operation_by_id(&request.operation) else {
            return Outcome::Failed {
                reason: Failure::UnknownOperation,
            };
        };

        // The result is written in the format the operation names.
        let Output::Beside(extension) = operation.output else {
            return Outcome::Failed {
                reason: Failure::NotImplemented,
            };
        };
        let Some(target) = Manifest::get().format(extension) else {
            return Outcome::Failed {
                reason: Failure::NotImplemented,
            };
        };

        let runner = SystemRunner;
        let converter = Converter {
            tools: &self.tools,
            runner: &runner,
            publisher: &self.publisher,
        };

        let mut outputs = Vec::new();
        let mut failed = 0usize;
        // Why the first file could not be converted.
        //
        // The reason says what went wrong and never says which file it
        // happened to. It is kept, so that the person is told what it was:
        // "the file is already in that format" says far more than "a file
        // could not be converted".
        let mut first_reason: Option<String> = None;
        for input in &request.inputs {
            match converter.convert(input, target) {
                Ok(path) => outputs.push(path),
                Err(reason) => {
                    // The reason says what went wrong and never says which
                    // file it happened to.
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
    use std::path::{Path, PathBuf};

    /// A publisher that does nothing, for a test that never gets that far.
    struct Nothing;

    impl Publisher for Nothing {
        fn publish(&self, _temporary: &Path, _destination: &Path) -> Result<(), PublishError> {
            Ok(())
        }
    }

    /// A folder of its own for a test, removed when the test is over.
    fn folder(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "open-tangerine-images-{}-{name}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("the test folder could be made");
        path
    }

    #[test]
    fn a_file_that_is_already_in_that_format_says_so() {
        // The picture is a PNG and the operation makes a PNG. The work is
        // refused before anything is run, and the reason says why, so that
        // the person is not told only that something went wrong.
        let folder = folder("same-format");
        let path = folder.join("photo.png");
        std::fs::write(&path, b"not really a picture").expect("the file could be made");

        let selection = Selection::inspect(std::slice::from_ref(&path));
        let request = Request {
            operation: "convert.png".to_string(),
            inputs: vec![path.clone()],
            ..Default::default()
        };
        let pictures = Pictures {
            tools: Tools::default(),
            publisher: Nothing,
        };
        let outcome = pictures.run(&request, &selection);
        match outcome {
            Outcome::Failed {
                reason: Failure::Error(message),
            } => assert!(
                message.contains("the file is already in that format"),
                "the message is: {message}"
            ),
            other => panic!("the wrong outcome came of it: {other:?}"),
        }
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn the_reason_of_a_failure_never_names_a_file() {
        let folder = folder("no-names");
        let path = folder.join("photo.png");
        std::fs::write(&path, b"not really a picture").expect("the file could be made");

        let selection = Selection::inspect(std::slice::from_ref(&path));
        let request = Request {
            operation: "convert.png".to_string(),
            inputs: vec![path.clone()],
            ..Default::default()
        };
        let pictures = Pictures {
            tools: Tools::default(),
            publisher: Nothing,
        };
        let outcome = pictures.run(&request, &selection);
        let message = match outcome {
            Outcome::Failed { reason } => reason.message(),
            other => panic!("the wrong outcome came of it: {other:?}"),
        };
        assert!(
            !message.contains('\\'),
            "the message carries a path: {message}"
        );
        assert!(
            !message.contains("photo.png"),
            "the message names a file: {message}"
        );
        let _ = std::fs::remove_dir_all(&folder);
    }
}
