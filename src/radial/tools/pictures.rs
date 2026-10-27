use std::path::{Path, PathBuf};
use crate::diagnostics::{self, Level};
use crate::media::manifest::Manifest;
use crate::media::publish::Publisher;
use crate::radial::execution::{Failure, Outcome, Request};
use super::{metadata_final_names, ToolError, Tools};

impl<P: Publisher> Tools<P> {
    /// Puts photos on framed canvases with backgrounds, padding, corners, and shadows.
    pub(super) fn run_frame_image(&self, request: &Request) -> Outcome {
        let manifest = Manifest::get();
        let mut outputs = Vec::new();
        let mut failed = 0usize;
        let mut first_reason: Option<String> = None;
        let settings = request.settings.frame_settings();

        for input in &request.inputs {
            let result = self.frame_image_one(input, &manifest, &settings);
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

    pub(super) fn frame_image_one(
        &self,
        input: &Path,
        manifest: &Manifest,
        settings: &crate::media::frame::FrameSettings,
    ) -> Result<PathBuf, ToolError> {
        let source_ext = input
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();
        let source = manifest
            .format_of_extension(&source_ext)
            .or_else(|| manifest.format(&source_ext))
            .ok_or(ToolError::UnsupportedKind)?;
        if crate::media::probe::kind_of(&source.format) != Some(crate::media::probe::Kind::Image) {
            return Err(ToolError::UnsupportedKind);
        }

        let stem = input
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "file".to_string());
        let temporary = input.with_file_name(format!("{stem}.frame-part.png"));

        crate::media::frame::frame_file(input, &temporary, settings)
            .map_err(|_| ToolError::WriteError)?;

        let finals = metadata_final_names(input, "framed", "png");
        self.publish(&finals, &temporary)
    }

    /// Combines two or more photographs into a collage composition.
    pub(super) fn run_collage(&self, request: &Request) -> Outcome {
        if request.inputs.len() < 2 {
            return Outcome::Failed {
                reason: Failure::InvalidInput("at least two photos are needed to make a collage".to_string()),
            };
        }
        let settings = request.settings.collage_settings();
        let first = &request.inputs[0];
        let stem = first
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "collage".to_string());
        let temporary = first.with_file_name(format!("{stem}.collage-part.jpg"));

        if let Err(e) = crate::media::collage::create_collage_file(&request.inputs, &temporary, &settings) {
            return Outcome::Failed {
                reason: Failure::Error(format!("collage generation failed: {e}")),
            };
        }

        let finals = metadata_final_names(first, "collage", "jpg");
        match self.publish(&finals, &temporary) {
            Ok(output) => Outcome::Succeeded {
                outputs: vec![output],
            },
            Err(reason) => Outcome::Failed {
                reason: Failure::Error(reason.name().to_string()),
            },
        }
    }

    /// Draws vector annotations (arrows, boxes, text, pen, blur) onto photos.
    pub(super) fn run_annotate(&self, request: &Request) -> Outcome {
        let manifest = Manifest::get();
        let mut outputs = Vec::new();
        let mut failed = 0usize;
        let mut first_reason: Option<String> = None;
        let annotations = &request.settings.annotations_data;

        for input in &request.inputs {
            let result = self.annotate_one(input, &manifest, annotations);
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

    pub(super) fn annotate_one(
        &self,
        input: &Path,
        manifest: &Manifest,
        annotations: &[crate::media::annotate::Annotation],
    ) -> Result<PathBuf, ToolError> {
        let source_ext = input
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();
        let source = manifest
            .format_of_extension(&source_ext)
            .or_else(|| manifest.format(&source_ext))
            .ok_or(ToolError::UnsupportedKind)?;
        if crate::media::probe::kind_of(&source.format) != Some(crate::media::probe::Kind::Image) {
            return Err(ToolError::UnsupportedKind);
        }

        let stem = input
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "file".to_string());
        let temporary = input.with_file_name(format!("{stem}.annotate-part.png"));

        crate::media::annotate::annotate_file(input, &temporary, annotations)
            .map_err(|_| ToolError::WriteError)?;

        let finals = metadata_final_names(input, "annotated", "png");
        self.publish(&finals, &temporary)
    }

    /// Adjusts photo brightness, contrast, saturation, warmth, and vignette.
    pub(super) fn run_edit_image(&self, request: &Request) -> Outcome {
        let manifest = Manifest::get();
        let mut outputs = Vec::new();
        let mut failed = 0usize;
        let mut first_reason: Option<String> = None;
        let adj = request.settings.photo_adjustments.unwrap_or_default();

        for input in &request.inputs {
            let result = self.adjust_image_one(input, &manifest, &adj);
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

    pub(super) fn adjust_image_one(
        &self,
        input: &Path,
        manifest: &Manifest,
        adj: &crate::media::adjustments::PhotoAdjustments,
    ) -> Result<PathBuf, ToolError> {
        let source_ext = input
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();
        let source = manifest
            .format_of_extension(&source_ext)
            .or_else(|| manifest.format(&source_ext))
            .ok_or(ToolError::UnsupportedKind)?;
        if crate::media::probe::kind_of(&source.format) != Some(crate::media::probe::Kind::Image) {
            return Err(ToolError::UnsupportedKind);
        }

        let stem = input
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "file".to_string());
        let temporary = input.with_file_name(format!("{stem}.adjusted-part.jpg"));

        crate::media::adjustments::adjust_file(input, &temporary, adj)
            .map_err(|_| ToolError::WriteError)?;

        let finals = metadata_final_names(input, "adjusted", "jpg");
        self.publish(&finals, &temporary)
    }
}
