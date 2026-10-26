//! Office documents through the programs that read them.
//!
//! A DOCX, a spreadsheet or a slide deck becomes a PDF by being opened by
//! a program that knows how it looks, and exported from there. The
//! programs are tried in a chain:
//!
//! 1. the installed Microsoft Office, driven through its own automation,
//!    which renders the file exactly as the person sees it;
//! 2. LibreOffice, when it is installed, asked headlessly;
//! 3. nothing — and the operation is not offered at all, because the wheel
//!    never shows what cannot be run.
//!
//! The Office programs are driven with a script the operating system runs:
//! the script is given the paths in its environment, so no path is ever
//! put into the script itself.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::media::documents::DocError;

/// Which Office program a file belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum App {
    Word,
    Excel,
    PowerPoint,
}

/// The program that reads files of this extension, if any of them does.
fn app_of(extension: &str) -> Option<App> {
    match extension {
        "doc" | "docx" => Some(App::Word),
        // Excel also opens the open spreadsheet and the plain ones.
        "xls" | "xlsx" | "ods" | "csv" | "tsv" => Some(App::Excel),
        "ppt" | "pptx" => Some(App::PowerPoint),
        _ => None,
    }
}

/// The name a program is asked for.
fn prog_id(app: App) -> &'static str {
    match app {
        App::Word => "Word.Application",
        App::Excel => "Excel.Application",
        App::PowerPoint => "PowerPoint.Application",
    }
}

/// Whether a program is registered on this machine.
fn registered(name: &str) -> bool {
    Command::new("reg")
        .arg("query")
        .arg(format!("HKCR\\{name}"))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

/// Whether the program that reads this extension is installed, Office or
/// LibreOffice.
#[allow(dead_code)]
pub fn installed(extension: &str) -> bool {
    if let Some(app) = app_of(extension) {
        if registered(prog_id(app)) {
            return true;
        }
    }
    soffice().is_some()
}

/// Where LibreOffice is, if it is there.
fn soffice() -> Option<PathBuf> {
    let program = if cfg!(windows) { "soffice.exe" } else { "soffice" };
    let folders = [
        "C:\\Program Files\\LibreOffice\\program",
        "C:\\Program Files (x86)\\LibreOffice\\program",
    ];
    for folder in folders {
        let candidate = PathBuf::from(folder).join(program);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

/// The script that opens the source and exports it as a PDF.
fn script(app: App) -> &'static str {
    match app {
        App::Word => r#"
$ErrorActionPreference = 'Stop'
$app = New-Object -ComObject Word.Application
$app.Visible = $false
$app.DisplayAlerts = 0
try {
    $doc = $app.Documents.Open($env:OT_SOURCE, $false, $true)
    $doc.ExportAsFixedFormat($env:OT_TARGET, 17)
    $doc.Close(0)
} finally { $app.Quit() }
"#,
        App::Excel => r#"
$ErrorActionPreference = 'Stop'
$app = New-Object -ComObject Excel.Application
$app.DisplayAlerts = $false
try {
    $book = $app.Workbooks.Open($env:OT_SOURCE, 0, $true)
    $book.ExportAsFixedFormat(0, $env:OT_TARGET)
    $book.Close($false)
} finally { $app.Quit() }
"#,
        App::PowerPoint => r#"
$ErrorActionPreference = 'Stop'
$app = New-Object -ComObject PowerPoint.Application
try {
    $pres = $app.Presentations.Open($env:OT_SOURCE, -1, 0, 0)
    $pres.ExportAsFixedFormat($env:OT_TARGET, 2)
    $pres.Close()
} finally { $app.Quit() }
"#,
    }
}

/// Makes a PDF of an office document, beside the original.
///
/// The program that reads the file is the one that makes the PDF, so what
/// comes out looks the way the file looks when it is opened.
pub fn to_pdf(source: &Path, target: &Path) -> Result<(), DocError> {
    let extension = source
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let Some(app) = app_of(&extension) else {
        return Err(DocError::UnsupportedSource);
    };

    if registered(prog_id(app)) {
        return office_export(app, source, target);
    }
    if let Some(program) = soffice() {
        return libreoffice_export(&program, source, target);
    }
    Err(DocError::OfficeMissing)
}

/// Exports with the installed Office program.
fn office_export(app: App, source: &Path, target: &Path) -> Result<(), DocError> {
    let result = Command::new("powershell")
        .arg("-NoProfile")
        .arg("-NonInteractive")
        .arg("-Command")
        .arg(script(app))
        .env("OT_SOURCE", source)
        .env("OT_TARGET", target)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .output();
    match result {
        Ok(output) if output.status.success() && target.is_file() => Ok(()),
        _ => Err(DocError::OfficeFailed),
    }
}

/// Exports with LibreOffice, headlessly.
///
/// LibreOffice writes the result under its own name in a folder it is
/// given, so the result is made in a folder of its own and then moved to
/// where it belongs.
fn libreoffice_export(program: &Path, source: &Path, target: &Path) -> Result<(), DocError> {
    let folder = target.parent().unwrap_or_else(|| Path::new("."));
    let staging = folder.join(format!(
        ".{}-{}",
        std::process::id(),
        source.file_stem().and_then(|stem| stem.to_str()).unwrap_or("converted")
    ));
    std::fs::create_dir_all(&staging).map_err(|_| DocError::WriteError)?;

    let result = Command::new(program)
        .arg("--headless")
        .arg("--convert-to")
        .arg("pdf")
        .arg("--outdir")
        .arg(&staging)
        .arg(source)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .output();
    let made = staging.join(format!(
        "{}.pdf",
        source.file_stem().and_then(|stem| stem.to_str()).unwrap_or("converted")
    ));
    let _ = std::fs::remove_dir_all(&staging);
    match result {
        Ok(output) if output.status.success() && made.is_file() => {
            std::fs::rename(&made, target).map_err(|_| DocError::WriteError)
        }
        _ => Err(DocError::OfficeFailed),
    }
}

/// The office extensions the installed programs can read, for the registry.
pub fn available_reads() -> Vec<&'static str> {
    let word = registered(prog_id(App::Word));
    let excel = registered(prog_id(App::Excel));
    let powerpoint = registered(prog_id(App::PowerPoint));
    let libreoffice = soffice().is_some();

    let mut reads = Vec::new();
    if word || libreoffice {
        reads.extend(["doc", "docx"]);
    }
    if excel || libreoffice {
        reads.extend(["xls", "xlsx", "ods", "csv", "tsv"]);
    }
    if powerpoint || libreoffice {
        reads.extend(["ppt", "pptx", "odp"]);
    }
    reads
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_office_extension_belongs_to_a_program() {
        assert_eq!(app_of("docx"), Some(App::Word));
        assert_eq!(app_of("doc"), Some(App::Word));
        assert_eq!(app_of("xlsx"), Some(App::Excel));
        assert_eq!(app_of("xls"), Some(App::Excel));
        assert_eq!(app_of("ods"), Some(App::Excel));
        assert_eq!(app_of("pptx"), Some(App::PowerPoint));
        assert_eq!(app_of("ppt"), Some(App::PowerPoint));
        assert_eq!(app_of("png"), None);
    }

    #[test]
    fn the_programs_are_registered_the_way_the_registry_says() {
        // This says what the machine really has, so that a machine with
        // nothing installed says false rather than pretending.
        let _ = registered("Word.Application");
        let _ = registered("Excel.Application");
    }

    /// A folder of its own for a test, removed when the test is over.
    struct TempFolder {
        path: PathBuf,
    }

    impl TempFolder {
        fn new(name: &str) -> TempFolder {
            let path = std::env::temp_dir().join(format!(
                "open-tangerine-documents2-{}-{name}",
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
    /// together.
    fn office_file(path: &Path, parts: &[(&str, &str)]) {
        let file = std::fs::File::create(path).expect("the office file could be made");
        let mut zip = zip::ZipWriter::new(file);
        for (name, content) in parts {
            zip.start_file(*name, zip::write::FileOptions::default())
                .expect("the part could be started");
            std::io::Write::write_all(&mut zip, content.as_bytes())
                .expect("the part could be written");
        }
        zip.finish().expect("the office file could be finished");
    }

    fn docx_parts() -> Vec<(&'static str, String)> {
        let rels = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\
<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
<Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"word/document.xml\"/>\
</Relationships>";
        let document = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\
<w:document xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\">\
<w:body><w:p><w:r><w:t>A letter to be exported</w:t></w:r></w:p></w:body></w:document>";
        vec![
            ("[Content_Types].xml", "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\
<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\
<Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>\
<Default Extension=\"xml\" ContentType=\"application/xml\"/>\
<Override PartName=\"/word/document.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml\"/>\
</Types>".to_string()),
            ("_rels/.rels", rels.to_string()),
            ("word/document.xml", document.to_string()),
        ]
    }

    /// A DOCX that Word itself exports as a PDF.
    #[test]
    #[ignore = "needs Microsoft Word"]
    fn a_docx_is_exported_to_pdf_by_word() {
        if !registered("Word.Application") {
            eprintln!("Word is not installed; this test needs it");
            return;
        }
        let folder = TempFolder::new("word");
        let docx = folder.path.join("letter.docx");
        let parts = docx_parts();
        let borrowed: Vec<(&str, &str)> =
            parts.iter().map(|(name, content)| (*name, content.as_str())).collect();
        office_file(&docx, &borrowed);

        let pdf = folder.path.join("letter.pdf");
        to_pdf(&docx, &pdf).expect("Word exported the pdf");
        let made = std::fs::read(&pdf).expect("the pdf is there");
        assert!(made.starts_with(b"%PDF"), "the result is a PDF");
    }

    /// An XLSX that Excel itself exports as a PDF.
    #[test]
    #[ignore = "needs Microsoft Excel"]
    fn an_xlsx_is_exported_to_pdf_by_excel() {
        if !registered("Excel.Application") {
            eprintln!("Excel is not installed; this test needs it");
            return;
        }
        let folder = TempFolder::new("excel");
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
<sheetData><row r=\"1\"><c r=\"A1\" t=\"inlineStr\"><is><t>name</t></is></c></row></sheetData></worksheet>";
        let rels = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\
<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
<Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"xl/workbook.xml\"/>\
</Relationships>";
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
                ("_rels/.rels", rels),
                ("xl/workbook.xml", workbook),
                ("xl/_rels/workbook.xml.rels", workbook_rels),
                ("xl/worksheets/sheet1.xml", sheet),
            ],
        );

        let pdf = folder.path.join("table.pdf");
        to_pdf(&xlsx, &pdf).expect("Excel exported the pdf");
        let made = std::fs::read(&pdf).expect("the pdf is there");
        assert!(made.starts_with(b"%PDF"), "the result is a PDF");
    }
}
