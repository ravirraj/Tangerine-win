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
use crate::media::publish::{PublishError, Publisher};
use crate::media::runner::SystemRunner;

/// The environment variable that says where the program is.
const WHERE: &str = "OPEN_TANGERINE_FFMPEG";

/// A publisher for the tests.
///
/// It moves the file and never overwrites what is there. The publisher the
/// application uses is tested where it lives, in the platform layer; these
/// tests are about the conversion, not about the move.
struct TestPublisher;

impl Publisher for TestPublisher {
    fn publish(&self, temporary: &Path, destination: &Path) -> Result<(), PublishError> {
        if destination.exists() {
            return Err(PublishError::DestinationExists);
        }
        std::fs::rename(temporary, destination).map_err(|_| PublishError::CouldNotMove)
    }
}

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
    // The picture: the top half is see-through, the bottom half is a solid
    // red. The bytes are built here and then checked, so that a picture
    // which is not what it should be is caught here and not three steps
    // later as a mysterious failure.
    let mut pixels = Vec::with_capacity(SIZE * SIZE * 4 * frames as usize);
    for _ in 0..frames {
        for row in 0..SIZE {
            let alpha = if transparent && row < SIZE / 2 {
                0
            } else {
                255
            };
            for _ in 0..SIZE {
                pixels.extend_from_slice(&[255, 0, 0, alpha]);
            }
        }
    }
    assert_eq!(pixels.len(), SIZE * SIZE * 4 * frames as usize);
    if transparent {
        let see_through = &pixels[0..4];
        let solid = &pixels[(SIZE * SIZE / 2) * 4..(SIZE * SIZE / 2) * 4 + 4];
        assert_eq!(
            see_through,
            &[255, 0, 0, 0],
            "the see-through part of the picture is not see-through"
        );
        assert_eq!(
            solid,
            &[255, 0, 0, 255],
            "the solid part of the picture is not solid"
        );
    }

    let path = folder.join(name);
    // The raw pixels are put in a file of their own, and that file is what
    // the program is given. The application never does this — it is only
    // how the tests make the pictures they work with.
    let raw = folder.join(format!("{name}.raw"));
    std::fs::write(&raw, &pixels).expect("the pixels could be written");

    let child = Command::new(program)
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
        ])
        .arg(&raw)
        .args(["-frames:v", &frames.to_string(), "-c:v", codec, "-f", muxer])
        .arg(&path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the program was started");
    let output = child.wait_with_output().expect("the program finished");
    let _ = std::fs::remove_file(&raw);
    assert!(
        output.status.success() && path.is_file(),
        "the program could not make {name}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    path
}

/// The pixels of a picture, as the program reads them back.
///
/// The demuxer is named, the same way the application names it, so that
/// the picture is read the same way every time.
fn pixels_of(program: &Path, picture: &Path, demuxer: &str) -> Vec<u8> {
    let output = Command::new(program)
        .args([
            "-hide_banner",
            "-nostdin",
            "-f",
            demuxer,
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
    let publisher = TestPublisher;

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
    let publisher = TestPublisher;

    // Each target that can hold transparency is reached from a source that
    // is not in that format, because a picture that is already in the
    // format is not converted at all.
    for (source, target) in [("png", "tiff"), ("png", "webp"), ("webp", "png")] {
        let Some(source_format) = manifest.format(source) else {
            continue;
        };
        let Some(target_format) = manifest.format(target) else {
            continue;
        };
        let picture = make_picture(
            &tools.ffmpeg,
            &folder.path,
            &format!("source-{source}.{}", source_format.extensions[0]),
            &source_format.write.encoder,
            &source_format.write.muxer,
            1,
            true,
        );

        let converter = Converter {
            tools: &tools,
            runner: &runner,
            publisher: &publisher,
        };
        let result = converter
            .convert(&picture, target_format)
            .unwrap_or_else(|reason| {
                panic!(
                    "{source} could not be converted into {target}: {}",
                    reason.name()
                )
            });
        let pixels = pixels_of(&tools.ffmpeg, &result, &target_format.read.demuxer);
        let top = first_pixel(&pixels);
        let lower = lower_pixel(&pixels);
        assert!(
            top[3] == 0,
            "the see-through part of the {target} is not see-through: {top:?}"
        );
        assert!(
            lower[3] == 255,
            "the solid part of the {target} is not solid: {lower:?}"
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
    let publisher = TestPublisher;

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
        let pixels = pixels_of(&tools.ffmpeg, &result, &target.read.demuxer);
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
    let publisher = TestPublisher;

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
    let publisher = TestPublisher;

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
    let publisher = TestPublisher;

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
fn a_picture_survives_a_trip_through_a_gif() {
    // The GIF operation is offered because this works: a picture that is
    // written as a GIF and read back is the picture it was.
    let Some(tools) = programs() else {
        eprintln!("the program is not there; this test needs it");
        return;
    };
    let folder = TempFolder::new("gif");
    let manifest = Manifest::get();
    let runner = SystemRunner;
    let publisher = TestPublisher;

    let picture = make_picture(
        &tools.ffmpeg,
        &folder.path,
        "source.png",
        "png",
        "image2pipe",
        1,
        false,
    );

    let to_gif = manifest.format("gif").expect("gif is in the manifest");
    let to_png = manifest.format("png").expect("png is in the manifest");

    let converter = Converter {
        tools: &tools,
        runner: &runner,
        publisher: &publisher,
    };
    let gif = converter
        .convert(&picture, to_gif)
        .expect("it was written as a gif");
    assert!(gif.is_file(), "the gif was not made");
    let gif_bytes = std::fs::read(&gif).expect("the gif is there");
    assert!(to_gif.matches(&gif_bytes), "what was written is not a gif");

    let back = converter.convert(&gif, to_png).expect("it was read back");
    let back_pixels = pixels_of(&tools.ffmpeg, &back, &to_png.read.demuxer);
    let original = pixels_of(&tools.ffmpeg, &picture, &to_png.read.demuxer);
    assert_eq!(
        back_pixels.len(),
        original.len(),
        "the picture came back a different size"
    );
    // A GIF holds a palette of a few hundred colours, so the picture comes
    // back the same picture and not quite the same numbers.
    for (came_back, was) in back_pixels.iter().zip(original.iter()) {
        assert!(
            (*came_back as i32 - *was as i32).abs() <= 8,
            "the picture came back a different colour: {came_back} and {was}"
        );
    }
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
    let publisher = TestPublisher;

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

#[test]
#[ignore = "needs the conversion program"]
fn awkward_names_are_converted_as_they_are() {
    let Some(tools) = programs() else {
        eprintln!("the program is not there; this test needs it");
        return;
    };
    let folder = TempFolder::new("awkward");
    let manifest = Manifest::get();
    let runner = SystemRunner;
    let publisher = TestPublisher;
    let target = manifest.format("jpeg").expect("jpeg is in the manifest");

    // The names a file may be called by on Windows. The characters
    // Windows does not allow in a name are not here: they cannot happen,
    // and a test that used them would be testing nothing.
    let names = [
        "photo with spaces.png",
        "café ☕ 日本語.png",
        "100%.png",
        "photo%03d.png",
        "a&b;c'd.png",
        "price$100.png",
    ];

    for name in names {
        let picture = make_picture(
            &tools.ffmpeg,
            &folder.path,
            name,
            "png",
            "image2pipe",
            1,
            false,
        );
        let converter = Converter {
            tools: &tools,
            runner: &runner,
            publisher: &publisher,
        };
        let result = converter
            .convert(&picture, target)
            .unwrap_or_else(|reason| panic!("{name} could not be converted: {}", reason.name()));
        assert_eq!(
            result,
            folder.path.join(name).with_extension("jpeg"),
            "{name} was given the wrong name"
        );
        assert!(result.is_file(), "{name} was not made");
    }
}

#[test]
#[ignore = "needs the conversion program"]
fn pictures_in_folders_are_converted_as_they_are() {
    let Some(tools) = programs() else {
        eprintln!("the program is not there; this test needs it");
        return;
    };
    let folder = TempFolder::new("nested");
    let manifest = Manifest::get();
    let runner = SystemRunner;
    let publisher = TestPublisher;
    let target = manifest.format("jpeg").expect("jpeg is in the manifest");

    // A folder of its own, with a space and a picture in it, and a folder
    // inside that one.
    let inner = folder.path.join("sub folder");
    std::fs::create_dir_all(&inner).expect("the folder could be made");
    let deeper = inner.join("one").join("two");
    std::fs::create_dir_all(&deeper).expect("the folder could be made");

    for (folder_to_use, name) in [(&inner, "photo.png"), (&deeper, "café ☕.png")] {
        let picture = make_picture(
            &tools.ffmpeg,
            folder_to_use,
            name,
            "png",
            "image2pipe",
            1,
            false,
        );
        let converter = Converter {
            tools: &tools,
            runner: &runner,
            publisher: &publisher,
        };
        let result = converter
            .convert(&picture, target)
            .unwrap_or_else(|reason| panic!("{name} could not be converted: {}", reason.name()));
        assert_eq!(
            result,
            folder_to_use.join(name).with_extension("jpeg"),
            "{name} was given the wrong name"
        );
        assert!(result.is_file(), "{name} was not made");
    }
}
