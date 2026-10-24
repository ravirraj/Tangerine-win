//! Document, subtitle, and archive conversions.
//!
//! These are handled in Rust without FFmpeg: subtitle format conversion
//! (SRT, VTT, TXT) and archive format conversion (ZIP, TAR, GZ).

use std::fs;
use std::io::{Cursor, Read, Write};
use std::path::Path;

use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use tar::Archive;
use tar::Builder;
use tar::Header;
use zip::ZipArchive;
use zip::ZipWriter;

/// Why a document conversion did not happen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DocError {
    /// The source format is not supported for conversion.
    UnsupportedSource,
    /// The target format is not supported for conversion.
    UnsupportedTarget,
    /// The source file could not be read.
    ReadError,
    /// The result could not be written.
    WriteError,
    /// The file is not a valid archive.
    InvalidArchive,
    /// The PDF reader is not installed.
    PdfLibraryMissing,
    /// A page of a PDF could not be rendered.
    RenderFailed,
}

impl DocError {
    pub fn name(&self) -> &'static str {
        match self {
            DocError::UnsupportedSource => "the source format is not supported",
            DocError::UnsupportedTarget => "the target format is not supported",
            DocError::ReadError => "the source file could not be read",
            DocError::WriteError => "the result could not be written",
            DocError::InvalidArchive => "the file is not a valid archive",
            DocError::PdfLibraryMissing => "the PDF reader is not installed",
            DocError::RenderFailed => "a page could not be rendered",
        }
    }
}

/// Subtitle format conversions: SRT, VTT, TXT.
pub mod subtitles {
    /// Convert subtitle content from one format to another.
    pub fn convert(content: &str, from: &str, to: &str) -> Result<String, super::DocError> {
        let entries = parse_subtitles(content, from)?;
        render_subtitles(&entries, to)
    }

    /// Parse subtitle content into a list of (start, end, text) entries.
    fn parse_subtitles(content: &str, from: &str) -> Result<Vec<(String, String, String)>, super::DocError> {
        match from {
            "srt" => parse_srt(content),
            "vtt" => parse_vtt(content),
            "txt" => parse_txt(content),
            _ => Err(super::DocError::UnsupportedSource),
        }
    }

    /// Render subtitle entries into the target format.
    fn render_subtitles(entries: &[(String, String, String)], to: &str) -> Result<String, super::DocError> {
        match to {
            "srt" => Ok::<String, super::DocError>(render_srt(entries)),
            "vtt" => Ok::<String, super::DocError>(render_vtt(entries)),
            "txt" => Ok::<String, super::DocError>(render_txt(entries)),
            _ => Err(super::DocError::UnsupportedTarget),
        }
    }

    // ----- SRT -----

    fn parse_srt(content: &str) -> Result<Vec<(String, String, String)>, super::DocError> {
        let mut entries = Vec::new();
        let blocks: Vec<&str> = content.split("\n\n").collect();
        for block in blocks {
            let lines: Vec<&str> = block.lines().collect();
            if lines.len() < 3 {
                continue;
            }
            // Line 0: index, Line 1: timestamps, Line 2+: text
            let timestamps = lines[1];
            let text = lines[2..].join("\n");
            let parts: Vec<&str> = timestamps.split(" --> ").collect();
            if parts.len() != 2 {
                continue;
            }
            entries.push((parts[0].trim().to_string(), parts[1].trim().to_string(), text));
        }
        Ok(entries)
    }

    fn render_srt(entries: &[(String, String, String)]) -> String {
        let mut out = String::new();
        for (i, (start, end, text)) in entries.iter().enumerate() {
            out.push_str(&format!("{}\n{} --> {}\n{}\n\n", i + 1, start, end, text));
        }
        out
    }

    // ----- VTT -----

    fn parse_vtt(content: &str) -> Result<Vec<(String, String, String)>, super::DocError> {
        let mut entries = Vec::new();
        let content = content.strip_prefix("WEBVTT").unwrap_or(content);
        let blocks: Vec<&str> = content.split("\n\n").collect();
        for block in blocks {
            let lines: Vec<&str> = block.lines().collect();
            if lines.len() < 2 {
                continue;
            }
            // Find the line with "-->"
            let mut timestamp_line = None;
            let mut text_start = 0;
            for (i, line) in lines.iter().enumerate() {
                if line.contains("-->") {
                    timestamp_line = Some(line);
                    text_start = i + 1;
                    break;
                }
            }
            let Some(ts) = timestamp_line else { continue };
            let parts: Vec<&str> = ts.split(" --> ").collect();
            if parts.len() != 2 {
                continue;
            }
            let text = lines[text_start..].join("\n");
            entries.push((parts[0].trim().to_string(), parts[1].trim().to_string(), text));
        }
        Ok(entries)
    }

    fn render_vtt(entries: &[(String, String, String)]) -> String {
        let mut out = String::from("WEBVTT\n\n");
        for (start, end, text) in entries {
            out.push_str(&format!("{} --> {}\n{}\n\n", start, end, text));
        }
        out
    }

    // ----- TXT -----

    fn parse_txt(content: &str) -> Result<Vec<(String, String, String)>, super::DocError> {
        let mut entries = Vec::new();
        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            entries.push(("00:00:00".to_string(), "00:00:00".to_string(), line.to_string()));
        }
        Ok(entries)
    }

    fn render_txt(entries: &[(String, String, String)]) -> String {
        entries.iter().map(|(_, _, text)| text.clone()).collect::<Vec<_>>().join("\n")
    }
}

/// Archive format conversions: ZIP, TAR, GZ.
pub mod archives {
    use super::DocError;
    use std::fs;
    use std::fs::File;
    use std::io::{Cursor, Read, Write};
    use std::path::Path;

    use flate2::read::GzDecoder;
    use flate2::write::GzEncoder;
    use tar::Archive;
    use tar::Builder;
    use tar::Header;
    use zip::ZipArchive;
    use zip::ZipWriter;

    /// Convert an archive from one format to another.
    ///
    /// The source archive is extracted to memory and re-packed into the
    /// target format.
    pub fn convert(source: &Path, target_path: &Path, from: &str, to: &str) -> Result<(), DocError> {
        let data = fs::read(source).map_err(|_| DocError::ReadError)?;
        let files = extract_archive(&data, from)?;
        repack_archive(&files, target_path, to)
    }

    /// Extract all files from an archive into a list of (name, bytes).
    fn extract_archive(data: &[u8], from: &str) -> Result<Vec<(String, Vec<u8>)>, DocError> {
        match from {
            "zip" => extract_zip(data),
            "tar" => extract_tar(data),
            "gz" => extract_gz(data),
            _ => Err(DocError::UnsupportedSource),
        }
    }

    /// Repack files into an archive.
    fn repack_archive(files: &[(String, Vec<u8>)], target_path: &Path, to: &str) -> Result<(), DocError> {
        match to {
            "zip" => write_zip(files, target_path),
            "tar" => write_tar(files, target_path),
            "gz" => write_gz(files, target_path),
            _ => Err(DocError::UnsupportedTarget),
        }
    }

    // ----- ZIP -----

    fn extract_zip(data: &[u8]) -> Result<Vec<(String, Vec<u8>)>, DocError> {
        let mut files = Vec::new();
        let reader = std::io::Cursor::new(data);
        let mut archive = zip::ZipArchive::new(reader).map_err(|_| DocError::InvalidArchive)?;
        for i in 0..archive.len() {
            let mut file = archive.by_index(i).map_err(|_| DocError::InvalidArchive)?;
            let name = file.name().to_string();
            if file.is_dir() {
                continue;
            }
            let mut bytes = Vec::new();
            file.read_to_end(&mut bytes).map_err(|_| DocError::ReadError)?;
            files.push((name, bytes));
        }
        Ok(files)
    }

    fn write_zip(files: &[(String, Vec<u8>)], target_path: &Path) -> Result<(), DocError> {
        let file = File::create(target_path).map_err(|_| DocError::WriteError)?;
        let mut writer = zip::ZipWriter::new(file);
        for (name, bytes) in files {
            writer
                .start_file(name, zip::write::FileOptions::default())
                .map_err(|_| DocError::WriteError)?;
            writer.write_all(bytes).map_err(|_| DocError::WriteError)?;
        }
        writer.finish().map_err(|_| DocError::WriteError)?;
        Ok(())
    }

    // ----- TAR -----

    fn extract_tar(data: &[u8]) -> Result<Vec<(String, Vec<u8>)>, DocError> {
        let mut files = Vec::new();
        let mut archive = tar::Archive::new(std::io::Cursor::new(data));
        for entry in archive.entries().map_err(|_| DocError::InvalidArchive)? {
            let mut entry = entry.map_err(|_| DocError::InvalidArchive)?;
            if entry.header().entry_type().is_dir() {
                continue;
            }
            let name = entry.path().map_err(|_| DocError::InvalidArchive)?.to_string_lossy().to_string();
            let mut bytes = Vec::new();
            entry.read_to_end(&mut bytes).map_err(|_| DocError::ReadError)?;
            files.push((name, bytes));
        }
        Ok(files)
    }

    fn write_tar(files: &[(String, Vec<u8>)], target_path: &Path) -> Result<(), DocError> {
        let file = File::create(target_path).map_err(|_| DocError::WriteError)?;
        let mut builder = tar::Builder::new(file);
        for (name, bytes) in files {
            let mut header = tar::Header::new_old();
            header.set_size(bytes.len() as u64);
            header.set_path(name).map_err(|_| DocError::WriteError)?;
            builder.append(&header, std::io::Cursor::new(bytes)).map_err(|_| DocError::WriteError)?;
        }
        builder.finish().map_err(|_| DocError::WriteError)?;
        Ok(())
    }

    // ----- GZ -----

    fn extract_gz(data: &[u8]) -> Result<Vec<(String, Vec<u8>)>, DocError> {
        // GZ is a single-file compressed stream. Treat the whole content as one file.
        let mut decoder = flate2::read::GzDecoder::new(data);
        let mut bytes = Vec::new();
        decoder.read_to_end(&mut bytes).map_err(|_| DocError::InvalidArchive)?;
        Ok(vec![("extracted".to_string(), bytes)])
    }

    fn write_gz(files: &[(String, Vec<u8>)], target_path: &Path) -> Result<(), DocError> {
        // GZ holds one file. If multiple files, concatenate them.
        let mut all_bytes = Vec::new();
        for (_, bytes) in files {
            all_bytes.extend_from_slice(bytes);
        }
        let file = File::create(target_path).map_err(|_| DocError::WriteError)?;
        let mut encoder = flate2::write::GzEncoder::new(file, flate2::Compression::default());
        encoder.write_all(&all_bytes).map_err(|_| DocError::WriteError)?;
        encoder.finish().map_err(|_| DocError::WriteError)?;
        Ok(())
    }
}

/// Convert a document file from one format to another.
///
/// Subtitles are converted in memory; archives are extracted and re-packed.
pub fn convert(source: &Path, target_path: &Path, from: &str, to: &str) -> Result<(), DocError> {
    match from {
        "srt" | "vtt" | "txt" => {
            let content = std::fs::read_to_string(source).map_err(|_| DocError::ReadError)?;
            let result = subtitles::convert(&content, from, to)?;
            std::fs::write(target_path, result).map_err(|_| DocError::WriteError)?;
            Ok(())
        }
        "zip" | "tar" | "gz" => {
            archives::convert(source, target_path, from, to)
        }
        "pdf" => convert_pdf(source, target_path, to),
        _ => Err(DocError::UnsupportedSource),
    }
}

/// Converts a PDF into pictures or text, beside the original.
///
/// Every page becomes a picture of its own. The first page takes the
/// target path, and each page after it takes the same name with a number.
/// A name that is already taken is not used — the next number is tried —
/// so nothing that is there already is ever overwritten.
fn convert_pdf(source: &Path, target_path: &Path, to: &str) -> Result<(), DocError> {
    use crate::media::pdf;

    if to == "txt" {
        let text = pdf::extract_text(source).map_err(pdf_error)?;
        std::fs::write(target_path, text).map_err(|_| DocError::WriteError)?;
        return Ok(());
    }
    let Some(encoding) = pdf::Encoding::of_format(to) else {
        return Err(DocError::UnsupportedTarget);
    };
    // The pages are rendered at 300 dots to the inch, the way the
    // reference product renders them.
    let pages = pdf::render_pages(source, 300, encoding).map_err(pdf_error)?;

    let folder = target_path.parent().unwrap_or_else(|| Path::new("."));
    let stem = target_path
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_else(|| "converted".to_string());
    let extension = target_path
        .extension()
        .map(|extension| extension.to_string_lossy().into_owned())
        .unwrap_or_default();

    let mut number = 1usize;
    for page in &pages {
        let name = loop {
            let candidate = if number == 1 {
                target_path.to_path_buf()
            } else {
                folder.join(format!("{stem} ({number}).{extension}"))
            };
            if !candidate.exists() {
                break candidate;
            }
            number += 1;
        };
        std::fs::write(&name, page).map_err(|_| DocError::WriteError)?;
        number += 1;
    }
    Ok(())
}

/// Says a PDF failure the way the documents say theirs.
fn pdf_error(reason: crate::media::pdf::PdfError) -> DocError {
    match reason {
        crate::media::pdf::PdfError::LibraryMissing => DocError::PdfLibraryMissing,
        crate::media::pdf::PdfError::NotAPdf => DocError::ReadError,
        crate::media::pdf::PdfError::RenderFailed
        | crate::media::pdf::PdfError::TextFailed => DocError::RenderFailed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn srt_to_vtt_conversion() {
        let srt = "1\n00:00:01,000 --> 00:00:02,000\nHello world\n\n2\n00:00:03,000 --> 00:00:04,000\nGoodbye\n";
        let vtt = subtitles::convert(srt, "srt", "vtt").expect("srt to vtt");
        assert!(vtt.starts_with("WEBVTT"));
        assert!(vtt.contains("Hello world"));
        assert!(vtt.contains("Goodbye"));
    }

    #[test]
    fn vtt_to_srt_conversion() {
        let vtt = "WEBVTT\n\n00:00:01.000 --> 00:00:02.000\nHello\n";
        let srt = subtitles::convert(vtt, "vtt", "srt").expect("vtt to srt");
        assert!(srt.contains("Hello"));
        assert!(srt.contains("-->"));
    }

    #[test]
    fn txt_to_srt_conversion() {
        let txt = "Hello\nGoodbye\n";
        let srt = subtitles::convert(txt, "txt", "srt").expect("txt to srt");
        assert!(srt.contains("Hello"));
        assert!(srt.contains("Goodbye"));
    }

    #[test]
    fn unsupported_source_is_rejected() {
        let result = subtitles::convert("hello", "xyz", "srt");
        assert_eq!(result.unwrap_err(), DocError::UnsupportedSource);
    }

    #[test]
    fn unsupported_target_is_rejected() {
        let result = subtitles::convert("hello", "srt", "xyz");
        assert_eq!(result.unwrap_err(), DocError::UnsupportedTarget);
    }
}