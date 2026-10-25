//! The interaction controller.
//!
//! The controller owns the wheel's state and decides when to show, update,
//! select, cancel or close it. It never touches the platform: everything
//! arrives as a typed [`InteractionEvent`] and everything leaves as a typed
//! [`Effect`].

use std::path::PathBuf;

use super::actions::{Menu, ModifierMap, RadialMode, Segment};
use super::geometry::Point;
use super::state::{Effect, InteractionEvent, InteractionSource, InteractionState};

/// The controller's view of the world.#[derive(Clone, Debug)]
pub struct RadialController {
    state: InteractionState,
    source: InteractionSource,
    mode: RadialMode,
    files: Vec<PathBuf>,
    /// What the wheels offer.
    ///
    /// The controller is told what may be offered; it does not decide. A wheel
    /// with nothing in it is a wheel with nothing on it.
    menu: Menu,
    highlighted: Option<usize>,
    center: Option<Point>,
    /// Whether the wheel takes clicks of its own. True for the development
    /// wheel and after a drag ended without a choice being made.
    interactive: bool,
    /// The mapping from modifier keys to wheels.
    modifiers: ModifierMap,
}

impl Default for RadialController {
    fn default() -> Self {
        Self::new()
    }
}

impl RadialController {
    pub fn new() -> Self {
        Self {
            state: InteractionState::Idle,
            source: InteractionSource::Development,
            mode: RadialMode::Convert,
            files: Vec::new(),
            menu: Menu::empty(),
            highlighted: None,
            center: None,
            interactive: false,
            modifiers: ModifierMap::DEFAULT,
        }
    }

    // ----------------------------------------------------------------- state

    pub fn state(&self) -> InteractionState {
        self.state
    }

    pub fn source(&self) -> InteractionSource {
        self.source
    }

    pub fn mode(&self) -> RadialMode {
        self.mode
    }

    pub fn files(&self) -> &[PathBuf] {
        &self.files
    }

    pub fn segments(&self) -> &[Segment] {
        self.menu.segments(self.mode)
    }

    /// Whether a wheel has anything on it.
    pub fn menu_is_empty(&self, mode: RadialMode) -> bool {
        self.menu.is_empty(mode)
    }

    /// Sets what the wheels may offer.
    ///
    /// The choices of the wheel that is showing may change, and the lit
    /// segment is put out: it may not be on the wheel any more.
    pub fn set_menu(&mut self, menu: Menu) {
        self.menu = menu;
        self.highlighted = None;
    }

    pub fn highlighted(&self) -> Option<usize> {
        self.highlighted
    }

    pub fn center(&self) -> Option<Point> {
        self.center
    }

    /// Whether the wheel should react to the mouse as a normal window.
    pub fn is_interactive(&self) -> bool {
        self.interactive
    }

    /// The mapping from modifier keys to wheels.
    pub fn modifier_map(&self) -> ModifierMap {
        self.modifiers
    }

    /// Changes the mapping from modifier keys to wheels.
    pub fn set_modifier_map(&mut self, map: ModifierMap) {
        self.modifiers = map;
    }
    /// Whether this is a real Explorer drag. The development wheel never is.
    pub fn is_explorer_drag(&self) -> bool {
        self.source == InteractionSource::ExplorerDrag
    }

    // -------------------------------------------------------------- commands

    /// Opens the wheel for the development shortcut.
    ///
    /// This is deliberately its own entry point: it never touches the file
    /// list, it is never reported as an Explorer drag, and the caller is
    /// expected to label it as a test.
    pub fn open_development_wheel(&mut self, center: Point) -> Vec<Effect> {
        if self.state != InteractionState::Idle {
            return vec![Effect::None];
        }
        self.source = InteractionSource::Development;
        self.mode = RadialMode::Convert;
        self.files.clear();
        self.highlighted = None;
        self.center = Some(center);
        self.interactive = true;
        self.state = InteractionState::WheelVisible;
        vec![Effect::ShowWheel]
    }

    /// Called when the chosen action actually starts running.
    pub fn mark_executing(&mut self) {
        if self.state == InteractionState::SelectionPending {
            self.state = InteractionState::Executing;
        }
    }

    /// Called when the chosen action has finished, successfully or not.
    pub fn finish_execution(&mut self) {
        self.reset();
    }

    fn reset(&mut self) {
        self.state = InteractionState::Idle;
        self.source = InteractionSource::Development;
        self.files.clear();
        // What the wheels may offer was for this interaction, and this
        // interaction is over. The application says what may be offered when
        // the next one begins.
        self.menu = Menu::empty();
        self.highlighted = None;
        self.center = None;
        self.interactive = false;
    }

    // ---------------------------------------------------------------- events

    /// Feeds an event to the machine and returns what should happen next.
    pub fn handle(&mut self, event: InteractionEvent) -> Vec<Effect> {
        match self.state {
            InteractionState::Idle => self.handle_idle(event),
            InteractionState::CandidateDrag => self.handle_candidate(event),
            InteractionState::WheelVisible => self.handle_visible(event),
            InteractionState::SelectionPending | InteractionState::Executing => {
                self.handle_busy(event)
            }
            InteractionState::Cancelling => self.handle_cancelling(event),
        }
    }

    fn handle_idle(&mut self, event: InteractionEvent) -> Vec<Effect> {
        match event {
            InteractionEvent::DragStarted { paths } => {
                self.source = InteractionSource::ExplorerDrag;
                self.files = paths;
                self.mode = RadialMode::Convert;
                self.highlighted = None;
                self.interactive = false;
                self.state = InteractionState::CandidateDrag;
                vec![Effect::None]
            }
            InteractionEvent::PointerMoved { position } => {
                self.center = Some(position);
                vec![Effect::None]
            }
            InteractionEvent::Failed { .. } => vec![Effect::None],
            _ => vec![Effect::None],
        }
    }

    fn handle_candidate(&mut self, event: InteractionEvent) -> Vec<Effect> {
        match event {
            InteractionEvent::PointerMoved { position } => {
                // The wheel opens where the cursor is, so keep the last position.
                self.center = Some(position);
                vec![Effect::None]
            }
            InteractionEvent::ModifiersChanged(modifiers) => {
                match self.modifiers.mode_for(modifiers) {
                    Some(mode) => {
                        self.mode = mode;
                        self.highlighted = None;
                        self.state = InteractionState::WheelVisible;
                        vec![Effect::ShowWheel]
                    }
                    // The drag carries on without the wheel: the user may
                    // still press the modifier a moment later.
                    None => vec![Effect::None],
                }
            }
            InteractionEvent::DragEnded => {
                self.reset();
                vec![Effect::None]
            }
            InteractionEvent::DragCancelled => {
                // The drag was cancelled before the wheel was even shown: the
                // wheel goes away and nothing is run.
                self.state = InteractionState::Cancelling;
                self.interactive = false;
                vec![Effect::HideWheel]
            }
            InteractionEvent::DragPayloadResolved { paths } => {
                // The shell's answer for this session is the list. It is set
                // even when it is empty: a session with no files must not keep
                // the files of another session.
                self.files = paths;
                vec![Effect::None]
            }
            InteractionEvent::Failed { .. } => {
                self.state = InteractionState::Cancelling;
                vec![Effect::None]
            }
            _ => vec![Effect::None],
        }
    }

    fn handle_visible(&mut self, event: InteractionEvent) -> Vec<Effect> {
        match event {
            InteractionEvent::ModifiersChanged(modifiers) => {
                // Letting go of the modifier leaves the wheel up: the drag is
                // still going and the user may still choose.
                let Some(wanted) = self.modifiers.mode_for(modifiers) else {
                    return vec![Effect::None];
                };
                if wanted == self.mode {
                    return vec![Effect::None];
                }
                self.mode = wanted;
                self.highlighted = None;
                vec![Effect::UpdateWheel]
            }
            InteractionEvent::PointerMoved { position } => {
                // The wheel stays where it opened; only the pointer moves, and
                // the platform tells us which segment it is over.
                let _ = position;
                vec![Effect::None]
            }
            InteractionEvent::SegmentEntered(index) => {
                let before = self.highlighted;
                self.highlighted = index.filter(|i| *i < self.segments().len());
                if before != self.highlighted {
                    // The log says what changed and what the drawing will do
                    // about it, so that a choice that does not show up can be
                    // traced.
                    let name = self
                        .segments()
                        .get(self.highlighted.unwrap_or(usize::MAX))
                        .map(|segment| segment.label.clone())
                        .unwrap_or_else(|| "the hub".to_string());
                    crate::diagnostics::log(
                        crate::diagnostics::Level::Info,
                        "wheel",
                        &format!(
                            "the pointer is over {name} (segment {:?}), the wheel will light it up",
                            self.highlighted
                        ),
                    );
                }
                vec![Effect::None]
            }
            InteractionEvent::DragPayloadResolved { paths } => {
                // The shell's answer for this session is the list. It is set
                // even when it is empty: a session with no files must not keep
                // the files of another session.
                self.files = paths;
                vec![Effect::None]
            }
            InteractionEvent::SegmentSelected { index } => self.select(index),
            InteractionEvent::DragStarted { paths } => {
                // A new drag after a drop that missed: the wheel is still up,
                // and it follows the new drag. A second drag *during* a drag is
                // ignored, so that one gesture never opens two wheels.
                if self.interactive && self.source == InteractionSource::ExplorerDrag {
                    self.interactive = false;
                    self.state = InteractionState::CandidateDrag;
                    self.files = paths;
                    self.highlighted = None;
                    return vec![Effect::None];
                }
                vec![Effect::None]
            }
            InteractionEvent::DragEnded => {
                // The mouse came up. If it came up on a choice, that choice is
                // taken; otherwise the wheel stays up and starts taking clicks.
                match self.highlighted {
                    Some(index) if index < self.segments().len() => self.select(index),
                    _ => {
                        self.interactive = true;
                        vec![Effect::UpdateWheel]
                    }
                }
            }
            InteractionEvent::DragCancelled => {
                // The drag was cancelled: the file was let go of somewhere
                // else, or Windows ended the drag. Nothing is run, and the
                // wheel goes away.
                self.state = InteractionState::Cancelling;
                self.interactive = false;
                vec![Effect::HideWheel]
            }
            InteractionEvent::ModeStepped(delta) => {
                self.mode = self.mode.stepped(delta);
                self.highlighted = None;
                crate::diagnostics::log(
                    crate::diagnostics::Level::Info,
                    "wheel",
                    &format!("the wheel is now the {} wheel", self.mode.name()),
                );
                vec![Effect::UpdateWheel]
            }
            InteractionEvent::ArrowPressed { clockwise } => {
                self.step_highlight(if clockwise { 1 } else { -1 });
                vec![Effect::None]
            }
            InteractionEvent::ActivatePressed => match self.highlighted {
                Some(index) if index < self.segments().len() => self.select(index),
                _ => vec![Effect::None],
            },
            InteractionEvent::EscapePressed | InteractionEvent::WheelDismissed => {
                self.state = InteractionState::Cancelling;
                self.interactive = false;
                vec![Effect::HideWheel]
            }
            InteractionEvent::Failed { .. } => {
                self.state = InteractionState::Cancelling;
                self.interactive = false;
                vec![Effect::HideWheel]
            }
        }
    }

    fn handle_busy(&mut self, event: InteractionEvent) -> Vec<Effect> {
        match event {
            InteractionEvent::EscapePressed | InteractionEvent::WheelDismissed => {
                self.state = InteractionState::Cancelling;
                vec![Effect::HideWheel]
            }
            InteractionEvent::Failed { .. } => {
                self.state = InteractionState::Cancelling;
                vec![Effect::HideWheel]
            }
            // A new drag while an action runs must not open a second wheel,
            // and a cancellation must not stop the work that is already
            // running: the work is on a worker of its own, and what comes of
            // it is reported when it is done.
            _ => vec![Effect::None],
        }
    }

    fn handle_cancelling(&mut self, event: InteractionEvent) -> Vec<Effect> {
        match event {
            InteractionEvent::WheelDismissed => {
                self.reset();
                vec![Effect::None]
            }
            _ => vec![Effect::None],
        }
    }

    // --------------------------------------------------------------- helpers

    fn step_highlight(&mut self, delta: i32) {
        if self.segments().is_empty() {
            return;
        }
        let count = self.segments().len() as i32;
        let current = self.highlighted.map(|i| i as i32).unwrap_or(-1);
        let next = (current + delta).rem_euclid(count);
        self.highlighted = Some(next as usize);
    }

    fn select(&mut self, index: usize) -> Vec<Effect> {
        let segments = self.segments();
        if index >= segments.len() {
            // An invalid choice must not run anything, but it must not wedge
            // the interaction either: the wheel stays up.
            return vec![Effect::None];
        }
        let segment = segments[index].clone();
        let files = self.files.clone();
        let source = self.source;
        let mode = self.mode;
        self.state = InteractionState::SelectionPending;
        self.interactive = false;
        vec![
            Effect::HideWheel,
            Effect::Run {
                source,
                segment,
                mode,
                files,
            },
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::radial::actions::{ModifierMap, Modifiers};
    use crate::radial::registry::design_menu;

    /// A controller with the design of the wheels on it.
    ///
    /// The controller is told what the wheels may offer; it does not decide.
    /// These tests are about the interaction, so they use the design as the
    /// menu. What may really be offered is the registry's business, and it has
    /// its own tests.
    fn controller() -> RadialController {
        let mut c = RadialController::new();
        c.set_menu(design_menu());
        c
    }

    fn paths() -> Vec<PathBuf> {
        vec![PathBuf::from("C:\\Users\\test\\photo.heic")]
    }

    fn open_wheel(c: &mut RadialController) -> Vec<Effect> {
        c.handle(InteractionEvent::DragStarted { paths: paths() });
        c.handle(InteractionEvent::PointerMoved {
            position: Point::new(500.0, 400.0),
        });
        c.handle(InteractionEvent::ModifiersChanged(Modifiers {
            shift: true,
            ..Modifiers::NONE
        }))
    }

    #[test]
    fn idle_to_candidate_drag() {
        let mut c = controller();
        assert_eq!(c.state(), InteractionState::Idle);
        let effects = c.handle(InteractionEvent::DragStarted { paths: paths() });
        assert_eq!(c.state(), InteractionState::CandidateDrag);
        assert_eq!(effects, vec![Effect::None]);
        assert_eq!(c.files().len(), 1);
    }

    #[test]
    fn candidate_drag_to_visible_wheel() {
        let mut c = controller();
        open_wheel(&mut c);
        assert_eq!(c.state(), InteractionState::WheelVisible);
        assert_eq!(c.mode(), RadialMode::Convert);
        assert_eq!(c.segments().len(), 30);
    }

    #[test]
    fn an_unrelated_mouse_move_does_not_open_the_wheel() {
        let mut c = controller();
        let effects = c.handle(InteractionEvent::PointerMoved {
            position: Point::new(10.0, 10.0),
        });
        assert_eq!(c.state(), InteractionState::Idle);
        assert_eq!(effects, vec![Effect::None]);
    }

    #[test]
    fn holding_shift_without_a_drag_does_nothing() {
        let mut c = controller();
        let effects = c.handle(InteractionEvent::ModifiersChanged(Modifiers {
            shift: true,
            ..Modifiers::NONE
        }));
        assert_eq!(c.state(), InteractionState::Idle);
        assert_eq!(effects, vec![Effect::None]);
    }

    #[test]
    fn releasing_the_modifier_before_the_wheel_opened_keeps_the_candidate() {
        let mut c = controller();
        c.handle(InteractionEvent::DragStarted { paths: paths() });
        c.handle(InteractionEvent::ModifiersChanged(Modifiers::NONE));
        assert_eq!(c.state(), InteractionState::CandidateDrag);
    }

    #[test]
    fn releasing_the_modifier_keeps_the_wheel_up() {
        let mut c = controller();
        open_wheel(&mut c);
        let effects = c.handle(InteractionEvent::ModifiersChanged(Modifiers::NONE));
        assert_eq!(c.state(), InteractionState::WheelVisible);
        assert_eq!(effects, vec![Effect::None]);
    }

    #[test]
    fn another_modifier_changes_the_mode() {
        let mut c = controller();
        c.set_modifier_map(ModifierMap::MACOS);
        open_wheel(&mut c);
        let effects = c.handle(InteractionEvent::ModifiersChanged(Modifiers {
            ctrl: true,
            ..Modifiers::NONE
        }));
        assert_eq!(c.state(), InteractionState::WheelVisible);
        assert_eq!(c.mode(), RadialMode::Tools);
        assert_eq!(c.segments().len(), 8);
        assert_eq!(effects, vec![Effect::UpdateWheel]);
    }

    #[test]
    fn the_modifiers_explorer_uses_switch_wheels_without_closing_them() {
        let mut c = controller();
        open_wheel(&mut c);
        // Control also means "copy" inside Explorer's drag, but the wheel
        // takes the drag before Explorer can act on that: the cursor
        // Explorer shows is the only effect. The wheel stays open, and the
        // modifier picks its wheel.
        let effects = c.handle(InteractionEvent::ModifiersChanged(Modifiers {
            ctrl: true,
            ..Modifiers::NONE
        }));
        assert_eq!(c.state(), InteractionState::WheelVisible);
        assert_eq!(c.mode(), RadialMode::Tools);
        assert_eq!(effects, vec![Effect::UpdateWheel]);
    }

    #[test]
    fn scrolling_steps_through_the_modes() {
        let mut c = controller();
        open_wheel(&mut c);
        c.handle(InteractionEvent::ModeStepped(1));
        assert_eq!(c.mode(), RadialMode::Tools);
        c.handle(InteractionEvent::ModeStepped(1));
        assert_eq!(c.mode(), RadialMode::Actions);
        c.handle(InteractionEvent::ModeStepped(1));
        assert_eq!(c.mode(), RadialMode::Convert);
        c.handle(InteractionEvent::ModeStepped(-1));
        assert_eq!(c.mode(), RadialMode::Actions);
    }

    #[test]
    fn a_valid_selection_runs_the_action() {
        let mut c = controller();
        open_wheel(&mut c);
        let effects = c.handle(InteractionEvent::SegmentSelected { index: 0 });
        assert_eq!(c.state(), InteractionState::SelectionPending);
        assert_eq!(effects.len(), 2);
        assert_eq!(effects[0], Effect::HideWheel);
        match &effects[1] {
            Effect::Run {
                source,
                segment,
                mode,
                files,
            } => {
                assert_eq!(*source, InteractionSource::ExplorerDrag);
                assert_eq!(segment.id, "convert.png");
                assert_eq!(*mode, RadialMode::Convert);
                assert_eq!(files.len(), 1);
            }
            other => panic!("expected a run effect, got {other:?}"),
        }
    }

    #[test]
    fn an_invalid_selection_runs_nothing() {
        let mut c = controller();
        open_wheel(&mut c);
        let effects = c.handle(InteractionEvent::SegmentSelected { index: 99 });
        assert_eq!(c.state(), InteractionState::WheelVisible);
        assert_eq!(effects, vec![Effect::None]);
    }

    #[test]
    fn escape_cancels_and_returns_to_idle() {
        let mut c = controller();
        open_wheel(&mut c);
        let effects = c.handle(InteractionEvent::EscapePressed);
        assert_eq!(c.state(), InteractionState::Cancelling);
        assert_eq!(effects, vec![Effect::HideWheel]);
        c.handle(InteractionEvent::WheelDismissed);
        assert_eq!(c.state(), InteractionState::Idle);
        assert!(c.files().is_empty());
        assert!(c.segments().is_empty());
    }

    #[test]
    fn a_drop_on_a_choice_hides_the_wheel_and_runs_the_operation_once() {
        let mut c = controller();
        open_wheel(&mut c);
        let effects = c.handle(InteractionEvent::SegmentSelected { index: 0 });
        // The wheel goes away at once, and the work is started once.
        assert!(effects.contains(&Effect::HideWheel));
        let runs = effects
            .iter()
            .filter(|effect| matches!(effect, Effect::Run { .. }))
            .count();
        assert_eq!(runs, 1);
    }

    #[test]
    fn a_cancelled_drag_hides_the_wheel_and_runs_nothing() {
        let mut c = controller();
        open_wheel(&mut c);
        let effects = c.handle(InteractionEvent::DragCancelled);
        assert_eq!(c.state(), InteractionState::Cancelling);
        assert!(effects.contains(&Effect::HideWheel));
        assert!(
            !effects
                .iter()
                .any(|effect| matches!(effect, Effect::Run { .. })),
            "a cancelled drag ran something"
        );
    }

    #[test]
    fn a_cancellation_does_not_stop_the_work_that_is_running() {
        let mut c = controller();
        open_wheel(&mut c);
        c.handle(InteractionEvent::SegmentSelected { index: 0 });
        c.mark_executing();
        assert_eq!(c.state(), InteractionState::Executing);
        // The work is on a worker of its own: a cancellation that comes
        // after the drop was accepted must not stop it.
        let effects = c.handle(InteractionEvent::DragCancelled);
        assert!(
            !effects
                .iter()
                .any(|effect| matches!(effect, Effect::Run { .. })),
            "the work was run again"
        );
        assert_ne!(c.state(), InteractionState::Cancelling);
    }

    #[test]
    fn a_new_drag_after_a_cancellation_starts_afresh() {
        let mut c = controller();
        open_wheel(&mut c);
        c.handle(InteractionEvent::DragCancelled);
        c.handle(InteractionEvent::WheelDismissed);
        assert_eq!(c.state(), InteractionState::Idle);
        // The next drag opens its own wheel, with its own files.
        let effects = c.handle(InteractionEvent::DragStarted { paths: paths() });
        assert_eq!(c.state(), InteractionState::CandidateDrag);
        assert_eq!(effects, vec![Effect::None]);
        assert_eq!(c.files().len(), 1);
    }

    #[test]
    fn a_cancellation_is_safe_to_do_twice() {
        let mut c = controller();
        open_wheel(&mut c);
        c.handle(InteractionEvent::DragCancelled);
        // The same cancellation again must not run anything or wedge the
        // interaction.
        let effects = c.handle(InteractionEvent::DragCancelled);
        assert!(
            !effects
                .iter()
                .any(|effect| matches!(effect, Effect::Run { .. })),
            "a second cancellation ran something"
        );
        c.handle(InteractionEvent::WheelDismissed);
        assert_eq!(c.state(), InteractionState::Idle);
    }

    #[test]
    fn repeated_interactions_work() {
        let mut c = controller();
        for _ in 0..5 {
            open_wheel(&mut c);
            assert_eq!(c.state(), InteractionState::WheelVisible);
            c.handle(InteractionEvent::EscapePressed);
            c.handle(InteractionEvent::WheelDismissed);
            assert_eq!(c.state(), InteractionState::Idle);
        }
    }

    #[test]
    fn a_second_drag_does_not_open_a_second_wheel() {
        let mut c = controller();
        open_wheel(&mut c);
        let effects = c.handle(InteractionEvent::DragStarted { paths: paths() });
        assert_eq!(c.state(), InteractionState::WheelVisible);
        assert_eq!(effects, vec![Effect::None]);
    }

    #[test]
    fn a_failure_cleans_up() {
        let mut c = controller();
        open_wheel(&mut c);
        let effects = c.handle(InteractionEvent::Failed {
            reason: "boom".into(),
        });
        assert_eq!(c.state(), InteractionState::Cancelling);
        assert_eq!(effects, vec![Effect::HideWheel]);
        c.handle(InteractionEvent::WheelDismissed);
        assert_eq!(c.state(), InteractionState::Idle);
    }

    #[test]
    fn the_development_wheel_is_not_an_explorer_drag() {
        let mut c = controller();
        let effects = c.open_development_wheel(Point::new(100.0, 100.0));
        assert_eq!(c.state(), InteractionState::WheelVisible);
        assert_eq!(c.source(), InteractionSource::Development);
        assert!(!c.is_explorer_drag());
        assert!(c.files().is_empty());
        assert!(c.is_interactive());
        assert_eq!(effects, vec![Effect::ShowWheel]);
    }

    #[test]
    fn the_development_wheel_cannot_be_opened_twice() {
        let mut c = controller();
        c.open_development_wheel(Point::new(100.0, 100.0));
        let effects = c.open_development_wheel(Point::new(100.0, 100.0));
        assert_eq!(effects, vec![Effect::None]);
    }

    #[test]
    fn arrow_keys_move_the_highlight() {
        let mut c = controller();
        open_wheel(&mut c);
        c.handle(InteractionEvent::ArrowPressed { clockwise: true });
        assert_eq!(c.highlighted(), Some(0));
        c.handle(InteractionEvent::ArrowPressed { clockwise: true });
        assert_eq!(c.highlighted(), Some(1));
        c.handle(InteractionEvent::ArrowPressed { clockwise: false });
        assert_eq!(c.highlighted(), Some(0));
    }

    #[test]
    fn enter_picks_the_highlighted_segment() {
        let mut c = controller();
        open_wheel(&mut c);
        c.handle(InteractionEvent::ArrowPressed { clockwise: true });
        let effects = c.handle(InteractionEvent::ActivatePressed);
        assert_eq!(c.state(), InteractionState::SelectionPending);
        assert_eq!(effects.len(), 2);
    }

    #[test]
    fn a_drop_that_misses_keeps_the_wheel_up_and_makes_it_clickable() {
        let mut c = controller();
        open_wheel(&mut c);
        c.handle(InteractionEvent::SegmentEntered(None));
        let effects = c.handle(InteractionEvent::DragEnded);
        assert_eq!(c.state(), InteractionState::WheelVisible);
        assert!(c.is_interactive());
        assert_eq!(effects, vec![Effect::UpdateWheel]);
    }

    #[test]
    fn a_drop_on_a_segment_takes_it() {
        let mut c = controller();
        open_wheel(&mut c);
        c.handle(InteractionEvent::SegmentEntered(Some(3)));
        let effects = c.handle(InteractionEvent::DragEnded);
        assert_eq!(c.state(), InteractionState::SelectionPending);
        assert!(matches!(effects[1], Effect::Run { .. }));
    }

    #[test]
    fn the_pointer_is_remembered_before_the_wheel_opens() {
        let mut c = controller();
        c.handle(InteractionEvent::DragStarted { paths: paths() });
        // The pointer is read before the modifier is acted on, so that the
        // wheel opens around the pointer and not around where it was.
        let where_the_pointer_is = Point::new(1234.0, 567.0);
        c.handle(InteractionEvent::PointerMoved {
            position: where_the_pointer_is,
        });
        assert_eq!(c.center(), Some(where_the_pointer_is));
        c.handle(InteractionEvent::ModifiersChanged(Modifiers {
            shift: true,
            ..Modifiers::NONE
        }));
        assert_eq!(c.state(), InteractionState::WheelVisible);
        assert_eq!(c.center(), Some(where_the_pointer_is));
    }

    #[test]
    fn the_wheel_stays_where_it_opened() {
        let mut c = controller();
        open_wheel(&mut c);
        let opened_at = c.center();
        // The pointer moves on, but the wheel does not follow it: only the
        // segment under the pointer changes.
        c.handle(InteractionEvent::PointerMoved {
            position: Point::new(9.0, 9.0),
        });
        assert_eq!(c.center(), opened_at);
    }

    #[test]
    fn a_new_drag_after_a_drop_that_missed_opens_the_wheel_again() {
        let mut c = controller();
        open_wheel(&mut c);
        // The drop misses every choice, so the wheel stays up and takes clicks.
        c.handle(InteractionEvent::SegmentEntered(None));
        c.handle(InteractionEvent::DragEnded);
        assert_eq!(c.state(), InteractionState::WheelVisible);
        assert!(c.is_interactive());

        // A new drag follows the wheel.
        c.handle(InteractionEvent::DragStarted { paths: paths() });
        assert_eq!(c.state(), InteractionState::CandidateDrag);
        c.handle(InteractionEvent::PointerMoved {
            position: Point::new(700.0, 300.0),
        });
        c.handle(InteractionEvent::ModifiersChanged(Modifiers {
            shift: true,
            ..Modifiers::NONE
        }));
        assert_eq!(c.state(), InteractionState::WheelVisible);
        assert_eq!(c.center(), Some(Point::new(700.0, 300.0)));
    }

    #[test]
    fn a_second_drag_during_a_drag_does_not_open_a_second_wheel() {
        let mut c = controller();
        open_wheel(&mut c);
        // The wheel is up and belongs to a drag that is still running.
        let effects = c.handle(InteractionEvent::DragStarted { paths: paths() });
        assert_eq!(c.state(), InteractionState::WheelVisible);
        assert_eq!(effects, vec![Effect::None]);
    }

    #[test]
    fn the_file_set_stays_stable_during_an_interaction() {
        let mut c = controller();
        open_wheel(&mut c);
        c.handle(InteractionEvent::PointerMoved {
            position: Point::new(1.0, 1.0),
        });
        assert_eq!(c.files().len(), 1);
    }

    #[test]
    fn the_platform_says_which_segment_the_pointer_is_over() {
        let mut c = controller();
        open_wheel(&mut c);
        c.handle(InteractionEvent::SegmentEntered(Some(4)));
        assert_eq!(c.highlighted(), Some(4));
        c.handle(InteractionEvent::SegmentEntered(None));
        assert_eq!(c.highlighted(), None);
    }

    #[test]
    fn a_segment_outside_the_wheel_is_not_highlighted() {
        let mut c = controller();
        open_wheel(&mut c);
        c.handle(InteractionEvent::SegmentEntered(Some(99)));
        assert_eq!(c.highlighted(), None);
    }

    #[test]
    fn the_payload_can_be_resolved_after_the_drag_started() {
        let mut c = controller();
        c.handle(InteractionEvent::DragStarted { paths: Vec::new() });
        c.handle(InteractionEvent::DragPayloadResolved { paths: paths() });
        assert_eq!(c.files().len(), 1);
    }

    // ------------------------------------------------------- session consistency

    #[test]
    fn a_new_drag_replaces_the_files_of_the_previous_drag() {
        let mut c = controller();
        open_wheel(&mut c);
        assert_eq!(c.files().len(), 1);

        // The next drag carries other files.
        c.handle(InteractionEvent::DragEnded);
        c.handle(InteractionEvent::DragStarted {
            paths: vec![
                PathBuf::from("C:\\Users\\test\\one.txt"),
                PathBuf::from("C:\\Users\\test\\two.txt"),
            ],
        });
        assert_eq!(c.files().len(), 2);
        assert_eq!(c.files()[0], PathBuf::from("C:\\Users\\test\\one.txt"));
    }

    #[test]
    fn a_drag_with_no_files_does_not_keep_the_files_of_the_previous_drag() {
        let mut c = controller();
        open_wheel(&mut c);
        assert_eq!(c.files().len(), 1);

        // The next drag has no files at all: the shell could not say what was
        // being dragged. The old files must not be shown for it.
        c.handle(InteractionEvent::DragEnded);
        c.handle(InteractionEvent::DragStarted { paths: Vec::new() });
        assert!(c.files().is_empty());
    }

    #[test]
    fn the_files_of_a_drag_are_kept_while_the_wheel_is_up() {
        let mut c = controller();
        open_wheel(&mut c);
        assert_eq!(c.files().len(), 1);
        // The drop missed every choice, so the wheel stays up and takes
        // clicks. The files of that drag stay with it, so that a click acts
        // on the files that were dragged.
        c.handle(InteractionEvent::SegmentEntered(None));
        c.handle(InteractionEvent::DragEnded);
        assert_eq!(c.files().len(), 1);
        // Once the interaction is over, the files are gone.
        c.finish_execution();
        assert!(c.files().is_empty());
    }

    #[test]
    fn a_resolved_payload_replaces_the_files_of_the_session() {
        let mut c = controller();
        open_wheel(&mut c);
        c.handle(InteractionEvent::DragPayloadResolved {
            paths: vec![PathBuf::from("C:\\Users\\test\\other.heic")],
        });
        assert_eq!(c.files().len(), 1);
        assert_eq!(c.files()[0], PathBuf::from("C:\\Users\\test\\other.heic"));
    }

    #[test]
    fn a_resolved_payload_of_no_files_clears_the_files_of_the_session() {
        let mut c = controller();
        open_wheel(&mut c);
        // The shell says the selection is empty. The files of the drag are
        // then empty as well, and never the ones of another drag.
        c.handle(InteractionEvent::DragPayloadResolved { paths: Vec::new() });
        assert!(c.files().is_empty());
    }

    #[test]
    fn a_drop_on_the_hub_leaves_the_wheel_up_and_runs_nothing() {
        let mut c = controller();
        open_wheel(&mut c);
        // The pointer was over a segment a moment ago.
        c.handle(InteractionEvent::SegmentEntered(Some(2)));
        assert_eq!(c.highlighted(), Some(2));

        // The drop landed on the hub, so there is no segment under it and
        // the lit one is put out.
        c.handle(InteractionEvent::SegmentEntered(None));
        assert_eq!(c.highlighted(), None);

        // The mouse came up, and nothing is run: the wheel stays up and takes
        // clicks instead.
        let effects = c.handle(InteractionEvent::DragEnded);
        assert_eq!(c.state(), InteractionState::WheelVisible);
        assert!(c.is_interactive());
        assert_eq!(effects, vec![Effect::UpdateWheel]);
    }

    #[test]
    fn a_drop_on_a_segment_chooses_it_and_runs_nothing_else() {
        let mut c = controller();
        open_wheel(&mut c);
        c.handle(InteractionEvent::SegmentEntered(Some(2)));
        c.handle(InteractionEvent::SegmentEntered(Some(4)));

        // The drop landed on the segment that is lit, and the choice is made.
        let effects = c.handle(InteractionEvent::SegmentSelected { index: 4 });
        assert_eq!(c.state(), InteractionState::SelectionPending);
        assert_eq!(effects.len(), 2);
        assert_eq!(effects[0], Effect::HideWheel);
        assert!(matches!(effects[1], Effect::Run { .. }));
    }

    // ------------------------------------------------------- the changing menu

    #[test]
    fn a_wheel_with_nothing_on_it_opens_and_nothing_can_be_chosen() {
        // Nothing may be done with the files that were dragged, so the wheel
        // has nothing on it. It still opens, and nothing can be chosen on it.
        let mut c = RadialController::new();
        c.set_menu(Menu::empty());
        open_wheel(&mut c);
        assert_eq!(c.state(), InteractionState::WheelVisible);
        assert!(c.segments().is_empty());

        // There is no segment to point at.
        c.handle(InteractionEvent::SegmentEntered(Some(0)));
        assert_eq!(c.highlighted(), None);

        // And nothing is chosen.
        let effects = c.handle(InteractionEvent::SegmentSelected { index: 0 });
        assert_eq!(effects, vec![Effect::None]);
        assert_eq!(c.state(), InteractionState::WheelVisible);
    }

    #[test]
    fn the_choices_of_the_wheel_follow_what_may_be_offered() {
        let mut c = controller();
        open_wheel(&mut c);
        assert_eq!(c.segments().len(), 30);

        // The files change, and less may be done with them.
        let mut smaller = Menu::empty();
        smaller.set(
            RadialMode::Convert,
            design_menu().segments(RadialMode::Convert)[..2].to_vec(),
        );
        c.set_menu(smaller);
        assert_eq!(c.segments().len(), 2);
    }

    #[test]
    fn a_change_of_the_menu_puts_out_the_lit_segment() {
        let mut c = controller();
        open_wheel(&mut c);
        c.handle(InteractionEvent::SegmentEntered(Some(4)));
        assert_eq!(c.highlighted(), Some(4));

        // The menu changed, and the segment that was lit may not be on the
        // wheel any more.
        c.set_menu(Menu::empty());
        assert_eq!(c.highlighted(), None);
    }

    #[test]
    fn switching_wheels_does_not_keep_the_choices_of_the_last_one() {
        let mut c = controller();
        open_wheel(&mut c);
        assert_eq!(c.segments().len(), 30);

        // The tools wheel has five choices of its own.
        c.handle(InteractionEvent::ModifiersChanged(Modifiers {
            shift: true,
            ..Modifiers::NONE
        }));
        assert_eq!(c.mode(), RadialMode::Convert);

        let mut menu = design_menu();
        menu.set(RadialMode::Convert, Vec::new());
        c.set_menu(menu);
        assert!(c.segments().is_empty());
        assert!(c.menu_is_empty(RadialMode::Convert));
        assert!(!c.menu_is_empty(RadialMode::Tools));
    }
}
