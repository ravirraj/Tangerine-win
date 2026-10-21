//! Recognising a file drag that comes out of Explorer or off the desktop.
//!
//! Two things have to be true before the wheel may open:
//!
//! 1. the press started on a file that can be dragged, and
//! 2. the pointer moved far enough to be a drag rather than a click.
//!
//! The detector finds out **that** a drag is happening and **which window** it
//! started in. It does not try to find out what is being dragged: Explorer's
//! drag keeps its payload to itself, and the only place that payload can be
//! read is the shell's own view of the window the drag started in. That is
//! `shellselection.rs`, and the detector hands the window over so that the
//! files can be asked for.
//!
//! The desktop is a source of its own. It is not an Explorer window: it is a
//! shell window with a list of icons in it, and a press is only a file drag
//! when it is on one of those icons. A click on the empty desktop is not a
//! file drag, and the wheel must not open for it.

use windows::Win32::Foundation::{HWND, LPARAM, POINT, WPARAM};
use windows::Win32::UI::Controls::{LVM_GETNEXTITEM, LVNI_SELECTED};
use windows::Win32::UI::WindowsAndMessaging::{
    FindWindowExW, GA_ROOT, GetAncestor, SendMessageW, WindowFromPoint,
};
use windows::core::{PCWSTR, w};

use crate::platform::windows::input;
use crate::radial::geometry::Point;

/// How far the pointer has to travel before a press counts as a drag.
pub const DRAG_THRESHOLD: f32 = 8.0;

/// The window classes Explorer uses for its file lists.
const EXPLORER_CLASSES: [&str; 2] = ["CabinetWClass", "ExploreWClass"];

/// The window classes the shell uses for the desktop.
///
/// The desktop is a shell window of its own, and its icons live in a list of
/// their own inside it.
const DESKTOP_CLASSES: [&str; 2] = ["Progman", "WorkerW"];

/// The class of the window that holds the desktop's icons.
const DESKTOP_VIEW_CLASS: PCWSTR = w!("SHELLDLL_DefView");

/// The class of the list the desktop's icons are in.
const DESKTOP_LIST_CLASS: PCWSTR = w!("SysListView32");

/// Whether a window belongs to Explorer's file list.
pub fn is_explorer_window(hwnd: HWND) -> bool {
    match input::window_class(hwnd) {
        Some(class) => EXPLORER_CLASSES.iter().any(|known| known == &class),
        None => false,
    }
}

/// Whether a window is the shell's desktop.
pub fn is_desktop_window(hwnd: HWND) -> bool {
    match input::window_class(hwnd) {
        Some(class) => DESKTOP_CLASSES.iter().any(|known| known == &class),
        None => false,
    }
}

/// The class name of the window under a screen point.
pub fn window_class_at_point(point: Point) -> Option<String> {
    let root = root_of(window_at_point(point)?);
    input::window_class(root)
}

/// What a press is on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PressKind {
    /// The file list of an Explorer window.
    ExplorerList,
    /// The desktop, with its icons.
    Desktop,
    /// Anything else at all.
    Other,
}

impl PressKind {
    /// Whether a press of this kind may be the start of a drag.
    ///
    /// Explorer's file lists and the desktop both hold files, so a drag that
    /// starts on one of them may be a drag of a file. A press on anything
    /// else holds no files, and a drag of it is not a drag of a file.
    ///
    /// Whether a drag that started here really carries a file is not known
    /// when the press is seen. The shell can say what Explorer's file list
    /// carries, but it cannot say what the desktop carries: the desktop is
    /// not in the shell's list of windows, and its list of icons answers
    /// nothing about the items in it. What a drag carries is therefore asked
    /// of the drop target, which is told.
    pub fn may_begin_a_drag(self) -> bool {
        matches!(self, PressKind::ExplorerList | PressKind::Desktop)
    }
}

/// Works out what a press is on.
///
/// The class of the top level window tells Explorer's file lists and the
/// desktop apart.
pub fn classify_press(root_class: &str) -> PressKind {
    if EXPLORER_CLASSES.iter().any(|known| known == &root_class) {
        PressKind::ExplorerList
    } else if DESKTOP_CLASSES.iter().any(|known| known == &root_class) {
        PressKind::Desktop
    } else {
        PressKind::Other
    }
}

/// Whether a drag may begin, given what the press was on.
///
/// Explorer's file lists always hold files, so a drag of one is a drag of a
/// file. The desktop holds files only where its icons are, and the shell says
/// so with its selection: it selects the icon that is pressed, and it clears
/// the selection when the empty desktop is pressed.
///
/// Anything else holds no files at all, and a drag of it is not a file drag.
pub fn drag_may_begin(kind: PressKind, desktop_has_selection: bool) -> bool {
    match kind {
        PressKind::ExplorerList => true,
        PressKind::Desktop => desktop_has_selection,
        PressKind::Other => false,
    }
}

/// The window a drag may start in, and what the press was on.
pub fn drag_origin_at_point(point: Point) -> Option<(HWND, PressKind)> {
    let root = root_of(window_at_point(point)?);
    let class = input::window_class(root).unwrap_or_default();
    let kind = classify_press(&class);
    if kind.may_begin_a_drag() {
        Some((root, kind))
    } else {
        None
    }
}

/// Whether the press was on one of the desktop's icons.
///
/// The desktop's icons live in a list of their own, and that list covers the
/// whole screen, so the window under the pointer cannot tell an icon from the
/// wallpaper around it. The list itself can: the shell selects the icon that
/// is pressed, and it clears the selection when the empty desktop is pressed.
///
/// The answer is about the selection rather than about the point, so it is
/// only worth asking once the pointer has moved.
pub fn desktop_has_selected_icon(desktop: HWND) -> bool {
    let Some(list) = desktop_icon_list(desktop) else {
        return false;
    };
    // The first selected item, or -1 when nothing is selected.
    // SAFETY: the window is a list view, and the message asks for a number.
    let answer = unsafe {
        SendMessageW(
            list,
            LVM_GETNEXTITEM,
            Some(WPARAM(usize::MAX)),
            Some(LPARAM(LVNI_SELECTED as isize)),
        )
    };
    answer.0 >= 0
}

/// The list the desktop's icons are in.
fn desktop_icon_list(desktop: HWND) -> Option<HWND> {
    // SAFETY: the handles are only read, and a window that is not there is
    // simply not found.
    unsafe {
        let view = FindWindowExW(Some(desktop), None, DESKTOP_VIEW_CLASS, PCWSTR::null()).ok()?;
        FindWindowExW(Some(view), None, DESKTOP_LIST_CLASS, PCWSTR::null()).ok()
    }
}

/// The window under a screen point.
fn window_at_point(point: Point) -> Option<HWND> {
    let native = POINT {
        x: point.x as i32,
        y: point.y as i32,
    };
    // SAFETY: `WindowFromPoint` takes a POINT by value and never dereferences it.
    let hit = unsafe { WindowFromPoint(native) };
    if hit.0.is_null() { None } else { Some(hit) }
}

/// The top level window a window belongs to.
fn root_of(window: HWND) -> HWND {
    // SAFETY: `GetAncestor` only walks the window tree.
    let root = unsafe { GetAncestor(window, GA_ROOT) };
    if root.0.is_null() { window } else { root }
}

/// What the drag detector noticed this frame.
#[derive(Clone, Debug, PartialEq)]
pub enum DragEvent {
    /// Nothing changed.
    None,
    /// The button went down on a window that holds files.
    ///
    /// This is said before the drag has begun, while Explorer is still idle.
    /// It is the moment to ask the shell about the window, because the shell
    /// has to wait for Explorer while a drag is going.
    Pressed {
        /// Which drag this is. The number goes up for every new drag.
        session: u64,
        /// The window the press was on: an Explorer file list, or the desktop.
        source: HWND,
    },
    /// A drag started.
    Started {
        /// Which drag this is. The number goes up for every new drag.
        session: u64,
        /// The window the drag started in. The files that are being dragged
        /// are asked for from the shell, using this window.
        source: HWND,
    },
    /// The drag ended. This is said once for every drag, and never again.
    Ended {
        /// Which drag ended.
        session: u64,
    },
    /// A drag began, but it carries no file.
    ///
    /// This is what a drag of the empty desktop is: the pointer moved, but
    /// nothing was picked up.
    Empty {
        /// Which drag it was.
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

/// Watches for a file drag.
///
/// The detector only ever looks at the pointer, the button and the window
/// under the pointer. It does not install a hook and it does not change any
/// input.
///
/// Every drag is a **session** with its own number. A session is opened by a
/// press on a window that holds files and closed by the button coming up, and
/// it can only be opened again after the button has been seen up in between.
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
    /// The window the drag started in.
    source: HWND,
    /// What the press was on.
    kind: PressKind,
    /// The first answer the shell gave about the desktop's selection.
    ///
    /// The shell makes its selection when it has seen the press, which is
    /// after the press has been noticed, so the first answer can still be the
    /// answer of the gesture before. The second answer is asked for.
    desktop_first_answer: Option<bool>,
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
            kind: PressKind::Other,
            desktop_first_answer: None,
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

    /// The window the drag started in.
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
            // The button has just gone down. Only a press that starts on
            // something that holds files can become a file drag: an Explorer
            // file list, or the desktop.
            let Some((window, kind)) = drag_origin_at_point(cursor) else {
                return DragEvent::Rejected {
                    reason: "the press was not on a file that can be dragged",
                };
            };
            self.session += 1;
            self.armed = true;
            self.reported = false;
            self.origin = Some(cursor);
            self.source = window;
            self.kind = kind;
            self.desktop_first_answer = None;
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
            if self.kind == PressKind::Desktop {
                // The desktop is asked here, and not on the press: the shell
                // makes its selection when it has seen the press, and that is
                // after the press has been noticed. The first answer is taken
                // now and the second one on the next frame, because the shell
                // may not have seen the press yet: its answer would then still
                // be the one of the gesture before, and the gesture before may
                // have had an icon selected.
                self.desktop_first_answer = Some(desktop_has_selected_icon(self.source));
                return DragEvent::None;
            }
            return DragEvent::Started {
                session: self.session,
                source: self.source,
            };
        }

        // A drag of the desktop waits for the shell's second answer.
        if self.desktop_first_answer.is_some() {
            self.desktop_first_answer = None;
            // The second answer is the one that is trusted: by now the shell
            // has seen the press and the movement, so it says what the drag
            // is really carrying.
            let carries = desktop_has_selected_icon(self.source);
            if !drag_may_begin(self.kind, carries) {
                return DragEvent::Empty {
                    session: self.session,
                };
            }
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
    fn a_press_on_an_explorer_file_list_may_begin_a_file_drag() {
        for class in ["CabinetWClass", "ExploreWClass"] {
            let kind = classify_press(class);
            assert_eq!(kind, PressKind::ExplorerList);
            assert!(kind.may_begin_a_drag());
            // Explorer's file lists always hold files, so the selection of
            // the desktop makes no difference here.
            assert!(drag_may_begin(kind, true));
            assert!(drag_may_begin(kind, false));
        }
    }

    #[test]
    fn a_drag_of_a_desktop_icon_may_begin_a_file_drag() {
        for class in ["Progman", "WorkerW"] {
            let kind = classify_press(class);
            assert_eq!(kind, PressKind::Desktop);
            assert!(kind.may_begin_a_drag());
            // The shell selects the icon that is pressed, so a selection
            // means that the drag carries a file.
            assert!(drag_may_begin(kind, true));
        }
    }

    #[test]
    fn a_drag_of_the_empty_desktop_may_not_begin_a_file_drag() {
        for class in ["Progman", "WorkerW"] {
            let kind = classify_press(class);
            assert_eq!(kind, PressKind::Desktop);
            // The shell clears the selection when the empty desktop is
            // pressed, so no selection means that nothing was picked up.
            assert!(!drag_may_begin(kind, false));
        }
    }

    #[test]
    fn a_drag_of_anything_else_may_not_begin_a_file_drag() {
        for class in ["Shell_TrayWnd", "ConsoleWindowClass", "Notepad", ""] {
            let kind = classify_press(class);
            assert_eq!(kind, PressKind::Other);
            assert!(!kind.may_begin_a_drag());
            assert!(!drag_may_begin(kind, true));
            assert!(!drag_may_begin(kind, false));
        }
    }

    #[test]
    fn only_the_desktop_asks_about_a_selection() {
        // The selection is the desktop's own answer. A window that is not the
        // desktop has no selection of its own, so the answer must not make
        // it a source.
        let kind = classify_press("CabinetWClass");
        assert!(drag_may_begin(kind, true));
        let kind = classify_press("Notepad");
        assert!(!drag_may_begin(kind, true));
    }

    #[test]
    fn a_press_that_is_on_no_window_at_all_is_rejected() {
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

    #[test]
    fn the_desktop_is_asked_twice_before_the_drag_is_judged() {
        // The shell makes its selection when it has seen the press, which is
        // after the press has been noticed, so the first answer can still be
        // the answer of the gesture before. The detector therefore asks again
        // before it decides, and the answer it waits for is the second one.
        let mut detector = DragDetector::new();
        detector.kind = PressKind::Desktop;
        detector.source = HWND(std::ptr::null_mut());
        detector.armed = true;
        detector.origin = Some(Point::new(0.0, 0.0));

        // The pointer moves past the threshold: the drag has begun, and the
        // first question is put to the shell.
        let event = detector.poll(Point::new(20.0, 0.0), true);
        assert_eq!(event, DragEvent::None);
        assert!(detector.reported);
        assert!(detector.desktop_first_answer.is_some());

        // The next frame asks again, and this time the answer is used.
        let event = detector.poll(Point::new(30.0, 0.0), true);
        assert!(matches!(
            event,
            DragEvent::Empty { .. } | DragEvent::Started { .. }
        ));
        assert!(detector.desktop_first_answer.is_none());
    }
}
