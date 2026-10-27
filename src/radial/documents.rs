//! The operations that turn one document into another.
//!
//! Subtitles (SRT, VTT, TXT) and archives (ZIP, TAR, GZ) are converted in
//! Rust without FFmpeg.
//!
//! Every conversion is written first beside its file, under a name of its
//! own, and only takes its final name when it is whole. The plain name —
//! or the next numbered one when that is taken — is given by the publisher:
//! nothing that is already there is ever written over.
//!
//! The work happens on a worker, never on the thread that draws.

use crate::diagnostics::{self, Level};
use crate::media::documents::convert as convert_document;
use crate::media::publish::{names_for, Publisher};
use crate::radial::execution::{Failure, Handler, Outcome, Request};
use crate::radial::registry::{operation_by_id, Output};
use crate::radial::selection::Selection;

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
            return self.put_pictures_in_one_pdf(&request.inputs, first, extension);
        }

        let mut outputs = Vec::new();
        let mut failed = 0usize;
        let mut first_reason: Option<String> = None;
        for input in &request.inputs {
            // The name the files are called by is read without its case:
            // a file called `NOTE.SRT` is a subtitle as much as a file
            // called `note.srt` is.
            let from = input
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_lowercase();
            match self.convert_one(input, &from, extension) {
                Ok(path) => outputs.push(path),
                Err(reason) => {
                    failed += 1;
                    diagnostics::log(
                        Level::Warn,
                        "action",
                        &format!("a file was not converted: {reason}"),
                    );
                    if first_reason.is_none() {
                        first_reason = Some(reason);
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

impl<P: Publisher> Documents<P> {
    /// Turns the pictures into one PDF, a page each, named after the
    /// first of them. The PDF is written beside the pictures under a
    /// name of its own first, and takes its final name only when it is
    /// whole.
    fn put_pictures_in_one_pdf(
        &self,
        inputs: &[std::path::PathBuf],
        first: &std::path::Path,
        extension: &str,
    ) -> Outcome {
        let staging = staging_name(first, extension);
        match crate::media::documents::images_to_pdf(inputs, &staging) {
            Ok(()) => {
                for finale in names_for(first, extension) {
                    if self.publisher.publish(&staging, &finale).is_ok() {
                        return Outcome::Succeeded {
                            outputs: vec![finale],
                        };
                    }
                }
                // Every name of the PDF was taken. The one that was
                // staged is tidied away, and nothing is said to have
                // happened.
                let _ = std::fs::remove_file(&staging);
                diagnostics::log(
                    Level::Warn,
                    "action",
                    "the pictures were put in one PDF, but every name for it was taken",
                );
                Outcome::Failed {
                    reason: Failure::Error(
                        "the PDF could not be given a name that was free".to_string(),
                    ),
                }
            }
            Err(reason) => {
                let _ = std::fs::remove_file(&staging);
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
        }
    }

    /// Converts one file. The conversion is written beside its file
    /// under a name of its own, and only when it is whole does it take
    /// its final name — the plain name, or the next numbered one when
    /// that is taken. Nothing that is already there is ever written
    /// over.
    ///
    /// The one exception is the spreadsheet put into CSV: it writes
    /// every sheet of its book, each already named by its sheet and
    /// numbered out of the way of whatever is there. That naming does
    /// not need the publisher's hand.
    fn convert_one(
        &self,
        input: &std::path::Path,
        from: &str,
        extension: &str,
    ) -> Result<std::path::PathBuf, String> {
        let plain = input.with_extension(extension);
        if matches!(from, "xlsx" | "xls" | "ods" | "tsv") && extension == "csv" {
            return match convert_document(input, &plain, from, extension) {
                Ok(()) => Ok(plain),
                Err(reason) => {
                    let _ = std::fs::remove_file(&plain);
                    Err(reason.name().to_string())
                }
            };
        }

        let staging = staging_name(input, extension);
        match convert_document(input, &staging, from, extension) {
            Ok(()) => {
                for finale in names_for(input, extension) {
                    match self.publisher.publish(&staging, &finale) {
                        Ok(()) => return Ok(finale),
                        // The name was taken: the next numbered one is
                        // tried, the way a file manager renames.
                        Err(crate::media::publish::PublishError::DestinationExists) => continue,
                        Err(crate::media::publish::PublishError::CouldNotMove) => break,
                    }
                }
                // No name could be taken, or the move was refused. What
                // was staged is tidied away; nothing is written over.
                let _ = std::fs::remove_file(&staging);
                Err("the converted file could not be put at its name".to_string())
            }
            Err(reason) => {
                let _ = std::fs::remove_file(&staging);
                Err(reason.name().to_string())
            }
        }
    }
}

/// The name a conversion is written to while it is being made.
///
/// The tools' own per-run staging name is used, so two conversions of
/// one file can never pour into one another's bucket, and a conversion
/// that stops short takes its work back with it.
fn staging_name(original: &std::path::Path, extension: &str) -> std::path::PathBuf {
    crate::radial::tools::staging_name(original, &format!("{extension}.part"))
}

/// Whether any of a run's staged work is still standing in the folder.
#[cfg(test)]
fn a_leftover_in(folder: &std::path::Path) -> bool {
    std::fs::read_dir(folder)
        .expect("the folder is there")
        .filter_map(|entry| entry.ok())
        .any(|entry| entry.file_name().to_string_lossy().contains(".part"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media::publish::PublishError;
    use std::path::Path;


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

    /// A publisher that moves the file the way the system does: the move
    /// is refused when something is already at the name.
    ///
    /// `std::fs::rename` is no use for the part of the shape: on Windows
    /// it replaces what is at the name, which is exactly what must never
    /// happen. So the check is said out loud, and only a free name is
    /// moved onto.
    struct Renamer;

    impl Publisher for Renamer {
        fn publish(&self, temporary: &Path, destination: &Path) -> Result<(), PublishError> {
            if destination.exists() {
                return Err(PublishError::DestinationExists);
            }
            std::fs::rename(temporary, destination).map_err(|_| PublishError::CouldNotMove)
        }
    }

    /// A subtitle of its own making.
    fn a_subtitle(folder: &Path, name: &str) -> std::path::PathBuf {
        let path = folder.join(name);
        std::fs::write(&path, "1\n00:00:00,000 --> 00:00:01,000\nHello.\n")
            .expect("the subtitle could be made");
        path
    }

    /// A fresh folder of a test, and its name.
    fn a_folder(alias: &str) -> std::path::PathBuf {
        let folder = std::env::temp_dir().join(format!(
            "open-tangerine-documents-{}-{alias}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).expect("the test folder could be made");
        folder
    }

    /// The name of a file is taken: the conversion is put at the next
    /// numbered name instead, and the file that was there is exactly as
    /// it was.
    #[test]
    fn a_conversion_never_writes_over_what_is_there() {
        let folder = a_folder("collision");
        let input = a_subtitle(&folder, "note.srt");
        std::fs::write(folder.join("note.vtt"), "the old video text")
            .expect("the existing file could be made");

        let selection = Selection::inspect(std::slice::from_ref(&input));
        let request = Request {
            operation: "convert.vtt".to_string(),
            inputs: vec![input.clone()],
            ..Default::default()
        };
        let documents = Documents { publisher: Renamer };
        let outcome = documents.run(&request, &selection);
        let outputs = match outcome {
            Outcome::Succeeded { outputs } => outputs,
            other => panic!("the conversion failed: {other:?}"),
        };
        assert_eq!(outputs.len(), 1, "one file came of one subtitle");
        assert_eq!(
            outputs[0], folder.join("note (2).vtt"),
            "the taken name was not taken again"
        );
        assert_eq!(
            std::fs::read_to_string(folder.join("note.vtt")).expect("the old file is there"),
            "the old video text",
            "the file that was there was written over"
        );
        assert!(
            !a_leftover_in(&folder),
            "no staging file was left behind"
        );
        let _ = std::fs::remove_dir_all(&folder);
    }

    /// A name that is free is taken as it is, with no number in it.
    #[test]
    fn a_conversion_whose_name_is_free_keeps_the_plain_name() {
        let folder = a_folder("freename");
        let input = a_subtitle(&folder, "note.srt");

        let selection = Selection::inspect(std::slice::from_ref(&input));
        let request = Request {
            operation: "convert.vtt".to_string(),
            inputs: vec![input.clone()],
            ..Default::default()
        };
        let documents = Documents { publisher: Renamer };
        let outcome = documents.run(&request, &selection);
        let outputs = match outcome {
            Outcome::Succeeded { outputs } => outputs,
            other => panic!("the conversion failed: {other:?}"),
        };
        assert_eq!(outputs, vec![folder.join("note.vtt")]);
        assert!(
            std::fs::read_to_string(folder.join("note.vtt"))
                .expect("the file is there")
                .contains("Hello."),
            "the conversion is the file at the new name"
        );
        assert!(
            !a_leftover_in(&folder),
            "no staging file was left behind"
        );
        let _ = std::fs::remove_dir_all(&folder);
    }

    /// The name the files are called by is read without its case: a
    /// file called `note.SRT` is a subtitle as much as a file called
    /// `note.srt` is, and its conversion finds its plain name.
    #[test]
    fn a_file_called_with_a_capped_extension_converts_all_the_same() {
        let folder = a_folder("case");
        let input = folder.join("note.SRT");
        std::fs::write(&input, "1\n00:00:00,000 --> 00:00:01,000\nHello.\n")
            .expect("the subtitle could be made");

        let selection = Selection::inspect(std::slice::from_ref(&input));
        let request = Request {
            operation: "convert.vtt".to_string(),
            inputs: vec![input.clone()],
            ..Default::default()
        };
        let documents = Documents { publisher: Renamer };
        let outcome = documents.run(&request, &selection);
        match outcome {
            Outcome::Succeeded { outputs } => {
                assert_eq!(outputs, vec![folder.join("note.vtt")]);
            }
            other => panic!("the conversion failed: {other:?}"),
        }
        assert!(
            std::fs::read_to_string(folder.join("note.vtt"))
                .expect("the file is there")
                .contains("Hello."),
            "the capped subtitle was really converted"
        );
        let _ = std::fs::remove_dir_all(&folder);
    }
}
