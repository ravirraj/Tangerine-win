//! A stand-in for the conversion program, for the tests.
//!
//! The real program is a separate program that the application starts as a
//! child process. The tests have to start a real child process too, and a
//! batch file cannot be one: Windows will only run a real executable
//! without a shell, and a shell in front of a path that came from the user
//! is exactly what this program must never do.
//!
//! So this is a real executable with its own `main`. It is built only when
//! the `test-helper` feature is asked for, so it never goes near a
//! release.
//!
//! What it answers is decided by the arguments it is given, so that a test
//! can ask for any of the things the real program is asked for:
//!
//! - `-version` and the listings: it answers with a small program that has
//!   the parts the manifest names;
//! - `-count_frames`: it answers with the number of frames the file should
//!   be taken to have — one, unless the file is named so that it is an
//!   animation, and nothing at all if the file is named so that it is
//!   broken;
//! - anything else with an input and an output: it writes the beginning of
//!   a file of the format that was asked for, so that the application finds
//!   the result to be what it should be.

use std::io::Write;
use std::process::ExitCode;

/// The name the frames are counted for, when the file is an animation.
const ANIMATED: &str = "animated";

/// The name the file is broken, when it is.
const BROKEN: &str = "broken";

fn main() -> ExitCode {
    let arguments: Vec<String> = std::env::args().skip(1).collect();

    if let Some(what) = arguments.iter().rev().find(|word| word.starts_with('-')) {
        match what.as_str() {
            "-version" => {
                println!("ffmpeg version 9.0.2 (the stand-in for the tests)");
                return ExitCode::SUCCESS;
            }
            "-encoders" => {
                print!("{}", listing(ENCODERS));
                return ExitCode::SUCCESS;
            }
            "-decoders" => {
                print!("{}", listing(DECODERS));
                return ExitCode::SUCCESS;
            }
            "-formats" => {
                print!("{}", listing(FORMATS));
                return ExitCode::SUCCESS;
            }
            "-filters" => {
                print!("{}", listing(FILTERS));
                return ExitCode::SUCCESS;
            }
            "-protocols" => {
                print!("{}", listing(PROTOCOLS));
                return ExitCode::SUCCESS;
            }
            _ => {}
        }
    }

    // The frames of a file are counted with these arguments. The file is
    // named at the end, not after an "-i", when the frames are counted.
    if arguments.iter().any(|word| word == "-count_frames") {
        let input = arguments.last().cloned().unwrap_or_default();
        if input.contains(BROKEN) {
            // The file cannot be read at all.
            return ExitCode::FAILURE;
        }
        let frames = if input.contains(ANIMATED) { 2 } else { 1 };
        println!("{frames}");
        return ExitCode::SUCCESS;
    }

    // A conversion: the input and the output are both named.
    let Some(input) = value_after(&arguments, "-i") else {
        eprintln!("the stand-in was asked to do something it does not know");
        return ExitCode::FAILURE;
    };
    if input.contains(BROKEN) {
        eprintln!("the stand-in could not read the file");
        return ExitCode::FAILURE;
    }
    let Some(codec) = value_after(&arguments, "-c:v") else {
        eprintln!("the stand-in was asked to convert without being told into what");
        return ExitCode::FAILURE;
    };
    let Some(bytes) = beginning_of(&codec) else {
        eprintln!("the stand-in does not know the format {codec}");
        return ExitCode::FAILURE;
    };

    // The result goes to the standard output, which is the file the
    // application made for it.
    let mut out = std::io::stdout();
    if out.write_all(&bytes).is_err() {
        return ExitCode::FAILURE;
    }
    let _ = out.flush();
    ExitCode::SUCCESS
}

/// The value that follows a word in the arguments.
fn value_after(arguments: &[String], word: &str) -> Option<String> {
    arguments
        .iter()
        .position(|argument| argument == word)
        .and_then(|at| arguments.get(at + 1))
        .cloned()
}

/// The beginning of a file of the format the codec names.
fn beginning_of(codec: &str) -> Option<Vec<u8>> {
    let bytes: Vec<u8> = match codec {
        "png" => vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A],
        "mjpeg" => vec![0xFF, 0xD8, 0xFF, 0xE0],
        "bmp" => vec![0x42, 0x4D, 0x00, 0x00],
        "tiff" => vec![0x49, 0x49, 0x2A, 0x00],
        "gif" => vec![0x47, 0x49, 0x46, 0x38, 0x39, 0x61],
        "libwebp" => vec![
            0x52, 0x49, 0x46, 0x46, 0x00, 0x00, 0x00, 0x00, 0x57, 0x45, 0x42, 0x50,
        ],
        _ => return None,
    };
    Some(bytes)
}

/// A listing in the shape the real program prints them.
fn listing(body: &str) -> String {
    body.to_string()
}

const ENCODERS: &str = "\
Encoders:
 V..... = Video
 ------
 V....D bmp                 BMP (Windows and OS/2 bitmap)
 V....D gif                 GIF (Graphics Interchange Format)
 V....D libwebp             libwebp WebP
 V....D mjpeg               MJPEG (Motion JPEG)
 V....D png                 PNG (Portable Network Graphics) image
 V....D rawvideo            raw video
 V....D tiff                TIFF image
";

const DECODERS: &str = "\
Decoders:
 V..... = Video
 ------
 V....D apng                APNG (Animated PNG) image
 V....D bmp                 BMP (Windows and OS/2 bitmap)
 V....D gif                 GIF (Graphics Interchange Format)
 V....D mjpeg               MJPEG (Motion JPEG)
 V....D png                 PNG (Portable Network Graphics) image
 V....D rawvideo            raw video
 V....D tiff                TIFF image
 V....D webp                WebP
 V....D webp_anim           WebP animation
";

const FORMATS: &str = "\
 File formats:
  D. = Demuxing supported
  .E = Muxing supported
 ---
 D  apng            Animated PNG sequence
 D  bmp_pipe         Piped bmp sequence
 D  gif              CompuServe Graphics Interchange Format (GIF)
 D  jpeg_pipe        Piped jpeg sequence
 D  png_pipe         Piped PNG sequence
 D  tiff_pipe        Piped TIFF sequence
 D  webp_anim        WebP animation
 D  webp_pipe        Piped WebP sequence
 DE image2pipe       Piped image2 sequence
 E  gif              CompuServe Graphics Interchange Format (GIF)
 E  image2pipe       Piped image2 sequence
 E  webp             WebP
";

const FILTERS: &str = "\
Filters:
  T.. = Timeline support
  | = Source or sink filter
 ... drawbox           ->V        Draw a colored box on the input image.
 ... format            ->V        Convert the input image to the pixel format.
 ... overlay           VV->V      Overlay a video source on top of the input.
 ... scale             ->V        Scale the input image.
 ... split             ->V        Pass on the source to the output.
";

const PROTOCOLS: &str = "\
Supported file protocols:
Input:
  file
  pipe
Output:
  file
  pipe
";
