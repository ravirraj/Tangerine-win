//! Asking the program what it can do.
//!
//! The listings the program prints say which parts it has. They do not
//! say which conversions work, so they are only ever used to rule a
//! conversion out: a part that is not there cannot be used, and a part
//! that is there is checked properly when the conversion is actually run.
//!
//! Everything here is asked of the program itself, never assumed.

use std::collections::BTreeSet;
use std::ffi::OsString;
use std::io;

use crate::media::command::COMPOSITE_FILTERS;
use crate::media::locator::Tools;
use crate::media::manifest::{Alpha, FormatSupport, Manifest};
use crate::media::runner::{ProcessRunner, RunResult};
use crate::radial::registry::{Environment, Processor};

/// The name of the processor the image operations ask for.
const IMAGE_PROCESSOR: &str = "image";

/// The name of the processor for document/archive/subtitle conversions,
/// handled in Rust without FFmpeg.
const DOCUMENT_PROCESSOR: &str = "document";

/// The name of the processor that reads PDF files with the PDFium library.
///
/// It is there only while the library is: when the library is missing, the
/// operations that need it are not offered, because a wheel never shows
/// what cannot be run.
const PDF_PROCESSOR: &str = "pdf";

/// What the program says it can do.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Capabilities {
    /// The programs that were asked.
    pub tools: Tools,
    /// The encoders that are there.
    pub encoders: BTreeSet<String>,
    /// The decoders that are there.
    pub decoders: BTreeSet<String>,
    /// The demuxers that are there.
    pub demuxers: BTreeSet<String>,
    /// The muxers that are there.
    pub muxers: BTreeSet<String>,
    /// The filters that are there.
    pub filters: BTreeSet<String>,
    /// The protocols that are there.
    pub protocols: BTreeSet<String>,
    /// Whether the program that inspects files runs.
    ///
    /// Without it a file that might be an animation cannot be told apart
    /// from a still one, so the formats that need the check are not offered.
    pub ffprobe_usable: bool,
}

impl Capabilities {
    /// Asks the programs what they can do.
    pub fn probe(tools: &Tools, runner: &dyn ProcessRunner) -> io::Result<Capabilities> {
        let mut capabilities = Capabilities {
            tools: tools.clone(),
            ..Capabilities::default()
        };

        // The program has to run at all.
        let version = run(runner, &tools.ffmpeg, &[OsString::from("-version")])?;
        if !version.succeeded() {
            return Err(io::Error::other("the conversion program did not run"));
        }

        let encoders = listing(runner, &tools.ffmpeg, "-encoders")?;
        let decoders = listing(runner, &tools.ffmpeg, "-decoders")?;
        let formats = listing(runner, &tools.ffmpeg, "-formats")?;
        let filters = listing(runner, &tools.ffmpeg, "-filters")?;
        let protocols_text = text(runner, &tools.ffmpeg, "-protocols")?;

        // In the encoder and decoder listings the flags say other things —
        // whether a codec draws directly, and so on — so every name in them
        // is a part that is there. Only the format listing says which
        // demuxers and which muxers are there.
        capabilities.encoders = encoders.into_iter().map(|(_, name)| name).collect();
        capabilities.decoders = decoders.into_iter().map(|(_, name)| name).collect();
        capabilities.demuxers = formats
            .iter()
            .filter(|(flags, _)| flags.contains('D'))
            .map(|(_, name)| name.clone())
            .collect();
        capabilities.muxers = formats
            .iter()
            .filter(|(flags, _)| flags.contains('E'))
            .map(|(_, name)| name.clone())
            .collect();
        capabilities.filters = filters.into_iter().map(|(_, name)| name).collect();
        capabilities.protocols = protocols(&protocols_text);

        // The program that inspects files is asked whether it runs.
        capabilities.ffprobe_usable = run(runner, &tools.ffprobe, &[OsString::from("-version")])
            .map(|result| result.succeeded())
            .unwrap_or(false);

        Ok(capabilities)
    }

    /// Whether a file of this format can be read.
    ///
    /// The picture is written through a pipe, so a program without the pipe
    /// protocol cannot be used for the conversion at all, whatever parts it
    /// says it has.
    pub fn can_read(&self, format: &FormatSupport) -> bool {
        self.has_pipe()
            && self.has_file()
            && self.demuxers.contains(&format.read.demuxer)
            && self.decoders.contains(&format.read.decoder)
    }

    /// Whether the frames of this format can be counted.
    ///
    /// A format that cannot be animated needs nothing. A format that can
    /// needs the parts that can see every frame, the parts that prove a
    /// file is a single image where the first ones refuse still files, and
    /// the program that does the counting.
    pub fn can_count_frames(&self, format: &FormatSupport) -> bool {
        let Some(animation) = format.animation_parts() else {
            // The format cannot be animated, so there is nothing to check.
            return true;
        };
        if !self.ffprobe_usable {
            return false;
        }
        if !self.demuxers.contains(&animation.demuxer)
            || !self.decoders.contains(&animation.decoder)
        {
            return false;
        }
        match &format.confirm {
            None => true,
            Some(confirm) => {
                self.demuxers.contains(&confirm.demuxer) && self.decoders.contains(&confirm.decoder)
            }
        }
    }

    /// Whether a file of this format can be produced.
    pub fn can_write(&self, format: &FormatSupport) -> bool {
        if !self.has_pipe()
            || !self.encoders.contains(&format.write.encoder)
            || !self.muxers.contains(&format.write.muxer)
        {
            return false;
        }
        self.filters_needed(format)
            .iter()
            .all(|filter| self.filters.contains(*filter))
    }

    /// Whether the program can write a picture out through a pipe.
    ///
    /// The picture is made in a file of its own and then moved to its name,
    /// and the program writes it to its standard output. A program without
    /// the pipe protocol cannot do that, so nothing can be converted with it.
    fn has_pipe(&self) -> bool {
        self.protocols.contains("pipe")
    }

    /// Whether the program can read a file from the disk.
    fn has_file(&self) -> bool {
        self.protocols.contains("file")
    }

    /// The filters a conversion of this format needs.
    pub fn filters_needed(&self, format: &FormatSupport) -> Vec<&'static str> {
        let mut needed: Vec<&'static str> = Manifest::get()
            .filters
            .iter()
            .map(|filter| filter.as_str())
            .filter(|filter| !COMPOSITE_FILTERS.contains(filter))
            .collect();
        if format.alpha == Alpha::Matte {
            needed.extend_from_slice(COMPOSITE_FILTERS);
        }
        needed
    }

    /// What the registry should be told the machine can do.
    pub fn environment(&self) -> Environment {
        let manifest = Manifest::get();
        let mut image_reads = Vec::new();
        let mut image_writes = Vec::new();
        let mut doc_reads = Vec::new();
        let mut doc_writes = Vec::new();

        for format in &manifest.formats {
            // Formats with no reader (write-only, like ICO) skip reading.
            if format.read.demuxer.is_empty() {
                // Document/archive formats with no FFmpeg parts are handled
                // by the document processor in Rust.
                if !format.write.encoder.is_empty() || !format.write.muxer.is_empty() {
                    // Write-only FFmpeg format (ICO): image processor can write.
                    image_writes.push(format.format.clone());
                } else {
                    // Pure document format (SRT, VTT, TXT, ZIP, TAR, GZ):
                    // document processor can read and write.
                    doc_reads.extend(format.extensions.iter().cloned());
                    doc_writes.push(format.format.clone());
                }
                continue;
            }
            if self.can_read(format) && self.can_count_frames(format) {
                image_reads.extend(format.extensions.iter().cloned());
            }
            if self.can_write(format) {
                image_writes.push(format.format.clone());
            }
        }

        // The document processor also reads the office documents that are
        // converted in Rust alone — the words of a DOCX, the sheets of a
        // spreadsheet — and those need nothing but the program itself, so
        // they are always offered.
        doc_reads.extend(
            ["docx", "xlsx", "xls", "ods", "csv", "tsv"]
                .iter()
                .map(|name| name.to_string()),
        );
        doc_writes.push("csv".to_string());

        // The PDF reader is a library beside the program, not a part of the
        // conversion program, so it is not in the manifest. It reads PDF
        // files and writes pictures and text, and it is only offered while
        // the library is there to be bound to.
        let mut processors = vec![
            Processor {
                name: IMAGE_PROCESSOR.to_string(),
                reads: image_reads,
                writes: image_writes,
            },
            Processor {
                name: DOCUMENT_PROCESSOR.to_string(),
                reads: doc_reads,
                writes: doc_writes,
            },
        ];
        if crate::media::pdf::library().is_some() {
            processors.push(Processor {
                name: PDF_PROCESSOR.to_string(),
                reads: vec!["pdf".to_string()],
                writes: vec![
                    "png".to_string(),
                    "jpeg".to_string(),
                    "txt".to_string(),
                ],
            });
        }

        // The office documents become PDFs through whatever reads them —
        // the installed Microsoft Office, or LibreOffice — and only the
        // extensions a program is really there for are offered.
        let office_reads = crate::media::documents2::available_reads();
        if !office_reads.is_empty() {
            processors.push(Processor {
                name: "office".to_string(),
                reads: office_reads.iter().map(|name| name.to_string()).collect(),
                writes: vec!["pdf".to_string()],
            });
        }

        Environment { processors }
    }
}

/// Runs the program and gives back what it said.
fn run(
    runner: &dyn ProcessRunner,
    program: &std::path::Path,
    args: &[OsString],
) -> io::Result<RunResult> {
    runner.run(program, args, None)
}

/// Runs the program and gives back what it printed.
fn text(runner: &dyn ProcessRunner, program: &std::path::Path, what: &str) -> io::Result<String> {
    let args = vec![OsString::from("-hide_banner"), OsString::from(what)];
    let result = run(runner, program, &args)?;
    if !result.succeeded() {
        return Err(io::Error::other(format!(
            "the program could not list {what}"
        )));
    }
    Ok(String::from_utf8_lossy(&result.stdout).into_owned())
}

/// The protocols the program has.
///
/// This listing is not in the shape the others are in. It has an "Input:"
/// and an "Output:" section, and the names of the protocols are indented
/// under them.
fn protocols(text: &str) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    for line in text.lines() {
        // The headings are not indented; the names are.
        if !line.starts_with(char::is_whitespace) {
            continue;
        }
        let name = line.trim();
        if name.is_empty() || name.ends_with(':') {
            continue;
        }
        names.insert(name.to_string());
    }
    names
}

/// The listing the program printed, with the flags each name carries.
fn listing(
    runner: &dyn ProcessRunner,
    program: &std::path::Path,
    what: &str,
) -> io::Result<Vec<(String, String)>> {
    let args = vec![OsString::from("-hide_banner"), OsString::from(what)];
    let result = run(runner, program, &args)?;
    if !result.succeeded() {
        return Err(io::Error::other(format!(
            "the program could not list {what}"
        )));
    }
    let text = String::from_utf8_lossy(&result.stdout);
    Ok(parse_listing(&text))
}

/// The names in a listing, with the flags each one carries.
///
/// The listings all have the same shape: a column of flags, then the name,
/// then a description. The lines that explain the flags are skipped, and so
/// is anything that is not a listing at all.
pub fn parse_listing(text: &str) -> Vec<(String, String)> {
    let mut names = Vec::new();
    for line in text.lines() {
        let mut parts = line.split_whitespace();
        let Some(flags) = parts.next() else {
            continue;
        };
        let Some(name) = parts.next() else {
            continue;
        };
        // A line that explains a flag says what the flag means.
        if name == "=" {
            continue;
        }
        if !is_flags(flags) {
            continue;
        }
        names.push((flags.to_string(), name.to_string()));
    }
    names
}

/// Whether a word is a column of flags.
///
/// The flags are letters, dots and the bar that marks a source or a sink.
fn is_flags(word: &str) -> bool {
    !word.is_empty()
        && word
            .chars()
            .all(|character| character.is_ascii_uppercase() || character == '.' || character == '|')
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    const CODECS: &str = "\
Encoders:
 V..... = Video
 ------
 V....D mjpeg                MJPEG (Motion JPEG)
 V....D png                 PNG (Portable Network Graphics) image
 V....D libwebp              libwebp WebP
 V....D apng                 APNG (Animated PNG)
 V....D webp_anim            WebP animation
 A....D aac                  AAC (Advanced Audio Coding)
";

    const FORMATS: &str = "\
 File formats:
  D. = Demuxing supported
  .E = Muxing supported
 ---
 DE image2pipe  Piped image2 sequence
 DE image2       image2 sequence
 D  png_pipe     Piped PNG sequence
 D  apng         Animated PNG sequence
 D  webp_anim    WebP animation
 E  webp         WebP
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

    const FILTERS: &str = "\
Filters:
  T.. = Timeline support
  | = Source or sink filter
 ... abench            A->A       Benchmark part of a filtergraph.
 ... format            ->V        Convert the input image to the pixel format.
 ... overlay           VV->V      Overlay a video source on top of the input.
 ... scale             ->V        Scale the input image.
 ... split             ->V        Pass on the source to the output.
";

    #[test]
    fn the_protocols_are_read() {
        // This listing is not in the shape the other listings are in: it has
        // an "Input:" and an "Output:" section, and the names are indented
        // under them.
        let listing = "\
Supported file protocols:
Input:
  file
  pipe
Output:
  file
  pipe
";
        let names = protocols(listing);
        assert!(names.contains("file"), "{names:?}");
        assert!(names.contains("pipe"), "{names:?}");
        assert_eq!(names.len(), 2, "{names:?}");
    }

    #[test]
    fn a_program_without_the_pipe_cannot_convert_anything() {
        let mut capabilities = Capabilities {
            tools: Tools {
                ffmpeg: PathBuf::from("ffmpeg"),
                ffprobe: PathBuf::from("ffprobe"),
            },
            ..Capabilities::default()
        };
        let manifest = Manifest::get();
        let png = manifest.format("png").expect("png is in the manifest");
        // With no pipe, nothing can be read, whatever parts are there.
        capabilities.encoders.insert("png".to_string());
        capabilities.decoders.insert("png".to_string());
        capabilities.demuxers.insert("png_pipe".to_string());
        capabilities.muxers.insert("image2pipe".to_string());
        capabilities.filters.insert("format".to_string());
        capabilities.filters.insert("scale".to_string());
        assert!(!capabilities.can_read(png));
        assert!(!capabilities.can_write(png));
        // With the pipe and the file, it can.
        capabilities.protocols.insert("pipe".to_string());
        capabilities.protocols.insert("file".to_string());
        assert!(capabilities.can_read(png));
        assert!(capabilities.can_write(png));
    }

    #[test]
    fn the_encoders_are_read() {
        let names = parse_listing(CODECS);
        // In this listing the flags say other things, so every name in it
        // is a part that is there.
        let encoders: Vec<&str> = names.iter().map(|(_, name)| name.as_str()).collect();
        assert_eq!(
            encoders,
            vec!["mjpeg", "png", "libwebp", "apng", "webp_anim", "aac"]
        );
    }

    #[test]
    fn the_demuxers_and_the_muxers_are_told_apart() {
        let names = parse_listing(FORMATS);
        let demuxers: Vec<&str> = names
            .iter()
            .filter(|(flags, _)| flags.contains('D'))
            .map(|(_, name)| name.as_str())
            .collect();
        let muxers: Vec<&str> = names
            .iter()
            .filter(|(flags, _)| flags.contains('E'))
            .map(|(_, name)| name.as_str())
            .collect();
        assert_eq!(
            demuxers,
            vec!["image2pipe", "image2", "png_pipe", "apng", "webp_anim"]
        );
        assert_eq!(muxers, vec!["image2pipe", "image2", "webp"]);
    }

    #[test]
    fn the_filters_are_read_and_the_legend_is_not() {
        let names = parse_listing(FILTERS);
        let filters: Vec<&str> = names.iter().map(|(_, name)| name.as_str()).collect();
        assert_eq!(
            filters,
            vec!["abench", "format", "overlay", "scale", "split"]
        );
    }

    #[test]
    fn a_word_that_is_not_a_column_of_flags_is_not_a_name() {
        // The header of a listing is not a name.
        let names = parse_listing("Encoders:\n");
        assert!(names.is_empty());
    }

    /// A runner that answers with whatever it is given.
    struct Scripted {
        answers: Vec<(String, String)>,
    }

    impl ProcessRunner for Scripted {
        fn run(
            &self,
            program: &std::path::Path,
            args: &[OsString],
            _stdout_to: Option<&std::fs::File>,
        ) -> io::Result<RunResult> {
            let last = args.last().map(|arg| arg.to_string_lossy().into_owned());
            let answer = last
                .and_then(|what| {
                    self.answers
                        .iter()
                        .find(|(asked, _)| asked == &what)
                        .map(|(_, text)| text.clone())
                })
                .unwrap_or_default();
            let _ = program;
            Ok(RunResult {
                status: Some(0),
                stdout: answer.into_bytes(),
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

    #[test]
    fn the_environment_says_what_can_be_read_and_written() {
        let runner = Scripted {
            answers: vec![
                ("-version".to_string(), "ffmpeg version 9.0.2".to_string()),
                ("-encoders".to_string(), CODECS.to_string()),
                ("-decoders".to_string(), CODECS.to_string()),
                ("-formats".to_string(), FORMATS.to_string()),
                ("-filters".to_string(), FILTERS.to_string()),
                ("-protocols".to_string(), PROTOCOLS.to_string()),
            ],
        };
        let capabilities = Capabilities::probe(&tools(), &runner).expect("the program answered");
        assert!(capabilities.ffprobe_usable);
        let environment = capabilities.environment();
        let processor = environment
            .processor("image")
            .expect("there is an image processor");
        // PNG can be read and written.
        assert!(processor.can_read("png"));
        assert!(processor.can_write("png"));
        // JPEG cannot be read here, because the jpeg_pipe demuxer is not
        // in this listing.
        assert!(!processor.can_read("jpg"));
    }

    #[test]
    fn a_format_that_cannot_be_counted_is_not_offered() {
        // This build has no apng demuxer, so a PNG cannot be checked for
        // animation, and so cannot be offered.
        let without_apng = "\
 File formats:
  D. = Demuxing supported
  .E = Muxing supported
 ---
 D  png_pipe     Piped PNG sequence
 D  image2pipe  Piped image2 sequence
 E  image2pipe  Piped image2 sequence
";
        let runner = Scripted {
            answers: vec![
                ("-version".to_string(), "ffmpeg version 9.0.2".to_string()),
                ("-encoders".to_string(), CODECS.to_string()),
                ("-decoders".to_string(), CODECS.to_string()),
                ("-formats".to_string(), without_apng.to_string()),
                ("-filters".to_string(), FILTERS.to_string()),
                ("-protocols".to_string(), PROTOCOLS.to_string()),
            ],
        };
        let capabilities = Capabilities::probe(&tools(), &runner).expect("the program answered");
        let manifest = Manifest::get();
        let png = manifest.format("png").expect("png is in the manifest");
        assert!(!capabilities.can_count_frames(png));
        // And so the format is not among the ones that can be read.
        let environment = capabilities.environment();
        let processor = environment
            .processor("image")
            .expect("there is a processor");
        assert!(!processor.can_read("png"));
    }
}
