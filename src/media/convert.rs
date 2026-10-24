//! Converting one file into another.
//!
//! The original is never touched. The result is made in the folder the
//! original is in, under a name of its own, and is only given its final
//! name once it has been made and looked at. If anything goes wrong, the
//! file that was being made is removed and the original is left exactly as
//! it was.

use std::fs::{File, OpenOptions};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use crate::media::animation::{self, Verdict};
use crate::media::command::convert as convert_arguments;
use crate::media::locator::Tools;
use crate::media::manifest::{FormatSupport, Manifest};
use crate::media::publish::{self, PublishError, Publisher};
use crate::media::runner::ProcessRunner;

/// Why a conversion did not happen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConvertError {
    /// The format of the file is not one that can be read.
    UnknownFormat,
    /// The file is already in the format it was to be put into.
    AlreadyThatFormat,
    /// The file holds more than one picture.
    Animation,
    /// It could not be told whether the file holds more than one picture.
    ///
    /// The file is not converted, because it might be an animation and
    /// nothing would say so.
    Uncertain,
    /// The program did not do the conversion.
    ProgramFailed,
    /// What came out is not what it should be.
    BadResult,
    /// The result could not be put at its name.
    CouldNotPublish,
}

impl ConvertError {
    /// The name used in the log.
    ///
    /// The name never carries a path: the log says what happened, not which
    /// file it happened to.
    pub fn name(&self) -> &'static str {
        match self {
            ConvertError::UnknownFormat => "the format of the file is not known",
            ConvertError::AlreadyThatFormat => "the file is already in that format",
            ConvertError::Animation => "the file holds more than one picture",
            ConvertError::Uncertain => "the file could not be read as a single picture",
            ConvertError::ProgramFailed => "the conversion did not finish",
            ConvertError::BadResult => "the result was not what it should be",
            ConvertError::CouldNotPublish => "the result could not be put at its name",
        }
    }
}

/// A counter that keeps the temporary files apart.
static COUNTER: AtomicU32 = AtomicU32::new(0);

/// Does the conversions, with the tools it is given.
pub struct Converter<'a> {
    /// The programs that do the work.
    pub tools: &'a Tools,
    /// The runner that starts them.
    pub runner: &'a dyn ProcessRunner,
    /// The publisher that puts the result at its name.
    pub publisher: &'a dyn Publisher,
}

impl<'a> Converter<'a> {
    /// Converts one file.
    ///
    /// The result is written beside the original, under a name of its own,
    /// and the original is left alone.
    pub fn convert(&self, input: &Path, target: &FormatSupport) -> Result<PathBuf, ConvertError> {
        let source = self.source_of(input).ok_or(ConvertError::UnknownFormat)?;

        // A file that is already in that format is not converted into a
        // copy of itself beside itself.
        if source.format == target.format {
            return Err(ConvertError::AlreadyThatFormat);
        }

        // A file that holds more than one picture is not converted, and a
        // file that cannot be told is not either.
        let verdict = animation::judge(self.runner, &self.tools.ffprobe, source, input);
        if !verdict.may_convert() {
            return Err(match verdict {
                Verdict::Animated => ConvertError::Animation,
                _ => ConvertError::Uncertain,
            });
        }

        let folder = input.parent().unwrap_or_else(|| Path::new("."));
        let temporary = self.temporary_file(folder, input);

        let result = self.run(input, source, target, &temporary);
        if let Err(reason) = result {
            let _ = std::fs::remove_file(&temporary);
            return Err(reason);
        }

        // If the target is ICO, wrap the image in an ICO container.
        if target.format == "ico" {
            let image_bytes = std::fs::read(&temporary).map_err(|_| ConvertError::BadResult)?;
            let ico_bytes = crate::media::ico::wrap(&image_bytes, false)
                .ok_or(ConvertError::BadResult)?;
            std::fs::write(&temporary, ico_bytes).map_err(|_| ConvertError::BadResult)?;
        }

        // What came out has to be what was asked for, or it is not the
        // result of anything.
        if !self.is_what_it_should_be(&temporary, target) {
            let _ = std::fs::remove_file(&temporary);
            return Err(ConvertError::BadResult);
        }

        // The result is given its name. If that name is taken, another one
        // is tried; the file that was made is not made again.
        for name in publish::names_for(input, &target.format) {
            match self.publisher.publish(&temporary, &name) {
                Ok(()) => return Ok(name),
                Err(PublishError::DestinationExists) => continue,
                Err(PublishError::CouldNotMove) => {
                    let _ = std::fs::remove_file(&temporary);
                    return Err(ConvertError::CouldNotPublish);
                }
            }
        }
        let _ = std::fs::remove_file(&temporary);
        Err(ConvertError::CouldNotPublish)
    }

    /// The format of a file, from the name it is called.
    fn source_of(&self, input: &Path) -> Option<&'static FormatSupport> {
        let extension = input.extension()?.to_string_lossy().to_lowercase();
        Manifest::get().format_of_extension(&extension)
    }

    /// The name the result is made under.
    ///
    /// It is in the folder the original is in, so that the move that
    /// follows is a move and not a copy, and it cannot be the name of
    /// anything the user has.
    fn temporary_file(&self, folder: &Path, input: &Path) -> PathBuf {
        let stem = input
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_else(|| "converted".to_string());
        let number = COUNTER.fetch_add(1, Ordering::Relaxed);
        folder.join(format!(".{stem}.{}.{number}.part", std::process::id()))
    }

    /// Runs the conversion, with the result going into the temporary file.
    fn run(
        &self,
        input: &Path,
        source: &FormatSupport,
        target: &FormatSupport,
        temporary: &Path,
    ) -> Result<(), ConvertError> {
        // Some muxers (mov, matroska, avi, asf) need a seekable output file
        // and cannot write to a pipe. For those, pass the temporary file
        // path as the output instead of writing to stdout.
        let needs_file = crate::media::command::muxer_needs_file(&target.write.muxer);

        let arguments = if needs_file {
            convert_arguments(input, source, target, Some(temporary), None)
        } else {
            let file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(temporary)
                .map_err(|_| ConvertError::CouldNotPublish)?;
            let args = convert_arguments(input, source, target, None, None);
            let result = self
                .runner
                .run(&self.tools.ffmpeg, &args, Some(&file))
                .map_err(|_| ConvertError::ProgramFailed)?;
            if !result.succeeded() {
                return Err(ConvertError::ProgramFailed);
            }
            return Ok(());
        };

        let result = self
            .runner
            .run(&self.tools.ffmpeg, &arguments, None)
            .map_err(|_| ConvertError::ProgramFailed)?;

        if !result.succeeded() {
            return Err(ConvertError::ProgramFailed);
        }
        Ok(())
    }

    /// Whether the file that was made is what it should be.
    fn is_what_it_should_be(&self, temporary: &Path, target: &FormatSupport) -> bool {
        let mut beginning = [0u8; 32];
        let mut file = match File::open(temporary) {
            Ok(file) => file,
            Err(_) => return false,
        };
        let read = match file.read(&mut beginning) {
            Ok(read) => read,
            Err(_) => return false,
        };
        if read == 0 {
            return false;
        }
        target.matches(&beginning[..read])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media::publish::PublishError;
    use crate::media::runner::RunResult;
    use std::ffi::OsString;

    /// A publisher that records where things were put.
    struct Recording {
        taken: Vec<PathBuf>,
        published: std::sync::Mutex<Vec<PathBuf>>,
    }

    impl Publisher for Recording {
        fn publish(&self, temporary: &Path, destination: &Path) -> Result<(), PublishError> {
            if self.taken.iter().any(|name| name == destination) {
                return Err(PublishError::DestinationExists);
            }
            // The file is moved, not copied, so that nothing is left behind.
            std::fs::rename(temporary, destination).expect("the file could be moved");
            self.published
                .lock()
                .expect("the record could be read")
                .push(destination.to_path_buf());
            Ok(())
        }
    }

    /// A runner that writes a file of the right kind.
    struct Writing {
        bytes: Vec<u8>,
        fails: bool,
    }

    impl ProcessRunner for Writing {
        fn run(
            &self,
            _program: &Path,
            args: &[OsString],
            stdout_to: Option<&File>,
        ) -> std::io::Result<RunResult> {
            // The frames are counted with these arguments, and the file holds
            // one picture.
            if args.iter().any(|arg| arg == "-count_frames") {
                return Ok(RunResult {
                    status: Some(0),
                    stdout: b"1\n".to_vec(),
                    stderr: String::new(),
                });
            }
            if self.fails {
                return Ok(RunResult {
                    status: Some(1),
                    stdout: Vec::new(),
                    stderr: String::new(),
                });
            }
            if let Some(file) = stdout_to {
                let mut file = file;
                use std::io::Write;
                file.write_all(&self.bytes)
                    .expect("the file could be written");
            }
            Ok(RunResult {
                status: Some(0),
                stdout: Vec::new(),
                stderr: String::new(),
            })
        }
    }

    fn tools() -> Tools {
        Tools {
            ffmpeg: PathBuf::from("ffmpeg"),
            ffprobe: PathBuf::from("ffprobe"),
        }
    }

    /// A folder of its own for a test, removed when the test is over.
    struct TempFolder {
        path: PathBuf,
    }

    impl TempFolder {
        fn new(name: &str) -> TempFolder {
            let path = std::env::temp_dir().join(format!(
                "open-tangerine-convert-{}-{name}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).expect("the test folder could be made");
            TempFolder { path }
        }

        fn file(&self, name: &str) -> PathBuf {
            let path = self.path.join(name);
            std::fs::write(&path, b"picture").expect("the test file could be made");
            path
        }
    }

    impl Drop for TempFolder {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    /// A runner that says the file holds one frame.
    struct OneFrame;

    impl ProcessRunner for OneFrame {
        fn run(
            &self,
            _program: &Path,
            _args: &[OsString],
            _stdout_to: Option<&File>,
        ) -> std::io::Result<RunResult> {
            Ok(RunResult {
                status: Some(0),
                stdout: b"1\n".to_vec(),
                stderr: String::new(),
            })
        }
    }

    #[test]
    fn a_file_that_is_already_in_that_format_is_not_converted() {
        let folder = TempFolder::new("same");
        let path = folder.file("photo.png");
        let manifest = Manifest::get();
        let target = manifest.format("png").expect("png is in the manifest");
        let tools = tools();
        let publisher = Recording {
            taken: Vec::new(),
            published: std::sync::Mutex::new(Vec::new()),
        };
        let converter = Converter {
            tools: &tools,
            runner: &OneFrame,
            publisher: &publisher,
        };
        assert_eq!(
            converter.convert(&path, target),
            Err(ConvertError::AlreadyThatFormat)
        );
    }

    #[test]
    fn the_reason_for_a_file_that_is_already_in_that_format_is_clear() {
        // The policy is that a file is not converted into the format it is
        // already in. The reason for it is said in words, so that the person
        // who asked for it is told what happened.
        assert_eq!(
            ConvertError::AlreadyThatFormat.name(),
            "the file is already in that format"
        );
    }

    #[test]
    fn a_result_that_is_not_what_it_should_be_is_removed() {
        let folder = TempFolder::new("bad-result");
        let path = folder.file("photo.png");
        let manifest = Manifest::get();
        let target = manifest.format("jpeg").expect("jpeg is in the manifest");
        let tools = tools();
        let publisher = Recording {
            taken: Vec::new(),
            published: std::sync::Mutex::new(Vec::new()),
        };
        // The program says it worked but wrote something that is not a
        // picture.
        let runner = Writing {
            bytes: b"not a picture at all".to_vec(),
            fails: false,
        };
        let converter = Converter {
            tools: &tools,
            runner: &runner,
            publisher: &publisher,
        };
        assert_eq!(
            converter.convert(&path, target),
            Err(ConvertError::BadResult)
        );
        // Nothing was left behind.
        let left: Vec<_> = std::fs::read_dir(&folder.path)
            .expect("the folder is there")
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.file_name().to_string_lossy().ends_with(".part"))
            .collect();
        assert!(left.is_empty(), "a temporary file was left behind");
    }

    #[test]
    fn a_result_is_put_at_a_name_that_is_free() {
        let folder = TempFolder::new("free-name");
        let path = folder.file("photo.png");
        let manifest = Manifest::get();
        let target = manifest.format("jpeg").expect("jpeg is in the manifest");
        let tools = tools();
        let publisher = Recording {
            taken: Vec::new(),
            published: std::sync::Mutex::new(Vec::new()),
        };
        let runner = Writing {
            bytes: vec![0xFF, 0xD8, 0xFF, 0xE0, 0, 0],
            fails: false,
        };
        let converter = Converter {
            tools: &tools,
            runner: &runner,
            publisher: &publisher,
        };
        let name = converter
            .convert(&path, target)
            .expect("the file was converted");
        assert_eq!(name, folder.path.join("photo.jpeg"));
        assert!(name.is_file());
    }

    #[test]
    fn a_name_that_is_taken_is_given_a_number() {
        let folder = TempFolder::new("taken-name");
        let path = folder.file("photo.png");
        std::fs::write(folder.path.join("photo.jpeg"), b"the old one")
            .expect("the file could be made");
        let manifest = Manifest::get();
        let target = manifest.format("jpeg").expect("jpeg is in the manifest");
        let tools = tools();
        let publisher = Recording {
            taken: vec![folder.path.join("photo.jpeg")],
            published: std::sync::Mutex::new(Vec::new()),
        };
        let runner = Writing {
            bytes: vec![0xFF, 0xD8, 0xFF, 0xE0, 0, 0],
            fails: false,
        };
        let converter = Converter {
            tools: &tools,
            runner: &runner,
            publisher: &publisher,
        };
        let name = converter
            .convert(&path, target)
            .expect("the file was converted");
        assert_eq!(name, folder.path.join("photo (2).jpeg"));
        // The file that was there is untouched.
        assert_eq!(
            std::fs::read(folder.path.join("photo.jpeg")).expect("the old file is there"),
            b"the old one"
        );
    }

    #[test]
    fn a_conversion_that_fails_leaves_nothing_behind() {
        let folder = TempFolder::new("fails");
        let path = folder.file("photo.png");
        let manifest = Manifest::get();
        let target = manifest.format("jpeg").expect("jpeg is in the manifest");
        let tools = tools();
        let publisher = Recording {
            taken: Vec::new(),
            published: std::sync::Mutex::new(Vec::new()),
        };
        let runner = Writing {
            bytes: Vec::new(),
            fails: true,
        };
        let converter = Converter {
            tools: &tools,
            runner: &runner,
            publisher: &publisher,
        };
        assert_eq!(
            converter.convert(&path, target),
            Err(ConvertError::ProgramFailed)
        );
        let left: Vec<_> = std::fs::read_dir(&folder.path)
            .expect("the folder is there")
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.file_name().to_string_lossy().ends_with(".part"))
            .collect();
        assert!(left.is_empty(), "a temporary file was left behind");
        assert!(
            publisher
                .published
                .lock()
                .expect("the record could be read")
                .is_empty()
        );
    }
}
