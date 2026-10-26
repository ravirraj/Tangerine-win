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

/// The look a covered part wears.
///
/// The reference's picker lines them up blur, pixelate, box; the mockup
/// stands with a solid box on a fresh page, so solid is what happens when
/// nothing was said.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RedactStyle {
    /// A box of one color.
    #[default]
    Solid,
    /// The part is smeared so nothing in it can be read.
    Blur,
    /// The part is painted in big squares of itself.
    Pixelate,
}

/// One rectangle to cover, in thousandths of the file's own size.
///
/// An area stands in fractions so one drawn rectangle can land on every
/// file of a batch at the same relative place — the way the reference's
/// editor remembers what was drawn. The time range says something only
/// to the video tool; a photo has no time to be in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RedactArea {
    /// Where it starts on the width, in thousandths of it.
    pub x_mille: u32,
    /// Where it starts on the height, in thousandths of it.
    pub y_mille: u32,
    /// How much of the width it covers, in thousandths of it.
    pub w_mille: u32,
    /// How much of the height it covers, in thousandths of it.
    pub h_mille: u32,
    /// Where in a video it starts to be covered, in milliseconds. A
    /// photo's rectangle covers everything.
    pub start_ms: Option<u64>,
    /// Where in a video it is uncovered again, in milliseconds.
    pub end_ms: Option<u64>,
}

impl RedactArea {
    /// The rectangle in pixels, clamped inside a picture of this size.
    ///
    /// What hangs off an edge is cut away; a rectangle that starts at the
    /// very edge, or was drawn with nothing in it, comes out empty.
    pub fn rect_on(&self, width: u32, height: u32) -> (u32, u32, u32, u32) {
        let at = |mille: u32, size: u32| (mille as u64 * size as u64 / 1000).min(size as u64) as u32;
        let x = at(self.x_mille, width);
        let y = at(self.y_mille, height);
        let w = at(self.w_mille, width).min(width.saturating_sub(x));
        let h = at(self.h_mille, height).min(height.saturating_sub(y));
        (x, y, w, h)
    }
}

/// One stretch of a sound to bleep over, in milliseconds.
///
/// Unlike a redaction rectangle, a bleep knows exactly where it stands in
/// time: the window drops markers and the fields are said in
/// milliseconds, not fractions — a sound has no width to be fractional
/// about.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BleepRange {
    /// Where it starts, in milliseconds.
    pub start_ms: u64,
    /// Where it stops, in milliseconds.
    pub end_ms: u64,
}

impl BleepRange {
    /// The stretch as a span of frames, clamped inside a sound of this
    /// many frames.
    ///
    /// What hangs off the end is cut away; a range that starts past the
    /// end of the sound, or was drawn with nothing in it, comes out
    /// empty.
    pub fn frame_span(&self, total_frames: usize, sample_rate: u32) -> (usize, usize) {
        let at =
            |ms: u64| (ms as u128 * sample_rate as u128 / 1000).min(total_frames as u128) as usize;
        let start = at(self.start_ms);
        let end = at(self.end_ms).max(start);
        (start, end)
    }
}

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
    /// Where a split stands, in milliseconds from the start, in order.
    ///
    /// Empty, the tool runs without split points — the way a tool that was
    /// run straight from the wheel, with no window, does. Only `tools.split`
    /// reads these; the tools that keep one stretch of time carry
    /// `start_ms`/`end_ms` instead.
    pub points_ms: Vec<u64>,
    /// The rectangle a crop keeps, in pixels of the source: where it
    /// starts, and how much of it there is.
    ///
    /// Only `tools.crop` reads these. A crop with no rectangle is not a
    /// crop, and the handler says so rather than guessing one.
    pub crop_x: Option<u32>,
    pub crop_y: Option<u32>,
    pub crop_w: Option<u32>,
    pub crop_h: Option<u32>,
    /// Whether the compress preset is the strong one. Nothing said, the
    /// balanced preset is what runs — the way the wheel compressed before
    /// there was a window to ask in.
    ///
    /// Only `tools.compress` reads this.
    pub compress_strong: bool,
    /// A size the compressed file is asked to fit under, in bytes, each
    /// file on its own.
    ///
    /// Only `tools.compress` reads this.
    pub target_bytes: Option<u64>,
    /// The longest edge a compressed video may stand, in pixels, and the
    /// most frames it may show in a second. Nothing said, a video keeps
    /// its size and its rate.
    ///
    /// Only `tools.compress` reads these.
    pub max_long_edge: Option<u32>,
    pub max_fps: Option<u32>,
    /// The entries to write into what a file says about itself, in order:
    /// the tag's name and what it is to say. An empty value takes the tag
    /// away.
    ///
    /// Only `tools.metadata` reads these.
    pub metadata_tags: Vec<(String, String)>,
    /// Whether every tag a file carries is to be taken away.
    ///
    /// Only `tools.metadata` reads this.
    pub metadata_remove_all: bool,
    /// Whether the tags that say where a file was made and by whom are to
    /// be taken away.
    ///
    /// Only `tools.metadata` reads this.
    pub metadata_remove_sensitive: bool,
    /// Loudness normalization target in LUFS, multiplied by 10 (e.g. -160 for -16.0 LUFS).
    ///
    /// Only `tools.normalize` reads this.
    pub normalize_lufs_x10: Option<i32>,
    /// Loudness normalization range in LU, multiplied by 10 (e.g. 110 for 11.0 LU).
    ///
    /// Only `tools.normalize` reads this.
    pub normalize_range_x10: Option<i32>,
    /// True peak limit in dBTP, multiplied by 10 (e.g. -15 for -1.5 dBTP).
    ///
    /// Only `tools.normalize` reads this.
    pub normalize_true_peak_x10: Option<i32>,
    /// Channel conversion mode: "stereo", "mono", "left_only", "right_only", "swap".
    ///
    /// Only `tools.channels` reads this.
    pub channel_mode: Option<String>,
    /// Left channel volume / gain percent (0..200, defaults to 100).
    ///
    /// Only `tools.channels` reads this.
    pub channel_left_gain: Option<u32>,
    /// Right channel volume / gain percent (0..200, defaults to 100).
    ///
    /// Only `tools.channels` reads this.
    pub channel_right_gain: Option<u32>,
    /// The rectangles to cover, in thousandths of the file's own width
    /// and height each: where a rectangle starts, and how much of it
    /// there is. The same fractions land on every file of a batch.
    ///
    /// Only `tools.redact_image` and `tools.redact_video` read these.
    pub redact_areas: Vec<RedactArea>,
    /// The look the covered parts wear: solid, blur, or pixelate.
    ///
    /// Only `tools.redact_image` and `tools.redact_video` read this.
    pub redact_style: Option<RedactStyle>,
    /// How hard the look bites, 0..100 (the reference's own strength is 0..1).
    ///
    /// Only `tools.redact_image` and `tools.redact_video` read this.
    pub redact_strength: Option<u32>,
    /// The color solid boxes wear, as hex "RRGGBB" without the `#`.
    ///
    /// Only `tools.redact_image` and `tools.redact_video` read this.
    pub redact_color: Option<String>,
    /// The stretches of a sound to bleep over, in milliseconds.
    ///
    /// Only `tools.bleep` reads these.
    pub bleep_ranges: Vec<BleepRange>,
    /// How high the bleep tone sings, in hertz (the reference's slider
    /// stands between 300 and 2000).
    ///
    /// Only `tools.bleep` reads this.
    pub bleep_frequency: Option<u32>,
    /// The moments of a video to take as pictures, in milliseconds.
    ///
    /// Only `tools.snapshots` reads these.
    pub snapshots_ms: Vec<u64>,
    /// The speed a video runs at, in hundredths: 100 is the video's own
    /// pace, 200 is twice as fast, 50 is half. Never a heartbeat is
    /// kept: a faster copy sings higher, the way a tape runs.
    ///
    /// Only `tools.speed` reads this.
    pub speed_percent: Option<u32>,
}

impl ToolSettings {
    /// The target integrated loudness in LUFS (defaults to -16.0 LUFS).
    pub fn target_lufs(&self) -> f64 {
        self.normalize_lufs_x10.map(|v| v as f64 / 10.0).unwrap_or(-16.0)
    }

    /// The loudness range in LU (defaults to 11.0 LU).
    pub fn loudness_range(&self) -> f64 {
        self.normalize_range_x10.map(|v| v as f64 / 10.0).unwrap_or(11.0)
    }

    /// The true peak limit in dBTP (defaults to -1.5 dBTP).
    pub fn true_peak(&self) -> f64 {
        self.normalize_true_peak_x10.map(|v| v as f64 / 10.0).unwrap_or(-1.5)
    }

    /// The channel conversion mode (defaults to "stereo").
    pub fn channel_mode(&self) -> &str {
        self.channel_mode.as_deref().unwrap_or("stereo")
    }

    /// Left channel gain multiplier (defaults to 1.0).
    pub fn left_gain_factor(&self) -> f64 {
        self.channel_left_gain.unwrap_or(100) as f64 / 100.0
    }

    /// Right channel gain multiplier (defaults to 1.0).
    pub fn right_gain_factor(&self) -> f64 {
        self.channel_right_gain.unwrap_or(100) as f64 / 100.0
    }

    /// The look the covered parts wear (a solid box when nothing was said —
    /// the way the reference's mockup stands before it is touched).
    pub fn redaction_style(&self) -> RedactStyle {
        self.redact_style.unwrap_or_default()
    }

    /// The strength of the look, 0.0..1.0 (half when nothing was said —
    /// the reference's own default).
    pub fn redaction_strength_fraction(&self) -> f64 {
        self.redact_strength.unwrap_or(50).clamp(0, 100) as f64 / 100.0
    }

    /// The color solid boxes wear, as three bytes (black when nothing was
    /// said, the way the reference's color well stands). Anything that is
    /// not six hex characters reads as the default.
    pub fn redaction_color(&self) -> [u8; 3] {
        match self.redact_color.as_deref() {
            Some(hex) if hex.len() == 6 => {
                let byte = |at: usize| u8::from_str_radix(&hex[at..at + 2], 16).unwrap_or(0);
                [byte(0), byte(2), byte(4)]
            }
            _ => [0, 0, 0],
        }
    }

    /// How strong the look is made, in pixels: between 2% and 30% of the
    /// picture's smaller side, growing with the strength — the reference's
    /// own rule for a blur smear and a pixel square alike.
    pub fn redaction_effect(&self, min_side: u32) -> f64 {
        let share = 0.02 + 0.28 * self.redaction_strength_fraction();
        (share * min_side as f64).max(2.0)
    }

    /// How high the bleep tone sings, in hertz (a thousand when nothing
    /// was said — the reference's own default — clamped inside the
    /// slider's own 300..2000, so a setting from an older file cannot
    /// push the tone where the window could never draw it).
    pub fn bleep_frequency_hz(&self) -> u32 {
        self.bleep_frequency.unwrap_or(1000).clamp(300, 2000)
    }
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
    fn a_handler_that_refuses_what_it_is_given_refuses() {
        let folder = TempFolder::new("not-implemented");
        let path = folder.file("one.png");
        let selection = Selection::inspect(std::slice::from_ref(&path));
        let request = Request {
            // The registry's own flag keeping things out is checked in
            // the registry's tests; here the refusal stands for every
            // handler that turns what it is given away.
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
