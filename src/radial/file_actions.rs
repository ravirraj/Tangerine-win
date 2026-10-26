//! File actions: copy path, duplicate, rename, reveal in Explorer.
//!
//! These are handled entirely by the application with Win32 APIs, no
//! conversion backend needed.
//!
//! The work happens on a worker, never on the thread that draws.

use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::diagnostics::{self, Level};
use crate::media::publish::Publisher;
use crate::radial::execution::{Failure, Handler, Outcome, Request};
use crate::radial::registry::operation_by_id;
use crate::radial::selection::Selection;

/// The handler for file actions.
pub struct FileActions<P: Publisher> {
    /// Puts a finished file at its name (for duplicate).
    pub publisher: P,
}

impl<P: Publisher> FileActions<P> {
    /// Runs one file action on one input file.
    fn run_one(&self, operation_id: &str, input: &Path) -> Result<PathBuf, FileActionError> {
        match operation_id {
            "actions.copy-path" => self.copy_path(input),
            "actions.duplicate" => self.duplicate(input),
            "actions.rename" => self.rename(input),
            "actions.reveal" => self.reveal(input),
            "actions.terminal" => self.terminal(input),
            "actions.extract" => self.extract(input),
            "actions.nearby-share" => self.share(input),
            _ => Err(FileActionError::NotImplemented),
        }
    }

    /// Put the selection in one archive, beside it.
    ///
    /// The other actions are one file's work said once for each file;
    /// an archive is made of all the files together, so the request is
    /// taken whole. The archive is named for the folder the files stand
    /// in when they share it, and for the first file's own name when
    /// they do not; a name already taken is refused, the way the other
    /// actions refuse to step on a finished file.
    fn zip_together(&self, inputs: &[PathBuf]) -> Result<PathBuf, FileActionError> {
        let Some(folder) = inputs[0]
            .parent()
            .map(|parent| parent.to_path_buf())
            .filter(|parent| !parent.as_os_str().is_empty())
        else {
            return Err(FileActionError::UnsupportedKind);
        };
        let shared = inputs
            .iter()
            .all(|input| input.parent() == Some(folder.as_path()));
        let name = if shared {
            folder
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
        } else {
            None
        }
        .unwrap_or_else(|| {
            inputs[0]
                .file_stem()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| "archive".to_string())
        });

        let entries = collect_entries(inputs)?;
        if entries.is_empty() {
            return Err(FileActionError::UnsupportedKind);
        }
        let target = folder.join(format!("{name}.zip"));
        if target.exists() {
            return Err(FileActionError::WriteError);
        }
        write_zip(&entries, &target)
            .map_err(|_| FileActionError::WriteError)?;
        Ok(target)
    }

    /// Copy the full path to the clipboard.
    fn copy_path(&self, input: &Path) -> Result<PathBuf, FileActionError> {
        crate::platform::windows::clipboard::set_text(input.to_string_lossy().as_ref())
            .map_err(|_| FileActionError::ToolFailed)?;
        Ok(input.to_path_buf())
    }

    /// Make a duplicate copy beside the original.
    fn duplicate(&self, input: &Path) -> Result<PathBuf, FileActionError> {
        let ext = input.extension().map(|e| e.to_string_lossy().to_string()).unwrap_or_default();
        let target = input.with_extension(format!("{}.copy{}", ext, ext));
        if target.exists() {
            return Err(FileActionError::WriteError);
        }
        fs::copy(input, &target).map_err(|_| FileActionError::WriteError)?;
        Ok(target)
    }

    /// Rename the file (requires user input for new name, handled by UI).
    fn rename(&self, _input: &Path) -> Result<PathBuf, FileActionError> {
        Err(FileActionError::NotImplemented)
    }

    /// Reveal the file in Explorer.
    fn reveal(&self, input: &Path) -> Result<PathBuf, FileActionError> {
        crate::platform::windows::shell::reveal_in_explorer(input)
            .map_err(|_| FileActionError::ToolFailed)?;
        Ok(input.to_path_buf())
    }

    /// Invoke the Windows Share contract on the file.
    fn share(&self, input: &Path) -> Result<PathBuf, FileActionError> {
        crate::platform::windows::shell::share_file(input)
            .map_err(|_| FileActionError::ToolFailed)?;
        Ok(input.to_path_buf())
    }

    /// Open a terminal at the file's folder.
    fn terminal(&self, input: &Path) -> Result<PathBuf, FileActionError> {
        let folder = if input.is_dir() {
            input.to_path_buf()
        } else {
            input.parent().unwrap_or(input).to_path_buf()
        };
        Command::new("cmd.exe")
            .current_dir(&folder)
            .spawn()
            .map_err(|_| FileActionError::ToolFailed)?;
        Ok(input.to_path_buf())
    }

    /// Extract archive contents to a folder beside the original.
    fn extract(&self, input: &Path) -> Result<PathBuf, FileActionError> {
        let ext = input.extension().map(|e| e.to_string_lossy().to_string()).unwrap_or_default();
        let target_dir = input.with_extension(format!("{}.extracted", ext));
        if target_dir.exists() {
            return Err(FileActionError::WriteError);
        }
        fs::create_dir_all(&target_dir).map_err(|_| FileActionError::WriteError)?;

        let file = fs::File::open(input).map_err(|_| FileActionError::ReadError)?;

        match ext.as_str() {
            "zip" => extract_zip(file, &target_dir)?,
            "tar" => extract_tar(file, &target_dir)?,
            "gz" => extract_gz(file, &target_dir)?,
            _ => return Err(FileActionError::UnsupportedKind),
        }

        Ok(target_dir)
    }
}

/// Why a file action did not happen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileActionError {
    NotImplemented,
    ToolFailed,
    WriteError,
    UnsupportedKind,
    ReadError,
}

impl FileActionError {
    pub fn name(&self) -> &'static str {
        match self {
            FileActionError::NotImplemented => "this action is not implemented yet",
            FileActionError::ToolFailed => "the action did not finish",
            FileActionError::WriteError => "the result could not be written",
            FileActionError::UnsupportedKind => "this action does not work on this kind of file",
            FileActionError::ReadError => "the source file could not be read",
        }
    }
}

/// Extract a ZIP archive into a directory.
fn extract_zip(file: fs::File, target_dir: &Path) -> Result<(), FileActionError> {
    let mut archive = zip::ZipArchive::new(file).map_err(|_| FileActionError::UnsupportedKind)?;
    for i in 0..archive.len() {
        let mut file = archive.by_index(i).map_err(|_| FileActionError::UnsupportedKind)?;
        if file.is_dir() {
            continue;
        }
        // Protect against Zip Slip: enclosed_name ensures path does not escape target_dir
        let Some(enclosed) = file.enclosed_name() else {
            continue;
        };
        let out_path = target_dir.join(enclosed);
        if let Some(parent) = out_path.parent() {
            fs::create_dir_all(parent).map_err(|_| FileActionError::WriteError)?;
        }
        let mut out_file = fs::File::create(&out_path).map_err(|_| FileActionError::WriteError)?;
        std::io::copy(&mut file, &mut out_file).map_err(|_| FileActionError::WriteError)?;
    }
    Ok(())
}

/// Extract a TAR archive into a directory.
fn extract_tar(file: fs::File, target_dir: &Path) -> Result<(), FileActionError> {
    let mut archive = tar::Archive::new(file);
    for entry in archive.entries().map_err(|_| FileActionError::UnsupportedKind)? {
        let mut entry = entry.map_err(|_| FileActionError::UnsupportedKind)?;
        if entry.header().entry_type().is_dir() {
            continue;
        }
        let path = entry.path().map_err(|_| FileActionError::UnsupportedKind)?;
        // Protect against Tar Slip: ensure no path components escape target_dir
        if path.components().any(|c| matches!(c, std::path::Component::ParentDir | std::path::Component::Prefix(_) | std::path::Component::RootDir)) {
            continue;
        }
        let out_path = target_dir.join(path);
        if let Some(parent) = out_path.parent() {
            fs::create_dir_all(parent).map_err(|_| FileActionError::WriteError)?;
        }
        let mut out_file = fs::File::create(&out_path).map_err(|_| FileActionError::WriteError)?;
        std::io::copy(&mut entry, &mut out_file).map_err(|_| FileActionError::WriteError)?;
    }
    Ok(())
}

/// Extract a GZ archive into a directory (single file).
fn extract_gz(file: fs::File, target_dir: &Path) -> Result<(), FileActionError> {
    let mut decoder = flate2::read::GzDecoder::new(file);
    let out_path = target_dir.join("extracted");
    let mut out_file = fs::File::create(&out_path).map_err(|_| FileActionError::WriteError)?;
    std::io::copy(&mut decoder, &mut out_file).map_err(|_| FileActionError::WriteError)?;
    Ok(())
}

/// What would go into the archive: a name said inside the archive and
/// the bytes the name stands for. Files are taken by their own name;
/// folders are taken apart, file by file, each by the road it walks
/// from the folder's own door.
fn collect_entries(inputs: &[PathBuf]) -> Result<Vec<(String, Vec<u8>)>, FileActionError> {
    let mut entries = Vec::new();
    for input in inputs {
        if input.is_dir() {
            gather_folder(input, input, &mut entries)?;
        } else {
            let name = input
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .ok_or(FileActionError::UnsupportedKind)?;
            let bytes = fs::read(input).map_err(|_| FileActionError::ReadError)?;
            entries.push((name, bytes));
        }
    }
    Ok(entries)
}

/// Reads a folder's files, said by the road they walk from the root.
fn gather_folder(
    root: &Path,
    under: &Path,
    entries: &mut Vec<(String, Vec<u8>)>,
) -> Result<(), FileActionError> {
    let mut roads = fs::read_dir(under).map_err(|_| FileActionError::ReadError)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| FileActionError::ReadError)?;
    // The archive is written the same way whoever looks in it will:
    // by name, whatever the system's own order says.
    roads.sort_by_key(|road| road.file_name());
    for road in roads {
        let road = road.path();
        let name = road
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .ok_or(FileActionError::UnsupportedKind)?;
        if road.is_dir() {
            gather_folder(root, &road, entries)?;
        } else {
            let said_road = road
                .strip_prefix(root)
                .unwrap_or(road.as_path())
                .components()
                .map(|part| part.as_os_str().to_string_lossy())
                .collect::<Vec<_>>()
                .join("/");
            let bytes = fs::read(&road).map_err(|_| FileActionError::ReadError)?;
            entries.push((said_road, bytes));
        }
    }
    Ok(())
}

/// Writes the names and the bytes they stand for into one ZIP archive.
fn write_zip(entries: &[(String, Vec<u8>)], target_path: &Path) -> Result<(), FileActionError> {
    let file = fs::File::create(target_path).map_err(|_| FileActionError::WriteError)?;
    let mut writer = zip::ZipWriter::new(file);
    for (name, bytes) in entries {
        writer
            .start_file(name.as_str(), zip::write::FileOptions::default())
            .map_err(|_| FileActionError::WriteError)?;
        writer
            .write_all(bytes)
            .map_err(|_| FileActionError::WriteError)?;
    }
    writer.finish().map_err(|_| FileActionError::WriteError)?;
    Ok(())
}


impl<P: Publisher> Handler for FileActions<P> {
    fn run(&self, request: &Request, _selection: &Selection) -> Outcome {
        let Some(operation) = operation_by_id(&request.operation) else {
            return Outcome::Failed {
                reason: Failure::UnknownOperation,
            };
        };

        let mut outputs = Vec::new();
        let mut failed = 0usize;
        let mut first_reason: Option<String> = None;
        // An archive is made of all the files the request carries: said
        // once whole, not once per file.
        if request.operation == "actions.zip" {
            if request.inputs.is_empty() {
                return Outcome::Failed {
                    reason: Failure::Error(
                        "no files to put in the archive".to_string(),
                    ),
                };
            }
            return match self.zip_together(&request.inputs) {
                Ok(target) => Outcome::Succeeded { outputs: vec![target] },
                Err(reason) => Outcome::Failed {
                    reason: Failure::Error(reason.name().to_string()),
                },
            };
        }
        for input in &request.inputs {
            match self.run_one(&request.operation, input) {
                Ok(path) => outputs.push(path),
                Err(reason) => {
                    failed += 1;
                    let name = reason.name();
                    diagnostics::log(Level::Warn, "action", &format!("a file was not processed: {name}"));
                    if first_reason.is_none() {
                        first_reason = Some(name.to_string());
                    }
                }
            }
        }

        if failed == 0 {
            Outcome::Succeeded { outputs }
        } else {
            let because = match first_reason {
                Some(reason) => format!(": {reason}"),
                None => String::new(),
            };
            Outcome::Failed {
                reason: Failure::Error(format!(
                    "{failed} of the files could not be processed{because}"
                )),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Nothing;

    impl Publisher for Nothing {
        fn publish(&self, _temporary: &Path, _destination: &Path) -> Result<(), crate::media::publish::PublishError> {
            Ok(())
        }
    }

    /// A folder of its own for a test.
    struct TempFolder {
        path: PathBuf,
    }

    impl TempFolder {
        /// Makes a folder that no other test uses.
        fn new(name: &str) -> TempFolder {
            static COUNTER: std::sync::atomic::AtomicU32 =
                std::sync::atomic::AtomicU32::new(0);
            let number =
                COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "open-tangerine-file-actions-{}-{}-{}",
                std::process::id(),
                number,
                name
            ));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).expect("the test folder was made");
            TempFolder { path }
        }

        /// Writes a file as its road is said inside the folder.
        fn write(&self, road: &str, said: &str) -> PathBuf {
            let path = self.path.join(road);
            if let Some(under) = path.parent() {
                std::fs::create_dir_all(under).expect("the files' room was made");
            }
            fs::write(&path, said).expect("the test file was written");
            path
        }

        /// The folder dropped like it was never there.
        fn try_clear(&self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    /// A request whose files are the ones that were dragged: the run
    /// checks that the two agree before it acts.
    fn zip_request(inputs: Vec<PathBuf>) -> Request {
        Request {
            operation: "actions.zip".to_string(),
            inputs,
            ..Default::default()
        }
    }

    /// What is inside an archive: names and the bytes, said plainly.
    fn in_the_archive(archive: &Path) -> Vec<(String, String)> {
        let data = fs::read(archive).expect("the archive was written");
        let mut reader = std::io::Cursor::new(data);
        let mut zip = zip::ZipArchive::new(&mut reader).expect("the archive opened");
        (0..zip.len())
            .map(|i| {
                let mut file = zip.by_index(i).expect("a file in the archive");
                let mut bytes = Vec::new();
                file.read_to_end(&mut bytes).expect("the bytes read");
                (
                    file.name().to_string(),
                    String::from_utf8(bytes).expect("letters, not noise"),
                )
            })
            .collect()
    }

    #[test]
    fn unknown_operation_is_rejected() {
        let actions = FileActions { publisher: Nothing };
        let environment = crate::radial::registry::Environment::none();
        let request = Request {
            operation: "actions.nonexistent".to_string(),
            inputs: Vec::new(),
            ..Default::default()
        };
        let outcome = crate::radial::execution::run(&request, &Selection::inspect(&[]), &environment, &actions);
        match outcome {
            Outcome::Failed { reason } => {
                assert!(matches!(reason, Failure::UnknownOperation));
            }
            other => panic!("unexpected outcome: {other:?}"),
        }
    }

    #[test]
    fn putting_files_in_an_archive_names_it_after_their_folder() {
        let actions = FileActions { publisher: Nothing };
        let folder = TempFolder::new("photos");
        let one = folder.write("photos/apple.txt", "round");
        let two = folder.write("photos/banana.txt", "long");
        let inputs = vec![one.clone(), two.clone()];
        let outcome = crate::radial::execution::run(
            &zip_request(inputs.clone()),
            &Selection::inspect(&inputs),
            &crate::radial::registry::Environment::none(),
            &actions,
        );
        let target = match outcome {
            Outcome::Succeeded { outputs } => outputs[0].clone(),
            other => panic!("unexpected outcome: {other:?}"),
        };
        assert_eq!(
            target.file_name().map(|name| name.to_string_lossy().into_owned()),
            Some("photos.zip".to_string())
        );
        // The files stand in it by their own names.
        assert_eq!(
            in_the_archive(&target),
            vec![
                ("apple.txt".to_string(), "round".to_string()),
                ("banana.txt".to_string(), "long".to_string()),
            ]
        );
        folder.try_clear();
    }

    #[test]
    fn a_folder_is_taken_file_by_file_by_its_own_roads() {
        let actions = FileActions { publisher: Nothing };
        let folder = TempFolder::new("album");
        let _ = folder.write("album/timeless/liner.txt", "quiet");
        let _ = folder.write("album/cover.txt", "painted");
        let album = folder.path.join("album");
        let outcome = crate::radial::execution::run(
            &zip_request(vec![album.clone()]),
            &Selection::inspect(&[album]),
            &crate::radial::registry::Environment::none(),
            &actions,
        );
        let target = match outcome {
            Outcome::Succeeded { outputs } => outputs[0].clone(),
            other => panic!("unexpected outcome: {other:?}"),
        };
        // The road each file walked from the folder's own door is what
        // the archive says.
        assert_eq!(
            in_the_archive(&target),
            vec![
                ("cover.txt".to_string(), "painted".to_string()),
                ("timeless/liner.txt".to_string(), "quiet".to_string()),
            ]
        );
        folder.try_clear();
    }

    #[test]
    fn files_that_share_no_room_are_named_for_the_first_one() {
        let actions = FileActions { publisher: Nothing };
        let folder = TempFolder::new("rooms");
        let one = folder.write("rooms/north/wave.txt", "a");
        let two = folder.write("rooms/south/wave.txt", "b");
        let inputs = vec![one.clone(), two.clone()];
        let outcome = crate::radial::execution::run(
            &zip_request(inputs.clone()),
            &Selection::inspect(&inputs),
            &crate::radial::registry::Environment::none(),
            &actions,
        );
        let target = match outcome {
            Outcome::Succeeded { outputs } => outputs[0].clone(),
            other => panic!("unexpected outcome: {other:?}"),
        };
        // Two rooms are not one place; the first file's own name says
        // what the archive is of.
        assert_eq!(
            target.file_name().map(|name| name.to_string_lossy().into_owned()),
            Some("wave.zip".to_string())
        );
        // And the files are in it by their own names, whatever room
        // they came from.
        assert_eq!(
            in_the_archive(&target),
            vec![("wave.txt".to_string(), "a".to_string()),
                 ("wave.txt".to_string(), "b".to_string())]
        );
        folder.try_clear();
    }

    #[test]
    fn an_archive_that_would_step_on_one_is_refused() {
        let actions = FileActions { publisher: Nothing };
        let folder = TempFolder::new("taken");
        let one = folder.write("pair.txt", "x");
        // The name the archive would take — the folder's own — stands
        // there already.
        let room = folder.path.file_name().unwrap().to_string_lossy();
        std::fs::write(folder.path.join(format!("{room}.zip")), b"already")
            .expect("the older archive was written");
        let outcome = crate::radial::execution::run(
            &zip_request(vec![one.clone()]),
            &Selection::inspect(std::slice::from_ref(&one)),
            &crate::radial::registry::Environment::none(),
            &actions,
        );
        match outcome {
            Outcome::Failed { reason } => {
                let said = format!("{reason:?}");
                assert!(said.contains("Error"), "unexpected reason: {said}");
            }
            other => panic!("unexpected outcome: {other:?}"),
        }
        folder.try_clear();
    }

    #[test]
    fn extract_zip_rejects_path_traversal_zip_slip() {
        let folder = TempFolder::new("zip_slip");
        let zip_path = folder.path.join("malicious.zip");
        {
            let file = fs::File::create(&zip_path).unwrap();
            let mut zip = zip::ZipWriter::new(file);
            let options = zip::write::FileOptions::default();
            let _ = zip.start_file("../outside.txt", options);
            let _ = zip.write_all(b"pwned");
            let _ = zip.finish();
        }
        let target_dir = folder.path.join("out");
        fs::create_dir_all(&target_dir).unwrap();
        let file = fs::File::open(&zip_path).unwrap();
        let _ = extract_zip(file, &target_dir);

        // Verify the file was NOT written outside target_dir
        assert!(!folder.path.join("outside.txt").exists(), "Zip slip successfully prevented!");
        folder.try_clear();
    }
}
