//! The selection: what the user has dragged, and what kind of thing it is.
//!
//! The paths come from the selection subsystem, which reads them from the
//! shell. This module only looks at them: it says what kind of thing each
//! one is, and what kind of selection they make together.
//!
//! Two rules shape it.
//!
//! The look is **cheap**: the attributes of each item are read, and nothing
//! else. No file is opened, no folder is looked inside, and no processor is
//! run. A selection of a thousand files costs a thousand attribute queries and
//! nothing else.
//!
//! And the look is **honest**: an item that cannot be read stays in the
//! selection and is reported as unreadable. It is not quietly left out, and
//! the rest of the selection is never treated as if it were the whole of it.
//!
//! A file category is not a capability. Knowing that a file is called
//! `photo.png` says what it is *called*, not what it is. Whether an operation
//! can actually run on it is a different question, and the registry answers
//! that one.

use std::path::{Path, PathBuf};

/// What kind of thing a selected item is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FileKind {
    /// A picture.
    Image,
    /// A film.
    Video,
    /// A sound.
    Audio,
    /// A document that is always a document.
    Pdf,
    /// A document.
    Document,
    /// A table.
    Spreadsheet,
    /// A slide deck.
    Presentation,
    /// A bundle of files.
    Archive,
    /// Plain text.
    Text,
    /// A folder.
    Folder,
    /// Something this program does not know about.
    Unknown,
}

impl FileKind {
    /// Every kind of file there is, folders apart.
    ///
    /// An operation that works on anything says so with this list.
    pub const FILES: [FileKind; 10] = [
        FileKind::Image,
        FileKind::Video,
        FileKind::Audio,
        FileKind::Pdf,
        FileKind::Document,
        FileKind::Spreadsheet,
        FileKind::Presentation,
        FileKind::Archive,
        FileKind::Text,
        FileKind::Unknown,
    ];

    /// The kind a file with this extension is.
    ///
    /// The extension says what a file is *called*, not what it holds, and
    /// this is only ever a good enough answer for a menu. A file called
    /// `.png` may hold anything at all, and a file with no extension may hold
    /// a picture. Whether an operation can really run on the file is a
    /// separate question, and the registry answers that one.
    pub fn from_extension(extension: &str) -> FileKind {
        match extension.to_ascii_lowercase().as_str() {
            "jpg" | "jpeg" | "jpe" | "jfif" | "png" | "gif" | "bmp" | "dib" | "tif" | "tiff"
            | "webp" | "heic" | "heif" | "avif" | "ico" | "tga" | "psd" | "raw" | "cr2" | "nef"
            | "arw" | "dng" | "svg" => FileKind::Image,
            "mp4" | "m4v" | "mov" | "qt" | "avi" | "mkv" | "webm" | "wmv" | "flv" | "mpg"
            | "mpeg" | "3gp" | "ts" => FileKind::Video,
            "mp3" | "wav" | "flac" | "aac" | "m4a" | "ogg" | "oga" | "opus" | "wma" | "aiff"
            | "mid" => FileKind::Audio,
            "pdf" => FileKind::Pdf,
            "doc" | "docx" | "odt" | "rtf" | "pages" | "epub" | "djvu" | "srt" | "vtt" => FileKind::Document,
            "xls" | "xlsx" | "xlsm" | "ods" | "csv" | "tsv" | "numbers" => FileKind::Spreadsheet,
            "ppt" | "pptx" | "odp" | "key" => FileKind::Presentation,
            "zip" | "rar" | "7z" | "tar" | "gz" | "bz2" | "xz" | "zst" | "tgz" | "cab" => {
                FileKind::Archive
            }
            "txt" | "log" | "md" | "markdown" | "json" | "xml" | "yaml" | "yml" | "ini"
            | "toml" | "html" | "htm" | "css" => FileKind::Text,
            _ => FileKind::Unknown,
        }
    }

    /// The name used in the log and in the tests.
    pub fn name(self) -> &'static str {
        match self {
            FileKind::Image => "image",
            FileKind::Video => "video",
            FileKind::Audio => "audio",
            FileKind::Pdf => "pdf",
            FileKind::Document => "document",
            FileKind::Spreadsheet => "spreadsheet",
            FileKind::Presentation => "presentation",
            FileKind::Archive => "archive",
            FileKind::Text => "text",
            FileKind::Folder => "folder",
            FileKind::Unknown => "unknown",
        }
    }
}

/// One item of the selection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SelectedItem {
    /// Where the item is.
    ///
    /// The path comes from the shell. A shell item with no path of its own
    /// never reaches this point, and no path is ever made up here.
    pub path: PathBuf,
    /// What kind of thing the item is.
    pub kind: FileKind,
    /// The format the item is written in, if it can be told.
    ///
    /// This is the name the format is usually written with, in lower case and
    /// without the dot: `png`, `jpeg`. It is worked out from the name of the
    /// file, which says what the file is *called* and not what it holds, so
    /// it is a guess in the same way the kind is. A file with no name that
    /// says anything has no format.
    pub format: Option<String>,
    /// Whether the item could be read at all.
    ///
    /// An item that is gone, or that is not there to be read, is still part
    /// of the selection. It is not quietly left out.
    pub readable: bool,
}

impl SelectedItem {
    /// Looks at a path and says what it is.
    ///
    /// The attributes of the item itself are read, without following
    /// anything. A cloud placeholder is then read without being fetched, which
    /// matters: fetching one can take seconds, and the wheel must not wait
    /// for that.
    pub fn inspect(path: &Path) -> SelectedItem {
        match std::fs::symlink_metadata(path) {
            Ok(metadata) => {
                let file_type = metadata.file_type();
                if file_type.is_symlink() {
                    // A link is looked at as the thing it points at.
                    return SelectedItem::follow(path);
                }
                if file_type.is_dir() {
                    return SelectedItem {
                        path: path.to_path_buf(),
                        kind: FileKind::Folder,
                        format: None,
                        readable: true,
                    };
                }
                SelectedItem {
                    path: path.to_path_buf(),
                    kind: kind_of(path),
                    format: format_of(path),
                    readable: true,
                }
            }
            Err(_) => SelectedItem {
                path: path.to_path_buf(),
                kind: FileKind::Unknown,
                format: None,
                readable: false,
            },
        }
    }

    /// Looks at what a link points at.
    fn follow(path: &Path) -> SelectedItem {
        match std::fs::metadata(path) {
            Ok(metadata) if metadata.is_dir() => SelectedItem {
                path: path.to_path_buf(),
                kind: FileKind::Folder,
                format: None,
                readable: true,
            },
            Ok(_) => SelectedItem {
                path: path.to_path_buf(),
                kind: kind_of(path),
                format: format_of(path),
                readable: true,
            },
            Err(_) => SelectedItem {
                path: path.to_path_buf(),
                kind: FileKind::Unknown,
                format: None,
                readable: false,
            },
        }
    }
}

/// The kind a path is called, from its extension.
fn kind_of(path: &Path) -> FileKind {
    match path.extension() {
        Some(extension) => FileKind::from_extension(&extension.to_string_lossy()),
        // A file with no extension is not known.
        None => FileKind::Unknown,
    }
}

/// The format a path is called, from its extension.
///
/// The name is in lower case and without the dot, the way the registry and
/// the processors name their formats.
fn format_of(path: &Path) -> Option<String> {
    let extension = path.extension()?;
    let name = extension.to_string_lossy().to_lowercase();
    if name.is_empty() { None } else { Some(name) }
}

/// The shape of the selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectionShape {
    /// Nothing is selected.
    Empty,
    /// One file.
    SingleFile,
    /// Several files of the same kind.
    SeveralFiles,
    /// Several files of different kinds.
    MixedFiles,
    /// One folder.
    SingleFolder,
    /// Several folders.
    SeveralFolders,
    /// Files and folders together.
    MixedFilesAndFolders,
}

impl SelectionShape {
    /// The name used in the log.
    pub fn name(self) -> &'static str {
        match self {
            SelectionShape::Empty => "nothing",
            SelectionShape::SingleFile => "one file",
            SelectionShape::SeveralFiles => "several files",
            SelectionShape::MixedFiles => "files of different kinds",
            SelectionShape::SingleFolder => "one folder",
            SelectionShape::SeveralFolders => "several folders",
            SelectionShape::MixedFilesAndFolders => "files and folders",
        }
    }
}

/// The selection: everything the user dragged.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Selection {
    items: Vec<SelectedItem>,
}

impl Selection {
    /// Looks at a list of paths and says what the selection is.
    pub fn inspect(paths: &[PathBuf]) -> Selection {
        Selection {
            items: paths
                .iter()
                .map(|path| SelectedItem::inspect(path))
                .collect(),
        }
    }

    /// The items of the selection.
    pub fn items(&self) -> &[SelectedItem] {
        &self.items
    }

    /// Where the items are.
    pub fn paths(&self) -> Vec<PathBuf> {
        self.items.iter().map(|item| item.path.clone()).collect()
    }

    /// The formats of the items, for the ones whose format is known.
    pub fn formats(&self) -> Vec<&str> {
        self.items
            .iter()
            .filter_map(|item| item.format.as_deref())
            .collect()
    }

    /// How many items there are.
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Whether there is nothing in the selection.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// How many of the items could not be read.
    pub fn unreadable(&self) -> usize {
        self.items.iter().filter(|item| !item.readable).count()
    }

    /// The kinds of thing in the selection, in the order they first appear.
    pub fn categories(&self) -> Vec<FileKind> {
        let mut kinds = Vec::new();
        for item in &self.items {
            if !kinds.contains(&item.kind) {
                kinds.push(item.kind);
            }
        }
        kinds
    }

    /// Whether the selection holds more than one kind of thing.
    pub fn is_mixed(&self) -> bool {
        self.categories().len() > 1
    }

    /// What shape the selection has.
    pub fn shape(&self) -> SelectionShape {
        match self.items.len() {
            0 => SelectionShape::Empty,
            1 => {
                if self.items[0].kind == FileKind::Folder {
                    SelectionShape::SingleFolder
                } else {
                    SelectionShape::SingleFile
                }
            }
            _ => {
                let folders = self
                    .items
                    .iter()
                    .filter(|item| item.kind == FileKind::Folder)
                    .count();
                if folders == 0 {
                    if self.is_mixed() {
                        SelectionShape::MixedFiles
                    } else {
                        SelectionShape::SeveralFiles
                    }
                } else if folders == self.items.len() {
                    SelectionShape::SeveralFolders
                } else {
                    SelectionShape::MixedFilesAndFolders
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
                "open-tangerine-selection-{}-{}-{}",
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

    // ------------------------------------------------------- the categories

    #[test]
    fn the_known_extensions_are_recognised() {
        assert_eq!(FileKind::from_extension("png"), FileKind::Image);
        assert_eq!(FileKind::from_extension("svg"), FileKind::Image);
        assert_eq!(FileKind::from_extension("mp4"), FileKind::Video);
        assert_eq!(FileKind::from_extension("mp3"), FileKind::Audio);
        assert_eq!(FileKind::from_extension("pdf"), FileKind::Pdf);
        assert_eq!(FileKind::from_extension("docx"), FileKind::Document);
        assert_eq!(FileKind::from_extension("xlsx"), FileKind::Spreadsheet);
        assert_eq!(FileKind::from_extension("pptx"), FileKind::Presentation);
        assert_eq!(FileKind::from_extension("zip"), FileKind::Archive);
        assert_eq!(FileKind::from_extension("txt"), FileKind::Text);
    }

    #[test]
    fn an_unknown_extension_is_unknown() {
        assert_eq!(FileKind::from_extension("qqq"), FileKind::Unknown);
        assert_eq!(FileKind::from_extension(""), FileKind::Unknown);
    }

    #[test]
    fn the_extension_is_matched_however_it_is_written() {
        assert_eq!(FileKind::from_extension("PNG"), FileKind::Image);
        assert_eq!(FileKind::from_extension("JpG"), FileKind::Image);
        assert_eq!(FileKind::from_extension("PDF"), FileKind::Pdf);
    }

    #[test]
    fn a_file_with_no_extension_is_unknown() {
        let folder = TempFolder::new("no-extension");
        let path = folder.file("README");
        let item = SelectedItem::inspect(&path);
        assert_eq!(item.kind, FileKind::Unknown);
        assert!(item.readable);
        // And there is no format to go on either.
        assert_eq!(item.format, None);
    }

    #[test]
    fn the_format_is_the_name_the_file_is_called_by() {
        let folder = TempFolder::new("format");
        let path = folder.file("photo.PNG");
        let item = SelectedItem::inspect(&path);
        // The format is in lower case and without the dot, the way the
        // registry and the processors name their formats.
        assert_eq!(item.format.as_deref(), Some("png"));
        assert_eq!(item.kind, FileKind::Image);
    }

    #[test]
    fn a_folder_has_no_format() {
        let folder = TempFolder::new("folder-format");
        let path = folder.folder("pictures");
        let item = SelectedItem::inspect(&path);
        assert_eq!(item.kind, FileKind::Folder);
        assert_eq!(item.format, None);
    }

    #[test]
    fn the_selection_says_what_formats_it_holds() {
        let folder = TempFolder::new("formats");
        let paths = vec![
            folder.file("one.png"),
            folder.file("two.jpeg"),
            folder.file("three.png"),
        ];
        let selection = Selection::inspect(&paths);
        let mut formats = selection.formats();
        formats.sort_unstable();
        assert_eq!(formats, vec!["jpeg", "png", "png"]);
        assert_eq!(selection.paths(), paths);
    }

    // ----------------------------------------------------- the items themselves

    #[test]
    fn a_file_is_known_by_its_extension() {
        let folder = TempFolder::new("file");
        let path = folder.file("photo.png");
        let item = SelectedItem::inspect(&path);
        assert_eq!(item.kind, FileKind::Image);
        assert!(item.readable);
        assert_eq!(item.path, path);
    }

    #[test]
    fn a_folder_is_a_folder() {
        let folder = TempFolder::new("folder");
        let path = folder.folder("pictures");
        let item = SelectedItem::inspect(&path);
        assert_eq!(item.kind, FileKind::Folder);
        assert!(item.readable);
    }

    #[test]
    fn a_path_that_is_not_there_cannot_be_read() {
        let folder = TempFolder::new("gone");
        let path = folder.path.join("not-here.png");
        let item = SelectedItem::inspect(&path);
        assert!(!item.readable);
        assert_eq!(item.kind, FileKind::Unknown);
    }

    // --------------------------------------------------------- the selection

    #[test]
    fn an_empty_selection_is_empty() {
        let selection = Selection::inspect(&[]);
        assert!(selection.is_empty());
        assert_eq!(selection.len(), 0);
        assert_eq!(selection.shape(), SelectionShape::Empty);
        assert_eq!(selection.unreadable(), 0);
        assert!(selection.categories().is_empty());
        assert!(!selection.is_mixed());
    }

    #[test]
    fn one_file_is_one_file() {
        let folder = TempFolder::new("one-file");
        let path = folder.file("photo.png");
        let selection = Selection::inspect(&[path]);
        assert_eq!(selection.len(), 1);
        assert_eq!(selection.shape(), SelectionShape::SingleFile);
        assert_eq!(selection.categories(), vec![FileKind::Image]);
        assert!(!selection.is_mixed());
    }

    #[test]
    fn one_folder_is_one_folder() {
        let folder = TempFolder::new("one-folder");
        let path = folder.folder("pictures");
        let selection = Selection::inspect(&[path]);
        assert_eq!(selection.shape(), SelectionShape::SingleFolder);
    }

    #[test]
    fn several_files_of_one_kind_are_several_files() {
        let folder = TempFolder::new("several");
        let paths = vec![folder.file("one.png"), folder.file("two.png")];
        let selection = Selection::inspect(&paths);
        assert_eq!(selection.shape(), SelectionShape::SeveralFiles);
        assert_eq!(selection.categories(), vec![FileKind::Image]);
        assert!(!selection.is_mixed());
    }

    #[test]
    fn files_of_different_kinds_are_mixed() {
        let folder = TempFolder::new("mixed");
        let paths = vec![folder.file("one.png"), folder.file("two.pdf")];
        let selection = Selection::inspect(&paths);
        assert_eq!(selection.shape(), SelectionShape::MixedFiles);
        assert!(selection.is_mixed());
        assert_eq!(selection.categories(), vec![FileKind::Image, FileKind::Pdf]);
    }

    #[test]
    fn several_folders_are_several_folders() {
        let folder = TempFolder::new("folders");
        let paths = vec![folder.folder("one"), folder.folder("two")];
        let selection = Selection::inspect(&paths);
        assert_eq!(selection.shape(), SelectionShape::SeveralFolders);
    }

    #[test]
    fn files_and_folders_together_are_mixed() {
        let folder = TempFolder::new("both");
        let paths = vec![folder.file("one.png"), folder.folder("pictures")];
        let selection = Selection::inspect(&paths);
        assert_eq!(selection.shape(), SelectionShape::MixedFilesAndFolders);
    }

    #[test]
    fn an_item_that_cannot_be_read_stays_in_the_selection() {
        let folder = TempFolder::new("unreadable");
        let good = folder.file("one.png");
        let missing = folder.path.join("not-here.png");
        let selection = Selection::inspect(&[good.clone(), missing]);
        // The item is not quietly left out.
        assert_eq!(selection.len(), 2);
        assert_eq!(selection.unreadable(), 1);
        // And the rest of the selection is not treated as the whole of it.
        assert_eq!(selection.shape(), SelectionShape::MixedFiles);
    }

    #[test]
    fn the_extension_says_nothing_about_whether_a_processor_is_there() {
        // A file that is called a picture is a picture as far as the
        // classification goes. Whether anything can be done with it is
        // another question entirely, and this module does not answer it.
        let folder = TempFolder::new("not-a-proof");
        let path = folder.file("pretend.png");
        let item = SelectedItem::inspect(&path);
        assert_eq!(item.kind, FileKind::Image);
        // The item carries nothing that says a processor is there.
        assert!(item.readable);
    }
}
