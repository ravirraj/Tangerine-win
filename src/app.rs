//! The application: the window that is always there and the loop that drives it.
//!
//! The application owns the pieces that have to know about each other: the
//! platform layer reads the world, the controller decides what should happen
//! and the UI draws it. Nothing below the controller knows about Windows.

use std::time::{Duration, Instant};

use windows::Win32::Foundation::{CloseHandle, HANDLE, HWND, WAIT_OBJECT_0};
use windows::Win32::System::Ole::{IDropTarget, OleInitialize, OleUninitialize};
use windows::Win32::System::Threading::{
    CancelWaitableTimer, CreateWaitableTimerW, SetWaitableTimer,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{VK_LBUTTON, VK_T};
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, MSG, MWMO_INPUTAVAILABLE, MsgWaitForMultipleObjectsEx, PM_REMOVE,
    PeekMessageW, QS_ALLINPUT, TranslateMessage,
};

use crate::diagnostics::{self, Level, ScopedTimer};
use crate::media::{locator, probe, runner};
use crate::platform::windows::droptarget::{DropState, make_drop_target};
use crate::platform::windows::explorer::{DragDetector, DragEvent};
use crate::platform::windows::input::{self, Key, KeyTracker};
use crate::platform::windows::monitors;
use crate::platform::windows::overlay::{Overlay, OverlayEvent};
use crate::platform::windows::shellselection;
use crate::radial::actions::{ModifierMap, Modifiers, RadialMode, Segment};
use crate::radial::controller::RadialController;
use crate::radial::execution;
use crate::radial::geometry::{Point, RadialGeometry, WheelPlacement};
use crate::radial::images;
use crate::radial::registry::{self, Environment};
use crate::radial::selection::Selection;
use crate::radial::state::{Effect, InteractionEvent, InteractionSource, InteractionState};
use crate::session::DragSession;
use crate::ui::animation::WheelAnimation;
use crate::ui::theme::Theme;
use crate::ui::wheel::{DISC_SIZE, ViewModel, WheelUi, build_view_model};

/// How often the application looks at the world, in milliseconds.
const TICK_INTERVAL: u32 = 16;

/// The revision this program was built from.
///
/// It is put in the program when it is built, so that a copy of it can say
/// where it came from.
pub fn revision() -> &'static str {
    env!("OPEN_TANGERINE_REVISION")
}

/// What the application was asked to do on the command line.
#[derive(Clone, Debug)]
pub struct Options {
    /// Open the wheel straight away, for a look at the design.
    pub dev_wheel: bool,
    /// Stop after this long. Used by the automated checks.
    pub exit_after: Option<Duration>,
    /// The modifier mapping to use.
    pub modifier_map: ModifierMap,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            dev_wheel: false,
            exit_after: None,
            modifier_map: ModifierMap::DEFAULT,
        }
    }
}

/// Reads the command line.
pub fn parse_options(arguments: &[String]) -> Result<Options, String> {
    let mut options = Options::default();
    let mut index = 1;
    while index < arguments.len() {
        match arguments[index].as_str() {
            "--dev-wheel" => options.dev_wheel = true,
            "--advanced-modifiers" => {
                // The mapping the macOS product uses, for comparison.
                options.modifier_map = ModifierMap::MACOS;
            }
            "--exit-after" => {
                index += 1;
                let value = arguments
                    .get(index)
                    .ok_or("--exit-after needs a number of seconds")?;
                let seconds: f64 = value
                    .parse()
                    .map_err(|_| "--exit-after needs a number of seconds")?;
                options.exit_after = Some(Duration::from_secs_f64(seconds));
            }
            "--version" => {
                return Err(format!(
                    "Open Tangerine for Windows, revision {}",
                    revision()
                ));
            }
            "--help" | "-h" => {
                return Err(
                    "Open Tangerine for Windows\n\n  --dev-wheel          open the wheel straight away\n  --advanced-modifiers  also let Ctrl and Alt open their wheels\n  --exit-after <s>     quit after so many seconds\n  --version            say which revision this program is\n"
                        .to_string(),
                );
            }
            other => return Err(format!("unknown option {other}")),
        }
        index += 1;
    }
    Ok(options)
}

/// What a worker sends back.
///
/// The workers are other threads, so what they have to say comes back
/// through a channel and is picked up when the next frame is drawn.
enum Message {
    /// The programs have been asked what they can do.
    Probed(probe::Capabilities),
    /// An operation has finished: the name of the operation, and what came
    /// of it.
    Finished(String, execution::Outcome),
}

/// Asks the programs what they can do.
///
/// The question is asked on a worker, because it starts other programs and
/// takes a moment. Nothing waits for it: the wheel shows what it can until
/// the answer comes, and the answer is used from then on.
fn ask_what_the_machine_can_do(sender: std::sync::mpsc::Sender<Message>) {
    std::thread::spawn(move || {
        let Some(tools) = locator::find() else {
            diagnostics::log(
                Level::Info,
                "media",
                "no conversion program was found, so nothing that needs one is offered",
            );
            return;
        };
        let system = runner::SystemRunner;
        match probe::Capabilities::probe(&tools, &system) {
            Ok(capabilities) => {
                let environment = capabilities.environment();
                let processor = environment.processor("image");
                diagnostics::log(
                    Level::Info,
                    "media",
                    &format!(
                        "the conversion program can read {} format(s) and write {}",
                        processor
                            .map(|processor| processor.reads.len())
                            .unwrap_or(0),
                        processor
                            .map(|processor| processor.writes.len())
                            .unwrap_or(0),
                    ),
                );
                let _ = sender.send(Message::Probed(capabilities));
            }
            Err(_) => diagnostics::log(
                Level::Warn,
                "media",
                "the conversion program could not be asked what it can do",
            ),
        }
    });
}

/// The running application.
struct App {
    controller: RadialController,
    animation: WheelAnimation,
    ui: WheelUi,
    overlay: Overlay,
    drag: DragDetector,
    keys: KeyTracker,
    last_modifiers: Modifiers,
    last_tick: Instant,
    scale: f32,
    options: Options,
    last_model: Option<ViewModel>,
    overlay_origin: (i32, i32),
    dev_key_down: bool,
    /// Whether the current press has already been reported.
    press_reported: bool,
    quit: bool,
    /// What the drag that is over the wheel is carrying.
    drop_state: std::rc::Rc<DropState>,
    /// How the drag that is over the wheel is getting on.
    ///
    /// This is where the acceptance of a drop lives, and where the leave of
    /// the pointer is given a moment to come back. Every delayed thing is
    /// checked against the drag it belongs to, so that a cancellation of one
    /// drag cannot touch another.
    lifecycle: crate::lifecycle::DragLifecycle,
    /// The message about what came of the last operation.
    ///
    /// The message is shown on the wheel's own window, so that it can be
    /// shown after the wheel has gone: the work goes on while the wheel is
    /// away.
    notice: Option<crate::ui::notice::Showing>,
    /// The colours the wheel is drawn with.
    theme: Theme,
    /// The COM object Windows talks to when a drag passes over the wheel.
    drop_target: IDropTarget,
    /// Watches the scroll wheel while the wheel is on screen.
    wheel_watcher: Option<input::WheelWatcher>,
    /// When the wheel was asked to open, to measure how long it took.
    opened_at: Option<Instant>,
    /// Whether the keyboard, rather than the pointer, chooses the segment.
    keyboard_owns_highlight: bool,
    /// Where the pointer was when the keyboard took over.
    cursor_when_the_keyboard_took_over: Point,
    /// How many frames have been drawn.
    frames: u64,
    /// How long the frames took altogether.
    frame_time: std::time::Duration,
    /// Whether OLE was started on this thread.
    ole: windows::core::Result<()>,
    /// Whether the wheel is on the screen.
    wheel_open: bool,
    /// Whether the placement of the wheel has been logged already.
    placement_logged: bool,
    /// Where the wheel was put the last time, so that it is only moved when it
    /// really has to be.
    placed_centre: Option<Point>,
    /// Whether the wheel has been put on the screen.
    placed: bool,
    /// Where the wheel is, and how big it is. The drawing and the pointer
    /// test both use this.
    placement: Option<WheelPlacement>,
    /// Whether the wheel still has to become a drop target.
    pending_drop_target: bool,
    /// Keeps a second copy of the program from opening a second wheel.
    /// It is only held, never read: closing it gives the right back.
    _instance: Option<windows::Win32::Foundation::HANDLE>,
    /// The files of the drag that is running, and where they came from.
    session: DragSession,
    /// The shell's object for the Explorer window the press was on.
    ///
    /// It is found while Explorer is idle, because the shell has to wait for
    /// Explorer while a drag is going.
    shell_window: Option<shellselection::Window>,
    /// The end of a drag that is waiting for the drop.
    ///
    /// Windows says where the mouse was let go of a moment after the mouse
    /// comes up, and that is a better answer than the pointer, which the drag
    /// may have moved about. So the choice waits for the drop.
    pending_release: Option<PendingRelease>,
    /// What the files of the drag are.
    selection: Selection,
    /// What the machine can do, once the programs have been asked.
    environment: Environment,
    /// What the programs can do, once they have been found and asked.
    ///
    /// There is nothing here until the probe has finished, and until then
    /// the operations that need a program are not offered.
    capabilities: Option<probe::Capabilities>,
    /// What the workers have to say.
    messages: std::sync::mpsc::Receiver<Message>,
    /// The end of the channel the workers are given.
    sender: std::sync::mpsc::Sender<Message>,
}

/// The end of a drag that has not been acted on yet.
struct PendingRelease {
    /// How many frames it has been waiting.
    frames: u32,
}

/// How many frames the choice waits for the drop before it takes the segment
/// that is lit instead.
///
/// The drop is handed over within a frame of the mouse coming up, so this is
/// a long wait in practice. It is only reached when the drop does not come at
/// all.
const RELEASE_WAIT_FRAMES: u32 = 4;

impl App {
    fn new(options: Options) -> Result<Self, String> {
        // Only one wheel at a time. A second copy of the program would put a
        // second wheel on the screen, in a different place, and the two would
        // fight over the screen.
        let instance = claim_single_instance()
            .ok_or_else(|| "another copy of Open Tangerine is already running".to_string())?;
        // The wheel is drawn at the scale of the monitor it is on, so the
        // process has to know about the scale of each monitor.
        set_dpi_awareness();
        // OLE has to be available on this thread before the drag payload can
        // be read from the OLE clipboard.
        // SAFETY: the call only prepares the thread for OLE.
        let ole = unsafe { OleInitialize(None) };
        if ole.is_err() {
            diagnostics::error("ole", "OLE could not be started on this thread");
        }
        // The scale of the display the pointer is on decides how big the
        // wheel is, and that has to be known before the window is made.
        let cursor = input::cursor_position().unwrap_or(Point::new(960.0, 540.0));
        let scale = detect_scale(cursor);
        // The wheel is drawn in the theme of the appearance of the system: a
        // person who has a light Windows gets a light wheel.
        let theme = Theme::for_appearance(crate::platform::windows::appearance::prefers_dark());
        diagnostics::log(
            Level::Info,
            "wheel",
            &format!(
                "the wheel is drawn in the {} theme",
                if theme == Theme::dark() {
                    "dark"
                } else {
                    "light"
                }
            ),
        );
        let disc = (DISC_SIZE * scale).round().max(64.0) as u32;
        let overlay = Overlay::new(disc, disc).map_err(|error| error.to_string())?;
        let drop_state = std::rc::Rc::new(DropState::default());
        let drop_target = make_drop_target(drop_state.clone());
        let ui = WheelUi::new(scale).map_err(|error| error.to_string())?;
        let mut controller = RadialController::new();
        controller.set_modifier_map(options.modifier_map);

        // The workers are other threads, and what they have to say comes
        // back through this channel.
        let (sender, messages) = std::sync::mpsc::channel::<Message>();
        ask_what_the_machine_can_do(sender.clone());

        Ok(Self {
            controller,
            animation: WheelAnimation::new(),
            ui,
            overlay,
            drag: DragDetector::new(),
            keys: KeyTracker::new(),
            last_modifiers: Modifiers::NONE,
            last_tick: Instant::now(),
            scale,
            options,
            last_model: None,
            overlay_origin: (0, 0),
            dev_key_down: false,
            press_reported: false,
            quit: false,
            drop_state,
            drop_target,
            lifecycle: crate::lifecycle::DragLifecycle::new(),
            notice: None,
            theme,
            wheel_watcher: None,
            opened_at: None,
            keyboard_owns_highlight: false,
            cursor_when_the_keyboard_took_over: Point::new(0.0, 0.0),
            frames: 0,
            frame_time: std::time::Duration::ZERO,
            ole,
            wheel_open: false,
            placement_logged: false,
            placed_centre: None,
            placed: false,
            placement: None,
            pending_drop_target: false,
            _instance: Some(instance),
            session: DragSession::new(),
            shell_window: None,
            pending_release: None,
            selection: Selection::default(),
            environment: Environment::none(),
            capabilities: None,
            messages,
            sender: sender.clone(),
        })
    }

    /// Hands an event to the controller and applies what comes back.
    fn dispatch(&mut self, event: InteractionEvent) {
        let before = self.controller.state();
        let effects = self.controller.handle(event);
        let after = self.controller.state();
        if before != after {
            diagnostics::state_change(before.name(), after.name(), "event");
        }
        for effect in effects {
            self.apply(effect);
        }
    }

    fn apply(&mut self, effect: Effect) {
        match effect {
            Effect::None => {}
            Effect::ShowWheel => {
                // The pointer is read at the moment the wheel is asked for, so
                // that the wheel opens around the pointer and not around
                // wherever it was a moment ago.
                let cursor = self.capture_cursor();
                let centre = self.controller.center().or(cursor);
                diagnostics::log(
                    Level::Info,
                    "wheel",
                    &format!(
                        "the wheel is opening in the {} mode with {} segment(s) on {} file(s), pointer {cursor:?}, centre {centre:?}, drag {}",
                        self.controller.mode().name(),
                        self.controller.segments().len(),
                        self.controller.files().len(),
                        self.drag.session()
                    ),
                );
                if self.wheel_open {
                    // The wheel is already up, because a new drag has
                    // followed a drop that missed. It moves to the new pointer
                    // and takes the drop over again, but it does not start
                    // its entrance again, which would make it jump.
                    // The wheel always takes the mouse. While a drag is running
                    // the drag owns it anyway, and the wheel has to be
                    // clickable once the drag is over.
                    self.overlay.set_mouse_through(false);
                    // The files the drop target is told belong to the drag
                    // that is running now, and to no other.
                    self.drop_state.session.set(self.session.id());
                    if self.controller.is_explorer_drag()
                        && let Err(error) = self.overlay.register_drop_target(&self.drop_target)
                    {
                        diagnostics::error(
                            "drag",
                            &format!("the wheel could not become a drop target: {error}"),
                        );
                    }
                    return;
                }
                self.wheel_open = true;
                diagnostics::log(
                    Level::Info,
                    "overlay",
                    &format!(
                        "the overlay is on the screen: {}, placed: {}",
                        self.overlay.is_visible(),
                        self.overlay.window_rect() != (0, 0, 0, 0)
                    ),
                );
                self.animation.show();
                self.animation.set_highlight(self.controller.highlighted());
                // The wheel always takes the mouse. While a drag is running the
                // drag owns it anyway, and the wheel has to be clickable once
                // the drag is over.
                self.overlay.set_mouse_through(false);
                self.opened_at = Some(Instant::now());
                // A finer timer while the wheel is on screen, so that the
                // motion is even.
                self.overlay.set_fine_timer(true);
                // The scroll wheel is watched only while the wheel is up.
                if self.wheel_watcher.is_none() {
                    match input::WheelWatcher::start() {
                        Ok(watcher) => self.wheel_watcher = Some(watcher),
                        Err(error) => diagnostics::error("input", &error),
                    }
                }
                // While a drag is running the wheel becomes a drop target.
                // That is the only way to learn what the drag is carrying,
                // and it is also how the drop is taken over instead of
                // landing in the folder underneath.
                //
                // The registration waits until the wheel has been drawn: the
                // drag looks for the window under the pointer, and a window
                // that is showing nothing cannot be found.
                if self.controller.is_explorer_drag() {
                    self.pending_drop_target = true;
                    // The selection may not have been complete when the drag
                    // began, so the shell is asked once more now that the
                    // wheel is opening.
                    self.refresh_payload();
                    // The files are looked at here, and not in the drag
                    // itself: the look is cheap, but a slow disk must not
                    // hold the wheel up.
                    self.refresh_selection();
                } else {
                    // The development wheel is a look at the design, and the
                    // design is every operation there is.
                    self.controller.set_menu(registry::design_menu());
                }
            }
            Effect::UpdateWheel => {
                self.animation.set_highlight(self.controller.highlighted());
            }
            Effect::HideWheel => {
                // The wheel is going away. Everything that belongs to it is
                // undone once, here, and not again on every frame.
                if !self.wheel_open {
                    return;
                }
                self.wheel_open = false;
                self.animation.hide();
                // The wheel is no longer a drop target, so a drag that is still
                // going is left to the program underneath.
                self.overlay.revoke_drop_target();
                // The scroll wheel is watched only while the wheel is up.
                self.wheel_watcher = None;
                // The timer goes back to what it was.
                self.overlay.set_fine_timer(false);
            }
            Effect::Run {
                source,
                segment,
                mode,
                files,
            } => {
                self.controller.mark_executing();
                self.run_action(source, &segment, mode, &files);
                self.controller.finish_execution();
            }
        }
    }

    /// Runs the chosen action.
    ///
    /// The work is done on a worker, so that the wheel keeps moving while
    /// it runs. What comes of it is reported when the next frame is drawn.
    fn run_action(
        &mut self,
        source: InteractionSource,
        segment: &Segment,
        mode: RadialMode,
        files: &[std::path::PathBuf],
    ) {
        diagnostics::log(
            Level::Info,
            "action",
            &format!(
                "{} ({}) in the {} wheel on {} file(s) from drag {} in the window {:?} ({}), source {:?}. The work is done on a worker.",
                segment.label,
                segment.id,
                mode.name(),
                files.len(),
                self.session.id(),
                self.session.source(),
                self.session.origin(),
                source
            ),
        );

        // What the registry knows about the operation that was chosen. The
        // name of the operation is its identity, so this is also the check
        // that the menu and the registry agree.
        //
        // The operation is looked up by that name and is checked again
        // against the files of the drag: what is on the screen must be what
        // is run. A choice that was made a moment ago, before the files
        // changed, is not run.
        let request = execution::Request {
            operation: segment.id.clone(),
            inputs: files.to_vec(),
        };
        self.start_operation(request);
    }

    /// Starts the work of an operation.
    ///
    /// The work is done on a worker: a conversion can take a while, and the
    /// wheel has to keep moving while it does. What comes of it is reported
    /// when the next frame is drawn.
    fn start_operation(&mut self, request: execution::Request) {
        let Some(capabilities) = self.capabilities.clone() else {
            diagnostics::log(
                Level::Warn,
                "action",
                "the operation was not started: the conversion program is not there",
            );
            // The person is told: nothing happened, and why.
            self.show_notice(
                "The program that does the work is not there".to_string(),
                true,
            );
            return;
        };
        let selection = self.selection.clone();
        let environment = self.environment();
        let handler = images::Pictures {
            tools: capabilities.tools,
            publisher: crate::platform::windows::publish::WindowsPublisher,
        };
        let sender = self.sender.clone();
        let operation = request.operation.clone();
        std::thread::spawn(move || {
            let outcome = execution::run(&request, &selection, &environment, &handler);
            let _ = sender.send(Message::Finished(operation, outcome));
        });
    }

    /// Takes what the workers have to say.
    fn take_messages(&mut self) {
        while let Ok(message) = self.messages.try_recv() {
            match message {
                Message::Probed(capabilities) => {
                    self.environment = capabilities.environment();
                    self.capabilities = Some(capabilities);
                    // The wheel may now offer more than it did.
                    let menu = registry::menu_for(&self.selection, &self.environment);
                    self.controller.set_menu(menu);
                    self.report_availability();
                }
                Message::Finished(operation, outcome) => {
                    self.report_outcome(&operation, &outcome);
                }
            }
        }
    }

    /// Says what came of an operation.
    fn report_outcome(&mut self, operation: &str, outcome: &execution::Outcome) {
        match outcome {
            execution::Outcome::Succeeded { outputs } => {
                diagnostics::log(
                    Level::Info,
                    "action",
                    &format!("the operation {operation} wrote {} file(s)", outputs.len()),
                );
                let what = registry::operation_by_id(operation)
                    .map(|operation| operation.label.to_string())
                    .unwrap_or_else(|| "The operation".to_string());
                let count = outputs.len();
                let noun = if count == 1 { "file" } else { "files" };
                self.show_notice(format!("{what}: {count} {noun} written"), false);
            }
            execution::Outcome::Cancelled => {
                diagnostics::log(
                    Level::Info,
                    "action",
                    &format!("the operation {operation} was cancelled"),
                );
                self.show_notice("The operation was cancelled".to_string(), true);
            }
            execution::Outcome::Failed { reason } => {
                // What the operation would have done to the files is said as
                // well: an operation that writes over the original is the one
                // that changes the user's files.
                let changes = registry::operation_by_id(operation)
                    .map(|operation| operation.is_destructive())
                    .unwrap_or(false);
                diagnostics::log(
                    Level::Warn,
                    "action",
                    &format!(
                        "the operation {operation} was not run: {} (it would {} the files, and the selection was {})",
                        reason.name(),
                        if changes { "change" } else { "leave" },
                        self.selection.shape().name(),
                    ),
                );
                // The message says what was wrong, and no more than that: no
                // name of a file and no path.
                self.show_notice(reason.message(), true);
            }
        }
    }

    /// Shows a message about what came of an operation.
    fn show_notice(&mut self, text: String, failed: bool) {
        diagnostics::log(Level::Info, "notice", &format!("the message is: {text}"));
        self.notice = Some(crate::ui::notice::Showing::new(
            text,
            failed,
            Instant::now(),
        ));
    }

    /// The scroll wheel moves between the three wheels.
    fn handle_scroll(&mut self) {
        let Some(watcher) = &self.wheel_watcher else {
            return;
        };
        if self.controller.state() != InteractionState::WheelVisible {
            return;
        }
        let scrolls = watcher.take_scrolls();
        for delta in scrolls {
            if delta == 0.0 {
                continue;
            }
            // Scrolling down moves on to the next wheel, the same way round
            // the wheel in the macOS product does.
            let step = if delta < 0.0 { 1 } else { -1 };
            diagnostics::log(
                Level::Info,
                "wheel",
                &format!("the wheel was scrolled by {delta}"),
            );
            self.dispatch(InteractionEvent::ModeStepped(step));
        }
    }

    /// Works out which segment the pointer is over and tells the controller.
    ///
    /// The pointer is the only thing that says where the pointer is. The drag
    /// that may be passing over the wheel is not asked: it would answer with a
    /// different point, in a different set of numbers, and the two would take
    /// turns deciding which segment is chosen.
    fn handle_hover(&mut self) {
        if self.controller.state() != InteractionState::WheelVisible {
            return;
        }
        let Some(cursor) = self.hover_point() else {
            return;
        };
        if self.keyboard_owns_highlight {
            // The keyboard chose the segment; the pointer has to move before
            // it takes over again.
            if cursor == self.cursor_when_the_keyboard_took_over {
                return;
            }
            self.keyboard_owns_highlight = false;
        }
        self.cursor_when_the_keyboard_took_over = cursor;

        self.ensure_placement();
        let index = self.segment_at(cursor);
        if index != self.controller.highlighted() {
            self.dispatch(InteractionEvent::SegmentEntered(index));
        }
    }

    /// Where the pointer is, in screen pixels.
    ///
    /// The pointer is the truth. The drag that may be passing over the wheel
    /// is not asked: it answers with the place it last heard of the pointer,
    /// and that place can be a long way behind the pointer that is on the
    /// screen, and can miss the places the pointer has been on the way. The
    /// two would then take turns deciding which segment is chosen.
    ///
    /// Both are in the same space: the pointer is read with `GetCursorPos`,
    /// which gives physical screen pixels, and the wheel's shape is worked out
    /// in physical screen pixels too.
    ///
    /// The drop is a different matter. There the place the drag hands over is
    /// the place the file was let go of, and that is what `handle_drop` uses.
    fn hover_point(&self) -> Option<Point> {
        input::cursor_position()
    }

    /// One frame.
    fn tick(&mut self) {
        let timer = ScopedTimer::new("frame");
        let now = Instant::now();
        let delta = (now - self.last_tick).as_secs_f32().min(0.1);
        self.last_tick = now;

        self.handle_keys();
        // What the workers have to say is taken first: the answer about
        // what the machine can do changes what the wheel offers.
        self.take_messages();
        // The drop comes before the end of the drag. Windows hands over the
        // place the mouse was let go of, and that is a better answer than the
        // pointer, which the drag may have moved about. The end of the drag
        // is only the end of the drag when there was no drop.
        self.handle_drop();
        self.handle_drag();
        self.handle_release();
        self.handle_leave(now);
        self.handle_modifiers();
        self.handle_scroll();
        self.handle_hover();
        self.update_visuals(delta);

        if let Some(limit) = self.options.exit_after
            && diagnostics::since_start() >= limit
        {
            self.quit = true;
        }
        self.frames += 1;
        self.frame_time += timer.elapsed();
        drop(timer);
    }

    /// How the application has been getting on.
    fn summary(&self) -> String {
        let frames = self.frames.max(1);
        let average = self.frame_time.as_secs_f64() * 1000.0 / frames as f64;
        let seconds = diagnostics::since_start().as_secs_f64();
        format!(
            "{} frame(s) in {:.1} s, {:.1} frames a second, {:.2} ms of work a frame",
            self.frames,
            seconds,
            self.frames as f64 / seconds.max(0.001),
            average
        )
    }

    /// The drag that is over the wheel.
    ///
    /// While a drag passes over the wheel, Windows tells us what it carries
    /// and where the pointer is. That is the only place the file list of an
    /// Explorer drag can be read from.
    fn handle_drop(&mut self) {
        let state = self.drop_state.clone();

        // The drop target may also have been told what is being dragged. That
        // answer is only a check: the list the shell gave is the one the
        // wheel uses, so that the count cannot change halfway through.
        let paths = state.paths.borrow().clone();
        if !paths.is_empty() && state.session.get() != self.session.id() {
            // The files are told to the application while the drag is over the
            // window. They belong to the drag that was running when the window
            // became a drop target, so a drag that came after them is not
            // given their files.
            diagnostics::log(
                Level::Debug,
                "drag",
                &format!(
                    "the drop target reported {} file(s) for drag {}, but drag {} is running",
                    paths.len(),
                    state.session.get(),
                    self.session.id()
                ),
            );
            return;
        }
        // The answer is taken once for a drag. The same file may be dragged
        // again, and then the answer is taken again, for the new drag.
        if self.session.take_drop_target_payload(&paths) {
            if !self.session.is_resolved() {
                // The shell could not say what is being dragged, which is the
                // case for a drag that started on the desktop. The drop target
                // is told what the drag carries, and its answer is the one the
                // wheel uses.
                let session = self.session.id();
                if self
                    .session
                    .resolve_from_drop_target(session, paths.clone())
                {
                    diagnostics::log(
                        Level::Info,
                        "drag",
                        &format!(
                            "the drop target reports {} file(s) for drag {session}",
                            paths.len()
                        ),
                    );
                    self.dispatch(InteractionEvent::DragPayloadResolved { paths });
                    self.refresh_selection();
                }
            } else if paths != self.session.paths() {
                diagnostics::log(
                    Level::Warn,
                    "drag",
                    &format!(
                        "the drop target reports {} file(s) but the shell reported {} for drag {}",
                        paths.len(),
                        self.session.paths().len(),
                        self.session.id()
                    ),
                );
            } else {
                diagnostics::log(
                    Level::Debug,
                    "drag",
                    &format!("the drop target agrees: {} file(s)", paths.len()),
                );
            }
        }

        // The drop itself. The `Drop` callback clears the "a drag is over the
        // wheel" flag, so the drop is not looked for there: it has a flag of
        // its own, and that flag is taken exactly once, so that a second
        // callback cannot choose a second time.
        if !state.take_drop() {
            return;
        }
        diagnostics::log(Level::Info, "drag", "the drag was dropped on the wheel");
        // The end of the drag no longer has to be acted on: the drop is the
        // choice.
        self.pending_release = None;
        if self.controller.state() != InteractionState::WheelVisible {
            // The wheel is not up any more. The drop was late, and there is
            // nothing left to choose.
            return;
        }
        // The drag says where the drop landed. That is the truth: the drag
        // owns the pointer while it runs and may have moved it about, and the
        // mouse may already be somewhere else by the time this is read.
        let point = state.screen_point().or_else(input::cursor_position);
        let index = point.and_then(|point| self.segment_at(point));
        diagnostics::log(
            Level::Info,
            "drag",
            &format!("the drop landed at {point:?}, on segment {index:?}"),
        );
        // The segment under the drop is lit, whether there is one or not, so
        // that the end of the drag cannot choose another one.
        self.dispatch(InteractionEvent::SegmentEntered(index));
        match index {
            Some(index) => {
                // The drop was accepted. Nothing that comes afterwards may
                // undo it: the work is started on a worker of its own, and it
                // goes on whether the wheel is up or not.
                self.lifecycle.accept();
                diagnostics::log(
                    Level::Info,
                    "drag",
                    &format!(
                        "the drop of drag {} was accepted on segment {index}",
                        self.lifecycle.session()
                    ),
                );
                // The choice is made with the files that belong to this drag,
                // and both are taken before the wheel is hidden.
                self.dispatch(InteractionEvent::SegmentSelected { index });
            }
            None => {
                // The file was let go of on the hub, or on a gap. That is a
                // cancellation and not a choice: nothing is run, and the
                // wheel goes away so that it is not in the way of the next
                // drag.
                diagnostics::log(
                    Level::Info,
                    "drag",
                    "the file was let go of on the hub, so nothing is run",
                );
                self.lifecycle.cancel();
                self.dispatch(InteractionEvent::DragCancelled);
            }
        }
    }

    /// Opens the session for a drag.
    ///
    /// The session is opened when the button goes down on an Explorer window,
    /// which is before there is a drag at all. The files of the drag before it
    /// are dropped here, so that a gesture that never becomes a drag cannot
    /// show the files of an earlier one.
    fn begin_session(&mut self, session: u64, source: HWND) {
        self.session.begin(session, source);
        // A new gesture makes the end of an old one irrelevant.
        self.pending_release = None;
        // The life of the drag before is over: nothing it left waiting may
        // act on this drag.
        self.lifecycle.begin(session);
        // The files the drop target told us about belong to the drag that
        // was running then. They are dropped here, so that a drag that starts
        // before the pointer is over the wheel is not given the files of the
        // drag before it.
        self.drop_state.paths.borrow_mut().clear();
    }

    /// Asks the shell what the current drag is carrying.
    ///
    /// The answer comes from the shell's own view of the Explorer window the
    /// drag started in. It is the only place the payload can be read from, and
    /// it is the only list the wheel is allowed to use: a list that was read
    /// for an earlier drag, or a list that was never read at all, is never
    /// used.
    ///
    /// The answer is remembered for the session, so that the files can be read
    /// again while the choice is being made, and so that a later answer for the
    /// same session can be told apart from an answer for a different one.
    fn resolve_payload(&mut self, session: u64, source: HWND) -> Vec<std::path::PathBuf> {
        let resolution = match &self.shell_window {
            Some(window) => window.selected_paths(),
            None => shellselection::selected_paths(source),
        };
        let paths = resolution.paths().to_vec();
        match resolution.reason() {
            None => diagnostics::log(
                Level::Info,
                "drag",
                &format!(
                    "the shell resolved {} file(s) for drag {session}",
                    paths.len()
                ),
            ),
            Some(reason) => diagnostics::log(
                Level::Warn,
                "drag",
                &format!("the shell could not resolve the files of drag {session}: {reason}"),
            ),
        }
        // The list of this session is the only one that counts. An answer
        // about another drag is refused, so the files of one drag are never
        // the files of another.
        self.session.resolve(session, resolution);
        self.session.paths().to_vec()
    }

    /// Acts on the end of a drag that has been waiting for the drop.
    ///
    /// The drop is handed over within a frame of the mouse coming up, so the
    /// wait is normally over before this is reached. When the drop does not
    /// come at all, the segment that is lit is taken instead, and the wheel
    /// carries on as it was.
    fn handle_release(&mut self) {
        let Some(waiting) = &mut self.pending_release else {
            return;
        };
        waiting.frames += 1;
        if waiting.frames < RELEASE_WAIT_FRAMES {
            return;
        }
        self.pending_release = None;
        diagnostics::log(
            Level::Debug,
            "drag",
            "the drop did not come, so the segment that is lit is taken",
        );
        // The drag is over: nothing that was waiting may act on it now.
        self.lifecycle.cancel();
        self.dispatch(InteractionEvent::DragEnded);
    }

    /// Watches the pointer being outside the wheel.
    ///
    /// The pointer leaves the wheel for a moment all the time: the file is
    /// moved a little, or the wheel is pushed out of the way. A pointer that
    /// comes back within a moment keeps the interaction alive. A pointer that
    /// stays outside ends it, so that the wheel is not left standing over a
    /// drag that is going somewhere else.
    ///
    /// The wait is only ended for the drag it belongs to, and a drag whose
    /// drop was already accepted is never ended by it.
    fn handle_leave(&mut self, now: Instant) {
        if self.drop_state.left.get() {
            if !self.lifecycle.is_leaving() {
                diagnostics::log(
                    Level::Debug,
                    "drag",
                    &format!(
                        "the pointer left the wheel in drag {}",
                        self.lifecycle.session()
                    ),
                );
            }
            self.lifecycle.pointer_left(self.session.id(), now);
        } else {
            self.lifecycle.pointer_returned();
        }
        let Some(session) = self.lifecycle.leave_expired(now) else {
            return;
        };
        if self.lifecycle.is_accepted() {
            // The drop was already accepted. Nothing that comes afterwards
            // may undo it.
            return;
        }
        if session != self.session.id() {
            // The leave belongs to a drag that is over. It may do what it
            // likes to a drag that is not running.
            return;
        }
        if self.controller.state() != InteractionState::WheelVisible {
            // The wheel is not up: there is nothing to cancel.
            return;
        }
        diagnostics::log(
            Level::Info,
            "drag",
            "the pointer stayed outside the wheel, so the interaction is cancelled",
        );
        self.lifecycle.cancel();
        self.dispatch(InteractionEvent::DragCancelled);
    }

    /// Looks at the files of the drag and says what they are.
    ///
    /// The files come from the shell, and the look is cheap: the attributes
    /// of each item are read, and nothing else. No file is opened, no folder
    /// is looked inside, and no processor is run.
    ///
    /// Only counts and kinds go into the log. The names of the files do not.
    fn refresh_selection(&mut self) {
        self.selection = Selection::inspect(self.session.paths());
        let kinds: Vec<&str> = self
            .selection
            .categories()
            .iter()
            .map(|kind| kind.name())
            .collect();
        let mut line = format!(
            "the selection holds {} item(s) of {} kind(s) ({})",
            self.selection.len(),
            kinds.len(),
            kinds.join(", ")
        );
        // The formats are said as well, so that a file that is not the one
        // that was expected can be told from a file that is. A format is a
        // short label, and no name is written down.
        let formats = self.selection.formats();
        if !formats.is_empty() {
            line.push_str(&format!(", in the format(s) {}", formats.join(", ")));
        }
        diagnostics::log(Level::Info, "selection", &line);
        // The wheel shows what may really be done with these files, and
        // nothing else. The registry says what that is.
        let menu = registry::menu_for(&self.selection, &self.environment());
        self.controller.set_menu(menu);
        self.report_availability();
    }

    /// Looks at the files of the drag that is starting, without saying so.
    ///
    /// This is used when a drag begins, so that the wheel offers the
    /// operations of the drag that is starting and not those of the drag
    /// before it. A drag that starts on the desktop has no files yet: the
    /// shell cannot say what it carries, and the drop target is told when
    /// the pointer is over the wheel. The wheel is empty until then.
    ///
    /// Nothing is said in the log here: the log says what the files are when
    /// the wheel opens and when they are resolved.
    fn refresh_selection_quietly(&mut self) {
        self.selection = Selection::inspect(self.session.paths());
        let menu = registry::menu_for(&self.selection, &self.environment());
        self.controller.set_menu(menu);
    }

    /// What the machine can do right now.
    ///
    /// This is what the programs said when they were asked. Until the
    /// answer comes, the environment is empty, and nothing that needs a
    /// program is offered.
    fn environment(&self) -> Environment {
        self.environment.clone()
    }

    /// Says what the registry makes of the files that were dragged.
    ///
    /// This is what the registry says about the operations of the wheel: how
    /// many could be used for these files, and, when none of them can, why
    /// the first one cannot.
    fn report_availability(&self) {
        let mode = self.controller.mode();
        let environment = self.environment();
        let total = registry::operations_for(mode).count();
        let offered = registry::eligible_operations(mode, &self.selection, &environment);
        let why = if offered.is_empty() {
            registry::operations_for(mode)
                .find_map(|operation| {
                    registry::eligibility(operation, &self.selection, &environment).err()
                })
                .map(|reason| reason.name())
        } else {
            None
        };
        let mut line = format!(
            "{} of the {} operations of the {} wheel can be used for it",
            offered.len(),
            total,
            mode.name()
        );
        if let Some(why) = why {
            line.push_str(&format!(": {why}"));
        }
        diagnostics::log(Level::Info, "selection", &line);
        // The wheel shows nothing when there is nothing to show, and that is
        // said plainly rather than left to be guessed at.
        let empty: Vec<&str> = RadialMode::ALL
            .iter()
            .filter(|mode| self.controller.menu_is_empty(**mode))
            .map(|mode| mode.name())
            .collect();
        if !empty.is_empty() {
            diagnostics::log(
                Level::Info,
                "selection",
                &format!("the {} wheel has nothing on it", empty.join(", the ")),
            );
        }
    }

    /// Asks the shell once more whether the files of this drag have changed.
    ///
    /// This is done when the wheel opens, because the selection may not have
    /// been complete when the drag began. The answer is only used when it
    /// belongs to the same drag.
    fn refresh_payload(&mut self) {
        let source = self.drag.source();
        if source.0.is_null() {
            return;
        }
        let session = self.drag.session();
        let resolution = match &self.shell_window {
            Some(window) => window.selected_paths(),
            None => shellselection::selected_paths(source),
        };
        if !resolution.is_resolved() {
            // The shell could not answer this time. The list that was read
            // before stays the list of this session; nothing is invented and
            // nothing from another drag is used.
            return;
        }
        let paths = resolution.paths().to_vec();
        if paths != self.session.paths() {
            diagnostics::log(
                Level::Info,
                "drag",
                &format!(
                    "the shell now reports {} file(s) for drag {session}",
                    paths.len()
                ),
            );
            self.session.resolve(session, resolution);
            self.dispatch(InteractionEvent::DragPayloadResolved { paths });
        }
    }

    /// The segment a point of the overlay is over.
    fn segment_at(&self, screen: Point) -> Option<usize> {
        self.placement?
            .segment_at(screen, self.controller.highlighted())
    }

    fn handle_keys(&mut self) {
        // The development shortcut is the only key read while the wheel is
        // closed.
        let modifiers = input::modifiers();
        let t_down = input::is_key_down(VK_T);
        if t_down && !self.dev_key_down && modifiers.ctrl && modifiers.alt {
            self.dev_key_down = t_down;
            self.open_development_wheel();
            return;
        }
        self.dev_key_down = t_down;

        // While the wheel is closed the other keys belong to other programs.
        if self.controller.state() != InteractionState::WheelVisible {
            return;
        }
        // The system shortcuts win over the wheel.
        if input::system_key_held() {
            return;
        }
        for key in self.keys.poll() {
            // From now on the keyboard decides which segment is chosen. The
            // pointer only takes over again once it moves.
            self.keyboard_owns_highlight = true;
            match key {
                Key::Escape => {
                    // Escape ends the interaction, so nothing that was
                    // waiting may act on the drag afterwards.
                    self.lifecycle.cancel();
                    self.dispatch(InteractionEvent::EscapePressed);
                }
                Key::Enter => self.dispatch(InteractionEvent::ActivatePressed),
                Key::Left => self.dispatch(InteractionEvent::ArrowPressed { clockwise: false }),
                Key::Right => self.dispatch(InteractionEvent::ArrowPressed { clockwise: true }),
                Key::Up => self.dispatch(InteractionEvent::ModeStepped(-1)),
                Key::Down => self.dispatch(InteractionEvent::ModeStepped(1)),
            }
        }
    }

    fn handle_drag(&mut self) {
        let Some(cursor) = input::cursor_position() else {
            return;
        };
        let left_down = input::is_key_down(VK_LBUTTON);
        let was_armed = self.drag.is_armed();
        match self.drag.poll(cursor, left_down) {
            DragEvent::Pressed { session, source } => {
                // The session for this gesture is opened here, before there is
                // a drag at all, so that the files of the drag before it are
                // dropped.
                self.begin_session(session, source);
                // The shell's object for the window is found now, while
                // Explorer is still idle. While a drag is going, Explorer is
                // busy and the shell has to wait for it, which would hold up
                // the wheel.
                self.shell_window = shellselection::Window::find(source);
                diagnostics::log(
                    Level::Debug,
                    "drag",
                    &format!(
                        "drag {session} was pressed on the window {source:?} (the shell {} it)",
                        if self.shell_window.is_some() {
                            "knows"
                        } else {
                            "does not know"
                        }
                    ),
                );
            }
            DragEvent::Started { session, source } => {
                // The files that are being dragged are asked for from the
                // shell, using the window the drag started in. The shell is
                // the only place the payload can be read from.
                let paths = self.resolve_payload(session, source);
                // The operations of the drag before are dropped here, so that
                // the wheel offers the operations of this drag and not those
                // of the one before it.
                self.refresh_selection_quietly();
                diagnostics::log(
                    Level::Info,
                    "drag",
                    &format!(
                        "drag {session} started in the window {source:?} (the foreground window is {:?}): {} file(s), resolution {}, payload from {}",
                        input::foreground_class(),
                        paths.len(),
                        if self.session.is_resolved() {
                            "resolved"
                        } else {
                            "unresolved"
                        },
                        self.session.origin(),
                    ),
                );
                // The drag is announced first, so that the controller is in the
                // right state, and the pointer is read straight afterwards, so
                // that the wheel opens around the pointer and not around
                // wherever it was when the drag began.
                self.dispatch(InteractionEvent::DragStarted { paths });
                self.dispatch(InteractionEvent::PointerMoved { position: cursor });
                // The modifier may already have been held when the drag began.
                let modifiers = input::modifiers();
                self.last_modifiers = modifiers;
                self.dispatch(InteractionEvent::ModifiersChanged(modifiers));
            }
            DragEvent::Ended { session } => {
                diagnostics::log(Level::Info, "drag", &format!("drag {session} ended"));
                if self.drop_state.over.get() {
                    // The mouse came up while the drag was over the wheel.
                    // Windows hands the drop over a moment later, and it says
                    // where the mouse was let go of, which is a better answer
                    // than the pointer: the drag may have moved the pointer
                    // about. So the end of the drag waits for the drop, and
                    // the wheel stays a drop target until then.
                    self.pending_release = Some(PendingRelease { frames: 0 });
                    diagnostics::log(
                        Level::Debug,
                        "drag",
                        "the end of the drag is waiting for the drop",
                    );
                } else {
                    // The file was let go of somewhere other than over the
                    // wheel, or Windows ended the drag. Nothing is chosen: the
                    // drag is cancelled and the wheel goes away.
                    diagnostics::log(
                        Level::Debug,
                        "drag",
                        "the drag ended outside the wheel, so it is cancelled",
                    );
                    self.lifecycle.cancel();
                    self.dispatch(InteractionEvent::DragCancelled);
                }
            }
            DragEvent::Empty { session } => {
                // The pointer moved, but nothing was picked up: a drag of the
                // empty desktop. There is nothing to open the wheel for.
                diagnostics::log(
                    Level::Debug,
                    "drag",
                    &format!("drag {session} carries no file, so it is not a file drag"),
                );
            }
            DragEvent::Rejected { reason } => {
                if !was_armed && left_down && !self.press_reported {
                    self.press_reported = true;
                    diagnostics::log(
                        Level::Debug,
                        "drag",
                        &format!(
                            "the press at {cursor:?} was not a file drag: {reason} (the window under it is {:?})",
                            crate::platform::windows::explorer::window_class_at_point(cursor)
                        ),
                    );
                }
            }
            DragEvent::None => {}
        }

        // Say when the press was not on an Explorer window, so that a gesture
        // that is not a file drag can be told apart from one that is.
        if !was_armed && self.drag.is_armed() {
            diagnostics::log(
                Level::Debug,
                "drag",
                &format!(
                    "drag {} started on a window that holds files at {cursor:?}",
                    self.drag.session()
                ),
            );
        }
        if !left_down {
            self.press_reported = false;
        }

        // While the wheel has not opened yet, its position follows the pointer.
        if self.controller.state() == InteractionState::CandidateDrag {
            self.dispatch(InteractionEvent::PointerMoved { position: cursor });
        }
    }

    fn handle_modifiers(&mut self) {
        let modifiers = input::modifiers();
        if modifiers == self.last_modifiers {
            return;
        }
        self.last_modifiers = modifiers;
        if modifiers.any() {
            diagnostics::log(
                Level::Debug,
                "keys",
                &format!(
                    "shift {} ctrl {} alt {} while {}",
                    modifiers.shift,
                    modifiers.ctrl,
                    modifiers.alt,
                    self.controller.state().name()
                ),
            );
        }
        // The pointer is read before the modifier is acted on, so that the
        // wheel opens exactly where the pointer is at that moment.
        self.capture_cursor();
        self.dispatch(InteractionEvent::ModifiersChanged(modifiers));
    }

    /// Reads the pointer and tells the controller where it is.
    ///
    /// The coordinates come from `GetCursorPos`, so they are in physical screen
    /// pixels, which is what the window placement wants.
    fn capture_cursor(&mut self) -> Option<Point> {
        let cursor = input::cursor_position()?;
        self.dispatch(InteractionEvent::PointerMoved { position: cursor });
        Some(cursor)
    }

    /// Handles what the overlay window reports.
    fn handle_overlay_event(&mut self, event: OverlayEvent) {
        match event {
            OverlayEvent::Wheel { delta, .. } => {
                // The scroll that reaches the window itself. The wheel is
                // usually not the focused window, so most scrolls are caught
                // by the hook instead.
                if self.controller.state() == InteractionState::WheelVisible {
                    let step = if delta < 0.0 { 1 } else { -1 };
                    self.dispatch(InteractionEvent::ModeStepped(step));
                }
            }
            OverlayEvent::PointerDown { x, y } => self.handle_click(x, y),
            OverlayEvent::PointerUp { .. } => {}
            OverlayEvent::AppearanceChanged => self.follow_the_appearance(),
        }
    }

    /// Draws the wheel in the appearance Windows is asking for now.
    ///
    /// The appearance can change while the program is running: the person may
    /// turn the dark appearance on. The wheel follows, without being put
    /// anywhere else on the screen.
    fn follow_the_appearance(&mut self) {
        let theme = Theme::for_appearance(crate::platform::windows::appearance::prefers_dark());
        let name = if theme == Theme::dark() {
            "dark"
        } else {
            "light"
        };
        diagnostics::log(
            Level::Info,
            "wheel",
            &format!(
                "the appearance of Windows changed, and the wheel is drawn in the {name} theme"
            ),
        );
        if theme == self.theme {
            return;
        }
        self.theme = theme;
    }

    fn handle_click(&mut self, x: f32, y: f32) {
        if self.controller.state() != InteractionState::WheelVisible
            || !self.controller.is_interactive()
        {
            return;
        }
        self.ensure_placement();
        let screen = Point::new(
            x + self.overlay_origin.0 as f32,
            y + self.overlay_origin.1 as f32,
        );
        match self.segment_at(screen) {
            Some(index) => self.dispatch(InteractionEvent::SegmentSelected { index }),
            // A click on the hub or on a gap cancels, the same way Escape does.
            None => self.dispatch(InteractionEvent::EscapePressed),
        }
    }

    /// Opens the wheel for the development shortcut.
    fn open_development_wheel(&mut self) {
        let cursor = input::cursor_position().unwrap_or(Point::new(960.0, 540.0));
        let effects = self.controller.open_development_wheel(cursor);
        diagnostics::log(
            Level::Info,
            "dev",
            "the development wheel was opened. This is a test, no files are involved.",
        );
        for effect in effects {
            self.apply(effect);
        }
    }

    /// Draws the wheel and puts it on the screen.
    fn update_visuals(&mut self, delta: f32) {
        let timer = ScopedTimer::new("frame");
        self.animation.update(delta);
        self.animation.set_highlight(self.controller.highlighted());

        // The message about the last operation goes away when its time is
        // up.
        let now = Instant::now();
        if self
            .notice
            .as_ref()
            .is_some_and(|notice| notice.is_over(now))
        {
            self.notice = None;
        }
        let notice = self.notice.clone();
        let notice_appear = notice
            .as_ref()
            .map(|notice| notice.appear(now))
            .unwrap_or(0.0);

        let visible = self.controller.state() == InteractionState::WheelVisible;
        if visible {
            self.ensure_placement();
            let geometry = self
                .placement
                .map(|placement| placement.geometry)
                .unwrap_or_else(|| {
                    RadialGeometry::new(self.controller.segments().len()).scaled(self.scale)
                });
            let model = build_view_model(
                self.controller.segments(),
                self.controller.mode(),
                self.controller.highlighted(),
                self.controller.files().len(),
                self.controller.source() == InteractionSource::Development,
                geometry,
                crate::ui::wheel::ViewOptions {
                    scale: self.scale,
                    appear: self.animation.appear(),
                    emphasis: self.animation.emphasis(),
                    fading: self.animation.fading(),
                    emphasis_index: self.animation.emphasis_index(),
                    fading_index: self.animation.fading_index(),
                    notice: notice.map(|showing| showing.notice),
                    notice_appear,
                    theme: self.theme,
                    // The pointer presses a choice only when the wheel is
                    // there to be clicked. While a file is being dragged the
                    // button is down too, but there the pointer is choosing.
                    preview_pressed: self.controller.is_interactive()
                        && input::is_key_down(VK_LBUTTON),
                },
            );
            self.last_model = Some(model.clone());
            self.ui.apply(&model);
        } else if let Some(model) = self.last_model.clone() {
            // Keep drawing the wheel while it fades out, and the message
            // about the last operation while it is there.
            let mut model = model;
            model.appear = self.animation.appear();
            model.emphasis = self.animation.emphasis();
            model.fading = self.animation.fading();
            model.notice_text = notice
                .as_ref()
                .map(|showing| showing.notice.text.clone())
                .unwrap_or_default();
            model.notice_failed = notice.as_ref().is_some_and(|showing| showing.notice.failed);
            model.notice_appear = notice_appear;
            self.ui.apply(&model);
        }

        // The wheel is on the screen while it is up, and while it is fading
        // away. It is placed before it is drawn, so it can never appear in
        // the wrong place, and it is shown and hidden exactly once each time.
        //
        // The window stays on the screen a moment longer when there is a
        // message to show: the work goes on while the wheel is away, and
        // what came of it is said on the same window.
        let on_screen = self.animation.appear() > 0.0 || notice_appear > 0.0;
        if on_screen {
            self.ensure_placement();
            let drawn = self.ui.render();
            if drawn {
                if let Err(error) =
                    self.overlay
                        .present(self.ui.pixels(), self.ui.width(), self.ui.height())
                {
                    diagnostics::error("overlay", &format!("could not draw the wheel: {error}"));
                    self.dispatch(InteractionEvent::Failed {
                        reason: "the overlay could not be drawn".to_string(),
                    });
                } else {
                    // The wheel is on the screen with something in it, and the
                    // window is showing, so the drag can find it now.
                    self.overlay.show();
                    if self.pending_drop_target {
                        self.pending_drop_target = false;
                        // The files that the drop target is told belong to the
                        // drag that is running now, and to no other.
                        self.drop_state.session.set(self.session.id());
                        if let Err(error) = self.overlay.register_drop_target(&self.drop_target) {
                            diagnostics::error(
                                "drag",
                                &format!("the wheel could not become a drop target: {error}"),
                            );
                        }
                    }
                    if let Some(opened) = self.opened_at.take() {
                        // How long it took from the gesture to the wheel being
                        // on the screen.
                        diagnostics::log(
                            Level::Info,
                            "wheel",
                            &format!(
                                "the wheel was on screen {:.1} ms after the gesture, at {:?}",
                                opened.elapsed().as_secs_f64() * 1000.0,
                                self.overlay.window_rect()
                            ),
                        );
                    }
                }
            }
        } else {
            self.overlay.hide();
            self.ui.request_redraw();
            // The wheel has finished leaving.
            if self.animation.is_gone() && self.controller.state() == InteractionState::Cancelling {
                self.dispatch(InteractionEvent::WheelDismissed);
            }
        }
        drop(timer);
    }

    /// Works out where the wheel is, and puts it there.
    ///
    /// The wheel is always placed, even when the controller has no centre yet:
    /// a window that is never placed stays in the corner of the screen, and
    /// `UpdateLayeredWindow` would draw the wheel there.
    ///
    /// The placement holds the centre and the shape of the wheel in physical
    /// pixels. The drawing and the pointer test both use it, so that the wheel
    /// lights up under the pointer and not somewhere else.
    ///
    /// The wheel is put in one place and stays there. The place comes from
    /// the gesture, which the controller knows. Once the gesture is over the
    /// controller forgets it, and then the wheel must not move: it is on its
    /// way out, and moving it now would make it jump to wherever the pointer
    /// happens to be.
    fn ensure_placement(&mut self) {
        let Some(centre) = wheel_centre(
            self.controller.center(),
            input::cursor_position(),
            self.placement.is_some(),
        ) else {
            return;
        };

        // The shape of the wheel follows the number of segments, and that
        // number can change while the wheel is up. So the shape is worked out
        // every time. The window is only moved when the place really changes:
        // the size of the wheel does not depend on the number of segments.
        let count = self.controller.segments().len();
        let disc = DISC_SIZE * self.scale;
        let radius = disc / 2.0;
        let monitor = monitors::monitor_for_point(centre.x as i32, centre.y as i32);
        let bounds = monitor
            .map(|monitor| monitor.bounds)
            .unwrap_or_else(monitors::virtual_bounds);
        // The shape of the wheel, at the scale of the display. The very same
        // shape is used to test the pointer against.
        let geometry = RadialGeometry::new(count).scaled(self.scale);
        let placement = geometry.placement(centre, radius, bounds);
        self.placement = Some(placement);

        // The wheel does not move once it has opened, so the work of placing it
        // is only done when the place really changes.
        if self.placed_centre == Some(centre) && self.placed {
            return;
        }
        diagnostics::log(
            Level::Info,
            "overlay",
            &format!(
                "the wheel is being placed at {centre:?}, it was at {:?}",
                self.placed_centre,
            ),
        );
        self.placed_centre = Some(centre);
        self.placed = true;

        let x = (placement.centre.x - radius).round() as i32;
        let y = (placement.centre.y - radius).round() as i32;
        self.overlay_origin = (x, y);
        self.overlay
            .place(x, y, disc.round() as u32, disc.round() as u32);

        if !self.placement_logged {
            self.placement_logged = true;
            diagnostics::log(
                Level::Info,
                "overlay",
                &format!(
                    "the wheel is placed at ({x}, {y}), {} by {} pixels, centre {:?}, monitor {bounds:?}, monitor dpi {}, window dpi {}, window {:?}",
                    disc.round(),
                    disc.round(),
                    placement.centre,
                    monitor.map(|monitor| monitor.dpi).unwrap_or(96),
                    monitors::dpi_for_window(self.overlay.hwnd()),
                    self.overlay.window_rect()
                ),
            );
        }
    }
}

/// Where the wheel goes, and whether it may move.
///
/// The wheel is put in one place and stays there for as long as it is on the
/// screen. The place comes from the gesture, which the controller knows while
/// the gesture lasts.
///
/// When the gesture is over the controller forgets where the wheel was, and
/// then the wheel must not move: it is on its way out, and moving it now would
/// make it jump to wherever the pointer happens to be. That is why the
/// pointer is only a guess, and only for a wheel that has never been placed.
fn wheel_centre(
    gesture: Option<Point>,
    pointer: Option<Point>,
    already_placed: bool,
) -> Option<Point> {
    match gesture {
        Some(centre) => Some(centre),
        // The gesture is over. The wheel stays where it is.
        None if already_placed => None,
        // The wheel has never been placed, so the pointer is the best guess.
        None => pointer,
    }
}

/// The scale factor of the display the pointer is on.
fn detect_scale(cursor: Point) -> f32 {
    let dpi = monitors::dpi_for_point(cursor.x as i32, cursor.y as i32);
    (dpi as f32 / 96.0).clamp(1.0, 4.0)
}

/// Tells Windows that the program knows about the scale of every monitor.
///
/// Without this, Windows would stretch the wheel for a monitor that is set to
/// anything but 100 %.
fn set_dpi_awareness() {
    use windows::Win32::UI::HiDpi::{
        DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext,
    };
    // SAFETY: the call only sets the awareness of this process.
    let result =
        unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
    if result.is_err() {
        diagnostics::log(
            Level::Debug,
            "dpi",
            "the program could not ask for per monitor scaling",
        );
    }
}

/// Claims the right to be the only copy of the program.
///
/// The handle is held for as long as the program runs, and closing it gives
/// the right back.
fn claim_single_instance() -> Option<windows::Win32::Foundation::HANDLE> {
    use windows::Win32::Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError};
    use windows::Win32::System::Threading::CreateMutexW;
    // SAFETY: the name is a static wide string.
    let handle =
        unsafe { CreateMutexW(None, true, windows::core::w!("Local\\OpenTangerineWheel")) }.ok()?;
    // SAFETY: the call only reads the last error.
    let error = unsafe { GetLastError() };
    if error == ERROR_ALREADY_EXISTS {
        // Another copy of the program is already running.
        // SAFETY: the handle is ours.
        unsafe {
            let _ = CloseHandle(handle);
        }
        return None;
    }
    Some(handle)
}

/// Starts the application and runs it until it is asked to stop.
pub fn run(options: Options) -> Result<(), String> {
    let started = Instant::now();
    let mut app = App::new(options.clone())?;

    if options.dev_wheel {
        app.open_development_wheel();
    }

    // The shell's automation object is made and asked a question now, while
    // nothing is happening: the shell takes its time the first time, and the
    // wheel cannot wait for that while a drag is going.
    let shell_warm_up = shellselection::warm_up();

    diagnostics::log(
        Level::Info,
        "start",
        &format!(
            "started in {:.1} ms, revision {}, scale {:.2}, timer {TICK_INTERVAL} ms, modifier map {:?}",
            started.elapsed().as_secs_f64() * 1000.0,
            revision(),
            app.scale,
            app.controller.modifier_map()
        ),
    );

    // What the shell can be asked, so that a later failure to read the files
    // can be told apart from the shell not knowing the window at all.
    if diagnostics::enabled(Level::Debug) {
        diagnostics::log(Level::Debug, "shell", &shell_warm_up);
    }

    // The frame is driven by a waitable timer rather than by `WM_TIMER`.
    // A window that never comes to the front has its `WM_TIMER` messages
    // throttled by Windows, and the wheel would move unevenly.
    let timer = FrameTimer::start(TICK_INTERVAL)?;

    let mut message = MSG::default();
    loop {
        // SAFETY: the timer handle is ours and the wait set is well formed.
        let result = unsafe {
            MsgWaitForMultipleObjectsEx(
                Some(&[timer.handle()]),
                u32::MAX,
                QS_ALLINPUT,
                MWMO_INPUTAVAILABLE,
            )
        };
        if result.0 == WAIT_OBJECT_0.0 {
            app.tick();
        } else if result.0 == WAIT_OBJECT_0.0 + 1 {
            // The messages are taken with `PeekMessage` rather than
            // `GetMessage`: a low level mouse hook sends the thread a message
            // that is not a window message, and `GetMessage` would wait for
            // the next real one.
            loop {
                // SAFETY: the queue is only read.
                let got = unsafe { PeekMessageW(&mut message, None, 0, 0, PM_REMOVE) };
                if !got.as_bool() {
                    break;
                }
                // SAFETY: the message was filled in by the call above.
                unsafe {
                    let _ = TranslateMessage(&message);
                    let _ = DispatchMessageW(&message);
                }
            }
            for event in app.overlay.drain_input() {
                app.handle_overlay_event(event);
            }
        }
        if app.quit {
            break;
        }
    }

    timer.stop();
    app.overlay.hide();
    if app.ole.is_ok() {
        // SAFETY: this thread started OLE, so this thread ends it.
        unsafe {
            OleUninitialize();
        }
    }
    diagnostics::log(
        Level::Info,
        "stop",
        &format!("the application is stopping: {}", app.summary()),
    );
    Ok(())
}

/// The timer that drives the application's frames.
struct FrameTimer {
    handle: HANDLE,
}

impl FrameTimer {
    /// Starts a timer that fires every `milliseconds`.
    fn start(milliseconds: u32) -> Result<Self, String> {
        // SAFETY: the handle is only read after it is created.
        let handle = unsafe {
            CreateWaitableTimerW(None, false, None)
                .map_err(|error| format!("the frame timer could not be created: {error}"))?
        };
        // SAFETY: the timer was just created and is not used by anyone else.
        unsafe {
            // A due time of one unit means the timer fires straight away.
            let due: i64 = 1;
            SetWaitableTimer(handle, &due, milliseconds as i32, None, None, false)
                .map_err(|error| format!("the frame timer could not be started: {error}"))?;
        }
        Ok(Self { handle })
    }

    fn handle(&self) -> HANDLE {
        self.handle
    }

    fn stop(&self) {
        // SAFETY: the handle is ours.
        unsafe {
            let _ = CancelWaitableTimer(self.handle);
            let _ = CloseHandle(self.handle);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_wheel_follows_the_gesture_while_the_gesture_lasts() {
        let centre = Point::new(400.0, 300.0);
        assert_eq!(
            wheel_centre(Some(centre), Some(Point::new(900.0, 100.0)), true),
            Some(centre)
        );
    }

    #[test]
    fn the_wheel_does_not_move_when_the_gesture_is_over() {
        // The gesture is over, so the controller has forgotten where the wheel
        // was. The pointer has moved on, and the wheel must not follow it: it
        // is on its way out, and moving it now would make it jump.
        assert_eq!(
            wheel_centre(None, Some(Point::new(900.0, 100.0)), true),
            None
        );
    }

    #[test]
    fn a_wheel_that_was_never_placed_uses_the_pointer() {
        // There is nowhere to keep the wheel yet, so the pointer is the best
        // guess. A window that is never placed stays in the corner of the
        // screen.
        assert_eq!(
            wheel_centre(None, Some(Point::new(900.0, 100.0)), false),
            Some(Point::new(900.0, 100.0))
        );
    }

    #[test]
    fn a_wheel_with_nothing_to_go_by_is_not_placed() {
        assert_eq!(wheel_centre(None, None, false), None);
    }
}
