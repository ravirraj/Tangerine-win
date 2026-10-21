//! The tests that run against the real conversion program.
//!
//! These are not part of an ordinary `cargo test`: they need the program
//! that was built from the manifest, and they are run with
//! `cargo test -- --ignored` once it has been built.
//!
//! The pictures they work with are made by the program itself, from raw
//! pixels, so that the tests do not depend on any picture being kept in
//! the repository.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::media::convert::{ConvertError, Converter};
use crate::media::locator::Tools;
use crate::media::manifest::Manifest;
use crate::media::publish::PlatformPublisher;
use crate::media::runner::SystemRunner;

/// The environment variable that says where the program is.
const WHERE: &str = "OPEN_TANGERINE_FFMPEG";

/// The programs, or nothing when they are not there.
///
/// The tests that need the program say so and stop when it is not there:
/// a test that quietly passes without testing anything is worse than no
/// test.
fn programs() -> Option<Tools> {
    if let Some(path) = std::env::var_os(WHERE) {
        let folder = PathBuf::from(path);
        let ffmpeg = folder.join("ffmpeg.exe");
        let ffprobe = folder.join("ffprobe.exe");
        if ffmpeg.is_file() && ffprobe.is_file() {
            return Some(Tools { ffmpeg, ffprobe });
        }
    }
    // The place a developer keeps it.
    let folder = PathBuf::from("vendor").join("ffmpeg");
    let ffmpeg = folder.join("ffmpeg.exe");
    let ffprobe = folder.join("ffprobe.exe");
    if ffmpeg.is_file() && ffprobe.is_file() {
        return Some(Tools { ffmpeg, ffprobe });
    }
    None
}

/// A folder of its own for a test, removed when the test is over.
struct TempFolder {
    path: PathBuf,
}

impl TempFolder {
    fn new(name: &str) -> TempFolder {
        let path =
            std::env::temp_dir().join(format!("open-tangerine-real-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("the test folder could be made");
        TempFolder { path }
    }
}

impl Drop for TempFolder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// The width and height of the pictures the tests make.
const SIZE: usize = 8;

/// Makes a picture of the given format, with the given number of frames.
///
/// The picture is made by the program itself, from raw pixels, so that
/// nothing has to be kept in the repository. A picture with more than one
/// frame is an animation.
fn make_picture(
    program: &Path,
    folder: &Path,
    name: &str,
    codec: &str,
    muxer: &str,
    frames: u32,
    transparent: bool,
) -> PathBuf {
    let mut pixels = Vec::new();
    for _ in 0..(SIZE * SIZE) {
        // The top half of the picture is see-through, the bottom half is
        // a solid red.
        pixels.extend_from_slice(&[255, 0, 0, 255]);
    }
    if transparent {
        for byte in 0..(SIZE * SIZE / 2) * 4 {
            pixels[byte] = 255;
            pixels[byte + 1] = 0;
            pixels[byte + 2] = 0;
            pixels[byte + 3] = 0;
        }
    }

    let path = folder.join(name);
    let mut child = Command::new(program)
        .args([
            "-hide_banner",
            "-nostdin",
            "-y",
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgba",
            "-s",
            &format!("{SIZE}x{SIZE}"),
            "-r",
            "5",
            "-i",
            "-",
            "-frames:v",
            &frames.to_string(),
            "-c:v",
            codec,
            "-f",
            muxer,
        ])
        .arg(&path)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the program was started");
    {
        use std::io::Write;
        let mut stdin = child.stdin.take().expect("the input is there");
        stdin
            .write_all(&pixels)
            .expect("the pixels could be written");
    }
    let output = child.wait_with_output().expect("the program finished");
    assert!(
        output.status.success() && path.is_file(),
        "the program could not make {name}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    path
}

/// The pixels of a picture, as the program reads them back.
fn pixels_of(program: &Path, picture: &Path) -> Vec<u8> {
    let output = Command::new(program)
        .args([
            "-hide_banner",
            "-nostdin",
            "-i",
            picture.to_str().expect("the path is text"),
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgba",
            "-",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .expect("the program was run");
    assert!(
        output.status.success(),
        "the program could not read the picture back"
    );
    output.stdout
}

/// The colour of the first pixel of a picture.
fn first_pixel(pixels: &[u8]) -> [u8; 4] {
    [pixels[0], pixels[1], pixels[2], pixels[3]]
}

/// The colour of a pixel in the bottom half of a picture.
fn lower_pixel(pixels: &[u8]) -> [u8; 4] {
    let at = ((SIZE - 1) * SIZE + 1) * 4;
    [pixels[at], pixels[at + 1], pixels[at + 2], pixels[at + 3]]
}

#[test]
#[ignore = "needs the conversion program"]
fn the_program_has_the_parts_the_manifest_asks_for() {
    let Some(tools) = programs() else {
        eprintln!("the program is not there; this test needs it");
        return;
    };
    let runner = SystemRunner;
    let capabilities =
        crate::media::probe::Capabilities::probe(&tools, &runner).expect("the program answered");
    let manifest = Manifest::get();
    for format in &manifest.formats {
        assert!(
            capabilities.can_read(format),
            "{} cannot be read",
            format.format
        );
        assert!(
            capabilities.can_count_frames(format),
            "{} cannot be checked for animation",
            format.format
        );
        assert!(
            capabilities.can_write(format),
            "{} cannot be written",
            format.format
        );
    }
}

#[test]
#[ignore = "needs the conversion program"]
fn every_picture_is_converted_into_every_format() {
    let Some(tools) = programs() else {
        eprintln!("the program is not there; this test needs it");
        return;
    };
    let folder = TempFolder::new("matrix");
    let manifest = Manifest::get();
    let runner = SystemRunner;
    let publisher = PlatformPublisher;

    // A picture of every format the program can read.
    let mut pictures = Vec::new();
    for source in &manifest.formats {
        let Some(extension) = source.extensions.first() else {
            continue;
        };
        let codec = source.write.encoder.clone();
        let muxer = source.write.muxer.clone();
        let picture = make_picture(
            &tools.ffmpeg,
            &folder.path,
            &format!("source.{extension}"),
            &codec,
            &muxer,
            1,
            false,
        );
        pictures.push((source, picture));
    }
    assert!(!pictures.is_empty(), "no picture was made");

    for (source, picture) in &pictures {
        for target in &manifest.formats {
            // A picture that is already in that format is not converted.
            if source.format == target.format {
                continue;
            }
            let converter = Converter {
                tools: &tools,
                runner: &runner,
                publisher: &publisher,
            };
            let name = converter.convert(picture, target).unwrap_or_else(|reason| {
                panic!(
                    "{} could not be converted into {}: {}",
                    source.format,
                    target.format,
                    reason.name()
                )
            });
            assert!(name.is_file(), "{} was not made", name.display());
            let result = std::fs::read(&name).expect("the result is there");
            assert!(
                target.matches(&result),
                "the result of {} to {} is not a {}",
                source.format,
                target.format,
                target.format
            );
        }
    }
}

#[test]
#[ignore = "needs the conversion program"]
fn a_picture_that_can_hold_transparency_keeps_it() {
    let Some(tools) = programs() else {
        eprintln!("the program is not there; this test needs it");
        return;
    };
    let folder = TempFolder::new("alpha");
    let manifest = Manifest::get();
    let runner = SystemRunner;
    let publisher = PlatformPublisher;

    let picture = make_picture(
        &tools.ffmpeg,
        &folder.path,
        "source.png",
        "png",
        "image2pipe",
        1,
        true,
    );

    for name in ["png", "tiff", "webp"] {
        let Some(target) = manifest.format(name) else {
            continue;
        };
        let converter = Converter {
            tools: &tools,
            runner: &runner,
            publisher: &publisher,
        };
        let result = converter
            .convert(&picture, target)
            .unwrap_or_else(|reason| panic!("it could not be converted: {}", reason.name()));
        let pixels = pixels_of(&tools.ffmpeg, &result);
        let top = first_pixel(&pixels);
        let lower = lower_pixel(&pixels);
        assert!(
            top[3] == 0,
            "the see-through part of the {name} is not see-through: {top:?}"
        );
        assert!(
            lower[3] == 255,
            "the solid part of the {name} is not solid: {lower:?}"
        );
    }
}

#[test]
#[ignore = "needs the conversion program"]
fn a_picture_that_cannot_hold_transparency_is_painted_onto_white() {
    let Some(tools) = programs() else {
        eprintln!("the program is not there; this test needs it");
        return;
    };
    let folder = TempFolder::new("matte");
    let manifest = Manifest::get();
    let runner = SystemRunner;
    let publisher = PlatformPublisher;

    let picture = make_picture(
        &tools.ffmpeg,
        &folder.path,
        "source.png",
        "png",
        "image2pipe",
        1,
        true,
    );

    for name in ["jpeg", "bmp"] {
        let Some(target) = manifest.format(name) else {
            continue;
        };
        let converter = Converter {
            tools: &tools,
            runner: &runner,
            publisher: &publisher,
        };
        let result = converter
            .convert(&picture, target)
            .unwrap_or_else(|reason| panic!("it could not be converted: {}", reason.name()));
        let pixels = pixels_of(&tools.ffmpeg, &result);
        let top = first_pixel(&pixels);
        // The background is white: the see-through part of the picture has
        // been painted onto it, and not left black.
        assert!(
            top[0] > 240 && top[1] > 240 && top[2] > 240,
            "the background of the {name} is not white: {top:?}"
        );
    }
}

#[test]
#[ignore = "needs the conversion program"]
fn a_picture_with_several_frames_is_refused() {
    let Some(tools) = programs() else {
        eprintln!("the program is not there; this test needs it");
        return;
    };
    let folder = TempFolder::new("animation");
    let manifest = Manifest::get();
    let runner = SystemRunner;
    let publisher = PlatformPublisher;

    // A picture with two frames is an animation, and is not converted.
    let picture = make_picture(
        &tools.ffmpeg,
        &folder.path,
        "animated.png",
        "apng",
        "apng",
        2,
        false,
    );

    let target = manifest.format("jpeg").expect("jpeg is in the manifest");
    let converter = Converter {
        tools: &tools,
        runner: &runner,
        publisher: &publisher,
    };
    assert_eq!(
        converter.convert(&picture, target),
        Err(ConvertError::Animation),
        "an animation was converted and its frames were thrown away"
    );
}

#[test]
#[ignore = "needs the conversion program"]
fn an_animated_webp_is_refused() {
    let Some(tools) = programs() else {
        eprintln!("the program is not there; this test needs it");
        return;
    };
    let folder = TempFolder::new("animated-webp");
    let manifest = Manifest::get();
    let runner = SystemRunner;
    let publisher = PlatformPublisher;

    // A WebP with two frames is an animation, and is not converted.
    let picture = make_picture(
        &tools.ffmpeg,
        &folder.path,
        "animated.webp",
        "libwebp",
        "webp",
        2,
        false,
    );

    let target = manifest.format("png").expect("png is in the manifest");
    let converter = Converter {
        tools: &tools,
        runner: &runner,
        publisher: &publisher,
    };
    assert_eq!(
        converter.convert(&picture, target),
        Err(ConvertError::Animation),
        "an animation was converted and its frames were thrown away"
    );
}

#[test]
#[ignore = "needs the conversion program"]
fn a_file_that_is_broken_is_refused() {
    let Some(tools) = programs() else {
        eprintln!("the program is not there; this test needs it");
        return;
    };
    let folder = TempFolder::new("broken");
    let manifest = Manifest::get();
    let runner = SystemRunner;
    let publisher = PlatformPublisher;

    // A file that says it is a picture and is not one.
    let picture = folder.path.join("broken.png");
    std::fs::write(&picture, b"\x89PNG\r\n\x1a\n and then nothing")
        .expect("the file could be made");

    let target = manifest.format("jpeg").expect("jpeg is in the manifest");
    let converter = Converter {
        tools: &tools,
        runner: &runner,
        publisher: &publisher,
    };
    let outcome = converter.convert(&picture, target);
    assert!(
        matches!(
            outcome,
            Err(ConvertError::Uncertain)
                | Err(ConvertError::ProgramFailed)
                | Err(ConvertError::BadResult)
        ),
        "a broken file was converted: {outcome:?}"
    );
}

#[test]
#[ignore = "needs the conversion program"]
fn a_webp_that_is_broken_is_not_taken_for_a_still_one() {
    let Some(tools) = programs() else {
        eprintln!("the program is not there; this test needs it");
        return;
    };
    let folder = TempFolder::new("broken-webp");
    let manifest = Manifest::get();
    let runner = SystemRunner;

    // A file that says it is a WebP and is not one.
    let picture = folder.path.join("broken.webp");
    let mut bytes = b"RIFF".to_vec();
    bytes.extend_from_slice(&[0, 0, 0, 0]);
    bytes.extend_from_slice(b"WEBP");
    bytes.extend_from_slice(b"and then something that is not a picture");
    std::fs::write(&picture, bytes).expect("the file could be made");

    let format = manifest.format("webp").expect("webp is in the manifest");
    let verdict = crate::media::animation::judge(&runner, &tools.ffprobe, format, &picture);
    assert!(
        !verdict.may_convert(),
        "a file that could not be read was taken for a still picture: {verdict:?}"
    );
    assert_eq!(verdict, crate::media::animation::Verdict::Uncertain);
}

#[test]
#[ignore = "needs the conversion program"]
fn the_original_is_left_alone() {
    let Some(tools) = programs() else {
        eprintln!("the program is not there; this test needs it");
        return;
    };
    let folder = TempFolder::new("original");
    let manifest = Manifest::get();
    let runner = SystemRunner;
    let publisher = PlatformPublisher;

    let picture = make_picture(
        &tools.ffmpeg,
        &folder.path,
        "photo.png",
        "png",
        "image2pipe",
        1,
        false,
    );
    let before = std::fs::read(&picture).expect("the picture is there");

    let target = manifest.format("jpeg").expect("jpeg is in the manifest");
    let converter = Converter {
        tools: &tools,
        runner: &runner,
        publisher: &publisher,
    };
    converter
        .convert(&picture, target)
        .expect("it was converted");
    assert_eq!(
        std::fs::read(&picture).expect("the picture is there"),
        before,
        "the original was changed"
    );
}
