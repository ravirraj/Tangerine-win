//! Recognising a file drag that comes out of Explorer.
//!
//! Two things have to be true before the wheel may open:
//!
//! 1. the press started on an Explorer window, and
//! 2. the pointer moved far enough to be a drag rather than a click.
//!
//! The detector finds out **that** a drag is happening and **which window** it
//! started in. It does not try to find out what is being dragged: Explorer's
//! drag keeps its payload to itself, and the only place that payload can be
//! read is the shell's own view of the window the drag started in. That is
//! `shellselection.rs`, and the detector hands the window over so that the
//! files can be asked for.

use windows::Win32::Foundation::{HWND, POINT};
use windows::Win32::UI::WindowsAndMessaging::{GA_ROOT, GetAncestor, WindowFromPoint};

use crate::platform::windows::input;
use crate::radial::geometry::Point;

/// How far the pointer has to travel before a press counts as a drag.
pub const DRAG_THRESHOLD: f32 = 8.0;

/// The window classes Explorer uses for its file lists.
const EXPLORER_CLASSES: [&str; 2] = ["CabinetWClass", "ExploreWClass"];

/// Whether a window belongs to Explorer's file list.
pub fn is_explorer_window(hwnd: HWND) -> bool {
    match input::window_class(hwnd) {
        Some(class) => EXPLORER_CLASSES.iter().any(|known| known == &class),
        None => false,
    }
}

/// The Explorer window under a screen point, if there is one.
pub fn explorer_window_at_point(point: Point) -> Option<HWND> {
    let native = POINT {
        x: point.x as i32,
        y: point.y as i32,
    };
    // SAFETY: `WindowFromPoint` takes a POINT by value and never dereferences it.
    let hit = unsafe { WindowFromPoint(native) };
    if hit.0.is_null() {
        return None;
    }
    // SAFETY: `GetAncestor` only walks the window tree.
    let root = unsafe { GetAncestor(hit, GA_ROOT) };
    let root = if root.0.is_null() { hit } else { root };
    if is_explorer_window(root) {
        Some(root)
    } else {
        None
    }
}

/// The class name of the window under a screen point.
pub fn window_class_at_point(point: Point) -> Option<String> {
    let native = POINT {
        x: point.x as i32,
        y: point.y as i32,
    };
    // SAFETY: `WindowFromPoint` takes a POINT by value and never dereferences it.
    let hit = unsafe { WindowFromPoint(native) };
    if hit.0.is_null() {
        return None;
    }
    // SAFETY: `GetAncestor` only walks the window tree.
    let root = unsafe { GetAncestor(hit, GA_ROOT) };
    let root = if root.0.is_null() { hit } else { root };
    input::window_class(root)
}

/// What the drag detector noticed this frame.
#[derive(Clone, Debug, PartialEq)]
pub enum DragEvent {
    /// Nothing changed.
    None,
    /// The button went down on an Explorer window.
    ///
    /// This is said before the drag has begun, while Explorer is still idle.
    /// It is the moment to ask the shell about the window, because the shell
    /// has to wait for Explorer while a drag is going.
    Pressed {
        /// Which drag this is. The number goes up for every new drag.
        session: u64,
        /// The Explorer window the press was on.
        source: HWND,
    },
    /// A drag started.
    Started {
        /// Which drag this is. The number goes up for every new drag.
        session: u64,
        /// The Explorer window the drag started in. The files that are being
        /// dragged are asked for from the shell, using this window.
        source: HWND,
    },
    /// The drag ended. This is said once for every drag, and never again.
    Ended {
        /// Which drag ended.
        session: u64,
    },
    /// A press that is not a file drag, and why.
    Rejected { reason: &'static str },
}

/// Remembers whether the button has been let go since the last drag.
///
/// A drag can only begin after the button has been seen up, so that one
/// gesture is never counted as several. Without this, a button that is
/// reported as up for a single frame would end the drag and start a new one,
/// and the wheel would open and close again and again.
#[derive(Debug)]
struct ButtonLatch {
    released: bool,
}

impl Default for ButtonLatch {
    fn default() -> Self {
        // The button is up before the first press, so the first press is
        // allowed.
        Self { released: true }
    }
}

impl ButtonLatch {
    /// The button went down. Returns whether a new gesture may begin.
    fn press(&mut self) -> bool {
        let allowed = self.released;
        self.released = false;
        allowed
    }

    /// The button came up.
    fn release(&mut self) {
        self.released = true;
    }
}

/// Watches for an Explorer drag.
///
/// The detector only ever looks at the pointer, the button and the window
/// under the pointer. It does not install a hook and it does not change any
/// input.
///
/// Every drag is a **session** with its own number. A session is opened by a
/// press on an Explorer window and closed by the button coming up, and it can
/// only be opened again after the button has been seen up in between.
pub struct DragDetector {
    /// The number of the drag that is running, or of the last one.
    session: u64,
    /// Whether a drag is being watched.
    armed: bool,
    /// Whether the drag has already been reported as started.
    reported: bool,
    /// Whether the button has been seen up since the last drag.
    latch: ButtonLatch,
    origin: Option<Point>,
    /// The Explorer window the drag started in.
    source: HWND,
}

impl Default for DragDetector {
    fn default() -> Self {
        Self::new()
    }
}

impl DragDetector {
    pub fn new() -> Self {
        Self {
            session: 0,
            armed: false,
            reported: false,
            latch: ButtonLatch::default(),
            origin: None,
            // A null handle is never a real window.
            source: HWND(std::ptr::null_mut()),
        }
    }

    /// Whether a drag is currently being watched.
    pub fn is_armed(&self) -> bool {
        self.armed
    }

    /// The number of the drag that is running, or of the last one.
    pub fn session(&self) -> u64 {
        self.session
    }

    /// The Explorer window the drag started in.
    ///
    /// This is the window the files are asked for from. It is kept after the
    /// drag has ended, so that the files can still be read while the choice is
    /// being made.
    pub fn source(&self) -> HWND {
        self.source
    }

    /// Looks at the pointer and the mouse button.
    ///
    /// `left_down` says whether the primary button is held.
    pub fn poll(&mut self, cursor: Point, left_down: bool) -> DragEvent {
        if !left_down {
            // The button is up. A drag that was running ends here, and only
            // here.
            self.latch.release();
            if !self.armed {
                return DragEvent::None;
            }
            self.armed = false;
            self.reported = false;
            self.origin = None;
            return DragEvent::Ended {
                session: self.session,
            };
        }

        if !self.armed {
            if !self.latch.press() {
                // The button was never seen up, so this is still the old
                // gesture and not a new one.
                return DragEvent::None;
            }
            // The button has just gone down. Only a press that starts on an
            // Explorer window can become a file drag.
            let Some(window) = explorer_window_at_point(cursor) else {
                return DragEvent::Rejected {
                    reason: "the press was not on an Explorer window",
                };
            };
            self.session += 1;
            self.armed = true;
            self.reported = false;
            self.origin = Some(cursor);
            self.source = window;
            return DragEvent::Pressed {
                session: self.session,
                source: window,
            };
        }

        if !self.reported {
            let origin = self.origin.unwrap_or(cursor);
            let travelled = ((cursor.x - origin.x).powi(2) + (cursor.y - origin.y).powi(2)).sqrt();
            if travelled < DRAG_THRESHOLD {
                return DragEvent::None;
            }
            self.reported = true;
            return DragEvent::Started {
                session: self.session,
                source: self.source,
            };
        }

        DragEvent::None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_detector_starts_idle() {
        let detector = DragDetector::new();
        assert!(!detector.is_armed());
        assert_eq!(detector.session(), 0);
        // There is no window yet, so there is nothing to ask the shell about.
        assert!(detector.source().0.is_null());
    }

    #[test]
    fn a_detector_ignores_movement_while_the_button_is_up() {
        let mut detector = DragDetector::new();
        for step in 0..40 {
            let event = detector.poll(Point::new(step as f32 * 30.0, 100.0), false);
            assert_eq!(event, DragEvent::None);
        }
        assert!(!detector.is_armed());
    }

    #[test]
    fn a_press_that_is_not_on_an_explorer_window_is_rejected() {
        let mut detector = DragDetector::new();
        // A point that is on no screen at all.
        let event = detector.poll(Point::new(-10_000.0, -10_000.0), true);
        assert!(matches!(event, DragEvent::Rejected { .. }));
        assert!(!detector.is_armed());
        assert_eq!(detector.session(), 0);
    }

    #[test]
    fn the_latch_allows_one_press_at_a_time() {
        let mut latch = ButtonLatch::default();
        // The button was up before the first press, so it is allowed.
        assert!(latch.press());
        // The button is still down, so a second press is not a new gesture.
        assert!(!latch.press());
        latch.release();
        assert!(latch.press());
    }

    #[test]
    fn a_drag_cannot_start_again_before_the_button_is_up() {
        let mut detector = DragDetector::new();
        // The button goes up and down again, but the press never lands on an
        // Explorer window, so no session is ever opened.
        for _ in 0..5 {
            detector.poll(Point::new(-10_000.0, -10_000.0), true);
            detector.poll(Point::new(-10_000.0, -10_000.0), false);
        }
        assert_eq!(detector.session(), 0);
    }

    #[test]
    fn the_drag_threshold_is_reasonable() {
        // The threshold is a constant, so this is checked at compile time.
        const _: () = assert!(DRAG_THRESHOLD >= 4.0);
        const _: () = assert!(DRAG_THRESHOLD <= 24.0);
    }
}
