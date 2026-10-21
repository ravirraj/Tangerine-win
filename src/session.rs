//! The drag session: the files of one drag, and where they came from.
//!
//! One drag is one session. The session owns the file list that the wheel is
//! allowed to use, and it owns the window the drag started in, so that the
//! files can be read again while the choice is being made.
//!
//! Two rules keep the list honest, and both are enforced here rather than in
//! the callers:
//!
//! 1. the files of one session are never the files of another session, and
//! 2. a session that could not be resolved has no files at all, rather than
//!    the files of an earlier drag.
//!
//! Nothing is ever invented. A path that was not read is not a path.

use std::path::PathBuf;

use windows::Win32::Foundation::HWND;

/// What the shell said about the files of a drag.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Resolution {
    /// The shell answered. The list may still be empty: an Explorer window
    /// with nothing selected has an empty selection.
    Resolved(Vec<PathBuf>),
    /// The shell could not answer. The reason is for the log; it never holds a
    /// path, because a path that was not read is not a path.
    Unresolved(&'static str),
}

impl Resolution {
    /// The paths, if the shell answered.
    pub fn paths(&self) -> &[PathBuf] {
        match self {
            Resolution::Resolved(paths) => paths,
            Resolution::Unresolved(_) => &[],
        }
    }

    /// Whether the shell answered at all.
    pub fn is_resolved(&self) -> bool {
        matches!(self, Resolution::Resolved(_))
    }

    /// Why the shell could not answer, when it could not.
    pub fn reason(&self) -> Option<&'static str> {
        match self {
            Resolution::Resolved(_) => None,
            Resolution::Unresolved(reason) => Some(reason),
        }
    }
}

/// The files of one drag, together with where they were read from.
#[derive(Clone, Debug)]
pub struct DragSession {
    /// The number of the drag this session belongs to.
    id: u64,
    /// The Explorer window the drag started in.
    source: Option<HWND>,
    /// The files, as the shell reported them.
    paths: Vec<PathBuf>,
    /// Whether the shell could answer for this drag.
    resolved: bool,
    /// Where the list of files came from.
    origin: &'static str,
}

impl Default for DragSession {
    fn default() -> Self {
        Self::new()
    }
}

impl DragSession {
    /// A session that has nothing in it yet.
    pub fn new() -> Self {
        Self {
            id: 0,
            source: None,
            paths: Vec::new(),
            resolved: false,
            origin: "nothing yet",
        }
    }

    /// The number of the drag this session belongs to.
    pub fn id(&self) -> u64 {
        self.id
    }

    /// The Explorer window the drag started in.
    pub fn source(&self) -> Option<HWND> {
        self.source
    }

    /// The files of this session.
    pub fn paths(&self) -> &[PathBuf] {
        &self.paths
    }

    /// Whether the shell could answer for this session.
    pub fn is_resolved(&self) -> bool {
        self.resolved
    }

    /// Where the list of files came from.
    pub fn origin(&self) -> &'static str {
        self.origin
    }

    /// Starts a new session for a drag.
    ///
    /// The files of the previous session are dropped here, so that a new drag
    /// can never be shown with the files of the drag before it.
    pub fn begin(&mut self, id: u64, source: HWND) {
        self.id = id;
        self.source = Some(source);
        self.paths.clear();
        self.resolved = false;
        self.origin = "nothing yet";
    }

    /// Records what the shell said about the files of this session.
    ///
    /// The answer has to be about this session. An answer about another drag
    /// is refused, because the files of one drag are never the files of
    /// another.
    pub fn resolve(&mut self, id: u64, resolution: Resolution) -> bool {
        if id != self.id {
            return false;
        }
        match resolution {
            Resolution::Resolved(paths) => {
                self.paths = paths;
                self.resolved = true;
                self.origin = "the shell";
            }
            Resolution::Unresolved(_) => {
                // The shell could not say what was being dragged. The session
                // has no files rather than the files of an earlier drag.
                self.paths.clear();
                self.resolved = false;
                self.origin = "unresolved";
            }
        }
        true
    }

    /// Records what the drop target said about the files of this session.
    ///
    /// The drop target is the answer for a drag the shell cannot answer for.
    /// A drag that started on the desktop is such a drag: the shell knows
    /// nothing about the desktop's selection, but the drop target is told
    /// what the drag carries.
    ///
    /// The answer has to be about this session, like every other answer.
    pub fn resolve_from_drop_target(&mut self, id: u64, paths: Vec<PathBuf>) -> bool {
        if id != self.id {
            return false;
        }
        self.paths = paths;
        self.resolved = true;
        self.origin = "the drop target";
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a_path(name: &str) -> PathBuf {
        PathBuf::from(format!("C:\\Users\\test\\{name}"))
    }

    #[test]
    fn a_new_session_has_no_files() {
        let session = DragSession::new();
        assert!(session.paths().is_empty());
        assert!(!session.is_resolved());
        assert_eq!(session.id(), 0);
        assert!(session.source().is_none());
    }

    #[test]
    fn a_new_session_drops_the_files_of_the_previous_one() {
        let mut session = DragSession::new();
        session.begin(1, HWND(std::ptr::null_mut()));
        session.resolve(
            1,
            Resolution::Resolved(vec![a_path("one.txt"), a_path("two.txt")]),
        );
        assert_eq!(session.paths().len(), 2);

        // The next drag has not been resolved yet, so it has no files.
        session.begin(2, HWND(std::ptr::null_mut()));
        assert!(session.paths().is_empty());
        assert!(!session.is_resolved());
    }

    #[test]
    fn an_answer_about_another_session_is_refused() {
        let mut session = DragSession::new();
        session.begin(1, HWND(std::ptr::null_mut()));
        session.resolve(1, Resolution::Resolved(vec![a_path("one.txt")]));

        // An answer that belongs to an earlier drag is not applied.
        let applied = session.resolve(0, Resolution::Resolved(vec![a_path("old.txt")]));
        assert!(!applied);
        assert_eq!(session.paths().len(), 1);
        assert_eq!(session.paths()[0], a_path("one.txt"));
    }

    #[test]
    fn an_unresolved_session_has_no_files_at_all() {
        let mut session = DragSession::new();
        session.begin(1, HWND(std::ptr::null_mut()));
        session.resolve(1, Resolution::Resolved(vec![a_path("one.txt")]));
        assert_eq!(session.paths().len(), 1);

        // The shell could not answer this time. The files of the earlier
        // answer are not kept.
        session.resolve(
            1,
            Resolution::Unresolved("the shell does not know that window"),
        );
        assert!(session.paths().is_empty());
        assert!(!session.is_resolved());
        assert_eq!(session.origin(), "unresolved");
    }

    #[test]
    fn an_empty_selection_is_resolved_but_empty() {
        let mut session = DragSession::new();
        session.begin(1, HWND(std::ptr::null_mut()));
        session.resolve(1, Resolution::Resolved(Vec::new()));
        assert!(session.paths().is_empty());
        assert!(session.is_resolved());
        assert_eq!(session.origin(), "the shell");
    }

    #[test]
    fn the_files_of_a_session_are_never_mixed() {
        let mut session = DragSession::new();
        session.begin(1, HWND(std::ptr::null_mut()));
        session.resolve(1, Resolution::Resolved(vec![a_path("one.txt")]));

        session.begin(2, HWND(std::ptr::null_mut()));
        session.resolve(
            2,
            Resolution::Resolved(vec![a_path("two.txt"), a_path("three.txt")]),
        );
        assert_eq!(session.paths().len(), 2);
        assert!(
            session
                .paths()
                .iter()
                .all(|path| path != &a_path("one.txt"))
        );
    }

    #[test]
    fn a_resolution_that_could_not_be_read_holds_no_path() {
        let resolution = Resolution::Unresolved("the shell does not know that window");
        assert!(resolution.paths().is_empty());
        assert!(!resolution.is_resolved());
        assert_eq!(
            resolution.reason(),
            Some("the shell does not know that window")
        );
    }

    #[test]
    fn a_resolution_that_was_read_holds_the_paths() {
        let resolution = Resolution::Resolved(vec![a_path("one.txt")]);
        assert_eq!(resolution.paths().len(), 1);
        assert!(resolution.is_resolved());
        assert_eq!(resolution.reason(), None);
    }

    #[test]
    fn the_drop_target_can_answer_for_a_drag_the_shell_could_not() {
        let mut session = DragSession::new();
        session.begin(1, HWND(std::ptr::null_mut()));
        // The shell knows nothing about the desktop, so it cannot answer.
        session.resolve(
            1,
            Resolution::Unresolved("the shell does not know the desktop"),
        );
        assert!(!session.is_resolved());
        assert!(session.paths().is_empty());

        // The drop target knows what the drag carries, and its answer is
        // used.
        let applied = session.resolve_from_drop_target(1, vec![a_path("one.txt")]);
        assert!(applied);
        assert!(session.is_resolved());
        assert_eq!(session.paths().len(), 1);
        assert_eq!(session.paths()[0], a_path("one.txt"));
        assert_eq!(session.origin(), "the drop target");
    }

    #[test]
    fn the_drop_target_cannot_answer_for_another_drag() {
        let mut session = DragSession::new();
        session.begin(2, HWND(std::ptr::null_mut()));
        let applied = session.resolve_from_drop_target(1, vec![a_path("old.txt")]);
        assert!(!applied);
        assert!(session.paths().is_empty());
        assert!(!session.is_resolved());
    }
}
