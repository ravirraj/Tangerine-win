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
use crate::media::publish::PlatformPublisher;
use crate::media::runner::SystemRunner;
use crate::radial::execution::{Failure, Handler, Outcome, Request};
use crate::radial::registry::{Output, operation_by_id};
use crate::radial::selection::Selection;

/// The handler for the picture operations.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pictures {
    /// The programs that do the work.
    pub tools: Tools,
}

impl Handler for Pictures {
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
        let publisher = PlatformPublisher;
        let converter = Converter {
            tools: &self.tools,
            runner: &runner,
            publisher: &publisher,
        };

        let mut outputs = Vec::new();
        let mut failed = 0usize;
        for input in &request.inputs {
            match converter.convert(input, target) {
                Ok(path) => outputs.push(path),
                Err(reason) => {
                    // The reason says what went wrong and never says which
                    // file it happened to.
                    failed += 1;
                    diagnostics::log(
                        Level::Warn,
                        "action",
                        &format!("a file was not converted: {}", reason.name()),
                    );
                }
            }
        }

        if failed == 0 {
            Outcome::Succeeded { outputs }
        } else {
            Outcome::Failed {
                reason: Failure::Error(format!("{failed} of the files could not be converted")),
            }
        }
    }
}
