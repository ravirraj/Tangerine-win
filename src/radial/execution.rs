//! Running an operation.
//!
//! The registry says what may be done with a selection. This module says what
//! happens when something is run, and what comes of it.
//!
//! The contract is small on purpose:
//!
//! - an operation is named by its **stable name**, the same name the menu
//!   shows and the registry knows, so there is no way to run one operation by
//!   pointing at another;
//! - the files it is to work on are **named** and have to be the files that
//!   were dragged;
//! - what comes back is one of three things: it worked, it did not work, or
//!   the user stopped it.
//!
//! The work itself is done by a [Handler]. A handler may take a long time —
//! converting a picture does — so the program that draws does not call it
//! directly: it hands the request to a worker and hears about the outcome
//! later.

use std::path::PathBuf;

use crate::radial::registry::{Environment, Ineligible, eligibility, operation_by_id};
use crate::radial::selection::Selection;

/// The settings a tool was given in its window.
///
/// A tool that is run with no window — from a drag that never opened one —
/// has none, and does what it does on its own.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ToolSettings {
    /// A width, in pixels.
    pub width: Option<u32>,
    /// A height, in pixels.
    pub height: Option<u32>,
    /// Where a stretch of time begins, in milliseconds from the start.
    pub start_ms: Option<u64>,
    /// Where a stretch of time ends, in milliseconds from the start.
    pub end_ms: Option<u64>,
}

/// A request to run an operation.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Request {
    /// The operation, by its stable name.
    pub operation: String,
    /// The files the operation is to work on.
    ///
    /// These have to be the files that were dragged: an operation is run on
    /// what the user chose, and on nothing else.
    pub inputs: Vec<PathBuf>,
    /// What the tool's window said, when it was asked.
    pub settings: ToolSettings,
}

/// What came of running an operation.
///
/// The three shapes are the contract: a handler that works says so, one that
/// fails says why, and one the user stopped says so. There is no handler yet,
/// so only the failure is produced today, and the other two are here because
/// a handler will produce them.
#[allow(dead_code)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// It worked, and these are the files it wrote.
    Succeeded { outputs: Vec<PathBuf> },
    /// It did not work, and this is why.
    Failed { reason: Failure },
    /// The user stopped it.
    Cancelled,
}

/// Why an operation did not work.
///
/// The shapes are the contract. Only the first four are produced today; the
/// last two are what a handler will say when it runs and finds the result
/// already there, or when something goes wrong while it runs.
#[allow(dead_code)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Failure {
    /// There is no operation with that name.
    UnknownOperation,
    /// The operation is not implemented.
    NotImplemented,
    /// The processor the operation needs is not there.
    ProcessorMissing(String),
    /// The files are not what the operation needs.
    InvalidInput(String),
    /// The result is already there, and the policy was to stop.
    OutputCollision { path: PathBuf },
    /// Something went wrong while it ran.
    ///
    /// The text says what went wrong in a few words. It is never a path and
    /// never the name of a file: the log has the detail.
    Error(String),
}

impl Failure {
    /// The name used in the log.
    ///
    /// The name never carries a path: the log says what happened, not which
    /// file it happened to.
    pub fn name(&self) -> &'static str {
        match self {
            Failure::UnknownOperation => "there is no operation with that name",
            Failure::NotImplemented => "the operation is not implemented",
            Failure::ProcessorMissing(_) => "the processor is not there",
            Failure::InvalidInput(_) => "the files are not what the operation needs",
            Failure::OutputCollision { .. } => "the result is already there",
            Failure::Error(_) => "something went wrong while it ran",
        }
    }

    /// The message the person sees.
    ///
    /// It says what happened, in as many words as the failure knows, and
    /// nothing else: no name of a file and no path. The log has the detail.
    pub fn message(&self) -> String {
        match self {
            Failure::UnknownOperation => "there is no such operation".to_string(),
            Failure::NotImplemented => "that operation is not implemented".to_string(),
            Failure::ProcessorMissing(_) => {
                "the program that does the work is not there".to_string()
            }
            // The reason says what was wrong with the files, and it never
            // carries a path.
            Failure::InvalidInput(reason) => reason.clone(),
            Failure::OutputCollision { .. } => "the result is already there".to_string(),
            Failure::Error(reason) => reason.clone(),
        }
    }
}

impl From<Ineligible> for Failure {
    fn from(reason: Ineligible) -> Failure {
        match reason {
            Ineligible::NotImplemented => Failure::NotImplemented,
            Ineligible::ProcessorMissing(name) => Failure::ProcessorMissing(name.to_string()),
            other => Failure::InvalidInput(other.name().to_string()),
        }
    }
}

/// The work an operation does.
///
/// The work is not done on the thread that draws: a handler is called by a
/// worker, and the outcome comes back through a channel.
pub trait Handler {
    /// Does the work of the operation that was asked for.
    fn run(&self, request: &Request, selection: &Selection) -> Outcome;
}

/// A handler for the operations that are not implemented.
///
/// It says so, rather than pretending to have done something.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NotImplemented;

impl Handler for NotImplemented {
    fn run(&self, _request: &Request, _selection: &Selection) -> Outcome {
        Outcome::Failed {
            reason: Failure::NotImplemented,
        }
    }
}

/// Runs an operation for the files that were dragged.
///
/// The request is checked before anything is run: the operation has to exist,
/// it has to be implemented, it has to be allowed for these files, and the
/// files it names have to be the files that were dragged.
///
/// What happens then is up to the handler.
pub fn run(
    request: &Request,
    selection: &Selection,
    environment: &Environment,
    handler: &dyn Handler,
) -> Outcome {
    let Some(operation) = operation_by_id(&request.operation) else {
        return Outcome::Failed {
            reason: Failure::UnknownOperation,
        };
    };

    // The files the request names have to be the files that were dragged.
    if request.inputs != selection.paths() {
        return Outcome::Failed {
            reason: Failure::InvalidInput(
                "the files are not the ones that were dragged".to_string(),
            ),
        };
    }

    // And the operation has to be one that may be used for these files.
    if let Err(reason) = eligibility(operation, selection, environment) {
        return Outcome::Failed {
            reason: reason.into(),
        };
    }

    handler.run(request, selection)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::radial::registry::{Batch, Collision, Environment, Operation, Output, Requirement};
    use crate::radial::selection::FileKind;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU32, Ordering};

    /// A folder of its own for a test, removed when the test is over.
    struct TempFolder {
        path: PathBuf,
    }

    impl TempFolder {
        /// Makes a folder that no other test uses.
        fn new(name: &str) -> TempFolder {
            static COUNTER: AtomicU32 = AtomicU32::new(0);
            let number = COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "open-tangerine-execution-{}-{}-{}",
                std::process::id(),
                number,
                name
            ));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).expect("the test folder could be made");
            TempFolder { path }
        }

        /// Puts a file with this name in the folder and gives back its path.
        fn file(&self, name: &str) -> PathBuf {
            let path = self.path.join(name);
            std::fs::write(&path, b"test").expect("the test file could be made");
            path
        }
    }

    impl Drop for TempFolder {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    /// An operation for a test, with the parts a test cares about spelled out.
    fn operation(name: &'static str) -> Operation {
        Operation {
            id: name,
            label: name,
            detail: name,
            icon: crate::radial::actions::IconKind::Picture,
            mode: crate::radial::actions::RadialMode::Convert,
            categories: &[FileKind::Image],
            batch: Batch::Several,
            folders: false,
            output: Output::Beside("png"),
            collision: Collision::Rename,
            implemented: true,
            requires: Requirement::None,
        }
    }

    #[test]
    fn an_operation_that_is_not_in_the_registry_is_not_run() {
        let folder = TempFolder::new("unknown");
        let path = folder.file("one.png");
        let selection = Selection::inspect(std::slice::from_ref(&path));
        let request = Request {
            operation: "there.is.no.such.operation".to_string(),
            inputs: vec![path],
            ..Default::default()
        };
        let outcome = run(&request, &selection, &Environment::none(), &NotImplemented);
        assert_eq!(
            outcome,
            Outcome::Failed {
                reason: Failure::UnknownOperation
            }
        );
    }

    #[test]
    fn an_operation_that_is_not_implemented_is_refused() {
        let folder = TempFolder::new("not-implemented");
        let path = folder.file("one.png");
        let selection = Selection::inspect(std::slice::from_ref(&path));
        let request = Request {
            // Putting files in an archive is not something that is implemented.
            operation: "actions.zip".to_string(),
            inputs: vec![path],
            ..Default::default()
        };
        let outcome = run(&request, &selection, &Environment::none(), &NotImplemented);
        assert_eq!(
            outcome,
            Outcome::Failed {
                reason: Failure::NotImplemented
            }
        );
    }

    #[test]
    fn files_that_are_not_the_ones_that_were_dragged_are_refused() {
        let folder = TempFolder::new("other-files");
        let path = folder.file("one.png");
        let other = folder.file("two.png");
        let selection = Selection::inspect(std::slice::from_ref(&path));
        // The request names a file that was not dragged.
        let request = Request {
            operation: "convert.png".to_string(),
            inputs: vec![other],
            ..Default::default()
        };
        let outcome = run(&request, &selection, &Environment::none(), &NotImplemented);
        match outcome {
            Outcome::Failed {
                reason: Failure::InvalidInput(_),
            } => {}
            other => panic!("the wrong files were accepted: {other:?}"),
        }
    }

    #[test]
    fn an_operation_that_does_not_fit_the_files_is_refused() {
        let folder = TempFolder::new("does-not-fit");
        let path = folder.file("paper.pdf");
        let selection = Selection::inspect(std::slice::from_ref(&path));
        let request = Request {
            operation: "convert.png".to_string(),
            inputs: vec![path],
            ..Default::default()
        };
        let outcome = run(&request, &selection, &Environment::none(), &NotImplemented);
        match outcome {
            Outcome::Failed {
                reason: Failure::InvalidInput(_),
            } => {}
            other => panic!("an operation that does not fit was run: {other:?}"),
        }
    }

    #[test]
    fn the_outcome_never_carries_a_path_into_the_log() {
        let folder = TempFolder::new("no-paths");
        let path = folder.file("one.png");
        let selection = Selection::inspect(std::slice::from_ref(&path));
        let request = Request {
            operation: "convert.png".to_string(),
            inputs: vec![path],
            ..Default::default()
        };
        let outcome = run(&request, &selection, &Environment::none(), &NotImplemented);
        let name = match &outcome {
            Outcome::Failed { reason } => reason.name(),
            _ => "",
        };
        assert!(!name.contains('\\'), "the reason carries a path: {name}");
    }

    #[test]
    fn an_operation_that_is_allowed_reaches_the_handler() {
        // The operation is allowed, so what is missing is the work: the
        // handler is what says whether the work was done.
        let folder = TempFolder::new("allowed");
        let path = folder.file("one.png");
        let selection = Selection::inspect(std::slice::from_ref(&path));
        let request = Request {
            operation: "convert.png".to_string(),
            inputs: vec![path],
            ..Default::default()
        };
        let environment = Environment {
            processors: vec![crate::radial::registry::Processor {
                name: "image".to_string(),
                reads: vec!["png".to_string()],
                writes: vec!["png".to_string()],
            }],
        };
        let outcome = run(&request, &selection, &environment, &NotImplemented);
        assert_eq!(
            outcome,
            Outcome::Failed {
                reason: Failure::NotImplemented
            }
        );
    }

    #[test]
    fn the_message_of_a_failure_says_what_went_wrong() {
        // Every failure has its own words, so that the person is told what
        // happened and not that something happened.
        assert_eq!(
            Failure::UnknownOperation.message(),
            "there is no such operation"
        );
        assert_eq!(
            Failure::NotImplemented.message(),
            "that operation is not implemented"
        );
        assert_eq!(
            Failure::ProcessorMissing("image".to_string()).message(),
            "the program that does the work is not there"
        );
        assert_eq!(
            Failure::InvalidInput("the file is already in that format".to_string()).message(),
            "the file is already in that format"
        );
        assert_eq!(
            Failure::OutputCollision {
                path: PathBuf::from("C:\\Users\\test\\photo.png")
            }
            .message(),
            "the result is already there"
        );
    }

    #[test]
    fn the_message_of_a_failure_never_carries_a_path() {
        // The result that is already there names no file: the message says
        // what happened, not which file it happened to.
        let message = Failure::OutputCollision {
            path: PathBuf::from("C:\\Users\\test\\photo.png"),
        }
        .message();
        assert!(
            !message.contains('\\'),
            "the message carries a path: {message}"
        );
        assert!(
            !message.contains("photo.png"),
            "the message names a file: {message}"
        );
    }

    #[test]
    fn the_operation_of_a_test_is_not_in_the_registry() {
        // The helper builds an operation that the registry does not know, so
        // that the rules can be tried without touching the real registry.
        let operation = operation("test.only");
        assert!(crate::radial::registry::operation_by_id("test.only").is_none());
        assert_eq!(operation.batch, Batch::Several);
    }
}
