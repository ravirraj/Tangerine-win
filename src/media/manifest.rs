//! The component manifest: what the FFmpeg build is made of.
//!
//! The file `ffmpeg/components.txt` is the single source of truth for the
//! build. The build script generates the configure flags and the
//! post-build assertions from it, and this module reads it — embedded in
//! the program when it is built — to learn the names of the parts and how
//! they fit together. They cannot disagree, because they are the same file.
//!
//! The manifest is small and is read once.

use std::sync::OnceLock;

/// The manifest, as it was when the program was built.
///
/// It is embedded rather than read from disk, so a program built without it
/// does not compile, and a program that runs has the manifest that matches
/// the code.
const TEXT: &str = include_str!("../../ffmpeg/components.txt");

/// Whether a format can hold a transparent area.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Alpha {
    /// The format keeps the transparency of the original.
    Keep,
    /// The format cannot hold transparency, so one is painted in.
    Matte,
}

/// A demuxer and the decoder that goes with it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Demux {
    /// The name of the demuxer, as FFmpeg knows it.
    pub demuxer: String,
    /// The name of the decoder, as FFmpeg knows it.
    pub decoder: String,
}

/// An encoder and the muxer that goes with it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mux {
    /// The name of the encoder, as FFmpeg knows it.
    pub encoder: String,
    /// The name of the muxer, as FFmpeg knows it.
    pub muxer: String,
}

/// One piece of the beginning of a file of a format.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Signature {
    /// How far into the file the bytes are.
    pub offset: usize,
    /// The bytes themselves.
    pub bytes: Vec<u8>,
}

/// What is known about one format.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FormatSupport {
    /// The name of the format, as the registry names it: `png`, `jpeg`.
    pub format: String,
    /// The extensions a file of this format is usually called by.
    pub extensions: Vec<String>,
    /// How a file of this format is opened.
    pub read: Demux,
    /// How a file of this format is produced.
    pub write: Mux,
    /// Whether the format can hold a transparent area.
    pub alpha: Alpha,
    /// The demuxer and decoder that can see every frame of the format.
    ///
    /// There is none for a format that cannot be animated.
    pub animation: Option<Demux>,
    /// The demuxer and decoder that proves a file is a single image.
    ///
    /// This is only needed where the animation demuxer rejects still files,
    /// so that a rejection is never taken for proof on its own.
    pub confirm: Option<Demux>,
    /// The ways a file of this format may begin.
    ///
    /// A file matches when it matches one of them, and a match means every
    /// piece of that one is there.
    pub magic: Vec<Vec<Signature>>,
}

impl FormatSupport {
    /// The demuxer and decoder that can see every frame of the format.
    ///
    /// There is none for a format that cannot be animated.
    pub fn animation_parts(&self) -> Option<&Demux> {
        self.animation.as_ref()
    }

    /// Whether these bytes are the beginning of a file of this format.
    pub fn matches(&self, bytes: &[u8]) -> bool {
        self.magic.iter().any(|alternative| {
            alternative.iter().all(|part| {
                bytes.len() >= part.offset + part.bytes.len()
                    && &bytes[part.offset..part.offset + part.bytes.len()] == part.bytes.as_slice()
            })
        })
    }
}

/// An external library that is compiled into the program.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Library {
    /// The name of the library.
    pub name: String,
    /// The licence it carries.
    pub licence: String,
    /// The version the build image pins, or `pinned` when it is not known
    /// yet.
    pub version: String,
}

/// The whole manifest.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Manifest {
    /// The programs the build produces.
    pub programs: Vec<String>,
    /// The protocols the conversion needs.
    pub protocols: Vec<String>,
    /// The filters the conversion needs.
    pub filters: Vec<String>,
    /// The formats that can be worked with.
    pub formats: Vec<FormatSupport>,
    /// The encoders that only the test suite needs.
    pub test_encoders: Vec<String>,
    /// The decoders that only the test suite needs.
    pub test_decoders: Vec<String>,
    /// The demuxers that only the test suite needs.
    pub test_demuxers: Vec<String>,
    /// The muxers that only the test suite needs.
    pub test_muxers: Vec<String>,
    /// The muxers that are kept for a later phase.
    pub reserved_muxers: Vec<String>,
    /// The external libraries that are compiled in.
    pub libraries: Vec<Library>,
}

impl Manifest {
    /// The manifest, read once.
    pub fn get() -> &'static Manifest {
        static MANIFEST: OnceLock<Manifest> = OnceLock::new();
        MANIFEST.get_or_init(|| Manifest::parse(TEXT))
    }

    /// The format with this name.
    pub fn format(&self, name: &str) -> Option<&FormatSupport> {
        self.formats.iter().find(|format| format.format == name)
    }

    /// The format a file with this extension usually holds.
    ///
    /// The extension is what the file is *called*; what it holds is the
    /// business of the program that reads it.
    pub fn format_of_extension(&self, extension: &str) -> Option<&FormatSupport> {
        self.formats
            .iter()
            .find(|format| format.extensions.iter().any(|known| known == extension))
    }

    /// Reads the manifest.
    fn parse(text: &str) -> Manifest {
        let mut manifest = Manifest::default();
        let mut current: Option<FormatSupport> = None;

        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let key = key.trim();
            let value = value.trim();

            match key {
                "programs" => manifest.programs = words(value),
                "protocols" => manifest.protocols = words(value),
                "filters" => manifest.filters = words(value),
                "test-encoders" => manifest.test_encoders = words(value),
                "test-decoders" => manifest.test_decoders = words(value),
                "test-demuxers" => manifest.test_demuxers = words(value),
                "test-muxers" => manifest.test_muxers = words(value),
                "reserved-muxers" => manifest.reserved_muxers = words(value),
                "library" => manifest.libraries.push(Library {
                    name: value
                        .split_whitespace()
                        .next()
                        .unwrap_or_default()
                        .to_string(),
                    licence: field(value, "licence").unwrap_or_default(),
                    version: field(value, "version").unwrap_or_default(),
                }),
                "format" => {
                    if let Some(format) = current.take() {
                        manifest.formats.push(format);
                    }
                    current = Some(FormatSupport {
                        format: value.to_string(),
                        extensions: Vec::new(),
                        read: Demux {
                            demuxer: String::new(),
                            decoder: String::new(),
                        },
                        write: Mux {
                            encoder: String::new(),
                            muxer: String::new(),
                        },
                        alpha: Alpha::Matte,
                        animation: None,
                        confirm: None,
                        magic: Vec::new(),
                    });
                }
                "extensions" => {
                    if let Some(format) = current.as_mut() {
                        format.extensions = words(value);
                    }
                }
                "read" => {
                    if let Some(format) = current.as_mut() {
                        format.read = Demux {
                            demuxer: field(value, "demuxer").unwrap_or_default(),
                            decoder: field(value, "decoder").unwrap_or_default(),
                        };
                    }
                }
                "write" => {
                    if let Some(format) = current.as_mut() {
                        format.write = Mux {
                            encoder: field(value, "encoder").unwrap_or_default(),
                            muxer: field(value, "muxer").unwrap_or_default(),
                        };
                    }
                }
                "alpha" => {
                    if let Some(format) = current.as_mut() {
                        format.alpha = if value == "keep" {
                            Alpha::Keep
                        } else {
                            Alpha::Matte
                        };
                    }
                }
                "animation" => {
                    if let Some(format) = current.as_mut() {
                        format.animation = demux(value);
                    }
                }
                "confirm" => {
                    if let Some(format) = current.as_mut() {
                        format.confirm = demux(value);
                    }
                }
                "magic" => {
                    if let Some(format) = current.as_mut() {
                        format.magic = magic(value);
                    }
                }
                _ => {}
            }
        }
        if let Some(format) = current {
            manifest.formats.push(format);
        }
        manifest
    }
}

/// The words of a value, with the punctuation taken off.
fn words(value: &str) -> Vec<String> {
    value
        .split_whitespace()
        .map(|word| word.trim_end_matches(',').to_string())
        .filter(|word| !word.is_empty() && word != "-")
        .collect()
}

/// One field of a value, such as `muxer=image2pipe`.
fn field(value: &str, name: &str) -> Option<String> {
    value.split_whitespace().find_map(|word| {
        let (key, found) = word.split_once('=')?;
        if key == name {
            Some(found.to_string())
        } else {
            None
        }
    })
}

/// A demuxer and a decoder, or nothing when the value says "-".
fn demux(value: &str) -> Option<Demux> {
    let demuxer = field(value, "demuxer")?;
    if demuxer == "-" {
        return None;
    }
    Some(Demux {
        demuxer,
        decoder: field(value, "decoder").unwrap_or_default(),
    })
}

/// The ways a file of a format may begin.
///
/// Each way is a list of pieces, and a file matches when every piece of
/// one of them is there.
fn magic(value: &str) -> Vec<Vec<Signature>> {
    value
        .split('|')
        .map(|alternative| {
            alternative
                .split_whitespace()
                .filter_map(|piece| {
                    let (offset, bytes) = piece.split_once(':')?;
                    let offset = offset.parse::<usize>().ok()?;
                    let bytes: Vec<u8> = bytes
                        .split_whitespace()
                        .filter_map(|byte| u8::from_str_radix(byte, 16).ok())
                        .collect();
                    if bytes.is_empty() {
                        None
                    } else {
                        Some(Signature { offset, bytes })
                    }
                })
                .collect()
        })
        .filter(|alternative: &Vec<Signature>| !alternative.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_manifest_is_read() {
        let manifest = Manifest::get();
        assert_eq!(manifest.programs, vec!["ffmpeg", "ffprobe"]);
        assert_eq!(manifest.protocols, vec!["file", "pipe"]);
        assert!(manifest.filters.contains(&"overlay".to_string()));
    }

    #[test]
    fn every_format_is_read_with_its_parts() {
        let manifest = Manifest::get();
        let png = manifest.format("png").expect("png is in the manifest");
        assert_eq!(png.read.demuxer, "png_pipe");
        assert_eq!(png.read.decoder, "png");
        assert_eq!(png.write.encoder, "png");
        assert_eq!(png.write.muxer, "image2pipe");
        assert_eq!(png.alpha, Alpha::Keep);
        assert!(png.animation_parts().is_some());
    }

    #[test]
    fn a_format_that_cannot_be_animated_says_so() {
        let manifest = Manifest::get();
        let jpeg = manifest.format("jpeg").expect("jpeg is in the manifest");
        assert!(jpeg.animation_parts().is_none());
        assert_eq!(jpeg.alpha, Alpha::Matte);
    }

    #[test]
    fn a_format_is_found_by_the_extension_it_is_called_by() {
        let manifest = Manifest::get();
        assert_eq!(
            manifest
                .format_of_extension("jpg")
                .map(|f| f.format.as_str()),
            Some("jpeg")
        );
        assert_eq!(
            manifest
                .format_of_extension("JPEG")
                .map(|f| f.format.as_str()),
            None
        );
        assert_eq!(
            manifest
                .format_of_extension("tif")
                .map(|f| f.format.as_str()),
            Some("tiff")
        );
    }

    #[test]
    fn the_libraries_are_read_with_their_licences() {
        let manifest = Manifest::get();
        let zlib = manifest
            .libraries
            .iter()
            .find(|library| library.name == "zlib")
            .expect("zlib is in the manifest");
        assert_eq!(zlib.licence, "zlib");
        let webp = manifest
            .libraries
            .iter()
            .find(|library| library.name == "libwebp")
            .expect("libwebp is in the manifest");
        assert_eq!(webp.licence, "BSD-3-Clause");
    }

    #[test]
    fn every_part_the_manifest_names_is_counted() {
        let manifest = Manifest::get();
        let mut names: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        for format in &manifest.formats {
            names.insert(format.read.demuxer.clone());
            names.insert(format.read.decoder.clone());
            names.insert(format.write.encoder.clone());
            names.insert(format.write.muxer.clone());
            if let Some(animation) = format.animation_parts() {
                names.insert(animation.demuxer.clone());
                names.insert(animation.decoder.clone());
            }
            if let Some(confirm) = &format.confirm {
                names.insert(confirm.demuxer.clone());
                names.insert(confirm.decoder.clone());
            }
        }
        names.extend(manifest.test_encoders.iter().cloned());
        names.extend(manifest.test_muxers.iter().cloned());
        names.extend(manifest.reserved_muxers.iter().cloned());
        // The parts the conversion needs, and the ones the tests need.
        for expected in [
            "png_pipe",
            "png",
            "image2pipe",
            "apng",
            "webp_anim",
            "libwebp",
            "gif",
            "tiff_pipe",
            "rawvideo",
        ] {
            assert!(names.contains(expected), "{expected} is not named");
        }
    }
}
