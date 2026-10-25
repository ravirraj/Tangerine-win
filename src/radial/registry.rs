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
use crate::radial::selection::{FileKind, Selection, SelectionShape};

/// What an operation needs in order to run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Requirement {
    /// Nothing but the application itself.
    ///
    /// These are the file actions the program does on its own: they need no
    /// conversion backend, and they work on any file.
    None,
    /// A processor, by name.
    Processor(&'static str),
    /// One of several processors - whichever can read the file.
    ///
    /// An operation that works on two kinds with two different backs - a
    /// crop that cuts a video with the conversion program and a picture
    /// with the program's own two hands - needs either of them to be
    /// there, and each dragged file has to be readable by the one that
    /// can take it.
    AnyProcessor(&'static [&'static str]),
}

/// How many items an operation takes, and whether they have to be alike.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Batch {
    /// One item at a time.
    One,
    /// Several items, of any of the kinds the operation works on.
    Several,
    /// Several items, all of the same kind.
    ///
    /// This is for the operations that put their inputs together, like
    /// merging: a picture and a sound are not one thing to merge.
    SeveralAlike,
}

/// What an operation does with the result.
///
/// This is the output policy: it says where the result goes and what it is
/// written in. An operation that writes over the original says so here, and
/// that is the one that changes the user's files.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Output {
    /// A new file beside the original, in this format.
    ///
    /// The format is in lower case and without the dot: `png`, `jpeg`. The
    /// original is left alone.
    Beside(&'static str),
    /// A new file beside the original, in the same format as the file it was
    /// given.
    BesideAsInput,
    /// The original itself, in place.
    ///
    /// This is the one that changes the user's files.
    InPlace,
    /// Nothing is written: the operation only reports.
    Nothing,
}

/// What to do when the result is already there.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Collision {
    /// Stop and ask the user.
    Ask,
    /// Use another name.
    Rename,
}

/// A backend that reads and writes files.
///
/// This is what a conversion runs on. What it can read and what it can write
/// is what it is good at, and the menu is built from that: an operation is
/// only offered when the files that were dragged are ones the processor can
/// read, and the format the operation is to write is one the processor can
/// write.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Processor {
    /// The name of the processor. This is what an operation asks for.
    pub name: String,
    /// The formats it can read, in lower case and without the dot.
    pub reads: Vec<String>,
    /// The formats it can write, in lower case and without the dot.
    pub writes: Vec<String>,
}

impl Processor {
    /// Whether the processor can read files of this format.
    pub fn can_read(&self, format: &str) -> bool {
        self.reads.iter().any(|known| known == format)
    }

    /// Whether the processor can write files of this format.
    pub fn can_write(&self, format: &str) -> bool {
        self.writes.iter().any(|known| known == format)
    }
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
    /// How many items it takes, and whether they have to be alike.
    pub batch: Batch,
    /// Whether it works on a whole folder.
    pub folders: bool,
    /// What it does with the result: where it goes and what it is written in.
    pub output: Output,
    /// What to do when the result is already there.
    pub collision: Collision,
    /// Whether the operation is implemented and can run.
    pub implemented: bool,
    /// What the operation needs in order to run.
    pub requires: Requirement,
}

impl Operation {
    /// The segment that shows this operation.
    pub fn as_segment(&self) -> Segment {
        Segment::new(self.id, self.label, self.detail, self.icon)
    }

    /// Whether the operation changes the files it is given.
    ///
    /// An operation that writes a new file beside the original does not. One
    /// that writes over the original does, and that is the one that has to be
    /// careful.
    pub fn is_destructive(&self) -> bool {
        self.output == Output::InPlace
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
        batch: Batch::Several,
        folders: false,
        output: Output::Beside("png"),
        collision: Collision::Rename,
        implemented: true,
        requires: Requirement::Processor("image"),
    },
    Operation {
        id: "convert.jpeg",
        label: "JPEG",
        detail: "Convert to JPEG",
        icon: IconKind::Picture,
        mode: RadialMode::Convert,
        categories: &[FileKind::Image],
        batch: Batch::Several,
        folders: false,
        output: Output::Beside("jpeg"),
        collision: Collision::Rename,
        implemented: true,
        requires: Requirement::Processor("image"),
    },
    Operation {
        id: "convert.webp",
        label: "WebP",
        detail: "Convert to WebP",
        icon: IconKind::Picture,
        mode: RadialMode::Convert,
        categories: &[FileKind::Image],
        batch: Batch::Several,
        folders: false,
        output: Output::Beside("webp"),
        collision: Collision::Rename,
        implemented: true,
        // The image processor writes WebP through the conversion program.
        requires: Requirement::Processor("image"),
    },
    Operation {
        id: "convert.pdf",
        label: "PDF",
        detail: "Convert to PDF",
        icon: IconKind::Document,
        mode: RadialMode::Convert,
        categories: &[FileKind::Image],
        batch: Batch::Several,
        folders: false,
        output: Output::Beside("pdf"),
        collision: Collision::Rename,
        implemented: true,
        requires: Requirement::Processor("document"),
    },
    Operation {
        id: "convert.gif",
        label: "GIF",
        detail: "Convert to GIF",
        icon: IconKind::Film,
        mode: RadialMode::Convert,
        categories: &[FileKind::Video, FileKind::Image],
        batch: Batch::Several,
        folders: false,
        output: Output::Beside("gif"),
        collision: Collision::Rename,
        implemented: true,
        requires: Requirement::Processor("image"),
    },
    Operation {
        id: "convert.tiff",
        label: "TIFF",
        detail: "Convert to TIFF",
        icon: IconKind::Picture,
        mode: RadialMode::Convert,
        categories: &[FileKind::Image],
        batch: Batch::Several,
        folders: false,
        output: Output::Beside("tiff"),
        collision: Collision::Rename,
        implemented: true,
        requires: Requirement::Processor("image"),
    },
    Operation {
        id: "convert.bmp",
        label: "BMP",
        detail: "Convert to BMP",
        icon: IconKind::Picture,
        mode: RadialMode::Convert,
        categories: &[FileKind::Image],
        batch: Batch::Several,
        folders: false,
        output: Output::Beside("bmp"),
        collision: Collision::Rename,
        implemented: true,
        requires: Requirement::Processor("image"),
    },
    Operation {
        id: "convert.ico",
        label: "ICO",
        detail: "Convert to ICO",
        icon: IconKind::Picture,
        mode: RadialMode::Convert,
        categories: &[FileKind::Image],
        batch: Batch::Several,
        folders: false,
        output: Output::Beside("ico"),
        collision: Collision::Rename,
        implemented: true,
        requires: Requirement::Processor("image"),
    },
    Operation {
        id: "convert.pdf-png",
        label: "PNG",
        detail: "PDF to PNG",
        icon: IconKind::Picture,
        mode: RadialMode::Convert,
        categories: &[FileKind::Pdf],
        batch: Batch::Several,
        folders: false,
        output: Output::Beside("png"),
        collision: Collision::Rename,
        implemented: true,
        requires: Requirement::Processor("pdf"),
    },
    Operation {
        id: "convert.pdf-jpeg",
        label: "JPG",
        detail: "PDF to JPG",
        icon: IconKind::Picture,
        mode: RadialMode::Convert,
        categories: &[FileKind::Pdf],
        batch: Batch::Several,
        folders: false,
        output: Output::Beside("jpeg"),
        collision: Collision::Rename,
        implemented: true,
        requires: Requirement::Processor("pdf"),
    },
    Operation {
        id: "convert.pdf-txt",
        label: "TXT",
        detail: "PDF to Text",
        icon: IconKind::Document,
        mode: RadialMode::Convert,
        categories: &[FileKind::Pdf],
        batch: Batch::Several,
        folders: false,
        output: Output::Beside("txt"),
        collision: Collision::Rename,
        implemented: true,
        requires: Requirement::Processor("pdf"),
    },
    Operation {
        id: "convert.docx-txt",
        label: "TXT",
        detail: "DOCX to TXT",
        icon: IconKind::Document,
        mode: RadialMode::Convert,
        categories: &[FileKind::Document],
        batch: Batch::Several,
        folders: false,
        output: Output::Beside("txt"),
        collision: Collision::Rename,
        implemented: true,
        requires: Requirement::Processor("document"),
    },
    Operation {
        id: "convert.csv",
        label: "CSV",
        detail: "Convert to CSV",
        icon: IconKind::Document,
        mode: RadialMode::Convert,
        categories: &[FileKind::Spreadsheet],
        batch: Batch::Several,
        folders: false,
        output: Output::Beside("csv"),
        collision: Collision::Rename,
        implemented: true,
        requires: Requirement::Processor("document"),
    },
    Operation {
        id: "convert.docx-pdf",
        label: "PDF",
        detail: "DOCX to PDF",
        icon: IconKind::Document,
        mode: RadialMode::Convert,
        categories: &[FileKind::Document],
        batch: Batch::Several,
        folders: false,
        output: Output::Beside("pdf"),
        collision: Collision::Rename,
        implemented: true,
        requires: Requirement::Processor("office"),
    },
    Operation {
        id: "convert.xlsx-pdf",
        label: "PDF",
        detail: "Spreadsheet to PDF",
        icon: IconKind::Document,
        mode: RadialMode::Convert,
        categories: &[FileKind::Spreadsheet],
        batch: Batch::Several,
        folders: false,
        output: Output::Beside("pdf"),
        collision: Collision::Rename,
        implemented: true,
        requires: Requirement::Processor("office"),
    },
    Operation {
        id: "convert.pptx-pdf",
        label: "PDF",
        detail: "Slides to PDF",
        icon: IconKind::Document,
        mode: RadialMode::Convert,
        categories: &[FileKind::Presentation],
        batch: Batch::Several,
        folders: false,
        output: Output::Beside("pdf"),
        collision: Collision::Rename,
        implemented: true,
        requires: Requirement::Processor("office"),
    },
    Operation {
        id: "convert.mp4",
        label: "MP4",
        detail: "Convert to MP4",
        icon: IconKind::Film,
        mode: RadialMode::Convert,
        categories: &[FileKind::Video],
        batch: Batch::Several,
        folders: false,
        output: Output::Beside("mp4"),
        collision: Collision::Rename,
        implemented: true,
        requires: Requirement::Processor("video"),
    },
    Operation {
        id: "convert.mp3",
        label: "MP3",
        detail: "Convert to MP3",
        icon: IconKind::Music,
        mode: RadialMode::Convert,
        categories: &[FileKind::Video, FileKind::Audio],
        batch: Batch::Several,
        folders: false,
        output: Output::Beside("mp3"),
        collision: Collision::Rename,
        implemented: true,
        requires: Requirement::Processor("audio"),
    },
    Operation {
        id: "convert.wav",
        label: "WAV",
        detail: "Convert to WAV",
        icon: IconKind::Music,
        mode: RadialMode::Convert,
        categories: &[FileKind::Audio],
        batch: Batch::Several,
        folders: false,
        output: Output::Beside("wav"),
        collision: Collision::Rename,
        implemented: true,
        requires: Requirement::Processor("audio"),
    },
    Operation {
        id: "convert.ogg",
        label: "OGG",
        detail: "Convert to OGG",
        icon: IconKind::Music,
        mode: RadialMode::Convert,
        categories: &[FileKind::Audio],
        batch: Batch::Several,
        folders: false,
        output: Output::Beside("ogg"),
        collision: Collision::Rename,
        implemented: true,
        requires: Requirement::Processor("audio"),
    },
    Operation {
        id: "convert.opus",
        label: "OPUS",
        detail: "Convert to Opus",
        icon: IconKind::Music,
        mode: RadialMode::Convert,
        categories: &[FileKind::Audio],
        batch: Batch::Several,
        folders: false,
        output: Output::Beside("opus"),
        collision: Collision::Rename,
        implemented: true,
        requires: Requirement::Processor("audio"),
    },
    Operation {
        id: "convert.aiff",
        label: "AIFF",
        detail: "Convert to AIFF",
        icon: IconKind::Music,
        mode: RadialMode::Convert,
        categories: &[FileKind::Audio],
        batch: Batch::Several,
        folders: false,
        output: Output::Beside("aiff"),
        collision: Collision::Rename,
        implemented: true,
        requires: Requirement::Processor("audio"),
    },
    Operation {
        id: "convert.m4a",
        label: "M4A",
        detail: "Convert to M4A",
        icon: IconKind::Music,
        mode: RadialMode::Convert,
        categories: &[FileKind::Audio],
        batch: Batch::Several,
        folders: false,
        output: Output::Beside("m4a"),
        collision: Collision::Rename,
        implemented: true,
        requires: Requirement::Processor("audio"),
    },
    Operation {
        id: "convert.wma",
        label: "WMA",
        detail: "Convert to WMA",
        icon: IconKind::Music,
        mode: RadialMode::Convert,
        categories: &[FileKind::Audio],
        batch: Batch::Several,
        folders: false,
        output: Output::Beside("wma"),
        collision: Collision::Rename,
        implemented: true,
        requires: Requirement::Processor("audio"),
    },
    Operation {
        id: "convert.srt",
        label: "SRT",
        detail: "Convert to SRT",
        icon: IconKind::Document,
        mode: RadialMode::Convert,
        categories: &[FileKind::Document],
        batch: Batch::Several,
        folders: false,
        output: Output::Beside("srt"),
        collision: Collision::Rename,
        implemented: true,
        requires: Requirement::Processor("document"),
    },
    Operation {
        id: "convert.vtt",
        label: "VTT",
        detail: "Convert to VTT",
        icon: IconKind::Document,
        mode: RadialMode::Convert,
        categories: &[FileKind::Document],
        batch: Batch::Several,
        folders: false,
        output: Output::Beside("vtt"),
        collision: Collision::Rename,
        implemented: true,
        requires: Requirement::Processor("document"),
    },
    Operation {
        id: "convert.txt",
        label: "TXT",
        detail: "Convert to TXT",
        icon: IconKind::Document,
        mode: RadialMode::Convert,
        categories: &[FileKind::Text],
        batch: Batch::Several,
        folders: false,
        output: Output::Beside("txt"),
        collision: Collision::Rename,
        implemented: true,
        requires: Requirement::Processor("document"),
    },
    Operation {
        id: "convert.zip",
        label: "ZIP",
        detail: "Convert to ZIP",
        icon: IconKind::Archive,
        mode: RadialMode::Convert,
        categories: &[FileKind::Archive],
        batch: Batch::Several,
        folders: false,
        output: Output::Beside("zip"),
        collision: Collision::Rename,
        implemented: true,
        requires: Requirement::Processor("document"),
    },
    Operation {
        id: "convert.tar",
        label: "TAR",
        detail: "Convert to TAR",
        icon: IconKind::Archive,
        mode: RadialMode::Convert,
        categories: &[FileKind::Archive],
        batch: Batch::Several,
        folders: false,
        output: Output::Beside("tar"),
        collision: Collision::Rename,
        implemented: true,
        requires: Requirement::Processor("document"),
    },
    Operation {
        id: "convert.gz",
        label: "GZ",
        detail: "Convert to GZ",
        icon: IconKind::Archive,
        mode: RadialMode::Convert,
        categories: &[FileKind::Archive],
        batch: Batch::Several,
        folders: false,
        output: Output::Beside("gz"),
        collision: Collision::Rename,
        implemented: true,
        requires: Requirement::Processor("document"),
    },
    // --------------------------------------------------------------- tools
    // The compress is offered only for the kinds it can really make
    // lighter: a picture by the program's own hands, a film and a sound
    // through the conversion program. A PDF's pages are the reader's to
    // redraw, and a document or an archive is not a thing to squeeze.
    Operation {
        id: "tools.compress",
        label: "Compress",
        detail: "Make the file smaller",
        icon: IconKind::Compress,
        mode: RadialMode::Tools,
        categories: &[FileKind::Image, FileKind::Video, FileKind::Audio],
        batch: Batch::Several,
        folders: false,
        output: Output::BesideAsInput,
        collision: Collision::Rename,
        implemented: true,
        requires: Requirement::AnyProcessor(&["image", "video", "audio"]),
    },
    Operation {
        id: "tools.crop",
        label: "Crop",
        detail: "Trim the edges away",
        icon: IconKind::Crop,
        mode: RadialMode::Tools,
        categories: &[FileKind::Image, FileKind::Video],
        batch: Batch::Several,
        folders: false,
        output: Output::BesideAsInput,
        collision: Collision::Rename,
        implemented: true,
        requires: Requirement::AnyProcessor(&["image", "video"]),
    },
    Operation {
        id: "tools.trim",
        label: "Trim",
        detail: "Cut the beginning or the end",
        icon: IconKind::Scissors,
        mode: RadialMode::Tools,
        categories: &[FileKind::Video, FileKind::Audio],
        batch: Batch::Several,
        folders: false,
        output: Output::BesideAsInput,
        collision: Collision::Rename,
        implemented: true,
        requires: Requirement::AnyProcessor(&["video", "audio"]),
    },
    // A split takes exactly one video: its clips are numbered from where the
    // video stands, and several files split at once reads as two words said
    // over each other. The output has the input's own format and stands
    // beside it, in a folder of its own the handler makes.
    Operation {
        id: "tools.split",
        label: "Split",
        detail: "Cut the video into clips",
        icon: IconKind::Scissors,
        mode: RadialMode::Tools,
        categories: &[FileKind::Video],
        batch: Batch::One,
        folders: false,
        output: Output::BesideAsInput,
        collision: Collision::Rename,
        implemented: true,
        requires: Requirement::Processor("video"),
    },
    Operation {
        id: "tools.rotate",
        label: "Rotate",
        detail: "Turn the image 90 degrees",
        icon: IconKind::Rotate,
        mode: RadialMode::Tools,
        categories: &[FileKind::Image],
        batch: Batch::Several,
        folders: false,
        output: Output::BesideAsInput,
        collision: Collision::Rename,
        implemented: true,
        requires: Requirement::Processor("image"),
    },
    Operation {
        id: "tools.resize",
        label: "Resize",
        detail: "Change the image size",
        icon: IconKind::Resize,
        mode: RadialMode::Tools,
        categories: &[FileKind::Image],
        batch: Batch::Several,
        folders: false,
        output: Output::BesideAsInput,
        collision: Collision::Rename,
        implemented: true,
        requires: Requirement::Processor("image"),
    },
    Operation {
        id: "tools.merge",
        label: "Merge",
        detail: "Join several files together",
        icon: IconKind::Merge,
        mode: RadialMode::Tools,
        categories: &[FileKind::Image, FileKind::Video, FileKind::Audio],
        // Files that are put together have to be alike: a picture and a sound
        // are not one thing to merge.
        batch: Batch::SeveralAlike,
        folders: false,
        output: Output::BesideAsInput,
        collision: Collision::Rename,
        implemented: true,
        requires: Requirement::Processor("merge"),
    },
    Operation {
        id: "tools.metadata",
        label: "Metadata",
        detail: "Edit the file metadata",
        icon: IconKind::Pencil,
        mode: RadialMode::Tools,
        categories: &[FileKind::Image, FileKind::Video, FileKind::Audio, FileKind::Pdf],
        batch: Batch::Several,
        folders: false,
        output: Output::BesideAsInput,
        collision: Collision::Rename,
        implemented: true,
        requires: Requirement::AnyProcessor(&["image", "video", "audio", "pdf"]),
    },
    // ------------------------------------------------------ file actions
    Operation {
        id: "actions.copy-path",
        label: "Path",
        detail: "Copy the full path",
        icon: IconKind::Path,
        mode: RadialMode::Actions,
        categories: &FileKind::FILES,
        batch: Batch::Several,
        folders: true,
        output: Output::Nothing,
        collision: Collision::Ask,
        implemented: true,
        // The program does this one itself: no backend is needed.
        requires: Requirement::None,
    },
    Operation {
        id: "actions.duplicate",
        label: "Copy",
        detail: "Make a duplicate",
        icon: IconKind::Copy,
        mode: RadialMode::Actions,
        categories: &FileKind::FILES,
        batch: Batch::Several,
        folders: true,
        output: Output::BesideAsInput,
        collision: Collision::Rename,
        implemented: true,
        // The program does this one itself: no backend is needed.
        requires: Requirement::None,
    },
    Operation {
        id: "actions.rename",
        label: "Rename",
        detail: "Give the file a new name",
        icon: IconKind::Pencil,
        mode: RadialMode::Actions,
        categories: &FileKind::FILES,
        // One thing at a time: one name, one file.
        batch: Batch::One,
        folders: true,
        // This one changes the file it is given.
        output: Output::InPlace,
        collision: Collision::Ask,
        implemented: true,
        // The program does this one itself: no backend is needed.
        requires: Requirement::None,
    },
    Operation {
        id: "actions.zip",
        label: "ZIP",
        detail: "Put the files in an archive",
        icon: IconKind::Archive,
        mode: RadialMode::Actions,
        categories: &FileKind::FILES,
        batch: Batch::Several,
        folders: true,
        output: Output::Beside("zip"),
        collision: Collision::Rename,
        implemented: false,
        requires: Requirement::Processor("archive"),
    },
    Operation {
        id: "actions.reveal",
        label: "Show",
        detail: "Reveal in Explorer",
        icon: IconKind::Folder,
        mode: RadialMode::Actions,
        categories: &FileKind::FILES,
        batch: Batch::Several,
        folders: true,
        output: Output::Nothing,
        collision: Collision::Ask,
        implemented: true,
        // The program does this one itself: no backend is needed.
        requires: Requirement::None,
    },
    Operation {
        id: "actions.terminal",
        label: "Terminal",
        detail: "Open in Terminal",
        icon: IconKind::Terminal,
        mode: RadialMode::Actions,
        categories: &FileKind::FILES,
        batch: Batch::Several,
        folders: true,
        output: Output::Nothing,
        collision: Collision::Ask,
        implemented: true,
        // The program does this one itself: no backend is needed.
        requires: Requirement::None,
    },
    Operation {
        id: "actions.extract",
        label: "Extract",
        detail: "Extract archive contents",
        icon: IconKind::Extract,
        mode: RadialMode::Actions,
        categories: &[FileKind::Archive],
        batch: Batch::Several,
        folders: false,
        output: Output::Nothing,
        collision: Collision::Rename,
        implemented: true,
        // The program does this one itself: no backend is needed.
        requires: Requirement::None,
    },
];

/// What the machine can do right now.
///
/// The registry says what an operation needs; the environment says what is
/// there. An operation that needs a processor cannot run until that
/// processor is in the environment, whatever the registry says.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Environment {
    /// The processors that are there.
    pub processors: Vec<Processor>,
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

    /// The processor with this name, if it is there.
    pub fn processor(&self, name: &str) -> Option<&Processor> {
        self.processors
            .iter()
            .find(|processor| processor.name == name)
    }
}

/// Why an operation cannot be used for a selection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Ineligible {
    /// Nothing is selected.
    NothingSelected,
    /// Some of the selected items could not be read, so it is not known what
    /// the selection really is.
    UnreadableItems(usize),
    /// The operation takes one item at a time, and there are several.
    SingleItemOnly,
    /// The operation needs the items to be alike, and they are not.
    MixedKinds,
    /// The operation does not work on a folder.
    NotFolders,
    /// The operation does not work on this kind of file.
    UnsupportedCategory(FileKind),
    /// The processor cannot read a file of this format.
    UnreadableFormat(String),
    /// The processor cannot write a file of this format.
    UnwritableFormat(String),
    /// The operation is not implemented.
    NotImplemented,
    /// The processor the operation needs is not there.
    ProcessorMissing(&'static str),
}

impl Ineligible {
    /// The name used in the log.
    pub fn name(&self) -> &'static str {
        match self {
            Ineligible::NothingSelected => "nothing is selected",
            Ineligible::UnreadableItems(_) => "some of the items could not be read",
            Ineligible::SingleItemOnly => "the operation takes one item at a time",
            Ineligible::MixedKinds => "the items are of different kinds",
            Ineligible::NotFolders => "the operation does not work on a folder",
            Ineligible::UnsupportedCategory(_) => {
                "the operation does not work on this kind of file"
            }
            Ineligible::UnreadableFormat(_) => "the processor cannot read this format",
            Ineligible::UnwritableFormat(_) => "the processor cannot write this format",
            Ineligible::NotImplemented => "the operation is not implemented",
            Ineligible::ProcessorMissing(_) => "the processor is not there",
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
/// The rules, in the order they are given:
///
/// 1. there is something selected, and every item of it could be read;
/// 2. the operation takes as many items as the selection has, and takes
///    items that are alike when it says so;
/// 3. every item is a kind the operation works on, and a folder only when
///    the operation works on folders;
/// 4. the operation is implemented, and the processor it needs is there;
/// 5. the processor can read every format that was selected, and can write
///    the format the operation is to produce.
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

    // How many items the operation takes.
    match operation.batch {
        Batch::One if selection.len() > 1 => return Err(Ineligible::SingleItemOnly),
        Batch::SeveralAlike => {
            // Files that are put together have to be alike.
            let shape = selection.shape();
            if !matches!(
                shape,
                SelectionShape::SeveralFiles | SelectionShape::SeveralFolders
            ) {
                return Err(Ineligible::MixedKinds);
            }
        }
        Batch::One | Batch::Several => {}
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
    // The processors this operation may call on, by its requirement.
    // An operation the application does itself has no processor at all —
    // there is nothing more the machine has to say about it: it can run.
    let processors: Vec<_> = match operation.requires {
        Requirement::None => return Ok(()),
        Requirement::Processor(name) => {
            let Some(processor) = environment.processor(name) else {
                return Err(Ineligible::ProcessorMissing(name));
            };
            vec![processor]
        }
        Requirement::AnyProcessor(names) => {
            let processors: Vec<_> = names
                .iter()
                .filter_map(|name| environment.processor(name))
                .collect();
            if processors.is_empty() {
                return Err(Ineligible::ProcessorMissing(names[0]));
            }
            processors
        }
    };

    // Each processor named has to be able to read every format that was
    // dragged - at least one of them, when the operation takes any.
    for format in selection.formats() {
        if !processors.iter().any(|processor| processor.can_read(format)) {
            return Err(Ineligible::UnreadableFormat(format.to_string()));
        }
    }

    // And one of them has to be able to write what the operation is to
    // produce.
    match operation.output {
        Output::Beside(format) => {
            if !processors.iter().any(|processor| processor.can_write(format)) {
                return Err(Ineligible::UnwritableFormat(format.to_string()));
            }
        }
        // The result keeps the format of the file it was given.
        Output::BesideAsInput | Output::InPlace => {
            for format in selection.formats() {
                if !processors.iter().any(|processor| processor.can_write(format)) {
                    return Err(Ineligible::UnwritableFormat(format.to_string()));
                }
            }
        }
        Output::Nothing => {}
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
            batch: Batch::Several,
            folders: false,
            output: Output::Nothing,
            collision: Collision::Ask,
            implemented: true,
            requires: Requirement::None,
        }
    }

    /// The environment of a test where everything is there.
    fn everything() -> Environment {
        Environment {
            processors: vec![Processor {
                name: "image".to_string(),
                reads: vec![
                    "png".to_string(),
                    "jpeg".to_string(),
                    "bmp".to_string(),
                    "tiff".to_string(),
                    "gif".to_string(),
                    "heif".to_string(),
                ],
                writes: vec![
                    "png".to_string(),
                    "jpeg".to_string(),
                    "bmp".to_string(),
                    "tiff".to_string(),
                    "heif".to_string(),
                ],
            }],
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
            batch: Batch::Several,
            folders: false,
            output: Output::Beside("png"),
            collision: Collision::Rename,
            implemented: true,
            requires: Requirement::None,
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
    fn a_wheel_with_nothing_implemented_has_an_empty_menu() {
        let folder = TempFolder::new("empty-menu");
        let selection = pictures(&folder, 1);
        let menu = menu_for(&selection, &everything());
        // All three wheels have implemented operations, so none are empty.
        assert!(
            !menu.segments(RadialMode::Convert).is_empty(),
            "the convert wheel has nothing on it"
        );
        assert!(
            !menu.segments(RadialMode::Tools).is_empty(),
            "the tools wheel has nothing on it"
        );
        assert!(
            !menu.segments(RadialMode::Actions).is_empty(),
            "the actions wheel has nothing on it"
        );
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
    fn a_conversion_into_the_format_the_file_is_already_in_is_offered() {
        // The policy about a file that is already in the format it is to be
        // put into is not in the wheel: the conversion is offered, and the
        // refusal is made when the work is done, where the reason can be said
        // in words.
        let folder = TempFolder::new("same-format");
        let selection = pictures(&folder, 1);
        let same = operation_by_id("convert.png").expect("there is a conversion to PNG");
        let other = operation_by_id("convert.jpeg").expect("there is a conversion to JPEG");
        assert!(eligibility(same, &selection, &everything()).is_ok());
        assert!(eligibility(other, &selection, &everything()).is_ok());
    }

    #[test]
    fn the_wheel_has_nothing_on_it_until_the_files_are_known() {
        let folder = TempFolder::new("not-known-yet");
        // A drag that started on the desktop: the shell cannot say what it
        // carries, so the wheel has nothing to offer.
        let nothing = Selection::inspect(&[]);
        let menu = menu_for(&nothing, &everything());
        assert!(
            menu.segments(RadialMode::Convert).is_empty(),
            "the convert wheel has something on it"
        );

        // The files are known: the wheel has the operations that may really
        // be done with them.
        let selection = pictures(&folder, 1);
        let menu = menu_for(&selection, &everything());
        assert!(
            !menu.segments(RadialMode::Convert).is_empty(),
            "the convert wheel has nothing on it"
        );
    }

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
        operation.requires = Requirement::Processor("image");
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
        operation.batch = Batch::One;
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

    // ------------------------------------------------------ the rules of the menu
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
    fn the_registry_says_exactly_what_is_implemented() {
        // The picture conversions are implemented, and nothing else is. The
        // registry has to say so plainly, because the menu is built from it.
        const IMPLEMENTED: &[&str] = &[
            "convert.png",
            "convert.jpeg",
            "convert.webp",
            "convert.pdf",
            "convert.bmp",
            "convert.tiff",
            "convert.gif",
            "convert.ico",
            "convert.pdf-png",
            "convert.pdf-jpeg",
            "convert.pdf-txt",
            "convert.docx-txt",
            "convert.csv",
            "convert.mp3",
            "convert.wav",
            "convert.ogg",
            "convert.opus",
            "convert.aiff",
            "convert.m4a",
            "convert.wma",
            "convert.docx-pdf",
            "convert.xlsx-pdf",
            "convert.pptx-pdf",
            "convert.mp3",
            "convert.wav",
            "convert.ogg",
            "convert.opus",
            "convert.aiff",
            "convert.m4a",
            "convert.wma",
            "convert.mp4",
            "convert.mp3",
            "convert.wav",
            "convert.ogg",
            "convert.opus",
            "convert.aiff",
            "convert.m4a",
            "convert.wma",
            "convert.srt",
            "convert.vtt",
            "convert.txt",
            "convert.zip",
            "convert.tar",
            "convert.gz",
            "tools.compress",
            "tools.crop",
            "tools.trim",
            "tools.split",
            "tools.rotate",
            "tools.resize",
            "tools.merge",
            "tools.metadata",
            "actions.copy-path",
            "actions.duplicate",
            "actions.rename",
            "actions.reveal",
            "actions.terminal",
            "actions.extract",
        ];
        for entry in OPERATIONS {
            assert_eq!(
                entry.implemented,
                IMPLEMENTED.contains(&entry.id),
                "{} says the wrong thing about being implemented",
                entry.id
            );
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
            // Only rename changes the file it is given; metadata writes a copy beside.
            if entry.id == "actions.rename" {
                assert!(entry.is_destructive(), "{} changes the original", entry.id);
            } else {
                assert!(
                    !entry.is_destructive(),
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
        entry.requires = Requirement::Processor("image");
        // The processor is not there.
        assert_eq!(
            eligibility(&entry, &selection, &Environment::none()),
            Err(Ineligible::ProcessorMissing("image"))
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
        entry.batch = Batch::One;
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
        assert_eq!(entry.batch, Batch::Several);
        assert_eq!(eligibility(&entry, &selection, &everything()), Ok(()));
    }

    // ------------------------------------------------- the capability of a
    // processor

    #[test]
    fn a_processor_says_what_it_can_read_and_write() {
        let processor = Processor {
            name: "image".to_string(),
            reads: vec!["png".to_string(), "jpeg".to_string()],
            writes: vec!["png".to_string()],
        };
        assert!(processor.can_read("png"));
        assert!(processor.can_read("jpeg"));
        assert!(!processor.can_read("webp"));
        assert!(processor.can_write("png"));
        assert!(!processor.can_write("jpeg"));
    }

    #[test]
    fn a_format_the_processor_cannot_read_is_not_offered() {
        let folder = TempFolder::new("cannot-read");
        let selection = Selection::inspect(&[folder.file("photo.webp")]);
        let mut entry = operation("test");
        entry.requires = Requirement::Processor("image");
        // The image processor reads png and jpeg, but not webp.
        assert_eq!(
            eligibility(&entry, &selection, &everything()),
            Err(Ineligible::UnreadableFormat("webp".to_string()))
        );
    }

    #[test]
    fn a_format_the_processor_cannot_write_is_not_offered() {
        let folder = TempFolder::new("cannot-write");
        let selection = Selection::inspect(&[folder.file("photo.png")]);
        let mut entry = operation("test");
        entry.requires = Requirement::Processor("image");
        entry.output = Output::Beside("webp");
        // The image processor writes png, but not webp.
        assert_eq!(
            eligibility(&entry, &selection, &everything()),
            Err(Ineligible::UnwritableFormat("webp".to_string()))
        );
    }

    #[test]
    fn a_result_that_keeps_the_format_needs_a_processor_that_can_write_it() {
        let folder = TempFolder::new("keeps-format");
        // The image processor reads a gif but cannot write one.
        let selection = Selection::inspect(&[folder.file("photo.gif")]);
        let mut entry = operation("test");
        entry.requires = Requirement::Processor("image");
        entry.output = Output::BesideAsInput;
        assert_eq!(
            eligibility(&entry, &selection, &everything()),
            Err(Ineligible::UnwritableFormat("gif".to_string()))
        );
    }

    #[test]
    fn a_processor_that_reads_and_writes_the_format_is_offered() {
        let folder = TempFolder::new("can-do-it");
        let selection = Selection::inspect(&[folder.file("photo.png")]);
        let mut entry = operation("test");
        entry.requires = Requirement::Processor("image");
        entry.output = Output::Beside("png");
        assert_eq!(eligibility(&entry, &selection, &everything()), Ok(()));
    }

    // ---------------------------------------------------- the shape of the
    // selection

    #[test]
    fn an_operation_that_needs_alike_items_is_not_offered_a_mixed_selection() {
        let folder = TempFolder::new("alike");
        let selection = Selection::inspect(&[folder.file("one.png"), folder.file("two.pdf")]);
        let mut entry = operation("test");
        entry.batch = Batch::SeveralAlike;
        // The kinds are not alike, so the operation is not offered.
        assert_eq!(
            eligibility(&entry, &selection, &everything()),
            Err(Ineligible::MixedKinds)
        );
    }

    #[test]
    fn an_operation_that_needs_alike_items_is_offered_alike_items() {
        let folder = TempFolder::new("alike-ok");
        let selection = Selection::inspect(&[folder.file("one.png"), folder.file("two.png")]);
        let mut entry = operation("test");
        entry.batch = Batch::SeveralAlike;
        assert_eq!(eligibility(&entry, &selection, &everything()), Ok(()));
    }

    // ------------------------------------------- the actions without a backend

    #[test]
    fn an_action_the_program_does_itself_needs_no_processor() {
        let folder = TempFolder::new("universal");
        let selection = Selection::inspect(&[folder.file("photo.png")]);
        let mut entry = operation("test");
        entry.requires = Requirement::None;
        // There is no processor at all, and the action is still offered.
        assert_eq!(
            eligibility(&entry, &selection, &Environment::none()),
            Ok(())
        );
    }

    #[test]
    fn the_actions_of_the_registry_that_need_no_backend_are_the_file_ones() {
        for entry in OPERATIONS {
            let needs_no_backend = matches!(entry.requires, Requirement::None);
            if entry.mode == RadialMode::Actions {
                assert!(
                    needs_no_backend || entry.id == "actions.zip",
                    "{} is a file action and should need no conversion backend",
                    entry.id
                );
            }
        }
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
                "convert.gif",
                "convert.tiff",
                "convert.bmp",
                "convert.ico",
                "convert.pdf-png",
                "convert.pdf-jpeg",
                "convert.pdf-txt",
                "convert.docx-txt",
                "convert.csv",
                "convert.docx-pdf",
                "convert.xlsx-pdf",
                "convert.pptx-pdf",
                "convert.mp4",
                "convert.mp3",
                "convert.wav",
                "convert.ogg",
                "convert.opus",
                "convert.aiff",
                "convert.m4a",
                "convert.wma",
                "convert.srt",
                "convert.vtt",
                "convert.txt",
                "convert.zip",
                "convert.tar",
                "convert.gz",
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
    fn trim_is_eligible_for_audio_and_video() {
        let folder = TempFolder::new("trim-kinds");
        let trim_op = operation_by_id("tools.trim").expect("tools.trim exists");
        let video_sel = Selection::inspect(&[folder.file("clip.mp4")]);
        let audio_sel = Selection::inspect(&[folder.file("track.mp3")]);
        let env = Environment {
            processors: vec![
                Processor {
                    name: "video".to_string(),
                    reads: vec!["mp4".to_string()],
                    writes: vec!["mp4".to_string()],
                },
                Processor {
                    name: "audio".to_string(),
                    reads: vec!["mp3".to_string()],
                    writes: vec!["mp3".to_string()],
                },
            ],
        };
        assert_eq!(eligibility(trim_op, &video_sel, &env), Ok(()));
        assert_eq!(eligibility(trim_op, &audio_sel, &env), Ok(()));
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
                    Requirement::Processor(name) => Some(Processor {
                        name: name.to_string(),
                        reads: Vec::new(),
                        writes: Vec::new(),
                    }),
                    Requirement::AnyProcessor(names) => {
                        let mut first = None;
                        for name in names {
                            if first.is_none() {
                                first = Some(Processor {
                                    name: name.to_string(),
                                    reads: Vec::new(),
                                    writes: Vec::new(),
                                });
                            }
                        }
                        first
                    }
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
