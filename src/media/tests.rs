//! The tests that start a real child process.
//!
//! The application starts the conversion program as a child process, and
//! the tests have to do the same, because a mock that is not a process
//! cannot show whether the arguments are right, whether the result is read
//! from the right place, or whether the file is left behind when
//! something goes wrong.
//!
//! The program they start is a stand-in: a real executable, with its own
//! `main`, that answers the way the real program is asked to answer. It is
//! built only when the `test-helper` feature is asked for.

#![cfg(feature = "test-helper")]

use std::path::{Path, PathBuf};

use crate::media::convert::{ConvertError, Converter};
use crate::media::locator::Tools;
use crate::media::manifest::Manifest;
use crate::media::probe::Capabilities;
use crate::media::publish::PlatformPublisher;
use crate::media::runner::SystemRunner;

/// The stand-in for the conversion program.
fn stand_in() -> PathBuf {
    // The test binary is in the build folder's `deps`, and the stand-in is
    // in the build folder beside it.
    let executable = std::env::current_exe().expect("the test binary is there");
    let folder = executable
        .parent()
        .and_then(|deps| deps.parent())
        .expect("the build folder is there");
    let name = if cfg!(windows) {
        "ot-fake-ffmpeg.exe"
    } else {
        "ot-fake-ffmpeg"
    };
    let path = folder.join(name);
    assert!(
        path.is_file(),
        "the stand-in was not built: {} is not there. Build it with \
         `cargo build --features test-helper --bins` before the tests.",
        path.display()
    );
    path
}

/// The two programs, as the locator would find them.
fn tools() -> Tools {
    let stand_in = stand_in();
    Tools {
        ffmpeg: stand_in.clone(),
        ffprobe: stand_in,
    }
}

/// A folder of its own for a test, removed when the test is over.
struct TempFolder {
    path: PathBuf,
}

impl TempFolder {
    fn new(name: &str) -> TempFolder {
        let path = std::env::temp_dir().join(format!(
            "open-tangerine-child-{}-{name}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("the test folder could be made");
        TempFolder { path }
    }

    fn file(&self, name: &str) -> PathBuf {
        let path = self.path.join(name);
        std::fs::write(&path, b"a picture").expect("the test file could be made");
        path
    }
}

impl Drop for TempFolder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// The files that are left in a folder that should not be.
///
/// The only ones that may not be there are the files the conversion was
/// making: they end in `.part`, and they are removed whether the
/// conversion worked or not.
fn temporaries_left(folder: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(folder)
        .expect("the folder is there")
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.ends_with(".part"))
        .collect();
    names.sort();
    names
}

#[test]
fn the_stand_in_is_found_and_asked_what_it_can_do() {
    let tools = tools();
    let runner = SystemRunner;
    let capabilities = Capabilities::probe(&tools, &runner).expect("the program answered");

    // The parts the manifest names are the ones the program says it has.
    let manifest = Manifest::get();
    for name in [
        "png_pipe",
        "png",
        "image2pipe",
        "apng",
        "webp_anim",
        "libwebp",
        "gif",
        "tiff_pipe",
    ] {
        assert!(
            capabilities.demuxers.contains(name)
                || capabilities.decoders.contains(name)
                || capabilities.encoders.contains(name)
                || capabilities.muxers.contains(name),
            "{name} is not in the program"
        );
    }
    assert!(
        capabilities.ffprobe_usable,
        "the program that inspects is not there"
    );
    assert!(manifest.formats.len() >= 6);
}

#[test]
fn the_environment_says_what_can_be_done() {
    let tools = tools();
    let runner = SystemRunner;
    let capabilities = Capabilities::probe(&tools, &runner).expect("the program answered");
    let environment = capabilities.environment();
    let processor = environment
        .processor("image")
        .expect("there is an image processor");
    assert!(processor.can_read("png"));
    assert!(processor.can_write("jpeg"));
    assert!(processor.can_write("webp"));
    assert!(processor.can_write("bmp"));
    assert!(processor.can_write("tiff"));
    // GIF is written by its own muxer, which the stand-in has.
    assert!(processor.can_write("gif"));
}

#[test]
fn a_picture_is_converted_and_the_original_is_left_alone() {
    let folder = TempFolder::new("converted");
    let path = folder.file("photo.png");
    let before = std::fs::read(&path).expect("the file is there");

    let tools = tools();
    let runner = SystemRunner;
    let publisher = PlatformPublisher;
    let converter = Converter {
        tools: &tools,
        runner: &runner,
        publisher: &publisher,
    };
    let manifest = Manifest::get();
    let target = manifest.format("jpeg").expect("jpeg is in the manifest");
    let name = converter
        .convert(&path, target)
        .expect("the file was converted");

    assert_eq!(name, folder.path.join("photo.jpeg"));
    assert!(name.is_file());
    // The result is what it should be.
    let result = std::fs::read(&name).expect("the result is there");
    assert_eq!(&result[..3], &[0xFF, 0xD8, 0xFF]);
    // The original is untouched.
    assert_eq!(std::fs::read(&path).expect("the file is there"), before);
    // And nothing was left behind.
    assert_eq!(temporaries_left(&folder.path), Vec::<String>::new());
}

#[test]
fn a_name_that_is_taken_is_given_a_number() {
    let folder = TempFolder::new("taken");
    let path = folder.file("photo.png");
    std::fs::write(folder.path.join("photo.jpeg"), b"the old one").expect("the file could be made");

    let tools = tools();
    let runner = SystemRunner;
    let publisher = PlatformPublisher;
    let converter = Converter {
        tools: &tools,
        runner: &runner,
        publisher: &publisher,
    };
    let manifest = Manifest::get();
    let target = manifest.format("jpeg").expect("jpeg is in the manifest");
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
fn a_file_that_holds_more_than_one_picture_is_not_converted() {
    let folder = TempFolder::new("animated");
    let path = folder.file("animated.png");

    let tools = tools();
    let runner = SystemRunner;
    let publisher = PlatformPublisher;
    let converter = Converter {
        tools: &tools,
        runner: &runner,
        publisher: &publisher,
    };
    let manifest = Manifest::get();
    let target = manifest.format("jpeg").expect("jpeg is in the manifest");

    assert_eq!(
        converter.convert(&path, target),
        Err(ConvertError::Animation)
    );
    // Nothing was written, and nothing was left behind.
    assert!(!folder.path.join("photo.jpeg").exists());
    assert_eq!(temporaries_left(&folder.path), Vec::<String>::new());
}

#[test]
fn a_file_that_cannot_be_read_is_not_converted() {
    let folder = TempFolder::new("broken");
    let path = folder.file("broken.png");

    let tools = tools();
    let runner = SystemRunner;
    let publisher = PlatformPublisher;
    let converter = Converter {
        tools: &tools,
        runner: &runner,
        publisher: &publisher,
    };
    let manifest = Manifest::get();
    let target = manifest.format("jpeg").expect("jpeg is in the manifest");

    assert_eq!(
        converter.convert(&path, target),
        Err(ConvertError::Uncertain)
    );
    assert_eq!(temporaries_left(&folder.path), Vec::<String>::new());
}

#[test]
fn a_file_that_is_already_in_that_format_is_not_converted() {
    let folder = TempFolder::new("same");
    let path = folder.file("photo.png");

    let tools = tools();
    let runner = SystemRunner;
    let publisher = PlatformPublisher;
    let converter = Converter {
        tools: &tools,
        runner: &runner,
        publisher: &publisher,
    };
    let manifest = Manifest::get();
    let target = manifest.format("png").expect("png is in the manifest");

    assert_eq!(
        converter.convert(&path, target),
        Err(ConvertError::AlreadyThatFormat)
    );
}
