//! PDF reading via PDFium.
//!
//! PDFium is a library, not a program: it lives beside the application as
//! `pdfium.dll`, the way the conversion program lives beside it, and it is
//! bound to when a PDF is first read. It renders each page of a PDF into a
//! picture, and it reads the text of each page.
//!
//! A PDF page is laid out in points, seventy-two to the inch, so the size
//! of the rendered picture is set by a number of dots per inch.

use std::io::Cursor;
use std::path::{Path, PathBuf};

use image::ImageFormat;
use pdfium_render::prelude::*;

/// Why a PDF was not read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PdfError {
    /// The library was not there to be bound to.
    LibraryMissing,
    /// The file could not be read as a PDF.
    NotAPdf,
    /// A page could not be rendered into a picture.
    RenderFailed,
    /// The text of a page could not be read.
    TextFailed,
}

impl PdfError {
    #[allow(dead_code)]
    pub fn name(&self) -> &'static str {
        match self {
            PdfError::LibraryMissing => "the PDF reader is not installed",
            PdfError::NotAPdf => "the file could not be read as a PDF",
            PdfError::RenderFailed => "a page could not be rendered",
            PdfError::TextFailed => "the text of a page could not be read",
        }
    }
}

/// The picture formats a page can be rendered into.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Encoding {
    Png,
    Jpeg,
}

impl Encoding {
    /// The encoding for a target format name, if it is one this seam makes.
    pub fn of_format(format: &str) -> Option<Encoding> {
        match format {
            "png" => Some(Encoding::Png),
            "jpg" | "jpeg" => Some(Encoding::Jpeg),
            _ => None,
        }
    }
}

/// Where the library is, if it is there.
///
/// It is looked for the way the conversion program is looked for: beside
/// the program first, because that is where a release puts it, and then in
/// the vendor folder of the repository, which is where a developer keeps
/// it. A program built by cargo sits in target/debug or target/release,
/// so the repository is two folders above it.
pub fn library() -> Option<PathBuf> {
    let dll = if cfg!(windows) { "pdfium.dll" } else { "libpdfium.so" };
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(program) = std::env::current_exe().ok()
        && let Some(folder) = program.parent()
    {
        candidates.push(folder.join(dll));
        // The place the repository keeps the library, when the program is
        // built by cargo: target\release, or target\debug\deps for the
        // tests. The repository is above the build folders, and the
        // vendor folder is kept there.
        //
        // The folder the program runs in is deliberately not asked: it is
        // whatever the person who started the program was standing in, and
        // a library found that way would be one of no known kind, which
        // binds and then faults.
        let developed = match folder.file_name().map(|name| name.to_string_lossy().into_owned()) {
            Some(name) => match name.as_str() {
                // The tests: target\debug\deps - the repository is three
                // folders above the folder of the program.
                "deps" => folder
                    .parent()
                    .and_then(Path::parent)
                    .and_then(Path::parent)
                    .map(|root| root.join("vendor").join("pdfium").join(dll)),
                // A build: target\release or target\debug - the repository
                // is two folders above the folder of the program.
                "debug" | "release" => folder
                    .parent()
                    .and_then(Path::parent)
                    .map(|root| root.join("vendor").join("pdfium").join(dll)),
                _ => None,
            },
            None => None,
        };
        if let Some(candidate) = developed {
            candidates.push(candidate);
        }
    }
    candidates.into_iter().find(|path| path.is_file())
}

/// The bound library, made once and shared by everything after.
///
/// The library lets itself be bound to only once in the life of the
/// program, so the binding is kept: the first one to need it binds it, and
/// the rest are given the same one.
fn pdfium() -> Result<&'static Pdfium, PdfError> {
    static PDFIUM: std::sync::OnceLock<Result<Pdfium, PdfError>> = std::sync::OnceLock::new();
    PDFIUM
        .get_or_init(|| {
            let path = library().ok_or(PdfError::LibraryMissing)?;
            let bindings =
                Pdfium::bind_to_library(&path).map_err(|_| PdfError::LibraryMissing)?;
            Ok(Pdfium::new(bindings))
        })
        .as_ref()
        .map_err(|reason| *reason)
}

/// Renders every page of a PDF into encoded picture bytes.
///
/// Every page becomes one picture, in the order the pages are in. A PDF
/// that holds no pages is not a PDF that can be shown.
pub fn render_pages(
    input: &Path,
    dots_per_inch: u16,
    encoding: Encoding,
) -> Result<Vec<Vec<u8>>, PdfError> {
    let pdfium = pdfium()?;
    let document = pdfium
        .load_pdf_from_file(input, None)
        .map_err(|_| PdfError::NotAPdf)?;
    let scale = dots_per_inch as f32 / 72.0;

    let mut pages = Vec::new();
    for page in document.pages().iter() {
        let size = page.page_size();
        // A page that says it has no size cannot be scaled into a picture.
        let width = size.width().value * scale;
        let height = size.height().value * scale;
        if !(width.is_finite() && height.is_finite()) || width < 1.0 || height < 1.0 {
            return Err(PdfError::RenderFailed);
        }
        let config = PdfRenderConfig::new()
            .set_target_width(width as i32)
            .set_target_height(height as i32);
        let bitmap = page
            .render_with_config(&config)
            .map_err(|_| PdfError::RenderFailed)?;
        let picture = bitmap.as_image().map_err(|_| PdfError::RenderFailed)?;
        let format = match encoding {
            Encoding::Png => ImageFormat::Png,
            Encoding::Jpeg => ImageFormat::Jpeg,
        };
        let mut bytes = Vec::new();
        picture
            .write_to(&mut Cursor::new(&mut bytes), format)
            .map_err(|_| PdfError::RenderFailed)?;
        pages.push(bytes);
    }
    if pages.is_empty() {
        return Err(PdfError::NotAPdf);
    }
    Ok(pages)
}

/// Renders the first page of a PDF document as an image.
pub fn render_first_page(input: &Path, max_dimension: u32) -> Result<image::DynamicImage, PdfError> {
    let pdfium = pdfium()?;
    let document = pdfium
        .load_pdf_from_file(input, None)
        .map_err(|_| PdfError::NotAPdf)?;
    let first_page = document.pages().get(0).map_err(|_| PdfError::NotAPdf)?;
    let size = first_page.page_size();
    let w = size.width().value;
    let h = size.height().value;
    if !(w.is_finite() && h.is_finite()) || w < 1.0 || h < 1.0 {
        return Err(PdfError::RenderFailed);
    }
    let scale = (max_dimension as f32 / w.max(h)).min(2.0).max(0.1);
    let target_w = (w * scale).max(1.0) as i32;
    let target_h = (h * scale).max(1.0) as i32;
    let config = PdfRenderConfig::new()
        .set_target_width(target_w)
        .set_target_height(target_h);
    let bitmap = first_page
        .render_with_config(&config)
        .map_err(|_| PdfError::RenderFailed)?;
    let picture = bitmap.as_image().map_err(|_| PdfError::RenderFailed)?;
    Ok(picture)
}

/// Returns the number of pages in the PDF document.
pub fn page_count(input: &Path) -> Result<usize, PdfError> {
    let pdfium = pdfium()?;
    let document = pdfium
        .load_pdf_from_file(input, None)
        .map_err(|_| PdfError::NotAPdf)?;
    Ok(document.pages().len() as usize)
}

/// Reads the text of every page, one page after another.
pub fn extract_text(input: &Path) -> Result<String, PdfError> {
    let pdfium = pdfium()?;
    let document = pdfium
        .load_pdf_from_file(input, None)
        .map_err(|_| PdfError::NotAPdf)?;

    let mut text = String::new();
    for page in document.pages().iter() {
        let words = page.text().map_err(|_| PdfError::TextFailed)?;
        text.push_str(&words.all());
        // The pages are kept apart, the way the pictures of them are.
        text.push_str("\n\n");
    }
    Ok(text)
}

/// Merges multiple PDF files into one document, in the order given.
pub fn merge(inputs: &[PathBuf], output: &Path) -> Result<(), PdfError> {
    if inputs.is_empty() {
        return Err(PdfError::NotAPdf);
    }
    let pdfium = pdfium()?;
    let mut merged = pdfium.create_new_pdf().map_err(|_| PdfError::NotAPdf)?;
    for input in inputs {
        let doc = pdfium
            .load_pdf_from_file(input, None)
            .map_err(|_| PdfError::NotAPdf)?;
        merged
            .pages_mut()
            .append(&doc)
            .map_err(|_| PdfError::NotAPdf)?;
    }
    if merged.pages().is_empty() {
        return Err(PdfError::NotAPdf);
    }
    merged.save_to_file(output).map_err(|_| PdfError::NotAPdf)?;
    Ok(())
}

/// A specification for one page in an assembled PDF: which document from
/// the inputs list, which page within that document, and how many degrees
/// clockwise it is turned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PageSpec {
    pub source_doc: usize,
    pub source_index: usize,
    pub rotation_degrees: i32,
}

/// Assembles a new PDF from selected pages and rotations of source documents.
pub fn assemble(
    inputs: &[PathBuf],
    pages: &[PageSpec],
    output: &Path,
) -> Result<(), PdfError> {
    if inputs.is_empty() || pages.is_empty() {
        return Err(PdfError::NotAPdf);
    }
    let pdfium = pdfium()?;
    let mut docs = Vec::with_capacity(inputs.len());
    for input in inputs {
        let doc = pdfium
            .load_pdf_from_file(input, None)
            .map_err(|_| PdfError::NotAPdf)?;
        docs.push(doc);
    }
    let mut destination = pdfium
        .create_new_pdf()
        .map_err(|_| PdfError::NotAPdf)?;

    for (dest_idx, spec) in pages.iter().enumerate() {
        let source = docs.get(spec.source_doc).ok_or(PdfError::NotAPdf)?;
        destination
            .pages_mut()
            .copy_page_from_document(source, spec.source_index as i32, dest_idx as i32)
            .map_err(|_| PdfError::NotAPdf)?;

        let rot = match ((spec.rotation_degrees % 360) + 360) % 360 {
            90 => PdfPageRenderRotation::Degrees90,
            180 => PdfPageRenderRotation::Degrees180,
            270 => PdfPageRenderRotation::Degrees270,
            _ => PdfPageRenderRotation::None,
        };

        if let Ok(mut page) = destination.pages_mut().get(dest_idx as i32) {
            page.set_rotation(rot);
        }
    }

    if destination.pages().is_empty() {
        return Err(PdfError::NotAPdf);
    }
    destination.save_to_file(output).map_err(|_| PdfError::NotAPdf)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A folder of its own for a test, removed when the test is over.
    struct TempFolder {
        path: PathBuf,
    }

    impl TempFolder {
        fn new(name: &str) -> TempFolder {
            let path = std::env::temp_dir().join(format!(
                "open-tangerine-pdf-{}-{name}",
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

    /// Makes a PDF of two blank pages, with the library itself.
    fn two_page_pdf(folder: &Path) -> PathBuf {
        let pdfium = pdfium().expect("the library could be bound to");
        let mut document = pdfium.create_new_pdf().expect("the document could be made");
        let size = PdfPagePaperSize::Custom(PdfPoints { value: 200.0 }, PdfPoints {
            value: 100.0,
        });
        document
            .pages_mut()
            .create_page_at_end(size)
            .expect("the first page could be made");
        document
            .pages_mut()
            .create_page_at_end(size)
            .expect("the second page could be made");
        let bytes = document.save_to_bytes().expect("the document could be saved");
        let path = folder.join("made.pdf");
        std::fs::write(&path, bytes).expect("the pdf could be written");
        path
    }

    #[test]
    #[ignore = "needs pdfium.dll"]
    fn every_page_of_a_pdf_is_rendered() {
        let Some(_) = library() else {
            eprintln!("the library is not there; this test needs it");
            return;
        };
        let folder = TempFolder::new("render");
        let pdf = two_page_pdf(&folder.path);

        let pages = render_pages(&pdf, 144, Encoding::Png).expect("the pages were rendered");
        assert_eq!(pages.len(), 2, "both pages became pictures");
        // A page of 200 x 100 points at 144 dots per inch is 400 x 200 dots.
        let picture =
            image::load_from_memory(&pages[0]).expect("the picture could be read back");
        assert_eq!(picture.width(), 400);
        assert_eq!(picture.height(), 200);
        // The pictures are PNG.
        assert!(pages[0].starts_with(b"\x89PNG"));
    }

    #[test]
    #[ignore = "needs pdfium.dll"]
    fn the_text_of_the_pages_is_read() {
        let Some(_) = library() else {
            eprintln!("the library is not there; this test needs it");
            return;
        };
        let folder = TempFolder::new("text");
        // A one-page PDF with one line of text in it, written out by hand:
        // the standard font needs no embedding, which is what makes a
        // hand-written PDF possible.
        let pdf = folder.path.join("written.pdf");
        let content = b"BT /F1 12 Tf 20 50 Td (Hello from the page) Tj ET";
        let text = format!(
            "%PDF-1.4\n\
             1 0 obj << /Type /Catalog /Pages 2 0 R >> endobj\n\
             2 0 obj << /Type /Pages /Kids [3 0 R] /Count 1 >> endobj\n\
             3 0 obj << /Type /Page /Parent 2 0 R /MediaBox [0 0 200 100] \
             /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >> endobj\n\
             4 0 obj << /Length {} >> stream\n{}\nendstream endobj\n\
             5 0 obj << /Type /Font /Subtype /Type1 /BaseFont /Helvetica >> endobj\n\
             trailer << /Root 1 0 R /Size 6 >>",
            content.len(),
            String::from_utf8_lossy(content)
        );
        std::fs::write(&pdf, text).expect("the pdf could be written");

        let words = extract_text(&pdf).expect("the text was read");
        assert!(
            words.contains("Hello from the page"),
            "the words of the page were read: {words:?}"
        );
    }

    #[test]
    #[ignore = "needs pdfium.dll"]
    fn a_file_that_is_not_a_pdf_is_refused() {
        let Some(_) = library() else {
            eprintln!("the library is not there; this test needs it");
            return;
        };
        let folder = TempFolder::new("not-a-pdf");
        let path = folder.path.join("pretend.pdf");
        std::fs::write(&path, b"not a pdf at all").expect("the file could be written");
        assert_eq!(render_pages(&path, 144, Encoding::Png), Err(PdfError::NotAPdf));
        assert_eq!(extract_text(&path), Err(PdfError::NotAPdf));
    }

    #[test]
    #[ignore = "needs pdfium.dll"]
    fn every_page_lands_beside_the_original_under_its_own_name() {
        let Some(_) = library() else {
            eprintln!("the library is not there; this test needs it");
            return;
        };
        let folder = TempFolder::new("beside");
        let pdf = two_page_pdf(&folder.path);

        // A page that is already there keeps its name: nothing is written
        // over, and the pages that come take the numbers after it.
        std::fs::write(folder.path.join("made.png"), b"the old one")
            .expect("the old picture could be made");

        crate::media::documents::convert(&pdf, &folder.path.join("made.png"), "pdf", "png")
            .expect("the pdf was converted");
        assert!(folder.path.join("made (2).png").is_file());
        assert!(folder.path.join("made (3).png").is_file());
        // What was there is untouched.
        assert_eq!(
            std::fs::read(folder.path.join("made.png")).expect("the old picture is there"),
            b"the old one"
        );
    }

    #[test]
    #[ignore = "needs pdfium.dll"]
    fn multiple_pdfs_are_merged_into_one() {
        let Some(_) = library() else {
            eprintln!("the library is not there; this test needs it");
            return;
        };
        let folder1 = TempFolder::new("merge1");
        let folder2 = TempFolder::new("merge2");
        let first = two_page_pdf(&folder1.path);
        let second = two_page_pdf(&folder2.path);
        let merged_folder = TempFolder::new("merged_out");
        let merged_path = merged_folder.path.join("combined.pdf");

        merge(&[first, second], &merged_path).expect("the documents were merged");
        assert!(merged_path.is_file(), "merged pdf exists");

        let pages = render_pages(&merged_path, 72, Encoding::Png).expect("rendered merged pages");
        assert_eq!(pages.len(), 4, "combined document has all 4 pages");
    }

    #[test]
    #[ignore = "needs pdfium.dll"]
    fn assemble_reorders_and_rotates_pages() {
        let Some(_) = library() else {
            eprintln!("the library is not there; this test needs it");
            return;
        };
        let folder = TempFolder::new("assemble_test");
        let src = two_page_pdf(&folder.path);
        let out = folder.path.join("organized.pdf");

        // Take page 1 first rotated 90°, then duplicate page 0 rotated 0°
        let specs = [
            PageSpec { source_doc: 0, source_index: 1, rotation_degrees: 90 },
            PageSpec { source_doc: 0, source_index: 0, rotation_degrees: 0 },
        ];
        assemble(&[src], &specs, &out).expect("assembled pdf");
        assert!(out.is_file(), "output exists");

        let pdfium = pdfium().expect("pdfium library");
        let doc = pdfium.load_pdf_from_file(&out, None).expect("load assembled");
        assert_eq!(doc.pages().len(), 2);
        assert_eq!(doc.pages().get(0).expect("p0").rotation().expect("rot"), PdfPageRenderRotation::Degrees90);
        assert_eq!(doc.pages().get(1).expect("p1").rotation().expect("rot"), PdfPageRenderRotation::None);
    }
}
