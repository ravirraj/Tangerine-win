use std::path::PathBuf;
use crate::media::publish::Publisher;
use crate::media::runner::ProcessRunner;
use crate::radial::execution::{Failure, Outcome, Request};
use super::{metadata_final_names, staging_name, SweepOf, ToolError, Tools};

impl<P: Publisher> Tools<P> {
    /// Joins several alike files into one: videos or PDFs.
    pub(super) fn run_merge(&self, request: &Request, _runner: &dyn ProcessRunner) -> Outcome {
        if request.inputs.len() < 2 {
            return Outcome::Failed {
                reason: Failure::InvalidInput("at least two files are needed to merge".to_string()),
            };
        }
        let first = &request.inputs[0];
        let ext = first
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();

        let result = if ext == "pdf" {
            self.merge_pdfs(&request.inputs)
        } else {
            self.merge_videos(&request.inputs)
        };

        match result {
            Ok(output) => Outcome::Succeeded {
                outputs: vec![output],
            },
            Err(reason) => {
                let name = reason.name();
                Outcome::Failed {
                    reason: Failure::Error(format!("the files could not be merged: {name}")),
                }
            }
        }
    }

    /// Merges multiple PDF files into one, all pages copied in sequence.
    pub(super) fn merge_pdfs(&self, inputs: &[PathBuf]) -> Result<PathBuf, ToolError> {
        let first = &inputs[0];
        let temporary = staging_name(first, "merge.pdf");
        let _sweep = SweepOf(temporary.clone());

        crate::media::pdf::merge(inputs, &temporary).map_err(|err| match err {
            crate::media::pdf::PdfError::LibraryMissing => ToolError::ToolFailed,
            crate::media::pdf::PdfError::NotAPdf => ToolError::ReadError,
            crate::media::pdf::PdfError::RenderFailed => ToolError::ReadError,
            crate::media::pdf::PdfError::TextFailed => ToolError::ReadError,
        })?;

        let dest_names = metadata_final_names(first, "merged", "pdf");
        self.publish(&dest_names, &temporary)
    }

    /// Assembles an organized PDF from pages and rotations specified in settings,
    /// or copies all pages if none were specified.
    pub(super) fn run_organize_pdf(&self, request: &Request) -> Outcome {
        if request.inputs.is_empty() {
            return Outcome::Failed {
                reason: Failure::InvalidInput("at least one file is needed to organize".to_string()),
            };
        }
        let first = &request.inputs[0];
        let temporary = staging_name(first, "organize.pdf");
        let _sweep = SweepOf(temporary.clone());

        let specs: Vec<crate::media::pdf::PageSpec> = if request.settings.pdf_pages.is_empty() {
            let mut all_specs = Vec::new();
            for (doc_idx, input) in request.inputs.iter().enumerate() {
                match crate::media::pdf::page_count(input) {
                    Ok(count) => {
                        for page_idx in 0..count {
                            all_specs.push(crate::media::pdf::PageSpec {
                                source_doc: doc_idx,
                                source_index: page_idx,
                                rotation_degrees: 0,
                            });
                        }
                    }
                    Err(err) => {
                        let name = match err {
                            crate::media::pdf::PdfError::LibraryMissing => ToolError::ToolFailed.name(),
                            _ => ToolError::ReadError.name(),
                        };
                        return Outcome::Failed {
                            reason: Failure::Error(format!("the file could not be read: {name}")),
                        };
                    }
                }
            }
            all_specs
        } else {
            request
                .settings
                .pdf_pages
                .iter()
                .map(|&(source_doc, source_index, rotation_degrees)| {
                    crate::media::pdf::PageSpec {
                        source_doc,
                        source_index,
                        rotation_degrees,
                    }
                })
                .collect()
        };

        if specs.is_empty() {
            return Outcome::Failed {
                reason: Failure::InvalidInput("the document would have no pages".to_string()),
            };
        }

        if let Err(err) = crate::media::pdf::assemble(&request.inputs, &specs, &temporary) {
            let name = match err {
                crate::media::pdf::PdfError::LibraryMissing => ToolError::ToolFailed.name(),
                _ => ToolError::ReadError.name(),
            };
            return Outcome::Failed {
                reason: Failure::Error(format!("the pages could not be organized: {name}")),
            };
        }

        let dest_names = metadata_final_names(first, "organized", "pdf");
        match self.publish(&dest_names, &temporary) {
            Ok(output) => Outcome::Succeeded {
                outputs: vec![output],
            },
            Err(reason) => {
                let name = reason.name();
                Outcome::Failed {
                    reason: Failure::Error(format!("the file could not be saved: {name}")),
                }
            }
        }
    }
}
