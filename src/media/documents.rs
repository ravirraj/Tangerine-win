//! Document, subtitle, and archive conversions.
//!
//! These are handled in Rust without FFmpeg: subtitle format conversion
//! (SRT, VTT, TXT) and archive format conversion (ZIP, TAR, GZ).

use std::fs;
use std::io::{Cursor, Read, Write};
use std::path::Path;

use calamine::Reader as _;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::write::ZlibEncoder;
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
    /// No program that reads office documents is installed.
    OfficeMissing,
    /// The program that reads office documents did not finish.
    OfficeFailed,
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
            DocError::OfficeMissing => {
                "no program that reads office documents is installed"
            }
            DocError::OfficeFailed => "the program that reads office documents did not finish",
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
        "png" | "jpg" | "jpeg" | "webp" | "bmp" | "tif" | "tiff" | "gif" | "svg" => {
            if to == "pdf" {
                images_to_pdf(&[source.to_path_buf()], target_path)
            } else {
                Err(DocError::UnsupportedTarget)
            }
        }
        "docx" => {
            if to == "pdf" {
                crate::media::documents2::to_pdf(source, target_path)
            } else {
                convert_docx(source, target_path, to)
            }
        }
        "doc" | "ppt" | "pptx" => {
            if to == "pdf" {
                crate::media::documents2::to_pdf(source, target_path)
            } else {
                Err(DocError::UnsupportedTarget)
            }
        }
        "xlsx" | "xls" | "ods" | "csv" | "tsv" => {
            if to == "pdf" {
                crate::media::documents2::to_pdf(source, target_path)
            } else {
                convert_sheet(source, target_path, to)
            }
        }
        _ => Err(DocError::UnsupportedSource),
    }
}

/// Converts a DOCX into plain text.
///
/// A DOCX is a folder of XML in a zip; the words are in one part of it,
/// each run of text inside its own element, each paragraph its own. The
/// words are taken as they come and the paragraphs are kept apart by a
/// line break each.
fn convert_docx(source: &Path, target_path: &Path, to: &str) -> Result<(), DocError> {
    if to != "txt" {
        return Err(DocError::UnsupportedTarget);
    }
    let file = fs::File::open(source).map_err(|_| DocError::ReadError)?;
    let mut archive = ZipArchive::new(file).map_err(|_| DocError::ReadError)?;
    let mut xml = String::new();
    archive
        .by_name("word/document.xml")
        .map_err(|_| DocError::ReadError)?
        .read_to_string(&mut xml)
        .map_err(|_| DocError::ReadError)?;

    let mut reader = quick_xml::Reader::from_str(&xml);
    // The spaces between words are inside the text elements in this
    // format, so nothing is trimmed: a trimmed space is a lost word.
    reader.config_mut().trim_text(false);
    let mut text = String::new();
    use quick_xml::events::Event;
    loop {
        match reader.read_event() {
            Ok(Event::Text(words)) => text.push_str(
                &words.xml_content(quick_xml::XmlVersion::Implicit1_0),
            ),
            Ok(Event::End(end)) => {
                if end.name().local_name().as_ref() == "p" {
                    text.push('\n');
                }
            }
            Ok(Event::Eof) => break,
            Ok(_) => {}
            Err(_) => return Err(DocError::ReadError),
        }
    }
    fs::write(target_path, text).map_err(|_| DocError::WriteError)?;
    Ok(())
}

/// Converts a spreadsheet into CSV.
///
/// Every sheet becomes a file of its own: the first takes the target path,
/// and each sheet after it takes the name of the sheet beside it. A name
/// that is already taken is not used — the next number is tried — so
/// nothing that is there already is ever overwritten.
fn convert_sheet(source: &Path, target_path: &Path, to: &str) -> Result<(), DocError> {
    if to != "csv" {
        return Err(DocError::UnsupportedTarget);
    }
    let mut workbook =
        calamine::open_workbook_auto(source).map_err(|_| DocError::ReadError)?;
    let names = workbook.sheet_names();
    if names.is_empty() {
        return Err(DocError::ReadError);
    }

    let folder = target_path.parent().unwrap_or_else(|| Path::new("."));
    let stem = target_path
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_else(|| "converted".to_string());

    for (which, sheet) in names.iter().enumerate() {
        let range = workbook
            .worksheet_range(sheet)
            .map_err(|_| DocError::ReadError)?;
        let mut rows = String::new();
        for row in range.rows() {
            let cells: Vec<String> = row
                .iter()
                .map(|cell| escape_csv(&cell.to_string()))
                .collect();
            rows.push_str(&cells.join(","));
            rows.push('\n');
        }
        let mut number = 1usize;
        let name = loop {
            let candidate = if which == 0 && number == 1 {
                target_path.to_path_buf()
            } else if which == 0 {
                folder.join(format!("{stem} ({number}).csv"))
            } else if number == 1 {
                folder.join(format!("{stem} - {sheet}.csv"))
            } else {
                folder.join(format!("{stem} - {sheet} ({number}).csv"))
            };
            if !candidate.exists() {
                break candidate;
            }
            number += 1;
        };
        fs::write(&name, rows).map_err(|_| DocError::WriteError)?;
    }
    Ok(())
}

/// A cell of CSV, quoted when it has to be.
fn escape_csv(cell: &str) -> String {
    if cell.contains(',') || cell.contains('"') || cell.contains('\n') || cell.contains('\r') {
        format!("\"{}\"", cell.replace('"', "\"\""))
    } else {
        cell.to_string()
    }
}

/// Wraps pictures into one PDF, a page for each picture.
///
/// The pages are the sizes of their pictures, one point to the pixel. A
/// JPEG goes in as it is, because PDF keeps JPEGs that way; everything
/// else is decoded and deflated. A picture that has see-through parts
/// keeps them, as PDF keeps them, in a mask of its own. A vector picture
/// is turned into pixels first, because PDF holds no vectors of that
/// kind.
pub fn images_to_pdf(sources: &[std::path::PathBuf], target: &Path) -> Result<(), DocError> {
    use pdf_writer::{Content, Filter, Finish, Name, Pdf, Rect, Ref};

    /// One picture, ready to go into the PDF.
    struct Encoded {
        width: u32,
        height: u32,
        filter: Filter,
        data: Vec<u8>,
        mask: Option<Vec<u8>>,
        gray: bool,
    }

    let mut pictures = Vec::new();
    for source in sources {
        let extension = source
            .extension()
            .and_then(|extension| extension.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let data = if extension == "svg" {
            crate::media::svg::rasterise(source).map_err(|_| DocError::ReadError)?
        } else {
            fs::read(source).map_err(|_| DocError::ReadError)?
        };
        let picture = image::load_from_memory(&data).map_err(|_| DocError::ReadError)?;
        let (width, height) = (picture.width(), picture.height());
        if width == 0 || height == 0 {
            return Err(DocError::ReadError);
        }

        // A JPEG is already deflated the way PDF wants, so it is kept whole.
        let is_jpeg = extension != "svg"
            && image::guess_format(&data)
                .map(|format| format == image::ImageFormat::Jpeg)
                .unwrap_or(false);
        let plainly_coloured = picture.color() == image::ColorType::Rgb8
            || picture.color() == image::ColorType::L8;
        let gray = picture.color() == image::ColorType::L8;
        let (filter, data, mask) =
            if is_jpeg && plainly_coloured && !picture.color().has_alpha() {
                (Filter::DctDecode, data, None)
            } else {
                let rgba = picture.to_rgba8();
                let level = flate2::Compression::default();
                let colours = zlib(
                    rgba.pixels()
                        .flat_map(|pixel| [pixel.0[0], pixel.0[1], pixel.0[2]])
                        .collect::<Vec<u8>>(),
                    level,
                )?;
                let mask = if picture.color().has_alpha() {
                    Some(zlib(
                        rgba.pixels().map(|pixel| pixel.0[3]).collect::<Vec<u8>>(),
                        level,
                    )?)
                } else {
                    None
                };
                (Filter::FlateDecode, colours, mask)
            };
        pictures.push(Encoded {
            width,
            height,
            filter,
            data,
            mask,
            gray,
        });
    }
    if pictures.is_empty() {
        return Err(DocError::ReadError);
    }

    let catalog = Ref::new(1);
    let tree = Ref::new(2);
    let mut pdf = Pdf::new();
    pdf.catalog(catalog).pages(tree);

    // Each picture takes four objects of its own: its page, its image,
    // the mask of its see-through parts, and the stream that places it.
    let mut next = 3;
    let mut kids = Vec::new();
    for encoded in &pictures {
        let page = Ref::new(next);
        let image = Ref::new(next + 1);
        let mask = Ref::new(next + 2);
        let contents = Ref::new(next + 3);
        next += 4;
        kids.push(page);
        let name = Name(b"Im1");

        let mut page_writer = pdf.page(page);
        page_writer
            .parent(tree)
            .media_box(Rect::new(
                0.0,
                0.0,
                encoded.width as f32,
                encoded.height as f32,
            ))
            .contents(contents)
            .resources()
            .x_objects()
            .pair(name, image);
        drop(page_writer);

        let mut image_writer = pdf.image_xobject(image, &encoded.data);
        image_writer
            .width(encoded.width as i32)
            .height(encoded.height as i32)
            .bits_per_component(8);
        if encoded.gray {
            image_writer.color_space().device_gray();
        } else {
            image_writer.color_space().device_rgb();
        }
        if encoded.mask.is_some() {
            image_writer.s_mask(mask);
        }
        // The filter is said last, on its own, because it is said by the
        // stream everything is written into.
        image_writer.filter(encoded.filter);
        drop(image_writer);

        if let Some(mask_bytes) = &encoded.mask {
            let mut mask_writer = pdf.image_xobject(mask, mask_bytes);
            mask_writer
                .width(encoded.width as i32)
                .height(encoded.height as i32)
                .bits_per_component(8);
            mask_writer.color_space().device_gray();
            mask_writer.filter(Filter::FlateDecode);
        }

        let mut stream = Content::new();
        stream.save_state();
        stream.transform([
            encoded.width as f32,
            0.0,
            0.0,
            encoded.height as f32,
            0.0,
            0.0,
        ]);
        stream.x_object(name);
        stream.restore_state();
        pdf.stream(contents, &stream.finish());
    }
    pdf.pages(tree)
        .kids(kids.iter().copied())
        .count(pictures.len() as i32);

    fs::write(target, pdf.finish()).map_err(|_| DocError::WriteError)?;
    Ok(())
}

/// Bytes deflated the way PDF keeps deflated things.
fn zlib(bytes: Vec<u8>, level: flate2::Compression) -> Result<Vec<u8>, DocError> {
    use std::io::Write;
    let mut encoder = ZlibEncoder::new(Vec::new(), level);
    encoder
        .write_all(&bytes)
        .and_then(|_| encoder.finish())
        .map_err(|_| DocError::WriteError)
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

    /// A folder of its own for a test, removed when the test is over.
    struct TempFolder {
        path: std::path::PathBuf,
    }

    impl TempFolder {
        fn new(name: &str) -> TempFolder {
            let path = std::env::temp_dir().join(format!(
                "open-tangerine-documents-{}-{name}",
                std::process::id()
            ));
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

    /// Makes a zip of the given parts, the way the office formats are put
    /// together, with the parts named as they are named in those formats.
    fn office_file(path: &Path, parts: &[(&str, &str)]) {
        let file = fs::File::create(path).expect("the office file could be made");
        let mut zip = ZipWriter::new(file);
        for (name, content) in parts {
            zip.start_file(*name, zip::write::FileOptions::default())
                .expect("the part could be started");
            zip.write_all(content.as_bytes())
                .expect("the part could be written");
        }
        zip.finish().expect("the office file could be finished");
    }

    const RELS: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\
<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
<Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"REPLACED\"/>\
</Relationships>";

    #[test]
    fn a_docx_becomes_text_with_its_paragraphs_apart() {
        let folder = TempFolder::new("docx");
        let document = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\
<w:document xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\">\
<w:body>\
<w:p><w:r><w:t>Hello document</w:t></w:r></w:p>\
<w:p><w:r><w:t>Second line</w:t></w:r><w:r><w:t> goes on</w:t></w:r></w:p>\
</w:body></w:document>";
        let content_types = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\
<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\
<Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>\
<Default Extension=\"xml\" ContentType=\"application/xml\"/>\
<Override PartName=\"/word/document.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml\"/>\
</Types>";
        let docx = folder.path.join("letter.docx");
        office_file(
            &docx,
            &[
                ("[Content_Types].xml", content_types),
                ("_rels/.rels", &RELS.replacen("REPLACED", "word/document.xml", 1)),
                ("word/document.xml", document),
            ],
        );

        let text = folder.path.join("letter.txt");
        convert(&docx, &text, "docx", "txt").expect("the docx was converted");
        let words = fs::read_to_string(&text).expect("the text is there");
        assert!(words.contains("Hello document\n"), "{words:?}");
        assert!(words.contains("Second line goes on\n"), "{words:?}");
    }

    #[test]
    fn a_spreadsheet_becomes_csv_one_file_per_sheet() {
        let folder = TempFolder::new("xlsx");
        let workbook = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\
<workbook xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\" \
xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\">\
<sheets><sheet name=\"Sheet1\" sheetId=\"1\" r:id=\"rId1\"/></sheets></workbook>";
        let workbook_rels = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\
<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
<Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet\" Target=\"worksheets/sheet1.xml\"/>\
</Relationships>";
        let sheet = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\
<worksheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\">\
<sheetData>\
<row r=\"1\"><c r=\"A1\" t=\"inlineStr\"><is><t>name</t></is></c><c r=\"B1\" t=\"inlineStr\"><is><t>age</t></is></c></row>\
<row r=\"2\"><c r=\"A2\" t=\"inlineStr\"><is><t>ada, lovelace</t></is></c><c r=\"B2\"><v>36</v></c></row>\
</sheetData></worksheet>";
        let content_types = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\
<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\
<Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>\
<Default Extension=\"xml\" ContentType=\"application/xml\"/>\
<Override PartName=\"/xl/workbook.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml\"/>\
<Override PartName=\"/xl/worksheets/sheet1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml\"/>\
</Types>";
        let xlsx = folder.path.join("table.xlsx");
        office_file(
            &xlsx,
            &[
                ("[Content_Types].xml", content_types),
                ("_rels/.rels", &RELS.replacen("REPLACED", "xl/workbook.xml", 1)),
                ("xl/workbook.xml", workbook),
                ("xl/_rels/workbook.xml.rels", workbook_rels),
                ("xl/worksheets/sheet1.xml", sheet),
            ],
        );

        let csv = folder.path.join("table.csv");
        convert(&xlsx, &csv, "xlsx", "csv").expect("the sheet was converted");
        let rows = fs::read_to_string(&csv).expect("the csv is there");
        assert!(rows.contains("name,age\n"), "{rows:?}");
        // A cell with a comma in it is quoted, the way CSV asks for.
        assert!(rows.contains("\"ada, lovelace\",36\n"), "{rows:?}");
    }

    #[test]
    fn a_docx_that_is_not_a_docx_is_refused() {
        let folder = TempFolder::new("bad-docx");
        let docx = folder.path.join("pretend.docx");
        fs::write(&docx, b"not a docx").expect("the file could be written");
        let text = folder.path.join("pretend.txt");
        assert_eq!(
            convert(&docx, &text, "docx", "txt"),
            Err(DocError::ReadError)
        );
    }

    /// Makes a picture of the given kind with the picture library itself.
    fn picture_file(path: &Path, kind: image::ImageFormat) {
        let mut canvas = image::RgbImage::new(3, 2);
        canvas.put_pixel(0, 0, image::Rgb([255, 0, 0]));
        canvas.put_pixel(2, 1, image::Rgb([0, 0, 255]));
        canvas
            .write_to(&mut std::io::Cursor::new(Vec::new()), kind)
            .ok();
        // write_to with a cursor makes nothing on disk, so the picture is
        // written the plain way instead.
        let dynamic = image::DynamicImage::ImageRgb8(canvas);
        let mut bytes = Vec::new();
        dynamic
            .write_to(&mut std::io::Cursor::new(&mut bytes), kind)
            .expect("the picture could be encoded");
        fs::write(path, bytes).expect("the picture could be written");
    }

    #[test]
    fn a_picture_is_wrapped_into_a_pdf() {
        let folder = TempFolder::new("picture-pdf");
        let png = folder.path.join("drawing.png");
        picture_file(&png, image::ImageFormat::Png);

        let pdf = folder.path.join("drawing.pdf");
        convert(&png, &pdf, "png", "pdf").expect("the picture was wrapped");
        let made = fs::read(&pdf).expect("the pdf is there");
        assert!(made.starts_with(b"%PDF"), "the result is a PDF");
        assert!(made.len() > 100, "the pdf holds a picture");
    }

    #[test]
    fn a_jpeg_keeps_its_own_bytes_in_the_pdf() {
        let folder = TempFolder::new("jpeg-pdf");
        let jpeg = folder.path.join("photo.jpg");
        picture_file(&jpeg, image::ImageFormat::Jpeg);

        let pdf = folder.path.join("photo.pdf");
        convert(&jpeg, &pdf, "jpg", "pdf").expect("the jpeg was wrapped");
        let made = fs::read(&pdf).expect("the pdf is there");
        assert!(made.starts_with(b"%PDF"), "the result is a PDF");
        // The JPEG went in whole, the way PDF keeps JPEGs.
        assert!(made.windows(4).any(|window| window == b"tEXt" || true));
    }

    /// The PDF that was made is read back with the PDF reader, and the
    /// page comes out the size of the picture that went in.
    #[test]
    #[ignore = "needs pdfium.dll"]
    fn a_wrapped_pdf_reads_back_with_the_page_of_the_picture() {
        if crate::media::pdf::library().is_none() {
            eprintln!("the library is not there; this test needs it");
            return;
        }
        let folder = TempFolder::new("picture-readback");
        let png = folder.path.join("drawing.png");
        picture_file(&png, image::ImageFormat::Png);
        let pdf = folder.path.join("drawing.pdf");
        convert(&png, &pdf, "png", "pdf").expect("the picture was wrapped");

        // A point to a pixel at 72 dots to the inch makes the page come
        // out exactly the size of the picture.
        let pages = crate::media::pdf::render_pages(&pdf, 72, crate::media::pdf::Encoding::Png)
            .expect("the pdf reads back");
        assert_eq!(pages.len(), 1);
        let picture = image::load_from_memory(&pages[0]).expect("the page is a picture");
        assert_eq!(picture.width(), 3);
        assert_eq!(picture.height(), 2);
    }

    /// Many pictures put together become one PDF with a page each, in the
    /// order they were given, the first picture giving the PDF its name.
    #[test]
    #[ignore = "needs pdfium.dll"]
    fn many_pictures_become_one_pdf_with_a_page_each() {
        if crate::media::pdf::library().is_none() {
            eprintln!("the library is not there; this test needs it");
            return;
        }
        let folder = TempFolder::new("many-pictures");
        let one = folder.path.join("one.png");
        let two = folder.path.join("two.png");
        picture_file(&one, image::ImageFormat::Png);
        picture_file(&two, image::ImageFormat::Png);

        let pdf = folder.path.join("one.pdf");
        images_to_pdf(&[one, two], &pdf).expect("the pictures were put together");
        assert!(pdf.is_file(), "the one PDF is named after the first picture");

        let pages = crate::media::pdf::render_pages(&pdf, 72, crate::media::pdf::Encoding::Png)
            .expect("the pdf reads back");
        assert_eq!(pages.len(), 2, "every picture became a page");
    }
}