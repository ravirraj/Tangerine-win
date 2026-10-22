//! The interaction state machine.
//!
//! Every rule about when the wheel appears, changes or closes lives here, and
//! none of it touches the platform or the renderer: the controller is handed
//! typed events and returns typed effects. That makes the whole interaction
//! testable without a mouse, a screen or Explorer.

use std::path::PathBuf;

use super::actions::{Modifiers, RadialMode, Segment};
use super::geometry::Point;

/// Where the current interaction came from.
///
/// The two sources are kept apart on purpose: a wheel opened by the
/// development shortcut must never be reported as a real Explorer drag.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InteractionSource {
    /// A verified file drag coming out of Explorer.
    ExplorerDrag,
    /// The development shortcut. No files are involved and nothing is real.
    Development,
}

/// The phases of an interaction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InteractionState {
    /// Nothing is happening.
    Idle,
    /// A drag was seen, waiting for the modifier that opens the wheel.
    CandidateDrag,
    /// The wheel is on screen.
    WheelVisible,
    /// A choice was made and the action is about to run.
    SelectionPending,
    /// The action is running.
    Executing,
    /// The wheel is on its way out.
    Cancelling,
}

impl InteractionState {
    pub fn name(self) -> &'static str {
        match self {
            InteractionState::Idle => "Idle",
            InteractionState::CandidateDrag => "CandidateDrag",
            InteractionState::WheelVisible => "WheelVisible",
            InteractionState::SelectionPending => "SelectionPending",
            InteractionState::Executing => "Executing",
            InteractionState::Cancelling => "Cancelling",
        }
    }
}

/// Everything that can happen to an interaction.
#[derive(Clone, Debug, PartialEq)]
pub enum InteractionEvent {
    /// A file drag coming out of Explorer was verified.
    DragStarted { paths: Vec<PathBuf> },
    /// The drag payload was read after the drag had already started.
    ///
    /// This is the one documented way the selected file set may change during
    /// an interaction.
    DragPayloadResolved { paths: Vec<PathBuf> },
    /// The mouse button came up, ending the drag.
    DragEnded,
    /// The drag was cancelled: it ended without a choice, or the pointer
    /// left the wheel and did not come back.
    ///
    /// This is not the same as the drag simply ending. A drag that ends
    /// without a drop on the wheel takes the choice that was lit, because
    /// the person let go of the file over it. A drag that is cancelled takes
    /// nothing: the person let go of the file somewhere else, or Windows
    /// ended the drag.
    DragCancelled,
    /// The modifier keys changed.
    ModifiersChanged(Modifiers),
    /// The pointer moved to a new screen position.
    PointerMoved { position: Point },
    /// The pointer moved onto a segment, or off every segment.
    SegmentEntered(Option<usize>),
    /// A segment was chosen, by a click or by dropping on it.
    SegmentSelected { index: usize },
    /// The wheel should start fading out.
    WheelDismissed,
    /// Escape was pressed.
    EscapePressed,
    /// Enter was pressed.
    ActivatePressed,
    /// An arrow key was pressed.
    ArrowPressed { clockwise: bool },
    /// The wheel was scrolled.
    ModeStepped(i32),
    /// Something went wrong and the interaction has to be cleaned up.
    Failed { reason: String },
}

/// What the controller asks the application to do.
#[derive(Clone, Debug, PartialEq)]
pub enum Effect {
    /// Nothing to do.
    None,
    /// Put the wheel on screen.
    ShowWheel,
    /// The wheel is already up but its contents changed.
    UpdateWheel,
    /// Start taking the wheel off the screen.
    HideWheel,
    /// Run the chosen action.
    Run {
        source: InteractionSource,
        segment: Segment,
        mode: RadialMode,
        files: Vec<PathBuf>,
    },
}
