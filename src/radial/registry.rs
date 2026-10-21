//! The registry of operations: what the application offers, and what it takes
//! to offer it.
//!
//! The registry holds the operations themselves. It does **not** decide
//! whether an operation may be used for a selection: that is `eligibility`,
//! and it is a different question from whether the operation is implemented at
//! all.
//!
//! Those three things are kept apart on purpose:
//!
//! - An operation is **defined** here.
//! - It is **compatible** with a selection when every selected item is one it
//!   can work on.
//! - It is **executable** when it is implemented *and* the processor it needs
//!   is there.
//!
//! Nothing in the registry claims to work. Every operation here says that it
//! is not implemented, because nothing is implemented yet: the actions in the
//! menu are still placeholders. The registry does not pretend otherwise, and
//! it does not hold a processor that is not there.

use crate::radial::actions::{IconKind, Menu, RadialMode, Segment};
use crate::radial::selection::{FileKind, Selection};

/// What an operation needs in order to run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Requirement {
    /// Nothing but the application itself.
    None,
    /// A processor, by name.
    Processor(&'static str),
}

/// An operation the application offers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Operation {
    /// The stable name of the operation.
    ///
    /// This is its identity. The label may be translated and the picture may
    /// change; the name may not.
    pub id: &'static str,
    /// The name shown on the segment.
    pub label: &'static str,
    /// The longer line shown while the segment is highlighted.
    pub detail: &'static str,
    /// The picture drawn on the segment.
    pub icon: IconKind,
    /// The wheel the operation belongs to.
    pub mode: RadialMode,
    /// The kinds of file the operation works on.
    ///
    /// A folder is not a kind of file: whether the operation works on a
    /// folder is said by `folders`.
    pub categories: &'static [FileKind],
    /// Whether the operation works on more than one item at a time.
    pub multiple: bool,
    /// Whether the operation works on a whole folder.
    pub folders: bool,
    /// Whether the operation is implemented and can run.
    pub implemented: bool,
    /// What the operation needs in order to run.
    pub requires: Requirement,
    /// Whether the operation changes the files it is given.
    ///
    /// An operation that writes a new file beside the original is not
    /// destructive. An operation that changes the original in place is. This
    /// is the flag that says where the care has to be taken.
    pub destructive: bool,
}

impl Operation {
    /// The segment that shows this operation.
    pub fn as_segment(&self) -> Segment {
        Segment::new(self.id, self.label, self.detail, self.icon)
    }
}

/// Every operation the application knows about.
///
/// The order is the order the segments are shown in. The three wheels are
/// fixed; what is inside them will change once the operations are real.
pub const OPERATIONS: &[Operation] = &[
    // ------------------------------------------------------------- convert
    Operation {
        id: "convert.png",
        label: "PNG",
        detail: "Convert to PNG",
        icon: IconKind::Picture,
        mode: RadialMode::Convert,
        categories: &[FileKind::Image],
        multiple: true,
        folders: false,
        implemented: false,
        requires: Requirement::Processor("encode/png"),
        destructive: false,
    },
    Operation {
        id: "convert.jpeg",
        label: "JPEG",
        detail: "Convert to JPEG",
        icon: IconKind::Picture,
        mode: RadialMode::Convert,
        categories: &[FileKind::Image],
        multiple: true,
        folders: false,
        implemented: false,
        requires: Requirement::Processor("encode/jpeg"),
        destructive: false,
    },
    Operation {
        id: "convert.webp",
        label: "WebP",
        detail: "Convert to WebP",
        icon: IconKind::Picture,
        mode: RadialMode::Convert,
        categories: &[FileKind::Image],
        multiple: true,
        folders: false,
        implemented: false,
        requires: Requirement::Processor("encode/webp"),
        destructive: false,
    },
    Operation {
        id: "convert.pdf",
        label: "PDF",
        detail: "Convert to PDF",
        icon: IconKind::Document,
        mode: RadialMode::Convert,
        categories: &[FileKind::Image],
        multiple: true,
        folders: false,
        implemented: false,
        requires: Requirement::Processor("encode/pdf"),
        destructive: false,
    },
    Operation {
        id: "convert.mp4",
        label: "MP4",
        detail: "Convert to MP4",
        icon: IconKind::Film,
        mode: RadialMode::Convert,
        categories: &[FileKind::Video],
        multiple: true,
        folders: false,
        implemented: false,
        requires: Requirement::Processor("encode/mp4"),
        destructive: false,
    },
    Operation {
        id: "convert.gif",
        label: "GIF",
        detail: "Convert to GIF",
        icon: IconKind::Film,
        mode: RadialMode::Convert,
        categories: &[FileKind::Video, FileKind::Image],
        multiple: true,
        folders: false,
        implemented: false,
        requires: Requirement::Processor("encode/gif"),
        destructive: false,
    },
    Operation {
        id: "convert.tiff",
        label: "TIFF",
        detail: "Convert to TIFF",
        icon: IconKind::Picture,
        mode: RadialMode::Convert,
        categories: &[FileKind::Image],
        multiple: true,
        folders: false,
        implemented: false,
        requires: Requirement::Processor("encode/tiff"),
        destructive: false,
    },
    Operation {
        id: "convert.bmp",
        label: "BMP",
        detail: "Convert to BMP",
        icon: IconKind::Picture,
        mode: RadialMode::Convert,
        categories: &[FileKind::Image],
        multiple: true,
        folders: false,
        implemented: false,
        requires: Requirement::Processor("encode/bmp"),
        destructive: false,
    },
    // --------------------------------------------------------------- tools
    Operation {
        id: "tools.compress",
        label: "Compress",
        detail: "Make the file smaller",
        icon: IconKind::Gauge,
        mode: RadialMode::Tools,
        categories: &[
            FileKind::Image,
            FileKind::Video,
            FileKind::Audio,
            FileKind::Pdf,
            FileKind::Document,
            FileKind::Archive,
        ],
        multiple: true,
        folders: false,
        implemented: false,
        requires: Requirement::Processor("compress"),
        destructive: false,
    },
    Operation {
        id: "tools.crop",
        label: "Crop",
        detail: "Trim the edges away",
        icon: IconKind::Scissors,
        mode: RadialMode::Tools,
        categories: &[FileKind::Image],
        multiple: true,
        folders: false,
        implemented: false,
        requires: Requirement::Processor("crop"),
        destructive: false,
    },
    Operation {
        id: "tools.trim",
        label: "Trim",
        detail: "Cut the beginning or the end",
        icon: IconKind::Scissors,
        mode: RadialMode::Tools,
        categories: &[FileKind::Image, FileKind::Video],
        multiple: true,
        folders: false,
        implemented: false,
        requires: Requirement::Processor("trim"),
        destructive: false,
    },
    Operation {
        id: "tools.merge",
        label: "Merge",
        detail: "Join several files together",
        icon: IconKind::Merge,
        mode: RadialMode::Tools,
        categories: &[FileKind::Image, FileKind::Video, FileKind::Audio],
        multiple: true,
        folders: false,
        implemented: false,
        requires: Requirement::Processor("merge"),
        destructive: false,
    },
    Operation {
        id: "tools.metadata",
        label: "Metadata",
        detail: "Edit the file metadata",
        icon: IconKind::Pencil,
        mode: RadialMode::Tools,
        categories: &FileKind::FILES,
        multiple: true,
        folders: true,
        implemented: false,
        requires: Requirement::Processor("metadata"),
        destructive: false,
    },
    // ------------------------------------------------------ file actions
    Operation {
        id: "actions.copy-path",
        label: "Path",
        detail: "Copy the full path",
        icon: IconKind::Lines,
        mode: RadialMode::Actions,
        categories: &FileKind::FILES,
        multiple: true,
        folders: true,
        implemented: false,
        requires: Requirement::None,
        destructive: false,
    },
    Operation {
        id: "actions.duplicate",
        label: "Copy",
        detail: "Make a duplicate",
        icon: IconKind::Copy,
        mode: RadialMode::Actions,
        categories: &FileKind::FILES,
        multiple: true,
        folders: true,
        implemented: false,
        requires: Requirement::None,
        destructive: false,
    },
    Operation {
        id: "actions.rename",
        label: "Rename",
        detail: "Give the file a new name",
        icon: IconKind::Pencil,
        mode: RadialMode::Actions,
        categories: &FileKind::FILES,
        // One thing at a time: one name, one file.
        multiple: false,
        folders: true,
        implemented: false,
        requires: Requirement::None,
        // This one changes the original.
        destructive: true,
    },
    Operation {
        id: "actions.zip",
        label: "ZIP",
        detail: "Put the files in an archive",
        icon: IconKind::Archive,
        mode: RadialMode::Actions,
        categories: &FileKind::FILES,
        multiple: true,
        folders: true,
        implemented: false,
        requires: Requirement::Processor("archive/zip"),
        destructive: false,
    },
    Operation {
        id: "actions.reveal",
        label: "Show",
        detail: "Reveal in Explorer",
        icon: IconKind::Folder,
        mode: RadialMode::Actions,
        categories: &FileKind::FILES,
        multiple: true,
        folders: true,
        implemented: false,
        requires: Requirement::None,
        destructive: false,
    },
];

/// What the machine can do right now.
///
/// The registry says what an operation needs; the environment says what is
/// there. An operation that needs a processor cannot run until that
/// processor is in the environment, whatever the registry says.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Environment {
    /// The processors that are there, by name.
    pub processors: Vec<String>,
}

impl Environment {
    /// An environment with nothing in it.
    ///
    /// This is the environment of the program today: nothing is implemented,
    /// so nothing that needs a processor can run.
    pub fn none() -> Environment {
        Environment {
            processors: Vec::new(),
        }
    }

    /// Whether a processor is there.
    pub fn has(&self, name: &str) -> bool {
        self.processors.iter().any(|processor| processor == name)
    }
}

/// Why an operation cannot be used for a selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ineligible {
    /// Nothing is selected.
    NothingSelected,
    /// Some of the selected items could not be read, so it is not known what
    /// the selection really is.
    UnreadableItems(usize),
    /// The operation works on one item at a time.
    SingleItemOnly,
    /// The operation does not work on a folder.
    NotFolders,
    /// The operation does not work on this kind of file.
    UnsupportedCategory(FileKind),
    /// The operation is not implemented.
    NotImplemented,
    /// The processor the operation needs is not there.
    ProcessorMissing(&'static str),
    /// There is no operation with that name.
    UnknownOperation,
}

impl Ineligible {
    /// The name used in the log.
    pub fn name(self) -> &'static str {
        match self {
            Ineligible::NothingSelected => "nothing is selected",
            Ineligible::UnreadableItems(_) => "some of the items could not be read",
            Ineligible::SingleItemOnly => "the operation works on one item at a time",
            Ineligible::NotFolders => "the operation does not work on a folder",
            Ineligible::UnsupportedCategory(_) => {
                "the operation does not work on this kind of file"
            }
            Ineligible::NotImplemented => "the operation is not implemented",
            Ineligible::ProcessorMissing(_) => "the processor is not there",
            Ineligible::UnknownOperation => "there is no operation with that name",
        }
    }
}

/// Whether an operation may be used for a selection.
///
/// The rules are in one place, and they look at the **whole** selection: an
/// operation is offered when every selected item is one it can work on, not
/// when one of them happens to be. An image and a PDF together are not an
/// image selection with a PDF in it.
///
/// The order of the rules is the order in which the reasons are given. The
/// selection is looked at first, because that is the more useful answer, and
/// the state of the operation afterwards.
pub fn eligibility(
    operation: &Operation,
    selection: &Selection,
    environment: &Environment,
) -> Result<(), Ineligible> {
    if selection.is_empty() {
        return Err(Ineligible::NothingSelected);
    }

    // An item that could not be read means the selection is not known, and an
    // operation that works on the known half of it is not safe to offer.
    let unreadable = selection.unreadable();
    if unreadable > 0 {
        return Err(Ineligible::UnreadableItems(unreadable));
    }

    if selection.len() > 1 && !operation.multiple {
        return Err(Ineligible::SingleItemOnly);
    }

    // Every item has to be one the operation can work on.
    for item in selection.items() {
        if item.kind == FileKind::Folder {
            if !operation.folders {
                return Err(Ineligible::NotFolders);
            }
        } else if !operation.categories.contains(&item.kind) {
            return Err(Ineligible::UnsupportedCategory(item.kind));
        }
    }

    // And then the operation itself has to be able to run.
    if !operation.implemented {
        return Err(Ineligible::NotImplemented);
    }
    if let Requirement::Processor(name) = operation.requires
        && !environment.has(name)
    {
        return Err(Ineligible::ProcessorMissing(name));
    }
    Ok(())
}

/// The operations of one wheel, in the order they are shown.
pub fn operations_for(mode: RadialMode) -> impl Iterator<Item = &'static Operation> {
    OPERATIONS
        .iter()
        .filter(move |operation| operation.mode == mode)
}

/// The choices the wheels may offer for a selection.
///
/// Only the operations that can really be run for these files are in it: the
/// ones that fit every selected item, that are implemented, and whose
/// processor is there. A wheel with nothing to offer is left empty, which is
/// what a selection that nothing can be done with looks like.
pub fn menu_for(selection: &Selection, environment: &Environment) -> Menu {
    menu_from(OPERATIONS, selection, environment)
}

/// The same, for a given set of operations.
///
/// The rule is the same one the registry uses; this is the rule with the
/// operations handed in, so that it can be tried with operations that are not
/// in the registry.
pub fn menu_from(
    operations: &[Operation],
    selection: &Selection,
    environment: &Environment,
) -> Menu {
    let mut menu = Menu::empty();
    for mode in RadialMode::ALL {
        let segments = eligible_operations_from(operations, mode, selection, environment)
            .into_iter()
            .map(Operation::as_segment)
            .collect();
        menu.set(mode, segments);
    }
    menu
}

/// The choices the wheels would show if everything could be run.
///
/// This is the design of the wheels, and it is what the development wheel
/// shows. It is not what a drag is offered: the registry says which of these
/// can really be run, and that is what `menu_for` gives.
pub fn design_menu() -> Menu {
    let mut menu = Menu::empty();
    for mode in RadialMode::ALL {
        let segments = operations_for(mode).map(Operation::as_segment).collect();
        menu.set(mode, segments);
    }
    menu
}

/// The operation with this name.
///
/// The name is the identity of an operation: the label may be translated and
/// the picture may change, the name may not.
pub fn operation_by_id(id: &str) -> Option<&'static Operation> {
    OPERATIONS.iter().find(|operation| operation.id == id)
}

/// The operations of one wheel that may be used for a selection, in the order
/// they are shown.
pub fn eligible_operations(
    mode: RadialMode,
    selection: &Selection,
    environment: &Environment,
) -> Vec<&'static Operation> {
    eligible_operations_from(OPERATIONS, mode, selection, environment)
}

/// The same, for a given set of operations.
///
/// The registry is the one place that knows what there is. This is the same
/// rule with the operations handed in, so that the rule can be tried with
/// operations that are not in the registry.
pub fn eligible_operations_from<'a>(
    operations: &'a [Operation],
    mode: RadialMode,
    selection: &Selection,
    environment: &Environment,
) -> Vec<&'a Operation> {
    operations
        .iter()
        .filter(|operation| operation.mode == mode)
        .filter(|operation| eligibility(operation, selection, environment).is_ok())
        .collect()
}

/// Whether a chosen operation may be run for the files of a drag.
///
/// The operation is found by its name, which is its identity, and it is
/// checked again against the files: what is on the screen must be what is
/// run. A choice that was made before the files changed is not run.
pub fn may_run(
    id: &str,
    selection: &Selection,
    environment: &Environment,
) -> Result<&'static Operation, Ineligible> {
    let operation = operation_by_id(id).ok_or(Ineligible::UnknownOperation)?;
    eligibility(operation, selection, environment)?;
    Ok(operation)
}

#[cfg(test)]
mod tests {
    use super::*;
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
                "open-tangerine-registry-{}-{}-{}",
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

        /// Puts a folder with this name in the folder and gives back its path.
        fn folder(&self, name: &str) -> PathBuf {
            let path = self.path.join(name);
            std::fs::create_dir_all(&path).expect("the test folder could be made");
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
            icon: IconKind::Picture,
            mode: RadialMode::Convert,
            categories: &[FileKind::Image],
            multiple: true,
            folders: false,
            implemented: true,
            requires: Requirement::None,
            destructive: false,
        }
    }

    /// The environment of a test where everything is there.
    fn everything() -> Environment {
        Environment {
            processors: vec!["encode/png".to_string()],
        }
    }

    /// An operation for a test that can really be run.
    fn ready(name: &'static str) -> Operation {
        Operation {
            id: name,
            label: name,
            detail: name,
            icon: IconKind::Picture,
            mode: RadialMode::Convert,
            categories: &[FileKind::Image],
            multiple: true,
            folders: false,
            implemented: true,
            requires: Requirement::None,
            destructive: false,
        }
    }

    /// A selection of pictures, for a test.
    fn pictures(folder: &TempFolder, count: usize) -> Selection {
        let paths: Vec<PathBuf> = (0..count)
            .map(|index| folder.file(&format!("picture{index}.png")))
            .collect();
        Selection::inspect(&paths)
    }

    // ------------------------------------------------------- the menu itself

    #[test]
    fn a_selection_with_nothing_to_do_has_an_empty_menu() {
        let folder = TempFolder::new("empty-menu");
        let selection = pictures(&folder, 1);
        // Nothing in the registry is implemented, so nothing may be offered.
        let menu = menu_for(&selection, &everything());
        for mode in RadialMode::ALL {
            assert!(
                menu.segments(mode).is_empty(),
                "the {} wheel has something on it",
                mode.name()
            );
        }
    }

    #[test]
    fn one_operation_that_may_be_run_gives_one_segment() {
        let folder = TempFolder::new("one-segment");
        let selection = pictures(&folder, 1);
        let operations = [ready("test.one")];
        let menu = menu_from(&operations, &selection, &everything());
        assert_eq!(menu.segments(RadialMode::Convert).len(), 1);
        assert_eq!(menu.segments(RadialMode::Convert)[0].id, "test.one");
        // The other wheels are not affected.
        assert!(menu.segments(RadialMode::Tools).is_empty());
        assert!(menu.segments(RadialMode::Actions).is_empty());
    }

    #[test]
    fn several_operations_give_several_segments_in_the_same_order() {
        let folder = TempFolder::new("several-segments");
        let selection = pictures(&folder, 1);
        let operations = [
            ready("test.first"),
            ready("test.second"),
            ready("test.third"),
        ];
        let menu = menu_from(&operations, &selection, &everything());
        let ids: Vec<String> = menu
            .segments(RadialMode::Convert)
            .iter()
            .map(|segment| segment.id.clone())
            .collect();
        assert_eq!(ids, vec!["test.first", "test.second", "test.third"]);
    }

    #[test]
    fn the_wheels_can_hold_different_numbers_of_choices() {
        let folder = TempFolder::new("different-counts");
        let selection = pictures(&folder, 1);
        let mut operations = vec![ready("test.convert.one"), ready("test.convert.two")];
        let mut tool = ready("test.tool.one");
        tool.mode = RadialMode::Tools;
        operations.push(tool);
        let menu = menu_from(&operations, &selection, &everything());
        assert_eq!(menu.segments(RadialMode::Convert).len(), 2);
        assert_eq!(menu.segments(RadialMode::Tools).len(), 1);
        assert!(menu.segments(RadialMode::Actions).is_empty());
    }

    #[test]
    fn an_empty_wheel_does_not_break_the_others() {
        let folder = TempFolder::new("empty-one-wheel");
        let selection = pictures(&folder, 1);
        let mut tool = ready("test.tool.one");
        tool.mode = RadialMode::Tools;
        let operations = [tool];
        let menu = menu_from(&operations, &selection, &everything());
        assert!(menu.segments(RadialMode::Convert).is_empty());
        assert_eq!(menu.segments(RadialMode::Tools).len(), 1);
    }

    #[test]
    fn a_segment_carries_the_name_of_its_operation() {
        let folder = TempFolder::new("segment-identity");
        let selection = pictures(&folder, 1);
        let mut operation = ready("test.named");
        operation.label = "Named";
        operation.detail = "What it does";
        let operations = [operation];
        let menu = menu_from(&operations, &selection, &everything());
        let segment = &menu.segments(RadialMode::Convert)[0];
        assert_eq!(segment.id, "test.named");
        assert_eq!(segment.label, "Named");
        assert_eq!(segment.detail, "What it does");
    }

    // ------------------------------------------------ the rules of the menu

    #[test]
    fn an_operation_that_is_not_implemented_is_never_offered() {
        let folder = TempFolder::new("not-implemented");
        let selection = pictures(&folder, 1);
        let mut operation = ready("test.not-implemented");
        operation.implemented = false;
        let operations = [operation];
        let menu = menu_from(&operations, &selection, &everything());
        assert!(menu.segments(RadialMode::Convert).is_empty());
    }

    #[test]
    fn an_operation_whose_processor_is_missing_is_not_offered() {
        let folder = TempFolder::new("no-processor");
        let selection = pictures(&folder, 1);
        let mut operation = ready("test.needs-processor");
        operation.requires = Requirement::Processor("encode/png");
        let operations = [operation];
        // The processor is not there.
        let menu = menu_from(&operations, &selection, &Environment::none());
        assert!(menu.segments(RadialMode::Convert).is_empty());
        // It is.
        let menu = menu_from(&operations, &selection, &everything());
        assert_eq!(menu.segments(RadialMode::Convert).len(), 1);
    }

    #[test]
    fn an_operation_of_another_kind_is_not_offered() {
        let folder = TempFolder::new("another-kind");
        let selection = Selection::inspect(&[folder.file("paper.pdf")]);
        let operations = [ready("test.for-images")];
        let menu = menu_from(&operations, &selection, &everything());
        assert!(menu.segments(RadialMode::Convert).is_empty());
    }

    #[test]
    fn the_menu_offers_nothing_when_the_operation_is_for_one_item_at_a_time() {
        let folder = TempFolder::new("one-at-a-time");
        let selection = pictures(&folder, 2);
        let mut operation = ready("test.one-at-a-time");
        operation.multiple = false;
        let operations = [operation];
        let menu = menu_from(&operations, &selection, &everything());
        assert!(menu.segments(RadialMode::Convert).is_empty());
        // One file is fine.
        let one = pictures(&folder, 1);
        let menu = menu_from(&operations, &one, &everything());
        assert_eq!(menu.segments(RadialMode::Convert).len(), 1);
    }

    #[test]
    fn the_menu_offers_nothing_when_the_operation_does_not_work_on_a_folder() {
        let folder = TempFolder::new("no-folders");
        let selection = Selection::inspect(&[folder.folder("pictures")]);
        let operations = [ready("test.for-files")];
        let menu = menu_from(&operations, &selection, &everything());
        assert!(menu.segments(RadialMode::Convert).is_empty());
    }

    #[test]
    fn a_selection_with_an_item_that_cannot_be_read_is_offered_nothing() {
        let folder = TempFolder::new("unreadable-item");
        let good = folder.file("one.png");
        let missing = folder.path.join("not-here.png");
        let selection = Selection::inspect(&[good, missing]);
        let operations = [ready("test.anything")];
        let menu = menu_from(&operations, &selection, &everything());
        assert!(menu.segments(RadialMode::Convert).is_empty());
    }

    #[test]
    fn an_empty_selection_is_offered_nothing() {
        let selection = Selection::inspect(&[]);
        let operations = [ready("test.anything")];
        let menu = menu_from(&operations, &selection, &everything());
        assert!(menu.segments(RadialMode::Convert).is_empty());
    }

    // ------------------------------------------------------ running an action

    #[test]
    fn a_choice_that_is_no_longer_valid_is_not_run() {
        let folder = TempFolder::new("stale");
        // The choice was made for a picture.
        let pictures_selection = pictures(&folder, 1);
        // The files have changed since.
        let papers = Selection::inspect(&[folder.file("paper.pdf")]);
        // The operation is looked up by its name and is checked again.
        let result = may_run("convert.png", &papers, &everything());
        assert!(result.is_err(), "a stale choice was run");
        // For the files it was made for, it is fine.
        let result = may_run("convert.png", &pictures_selection, &everything());
        assert!(
            result.is_err(),
            "an operation that is not implemented was run"
        );
    }

    #[test]
    fn a_choice_of_an_operation_that_does_not_exist_is_not_run() {
        let folder = TempFolder::new("unknown");
        let selection = pictures(&folder, 1);
        let result = may_run("there.is.no.such.operation", &selection, &everything());
        assert_eq!(result.err(), Some(Ineligible::UnknownOperation));
    }

    // ------------------------------------------------------- the registry

    #[test]
    fn every_operation_has_a_name_of_its_own() {
        let mut ids: Vec<&str> = OPERATIONS.iter().map(|operation| operation.id).collect();
        let count = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), count, "two operations share a name");
    }

    #[test]
    fn every_operation_belongs_to_a_wheel() {
        for mode in RadialMode::ALL {
            assert!(
                operations_for(mode).next().is_some(),
                "the {} wheel has no operations",
                mode.name()
            );
        }
    }

    #[test]
    fn the_registry_does_not_claim_that_anything_works() {
        // Nothing is implemented yet, and the registry must say so rather
        // than pretend otherwise.
        for entry in OPERATIONS {
            assert!(!entry.implemented, "{} says it is implemented", entry.id);
        }
    }

    #[test]
    fn the_registry_has_no_folder_in_a_file_list() {
        for entry in OPERATIONS {
            assert!(
                !entry.categories.contains(&FileKind::Folder),
                "{} says it works on folders through its file kinds",
                entry.id
            );
        }
    }

    #[test]
    fn the_operations_that_change_the_original_say_so() {
        for entry in OPERATIONS {
            if entry.id == "actions.rename" {
                assert!(entry.destructive, "renaming changes the original");
            } else {
                assert!(
                    !entry.destructive,
                    "{} says it changes the original",
                    entry.id
                );
            }
        }
    }

    // ------------------------------------------------------ the eligibility

    #[test]
    fn nothing_is_eligible_when_nothing_is_selected() {
        let selection = Selection::inspect(&[]);
        let result = eligibility(&operation("test"), &selection, &everything());
        assert_eq!(result, Err(Ineligible::NothingSelected));
    }

    #[test]
    fn an_operation_works_on_the_kind_it_supports() {
        let folder = TempFolder::new("supported");
        let selection = Selection::inspect(&[folder.file("photo.png")]);
        assert_eq!(
            eligibility(&operation("test"), &selection, &everything()),
            Ok(())
        );
    }

    #[test]
    fn an_operation_does_not_work_on_a_kind_it_does_not_support() {
        let folder = TempFolder::new("unsupported");
        let selection = Selection::inspect(&[folder.file("paper.pdf")]);
        assert_eq!(
            eligibility(&operation("test"), &selection, &everything()),
            Err(Ineligible::UnsupportedCategory(FileKind::Pdf))
        );
    }

    #[test]
    fn one_matching_item_is_not_enough() {
        // An image and a PDF together. The operation only works on images, so
        // it is not offered, even though one of the two is an image.
        let folder = TempFolder::new("one-match");
        let selection = Selection::inspect(&[folder.file("photo.png"), folder.file("paper.pdf")]);
        assert_eq!(
            eligibility(&operation("test"), &selection, &everything()),
            Err(Ineligible::UnsupportedCategory(FileKind::Pdf))
        );
    }

    #[test]
    fn a_mixed_selection_works_when_every_kind_is_supported() {
        let folder = TempFolder::new("mixed-ok");
        let mut entry = operation("test");
        entry.categories = &[FileKind::Image, FileKind::Pdf];
        let selection = Selection::inspect(&[folder.file("photo.png"), folder.file("paper.pdf")]);
        assert_eq!(eligibility(&entry, &selection, &everything()), Ok(()));
    }

    #[test]
    fn an_operation_that_is_not_implemented_is_never_eligible() {
        let folder = TempFolder::new("not-implemented");
        let selection = Selection::inspect(&[folder.file("photo.png")]);
        let mut entry = operation("test");
        entry.implemented = false;
        assert_eq!(
            eligibility(&entry, &selection, &everything()),
            Err(Ineligible::NotImplemented)
        );
    }

    #[test]
    fn an_operation_that_needs_a_processor_needs_it_to_be_there() {
        let folder = TempFolder::new("processor");
        let selection = Selection::inspect(&[folder.file("photo.png")]);
        let mut entry = operation("test");
        entry.requires = Requirement::Processor("encode/png");
        // The processor is not there.
        assert_eq!(
            eligibility(&entry, &selection, &Environment::none()),
            Err(Ineligible::ProcessorMissing("encode/png"))
        );
        // It is.
        assert_eq!(eligibility(&entry, &selection, &everything()), Ok(()));
    }

    #[test]
    fn an_operation_that_needs_nothing_but_the_application_always_has_what_it_needs() {
        let folder = TempFolder::new("no-processor");
        let selection = Selection::inspect(&[folder.file("photo.png")]);
        let mut entry = operation("test");
        entry.requires = Requirement::None;
        assert_eq!(
            eligibility(&entry, &selection, &Environment::none()),
            Ok(())
        );
    }

    #[test]
    fn the_menu_offers_nothing_when_the_operation_is_for_one_item_at_a_time_only() {
        let folder = TempFolder::new("single-only");
        let selection = Selection::inspect(&[folder.file("one.png"), folder.file("two.png")]);
        let mut entry = operation("test");
        entry.multiple = false;
        assert_eq!(
            eligibility(&entry, &selection, &everything()),
            Err(Ineligible::SingleItemOnly)
        );
        // One item is fine.
        let one = Selection::inspect(&[folder.file("one.png")]);
        assert_eq!(eligibility(&entry, &one, &everything()), Ok(()));
    }

    #[test]
    fn an_operation_that_works_on_one_at_a_time_is_offered_for_several_when_it_says_so() {
        let folder = TempFolder::new("several-ok");
        let selection = Selection::inspect(&[folder.file("one.png"), folder.file("two.png")]);
        let entry = operation("test");
        assert!(entry.multiple);
        assert_eq!(eligibility(&entry, &selection, &everything()), Ok(()));
    }

    #[test]
    fn the_menu_offers_nothing_when_the_operation_does_not_work_on_folders() {
        let folder = TempFolder::new("no-folders");
        let selection = Selection::inspect(&[folder.folder("pictures")]);
        assert_eq!(
            eligibility(&operation("test"), &selection, &everything()),
            Err(Ineligible::NotFolders)
        );
    }

    #[test]
    fn an_operation_that_works_on_folders_is_offered_for_one() {
        let folder = TempFolder::new("folders-ok");
        let selection = Selection::inspect(&[folder.folder("pictures")]);
        let mut entry = operation("test");
        entry.folders = true;
        assert_eq!(eligibility(&entry, &selection, &everything()), Ok(()));
    }

    #[test]
    fn a_selection_with_an_item_that_cannot_be_read_is_not_offered() {
        let folder = TempFolder::new("unreadable");
        let good = folder.file("photo.png");
        let missing = folder.path.join("not-here.png");
        let selection = Selection::inspect(&[good, missing]);
        assert_eq!(
            eligibility(&operation("test"), &selection, &everything()),
            Err(Ineligible::UnreadableItems(1))
        );
    }

    #[test]
    fn the_operations_of_a_wheel_keep_their_order() {
        let order: Vec<&str> = operations_for(RadialMode::Convert)
            .map(|entry| entry.id)
            .collect();
        assert_eq!(
            order,
            vec![
                "convert.png",
                "convert.jpeg",
                "convert.webp",
                "convert.pdf",
                "convert.mp4",
                "convert.gif",
                "convert.tiff",
                "convert.bmp",
            ]
        );
    }

    #[test]
    fn the_operations_of_a_wheel_match_the_menu() {
        // The menu and the registry must agree on what is in each wheel.
        // The menu shows the operations of the wheel, in the same order.
        let design = design_menu();
        let menu: Vec<String> = design
            .segments(RadialMode::Tools)
            .iter()
            .map(|segment| segment.id.clone())
            .collect();
        let registry: Vec<&str> = operations_for(RadialMode::Tools)
            .map(|entry| entry.id)
            .collect();
        assert_eq!(menu, registry);
    }

    #[test]
    fn nothing_is_offered_while_nothing_is_implemented() {
        // The registry holds real operations, and none of them is
        // implemented. So none of them may be offered, whatever the
        // selection is and whatever the machine can do.
        let folder = TempFolder::new("not-offered");
        let selection = Selection::inspect(&[folder.file("photo.png")]);
        let environment = Environment {
            processors: OPERATIONS
                .iter()
                .filter_map(|entry| match entry.requires {
                    Requirement::Processor(name) => Some(name.to_string()),
                    Requirement::None => None,
                })
                .collect(),
        };
        let offered = eligible_operations(RadialMode::Convert, &selection, &environment);
        assert!(
            offered.is_empty(),
            "an operation that is not implemented was offered"
        );
    }
}
