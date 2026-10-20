//! The application: the window that is always there and the loop that drives it.
//!
//! The application owns the pieces that have to know about each other: the
//! platform layer reads the world, the controller decides what should happen
//! and the UI draws it. Nothing below the controller knows about Windows.

use std::time::{Duration, Instant};

use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0};
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
use crate::platform::windows::droptarget::{DropState, make_drop_target};
use crate::platform::windows::explorer::{DragDetector, DragEvent};
use crate::platform::windows::input::{self, Key, KeyTracker};
use crate::platform::windows::monitors;
use crate::platform::windows::overlay::{Overlay, OverlayEvent};
use crate::radial::actions::{ModifierMap, Modifiers, RadialMode, Segment};
use crate::radial::controller::RadialController;
use crate::radial::geometry::{Point, clamp_center};
use crate::radial::state::{Effect, InteractionEvent, InteractionSource, InteractionState};
use crate::ui::animation::WheelAnimation;
use crate::ui::wheel::{DISC_SIZE, ViewModel, WheelUi, build_view_model};

/// How often the application looks at the world, in milliseconds.
const TICK_INTERVAL: u32 = 16;

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
            "--help" | "-h" => {
                return Err(
                    "Open Tangerine for Windows\n\n  --dev-wheel          open the wheel straight away\n  --advanced-modifiers  also let Ctrl and Alt open their wheels\n  --exit-after <s>     quit after so many seconds\n"
                        .to_string(),
                );
            }
            other => return Err(format!("unknown option {other}")),
        }
        index += 1;
    }
    Ok(options)
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
    /// The COM object Windows talks to when a drag passes over the wheel.
    drop_target: IDropTarget,
    /// Watches the scroll wheel while the wheel is on screen.
    wheel_watcher: Option<input::WheelWatcher>,
    /// The file list the drop target last reported.
    reported_paths: Vec<std::path::PathBuf>,
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
}

impl App {
    fn new(options: Options) -> Result<Self, String> {
        // OLE has to be available on this thread before the drag payload can
        // be read from the OLE clipboard.
        // SAFETY: the call only prepares the thread for OLE.
        let ole = unsafe { OleInitialize(None) };
        if ole.is_err() {
            diagnostics::error("ole", "OLE could not be started on this thread");
        }
        let overlay = Overlay::new().map_err(|error| error.to_string())?;
        let drop_state = std::rc::Rc::new(DropState::default());
        let drop_target = make_drop_target(drop_state.clone());
        let scale = detect_scale(&overlay);
        let ui = WheelUi::new(scale).map_err(|error| error.to_string())?;
        let mut controller = RadialController::new();
        controller.set_modifier_map(options.modifier_map);

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
            wheel_watcher: None,
            reported_paths: Vec::new(),
            opened_at: None,
            keyboard_owns_highlight: false,
            cursor_when_the_keyboard_took_over: Point::new(0.0, 0.0),
            frames: 0,
            frame_time: std::time::Duration::ZERO,
            ole,
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
                self.animation.show();
                self.animation.set_highlight(self.controller.highlighted());
                // While a drag is running the wheel becomes a drop target.
                // That is the only way to learn what the drag is carrying,
                // and it is also how the drop is taken over instead of
                // landing in the folder underneath.
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
                if self.controller.is_explorer_drag()
                    && let Err(error) = self.overlay.register_drop_target(&self.drop_target)
                {
                    diagnostics::error(
                        "drag",
                        &format!("the wheel could not become a drop target: {error}"),
                    );
                }
                diagnostics::log(
                    Level::Info,
                    "wheel",
                    &format!(
                        "the wheel is opening in the {} mode with {} segment(s) on {} file(s)",
                        self.controller.mode().name(),
                        self.controller.segments().len(),
                        self.controller.files().len()
                    ),
                );
            }
            Effect::UpdateWheel => {
                self.animation.set_highlight(self.controller.highlighted());
            }
            Effect::HideWheel => {
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
    /// Milestone 1 does not carry out any file operation: the choice is
    /// reported and nothing else happens.
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
                "placeholder: {} ({}) in the {} wheel on {} file(s), source {:?}. Nothing is executed in Milestone 1.",
                segment.label,
                segment.id,
                mode.name(),
                files.len(),
                source
            ),
        );
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
    /// While a drag is over the wheel, the drag itself says where the
    /// pointer is, and that answer is used instead.
    fn handle_hover(&mut self) {
        if self.controller.state() != InteractionState::WheelVisible {
            return;
        }
        if self.drop_state.over.get() {
            return;
        }
        let (Some(cursor), Some(center)) = (input::cursor_position(), self.controller.center())
        else {
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
        let offset = Point::new(cursor.x - center.x, cursor.y - center.y);
        let index = self.controller.geometry().hit_test(offset);
        if index != self.controller.highlighted() {
            self.dispatch(InteractionEvent::SegmentEntered(index));
        }
    }

    /// One frame.
    fn tick(&mut self) {
        let timer = ScopedTimer::new("frame");
        let now = Instant::now();
        let delta = (now - self.last_tick).as_secs_f32().min(0.1);
        self.last_tick = now;

        self.handle_keys();
        self.handle_drag();
        self.handle_modifiers();
        self.handle_scroll();
        self.handle_drop();
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

        // The file list of the drag.
        let paths = state.paths.borrow().clone();
        if !paths.is_empty() && paths != self.reported_paths {
            self.reported_paths = paths.clone();
            diagnostics::log(
                Level::Debug,
                "drag",
                &format!("the drag carries {} file(s)", paths.len()),
            );
            self.dispatch(InteractionEvent::DragPayloadResolved { paths });
        }

        if !state.over.get() {
            return;
        }

        // The pointer, as the drag sees it.
        let index = self.segment_at(state.point.get());
        if index != self.controller.highlighted() {
            self.dispatch(InteractionEvent::SegmentEntered(index));
        }

        if state.take_drop() {
            diagnostics::log(Level::Info, "drag", "the drag was dropped on the wheel");
            if self.controller.state() == InteractionState::WheelVisible
                && let Some(index) = index
            {
                self.dispatch(InteractionEvent::SegmentEntered(Some(index)));
                self.dispatch(InteractionEvent::SegmentSelected { index });
            }
        }
    }

    /// The segment a point of the overlay is over.
    fn segment_at(&self, point: (i32, i32)) -> Option<usize> {
        let center = self.controller.center()?;
        let screen = Point::new(
            self.overlay_origin.0 as f32 + point.0 as f32,
            self.overlay_origin.1 as f32 + point.1 as f32,
        );
        let offset = Point::new(screen.x - center.x, screen.y - center.y);
        self.controller.geometry().hit_test(offset)
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
                Key::Escape => self.dispatch(InteractionEvent::EscapePressed),
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
            DragEvent::Started { paths } => {
                diagnostics::log(
                    Level::Info,
                    "drag",
                    &format!(
                        "Explorer drag started with {} file(s), foreground window {:?}",
                        paths.len(),
                        input::foreground_class()
                    ),
                );
                self.dispatch(InteractionEvent::DragStarted { paths });
                // The modifier may already have been held when the drag began.
                let modifiers = input::modifiers();
                self.last_modifiers = modifiers;
                self.dispatch(InteractionEvent::ModifiersChanged(modifiers));
            }
            DragEvent::Ended => {
                diagnostics::log(Level::Info, "drag", "the drag ended");
                self.dispatch(InteractionEvent::DragEnded);
            }
            DragEvent::None => {}
        }

        // Say when the press was not on an Explorer window, so that a gesture
        // that is not a file drag can be told apart from one that is.
        if !was_armed && self.drag.is_armed() {
            diagnostics::log(
                Level::Debug,
                "drag",
                &format!("the press started on an Explorer window at {cursor:?}"),
            );
        } else if !was_armed && left_down && !self.press_reported {
            self.press_reported = true;
            diagnostics::log(
                Level::Debug,
                "drag",
                &format!(
                    "the press at {cursor:?} was on {:?}, not on an Explorer window",
                    crate::platform::windows::explorer::window_class_at_point(cursor)
                ),
            );
        }
        if !left_down {
            self.press_reported = false;
        }

        // The shell publishes the payload a moment after the drag begins.
        if self.drag.is_armed() {
            let paths = self.drag.paths();
            if !paths.is_empty() && paths != self.controller.files() {
                let paths = paths.to_vec();
                diagnostics::log(
                    Level::Debug,
                    "drag",
                    &format!("the drag payload resolved to {} file(s)", paths.len()),
                );
                self.dispatch(InteractionEvent::DragPayloadResolved { paths });
            }
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
        self.dispatch(InteractionEvent::ModifiersChanged(modifiers));
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
        }
    }

    fn handle_click(&mut self, x: f32, y: f32) {
        if self.controller.state() != InteractionState::WheelVisible
            || !self.controller.is_interactive()
        {
            return;
        }
        let Some(center) = self.controller.center() else {
            return;
        };
        let screen = Point::new(
            x + self.overlay_origin.0 as f32,
            y + self.overlay_origin.1 as f32,
        );
        let offset = Point::new(screen.x - center.x, screen.y - center.y);
        match self.controller.geometry().hit_test(offset) {
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

        let visible = self.controller.state() == InteractionState::WheelVisible;
        if visible {
            let model = build_view_model(
                self.controller.segments(),
                self.controller.mode(),
                self.controller.highlighted(),
                self.controller.files().len(),
                self.controller.source() == InteractionSource::Development,
                crate::ui::wheel::ViewOptions {
                    scale: self.scale,
                    appear: self.animation.appear(),
                    emphasis: self.animation.emphasis(),
                    fading: self.animation.fading(),
                    emphasis_index: self.animation.emphasis_index(),
                    fading_index: self.animation.fading_index(),
                },
            );
            self.last_model = Some(model.clone());
            self.ui.apply(&model);
        } else if let Some(model) = self.last_model.clone() {
            // Keep drawing the wheel while it fades out.
            let mut model = model;
            model.appear = self.animation.appear();
            model.emphasis = self.animation.emphasis();
            model.fading = self.animation.fading();
            self.ui.apply(&model);
        }

        if self.animation.is_present() || self.animation.appear() > 0.0 {
            self.layout();
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
                } else if let Some(opened) = self.opened_at.take() {
                    // How long it took from the gesture to the wheel being on
                    // the screen.
                    diagnostics::log(
                        Level::Info,
                        "wheel",
                        &format!(
                            "the wheel was on screen {:.1} ms after the gesture",
                            opened.elapsed().as_secs_f64() * 1000.0
                        ),
                    );
                }
            }
            if !self.overlay.is_visible() {
                self.overlay.show();
            }
        } else {
            if self.overlay.is_visible() {
                self.overlay.hide();
                self.ui.request_redraw();
            }
            // The wheel has finished leaving.
            if self.animation.is_gone() && self.controller.state() == InteractionState::Cancelling {
                self.dispatch(InteractionEvent::WheelDismissed);
            }
        }
        drop(timer);
    }

    /// Puts the wheel on screen around the cursor, inside the monitor.
    fn layout(&mut self) {
        let Some(center) = self.controller.center() else {
            return;
        };
        let disc = DISC_SIZE * self.scale;
        let radius = disc / 2.0;
        let bounds = monitors::monitor_for_point(center.x as i32, center.y as i32)
            .map(|monitor| monitor.bounds)
            .unwrap_or_else(monitors::virtual_bounds);
        let clamped = clamp_center(center, radius, bounds);
        let x = (clamped.x - radius).round() as i32;
        let y = (clamped.y - radius).round() as i32;
        self.overlay_origin = (x, y);
        self.overlay
            .place(x, y, disc.round() as u32, disc.round() as u32);
    }
}

/// The scale factor of the display the pointer is on.
fn detect_scale(overlay: &Overlay) -> f32 {
    let cursor = input::cursor_position().unwrap_or(Point::new(960.0, 540.0));
    let dpi = monitors::dpi_for_window(overlay.hwnd());
    let dpi = if dpi > 0 { dpi } else { 96 };
    let _ = cursor;
    (dpi as f32 / 96.0).clamp(1.0, 4.0)
}

/// Starts the application and runs it until it is asked to stop.
pub fn run(options: Options) -> Result<(), String> {
    let started = Instant::now();
    let mut app = App::new(options.clone())?;

    if options.dev_wheel {
        app.open_development_wheel();
    }

    diagnostics::log(
        Level::Info,
        "start",
        &format!(
            "started in {:.1} ms, scale {:.2}, timer {TICK_INTERVAL} ms, modifier map {:?}",
            started.elapsed().as_secs_f64() * 1000.0,
            app.scale,
            app.controller.modifier_map()
        ),
    );

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
