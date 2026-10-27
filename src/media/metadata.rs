//! Reading and describing metadata on media files.
//!
//! Every file carries facts about itself: the system's own dates and size,
//! the picture or video's dimensions, duration and frame rate, and the tags
//! (artist, title, comments) written into the container.
//!
//! This module reads these facts into grouped [`MetadataField`] items for
//! display, and translates between common [`MetadataTag`] values and the
//! container keys the conversion program expects.

use std::collections::HashMap;
use std::ffi::OsString;
use std::path::Path;
use std::time::SystemTime;

use crate::media::locator::Tools;
use crate::media::probe::{kind_of, Kind};
use crate::media::runner::{ProcessRunner, SystemRunner};

/// One piece of metadata read from a file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MetadataField {
    /// The section or group the field belongs to (e.g. "File", "Media", "Tags").
    pub group: String,
    /// The name of the field.
    pub key: String,
    /// What the field holds.
    pub value: String,
    /// Whether this field contains sensitive personal information
    /// (e.g. location, author, camera serial).
    pub sensitive: bool,
}

/// The common fields people edit across pictures, video, audio and documents.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MetadataTag {
    Title,
    Artist,
    Album,
    Genre,
    Date,
    Description,
    Comment,
    Keywords,
    Copyright,
}

impl MetadataTag {
    /// Returns the tags offered for a given kind of file.
    pub fn for_kind(kind: Option<Kind>, is_pdf: bool) -> &'static [MetadataTag] {
        if is_pdf {
            return &[
                MetadataTag::Title,
                MetadataTag::Artist,
                MetadataTag::Description,
                MetadataTag::Keywords,
            ];
        }
        match kind {
            Some(Kind::Image) => &[
                MetadataTag::Title,
                MetadataTag::Description,
                MetadataTag::Artist,
                MetadataTag::Keywords,
                MetadataTag::Copyright,
                MetadataTag::Date,
                MetadataTag::Comment,
            ],
            Some(Kind::Audio) => &[
                MetadataTag::Title,
                MetadataTag::Artist,
                MetadataTag::Album,
                MetadataTag::Genre,
                MetadataTag::Date,
                MetadataTag::Comment,
                MetadataTag::Copyright,
            ],
            Some(Kind::Video) => &[
                MetadataTag::Title,
                MetadataTag::Artist,
                MetadataTag::Description,
                MetadataTag::Comment,
                MetadataTag::Date,
                MetadataTag::Copyright,
            ],
            None => &[
                MetadataTag::Title,
                MetadataTag::Description,
                MetadataTag::Artist,
                MetadataTag::Keywords,
                MetadataTag::Copyright,
                MetadataTag::Date,
                MetadataTag::Comment,
            ],
        }
    }

    /// The human-facing label for this tag on a given file kind.
    pub fn label(&self, kind: Option<Kind>, is_pdf: bool) -> &'static str {
        match self {
            MetadataTag::Title => "Title",
            MetadataTag::Artist => {
                if matches!(kind, Some(Kind::Audio)) {
                    "Artist"
                } else {
                    "Author"
                }
            }
            MetadataTag::Album => "Album",
            MetadataTag::Genre => "Genre",
            MetadataTag::Date => match kind {
                Some(Kind::Image) => "Date taken",
                Some(Kind::Audio) => "Year",
                _ => "Date",
            },
            MetadataTag::Description => {
                if is_pdf {
                    "Subject"
                } else {
                    "Description"
                }
            }
            MetadataTag::Comment => "Comment",
            MetadataTag::Keywords => "Keywords",
            MetadataTag::Copyright => "Copyright",
        }
    }

    /// The grey hint shown in an empty field.
    pub fn placeholder(&self, kind: Option<Kind>) -> &'static str {
        match self {
            MetadataTag::Date => match kind {
                Some(Kind::Image) => "2026:10:19 14:30:00",
                Some(Kind::Audio) => "2026",
                _ => "2026-10-19",
            },
            MetadataTag::Keywords => "Separated by commas",
            _ => "",
        }
    }

    /// The key name the conversion program uses for this tag.
    pub fn ffmpeg_key(&self) -> &'static str {
        match self {
            MetadataTag::Title => "title",
            MetadataTag::Artist => "artist",
            MetadataTag::Album => "album",
            MetadataTag::Genre => "genre",
            MetadataTag::Date => "date",
            MetadataTag::Description => "description",
            MetadataTag::Comment => "comment",
            MetadataTag::Keywords => "keywords",
            MetadataTag::Copyright => "copyright",
        }
    }

    /// Matches a tag key from ffprobe output.
    pub fn from_key(key: &str) -> Option<MetadataTag> {
        match key.to_lowercase().as_str() {
            "title" => Some(MetadataTag::Title),
            "artist" | "author" | "composer" | "by-line" => Some(MetadataTag::Artist),
            "album" => Some(MetadataTag::Album),
            "genre" => Some(MetadataTag::Genre),
            "date" | "year" | "creation_time" => Some(MetadataTag::Date),
            "description" | "subject" => Some(MetadataTag::Description),
            "comment" => Some(MetadataTag::Comment),
            "keywords" => Some(MetadataTag::Keywords),
            "copyright" => Some(MetadataTag::Copyright),
            _ => None,
        }
    }
}

/// Reads the metadata fields from a file, grouped for display.
pub fn read_fields(path: &Path, tools: Option<&Tools>) -> Vec<MetadataField> {
    let mut fields = Vec::new();

    // 1. File group (filesystem facts)
    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("File")
        .to_string();
    fields.push(MetadataField {
        group: "File".to_string(),
        key: "Name".to_string(),
        value: file_name,
        sensitive: false,
    });

    if let Ok(meta) = std::fs::metadata(path) {
        fields.push(MetadataField {
            group: "File".to_string(),
            key: "Size".to_string(),
            value: format_file_size(meta.len()),
            sensitive: false,
        });

        if let Ok(created) = meta.created() {
            fields.push(MetadataField {
                group: "File".to_string(),
                key: "Created".to_string(),
                value: format_system_time(created),
                sensitive: false,
            });
        }
        if let Ok(modified) = meta.modified() {
            fields.push(MetadataField {
                group: "File".to_string(),
                key: "Modified".to_string(),
                value: format_system_time(modified),
                sensitive: false,
            });
        }
    }

    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    let kind = kind_of(&ext);

    // 2. Kind-specific dimensions and duration
    match kind {
        Some(Kind::Image) => {
            if let Ok((w, h)) = image::image_dimensions(path) {
                fields.push(MetadataField {
                    group: "Image".to_string(),
                    key: "Dimensions".to_string(),
                    value: format!("{w} \u{d7} {h}"),
                    sensitive: false,
                });
            }
            fields.push(MetadataField {
                group: "Image".to_string(),
                key: "Format".to_string(),
                value: ext.to_uppercase(),
                sensitive: false,
            });
        }
        Some(Kind::Video) => {
            if let Some(tools) = tools {
                if let Some(duration) = crate::media::frames::duration_of(tools, path) {
                    fields.push(MetadataField {
                        group: "Media".to_string(),
                        key: "Duration".to_string(),
                        value: format_duration(duration),
                        sensitive: false,
                    });
                }
                if let Some((w, h, fps)) = crate::media::frames::shape_of(tools, path) {
                    fields.push(MetadataField {
                        group: "Media".to_string(),
                        key: "Dimensions".to_string(),
                        value: format!("{w} \u{d7} {h}"),
                        sensitive: false,
                    });
                    if fps > 0.0 {
                        fields.push(MetadataField {
                            group: "Media".to_string(),
                            key: "Frame rate".to_string(),
                            value: format!("{fps:.2} fps"),
                            sensitive: false,
                        });
                    }
                }
            }
        }
        Some(Kind::Audio) => {
            if let Some(tools) = tools {
                if let Some(duration) = crate::media::frames::duration_of(tools, path) {
                    fields.push(MetadataField {
                        group: "Media".to_string(),
                        key: "Duration".to_string(),
                        value: format_duration(duration),
                        sensitive: false,
                    });
                }
            }
        }
        _ => {
            if ext == "pdf" {
                fields.push(MetadataField {
                    group: "Document".to_string(),
                    key: "Format".to_string(),
                    value: "PDF".to_string(),
                    sensitive: false,
                });
            }
        }
    }

    // 3. Probed tags via ffprobe if tools are available
    if kind == Some(Kind::Image) {
        // A photograph says its tags in the EXIF block, in its own
        // letters; those are shown before whatever ffprobe answers.
        for (tag, value) in read_image_tags(path).unwrap_or_default() {
            let key = tag.ffmpeg_key();
            fields.push(MetadataField {
                group: "Tags".to_string(),
                key: key.to_string(),
                value,
                sensitive: is_sensitive_key(key),
            });
        }
    }
    if let Some(tools) = tools {
        for (key, val) in probe_format_tags(tools, path) {
            let sensitive = is_sensitive_key(&key);
            fields.push(MetadataField {
                group: "Tags".to_string(),
                key,
                value: val,
                sensitive,
            });
        }
    }

    fields
}

/// Reads existing common tags from a file.
///
/// The photo containers speak a different alphabet from the sound and
/// movie ones: a photograph carries its tags in an EXIF block. This
/// merges the two — the EXIF slots are asked first, the watched common
/// tags read through ffprobe fill what is left, so the window shows what
/// the file actually says rather than only what ffprobe answers.
pub fn read_tags(path: &Path, tools: Option<&Tools>) -> HashMap<MetadataTag, String> {
    let mut result = read_image_tags(path).unwrap_or_default();
    if let Some(tools) = tools {
        let tags = probe_format_tags(tools, path);
        for (key, val) in tags {
            if let Some(tag) = MetadataTag::from_key(&key) {
                result.entry(tag).or_insert(val);
            }
        }
    }
    result
}

/// Runs ffprobe to extract format_tags from a file.
fn probe_format_tags(tools: &Tools, path: &Path) -> Vec<(String, String)> {
    let arguments: Vec<OsString> = [
        "-v",
        "error",
        "-show_entries",
        "format_tags",
        "-of",
        "default=noprint_wrappers=1",
    ]
    .iter()
    .map(|s| OsString::from(*s))
    .chain([path.as_os_str().to_os_string()])
    .collect();

    let Ok(run) = SystemRunner.run(&tools.ffprobe, &arguments, None) else {
        return Vec::new();
    };

    if !run.succeeded() {
        return Vec::new();
    }

    let output = String::from_utf8_lossy(&run.stdout);
    let mut tags = Vec::new();
    for line in output.lines() {
        if let Some((k, v)) = line.split_once('=') {
            let key = k.trim().replace("TAG:", "");
            let val = v.trim();
            if !key.is_empty() && !val.is_empty() {
                tags.push((key, val.to_string()));
            }
        }
    }
    tags
}

/// Whether a tag key represents sensitive information (GPS, location, author, personal).
fn is_sensitive_key(key: &str) -> bool {
    let lower = key.to_lowercase();
    lower.contains("location")
        || lower.contains("gps")
        || lower.contains("author")
        || lower.contains("artist")
        || lower.contains("creator")
        || lower.contains("producer")
        || lower.contains("serial")
        || lower.contains("camera")
}

/// Converts bytes to readable units: "500 B", "1.4 MB", etc.
pub fn format_file_size(bytes: u64) -> String {
    if bytes >= 1_000_000_000 {
        format!("{:.1} GB", bytes as f64 / 1_000_000_000.0)
    } else if bytes >= 1_000_000 {
        format!("{:.1} MB", bytes as f64 / 1_000_000.0)
    } else if bytes >= 1_000 {
        format!("{:.0} KB", (bytes as f64 / 1_000.0).round())
    } else {
        format!("{bytes} B")
    }
}

/// Formats a duration in seconds as clock time "0:00" or "0:00.0".
fn format_duration(seconds: f64) -> String {
    let total = seconds.round() as u64;
    let mins = total / 60;
    let secs = total % 60;
    format!("{mins}:{secs:02}")
}

/// The picture containers the EXIF block is spoken in, by extension.
const EXIF_EXTENSIONS: &[&str] = &["jpg", "jpeg", "jpe", "jfif", "tif", "tiff", "png", "webp"];

/// The slots the system writes a photograph's words into, spoken by
/// number: the ones two-letters-apart carry the title, the saying, the
/// marks and the author in the letters the file manager reads.
const TAG_XP_TITLE: exif::Tag = exif::Tag(exif::Context::Tiff, 0x9C9B);
const TAG_XP_COMMENT: exif::Tag = exif::Tag(exif::Context::Tiff, 0x9C9C);
const TAG_XP_AUTHOR: exif::Tag = exif::Tag(exif::Context::Tiff, 0x9C9D);
const TAG_XP_KEYWORDS: exif::Tag = exif::Tag(exif::Context::Tiff, 0x9C9E);
const TAG_XP_SUBJECT: exif::Tag = exif::Tag(exif::Context::Tiff, 0x9C9F);

/// The slots a photo's tags are carried in.
///
/// The title, the saying and the marks are the ones the system itself
/// writes into a photograph, held as doubled letters without the first
/// byte spoken — the way the file manager reads them. The author and the
/// copyright are plain letters, and the moment the picture was taken has
/// its own field in the picture's own section.
const UNICODE_SLOTS: &[(
    MetadataTag,
    exif::Tag,
)] = &[
    (MetadataTag::Title, TAG_XP_TITLE),
    (MetadataTag::Description, TAG_XP_SUBJECT),
    (MetadataTag::Comment, TAG_XP_COMMENT),
    (MetadataTag::Keywords, TAG_XP_KEYWORDS),
];

/// Reads a photograph's raw EXIF fields, as they stand.
///
/// The fields are carried over whole when a photograph is saved with a
/// fresh encoding — the camera's own facts survive a save that way. A
/// photograph of a kind that carries no block answers with nothing.
pub(crate) fn exif_fields(path: &Path) -> Option<Vec<exif::Field>> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    if !EXIF_EXTENSIONS.contains(&ext.as_str()) {
        return None;
    }

    let file = std::fs::File::open(path).ok()?;
    let mut reader = std::io::BufReader::new(file);
    let exif = exif::Reader::new().read_from_container(&mut reader).ok()?;
    Some(exif.fields().cloned().collect())
}

/// The tags the settings asked for, spoken back in this house's names.
///
/// The windows say what they asked in the names the watched common tags
/// use; the photographs want the same few.
pub(crate) fn edits_from_ffmpeg_keys(
    asked: &[(String, String)],
) -> Vec<(MetadataTag, String)> {
    asked
        .iter()
        .map(|(key, value)| MetadataTag::from_key(key).map(|tag| (tag, value.trim().to_string())))
        .flatten()
        .collect()
}

/// Reads a photograph's tags out of its EXIF block.
///
/// A photograph that is not of a kind that carries the block — or a
/// picture that carries nothing — answers with an empty map, never with
/// an error: the window is still owed its rows.
pub(crate) fn read_image_tags(path: &Path) -> Option<HashMap<MetadataTag, String>> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    if !EXIF_EXTENSIONS.contains(&ext.as_str()) {
        return None;
    }

    let fields = exif_fields(path)?;
    let mut tags = HashMap::new();

    let mut take = |meta: &MetadataTag, slots: &[exif::Tag]| -> bool {
        let said = slots
            .iter()
            .find_map(|slot| {
                fields
                    .iter()
                    .find(|field| field.tag == *slot)
                    .and_then(exif_field_string)
            });
        if let Some(value) = said {
            tags.insert(*meta, value);
            true
        } else {
            false
        }
    };

    for (metadata_tag, slot) in UNICODE_SLOTS {
        take(metadata_tag, &[*slot]);
    }
    take(
        &MetadataTag::Artist,
        &[exif::Tag::Artist, TAG_XP_AUTHOR],
    );
    take(&MetadataTag::Copyright, &[exif::Tag::Copyright]);
    let date_slots = [
        exif::Tag::DateTimeOriginal,
        exif::Tag::DateTimeDigitized,
        exif::Tag::DateTime,
    ];
    let said = fields
        .iter()
        .filter(|field| date_slots.contains(&field.tag))
        .find_map(exif_field_string);
    if let Some(value) = said {
        tags.insert(MetadataTag::Date, value);
    }
    // A photograph with no saying of its own lets the description show
    // in its place: the picture description is what the cameras write.
    if !tags.contains_key(&MetadataTag::Title) {
        if let Some(field) = fields
            .iter()
            .find(|field| field.tag == exif::Tag::ImageDescription)
            .and_then(exif_field_string)
        {
            tags.insert(MetadataTag::Title, field);
        }
    }
    Some(tags)
}

/// The words a field says, when it is one of the letters the window
/// asks for.
///
/// The plain-letter fields answer directly. The doubled-letter fields
/// hold bytes that are two at a time one letter, which is decoded here.
fn exif_field_string(field: &exif::Field) -> Option<String> {
    match &field.value {
        exif::Value::Ascii(strings) => {
            let first = strings.first()?;
            let text = String::from_utf8_lossy(first).trim_end_matches('\u{0}').to_string();
            if text.is_empty() {
                None
            } else {
                Some(text)
            }
        }
        exif::Value::Undefined(bytes, ..) => {
            let text = decode_utf16le(bytes);
            if text.is_empty() {
                None
            } else {
                Some(text)
            }
        }
        _ => None,
    }
}

/// Decodes the doubled letters the system writes into a photograph.
fn decode_utf16le(bytes: &[u8]) -> String {
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .take_while(|unit| *unit != 0)
        .filter(|unit| *unit != 0xFEFF) // the first-byte marker, when it is there
        .collect();
    String::from_utf16_lossy(&units)
}

/// Encodes text as the doubled letters the system writes, ended the
/// way the fields end: with nothing after the last word.
fn encode_utf16le(text: &str) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(text.len() * 2);
    for unit in text.encode_utf16() {
        bytes.extend_from_slice(&unit.to_le_bytes());
    }
    bytes.extend_from_slice(&[0, 0]);
    bytes
}

/// Builds the fields one edit writes into a photograph.
pub(crate) fn exif_fields_for_edits(edits: &[(MetadataTag, String)]) -> Vec<exif::Field> {
    let mut fields = Vec::new();
    for (metadata_tag, value) in edits {
        if value.is_empty() {
            continue;
        }
        match metadata_tag {
            MetadataTag::Title => {
                fields.push(unicode_field(TAG_XP_TITLE, value));
            }
            MetadataTag::Description => {
                fields.push(unicode_field(TAG_XP_SUBJECT, value));
            }
            MetadataTag::Comment => {
                fields.push(unicode_field(TAG_XP_COMMENT, value));
            }
            MetadataTag::Keywords => {
                // The marks are held the way the file manager holds
                // them, separated by semicolons.
                fields.push(unicode_field(TAG_XP_KEYWORDS, &value.replace(',', ";")));
            }
            MetadataTag::Artist => {
                fields.push(ascii_field(exif::Tag::Artist, value, exif::In::PRIMARY));
            }
            MetadataTag::Copyright => {
                fields.push(ascii_field(exif::Tag::Copyright, value, exif::In::PRIMARY));
            }
            MetadataTag::Date => {
                // The moment is one of the picture's own fields: the
                // block's house puts it in its own section by the tag's
                // kind, and the field is pointed at the top of the
                // block.
                fields.push(ascii_field(exif::Tag::DateTimeOriginal, value, exif::In::PRIMARY));
            }
            // An album and a kind are for sounds and songs; a
            // photograph has no place for them.
            MetadataTag::Album | MetadataTag::Genre => {}
        }
    }
    fields
}

fn unicode_field(tag: exif::Tag, text: &str) -> exif::Field {
    exif::Field {
        tag,
        ifd_num: exif::In::PRIMARY,
        // The second word is where the value stood in the file it was
        // read out of; the writer says where it stands when it writes.
        value: exif::Value::Undefined(encode_utf16le(text), 0),
    }
}

fn ascii_field(tag: exif::Tag, text: &str, ifd_num: exif::In) -> exif::Field {
    exif::Field {
        tag,
        ifd_num,
        value: exif::Value::Ascii(vec![format!("{text}\u{0}").into_bytes()]),
    }
}

/// Puts a photograph's own fields together with what was asked of it.
///
/// The original fields are kept — the camera's make, when it was taken
/// and where it stood are facts worth keeping unless the person asked
/// them gone. The asked fields take the place of their own slots: an
/// edited title is written to the title's slot, and the old one is
/// taken away so no other reader shows the first story.
///
/// When the sensitive fields were asked gone, the where-it-stood facts
/// and the words carrying a person's name are left out.
pub(crate) fn recompose_exif_fields(
    original: Vec<exif::Field>,
    edits: &[(MetadataTag, String)],
    remove_sensitive: bool,
) -> Vec<exif::Field> {
    let mut fields: Vec<exif::Field> = original
        .into_iter()
        .filter(|field| !is_sensitive_field(field) || !remove_sensitive)
        .filter(|field| !slot_is_retaken(field, edits))
        .collect();
    fields.extend(exif_fields_for_edits(edits));
    fields
}

/// The fields that carry a person's place or a person's name.
fn is_sensitive_field(field: &exif::Field) -> bool {
    match field.tag.context() {
        exif::Context::Gps => true,
        _ => matches!(
            field.tag,
            exif::Tag::Artist
                | TAG_XP_AUTHOR
                | TAG_XP_COMMENT
                | exif::Tag::UserComment
                | exif::Tag::Copyright
        ),
    }
}

/// Whether an edit asks this field's slot for itself.
///
/// The picture description is read as a title when the title's own slot
/// says nothing of itself, so an edited title takes it away too; a
/// saying kept for the camera stays where it is.
fn slot_is_retaken(field: &exif::Field, edits: &[(MetadataTag, String)]) -> bool {
    let taken = |slot: exif::Tag| field.tag == slot;
    for (metadata_tag, value) in edits {
        if value.is_empty() {
            continue;
        }
        match metadata_tag {
            MetadataTag::Title => {
                if taken(TAG_XP_TITLE) || taken(exif::Tag::ImageDescription) {
                    return true;
                }
            }
            MetadataTag::Description => {
                if taken(TAG_XP_SUBJECT) {
                    return true;
                }
            }
            MetadataTag::Comment => {
                if taken(TAG_XP_COMMENT) {
                    return true;
                }
            }
            MetadataTag::Keywords => {
                if taken(TAG_XP_KEYWORDS) {
                    return true;
                }
            }
            MetadataTag::Artist => {
                if taken(exif::Tag::Artist) || taken(TAG_XP_AUTHOR) {
                    return true;
                }
            }
            MetadataTag::Copyright => {
                if taken(exif::Tag::Copyright) {
                    return true;
                }
            }
            MetadataTag::Date => {
                if taken(exif::Tag::DateTimeOriginal) {
                    return true;
                }
            }
            MetadataTag::Album | MetadataTag::Genre => {}
        }
    }
    false
}

/// Puts the EXIF fields into a fresh photograph's bytes.
///
/// The picture library writes a photograph with no block of its own, so
/// the block is built and laid into the file as one segment, standing
/// right after the first one. A photograph asked to carry nothing is
/// left exactly as the library made it.
pub(crate) fn jpeg_with_exif(fresh: &[u8], fields: &[exif::Field]) -> Result<Vec<u8>, ()> {
    if fields.is_empty() {
        return Ok(fresh.to_vec());
    }

    let mut writer = exif::experimental::Writer::new();
    for field in fields {
        writer.push_field(field);
    }
    let mut block = Vec::new();
    writer
        .write(&mut std::io::Cursor::new(&mut block), false)
        .map_err(|_| ())?;

    // One segment: the marker, its length (the payload plus the two
    // length bytes themselves), and the payload — the block's own name
    // and then the block.
    let payload = b"Exif\0\0".len() + block.len();
    let segment = u16::try_from(payload + 2).map_err(|_| ())?;
    let mut app1 = Vec::with_capacity(payload + 4);
    app1.extend_from_slice(&[0xFF, 0xE1]);
    app1.extend_from_slice(&segment.to_be_bytes());
    app1.extend_from_slice(b"Exif\0\0");
    app1.extend_from_slice(&block);

    // The block goes after the first segment, so the one that names
    // the picture's family stays first — and after the opening itself.
    let mut at = 2usize;
    if fresh.len() >= 6 && fresh[2] == 0xFF && fresh[3] == 0xE0 {
        let length =
            u16::from_be_bytes([fresh[4], fresh[5]]) as usize;
        at = (4 + length).min(fresh.len());
    }

    let mut out = Vec::with_capacity(fresh.len() + app1.len());
    out.extend_from_slice(&fresh[..at]);
    out.extend_from_slice(&app1);
    out.extend_from_slice(&fresh[at..]);
    Ok(out)
}

/// Formats a system time into a human-readable string.
pub fn format_system_time(time: SystemTime) -> String {
    let Ok(elapsed) = time.duration_since(SystemTime::UNIX_EPOCH) else {
        return "Unknown".to_string();
    };
    let secs = elapsed.as_secs();
    // Simple date representation without large chrono dependency
    let days = secs / 86400;
    // Approximated year/month/day from days since 1970-01-01
    let year = 1970 + days / 365;
    let day_of_year = (days % 365) + 1;
    let month = (day_of_year / 30).min(11) + 1;
    let day = (day_of_year % 30) + 1;
    format!("{year:04}-{month:02}-{day:02}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use std::path::PathBuf;

    #[test]
    fn metadata_tag_labels_respect_kind() {
        assert_eq!(MetadataTag::Artist.label(Some(Kind::Audio), false), "Artist");
        assert_eq!(MetadataTag::Artist.label(Some(Kind::Image), false), "Author");
        assert_eq!(MetadataTag::Date.label(Some(Kind::Image), false), "Date taken");
        assert_eq!(MetadataTag::Date.label(Some(Kind::Audio), false), "Year");
        assert_eq!(MetadataTag::Description.label(None, true), "Subject");
    }

    #[test]
    fn metadata_tag_from_key_matches_common_variations() {
        assert_eq!(MetadataTag::from_key("title"), Some(MetadataTag::Title));
        assert_eq!(MetadataTag::from_key("artist"), Some(MetadataTag::Artist));
        assert_eq!(MetadataTag::from_key("author"), Some(MetadataTag::Artist));
        assert_eq!(MetadataTag::from_key("year"), Some(MetadataTag::Date));
        assert_eq!(MetadataTag::from_key("creation_time"), Some(MetadataTag::Date));
        assert_eq!(MetadataTag::from_key("unknown_key_xyz"), None);
    }

    #[test]
    fn format_file_size_converts_properly() {
        assert_eq!(format_file_size(450), "450 B");
        assert_eq!(format_file_size(2_000), "2 KB");
        assert_eq!(format_file_size(1_500_000), "1.5 MB");
        assert_eq!(format_file_size(2_400_000_000), "2.4 GB");
    }

    #[test]
    fn format_duration_formats_clock() {
        assert_eq!(format_duration(45.0), "0:45");
        assert_eq!(format_duration(75.0), "1:15");
        assert_eq!(format_duration(3600.0), "60:00");
    }

    #[test]
    fn sensitive_keys_are_flagged() {
        assert!(is_sensitive_key("location"));
        assert!(is_sensitive_key("gps_latitude"));
        assert!(is_sensitive_key("camera_serial"));
        assert!(is_sensitive_key("artist"));
        assert!(!is_sensitive_key("title"));
        assert!(!is_sensitive_key("genre"));
    }

    /// A plain photograph, fresh out of the picture library: no block
    /// of tags of its own.
    fn make_photo(folder: &Path, name: &str) -> PathBuf {
        let path = folder.join(name);
        image::DynamicImage::new_rgb8(40, 30)
            .save(&path)
            .expect("the picture was made");
        path
    }

    /// The field a camera writes: what made the picture.
    fn camera_field() -> exif::Field {
        exif::Field {
            tag: exif::Tag::Model,
            ifd_num: exif::In::PRIMARY,
            value: exif::Value::Ascii(vec![b"Camera One\0".to_vec()]),
        }
    }

    /// A photograph carrying a block: the picture with one segment
    /// laid in, holding exactly the fields given.
    fn photo_with_fields(folder: &Path, name: &str, fields: &[exif::Field]) -> PathBuf {
        let plain = make_photo(folder, name);
        let fresh = std::fs::read(&plain).expect("the picture was read");
        let written = jpeg_with_exif(&fresh, fields).expect("the block was laid in");
        std::fs::write(&plain, written).expect("the picture was written");
        plain
    }

    #[test]
    fn a_photograph_carries_its_words_through_a_save() {
        let scratch = std::env::temp_dir().join("open-tangerine-photo-tags");
        let _ = std::fs::remove_dir_all(&scratch);
        std::fs::create_dir_all(&scratch).expect("the scratch folder was made");
        let photo = make_photo(&scratch, "harbor.jpg");
        let fresh = std::fs::read(&photo).expect("the picture was read");

        let edits = vec![
            (MetadataTag::Title, "At the harbor".to_string()),
            (MetadataTag::Description, "A quiet evening".to_string()),
            (MetadataTag::Artist, "Ravi".to_string()),
            (MetadataTag::Keywords, "boats, water".to_string()),
            (MetadataTag::Copyright, "(c) Ravi".to_string()),
            (MetadataTag::Date, "2026:10:19 14:30:00".to_string()),
        ];
        let fields = recompose_exif_fields(Vec::new(), &edits, false);
        let written = jpeg_with_exif(&fresh, &fields).expect("the block was laid in");
        let saved = scratch.join("harbor saved.jpg");
        std::fs::write(&saved, written).expect("the copy was written");

        let said = read_image_tags(&saved).expect("the block read back");
        assert_eq!(said.get(&MetadataTag::Title).map(String::as_str), Some("At the harbor"));
        assert_eq!(said.get(&MetadataTag::Description).map(String::as_str), Some("A quiet evening"));
        assert_eq!(said.get(&MetadataTag::Artist).map(String::as_str), Some("Ravi"));
        // The marks are held the way the file manager holds them.
        assert_eq!(said.get(&MetadataTag::Keywords).map(String::as_str), Some("boats; water"));
        assert_eq!(said.get(&MetadataTag::Copyright).map(String::as_str), Some("(c) Ravi"));
        assert_eq!(said.get(&MetadataTag::Date).map(String::as_str), Some("2026:10:19 14:30:00"));
        let _ = std::fs::remove_dir_all(&scratch);
    }

    #[test]
    fn the_camera_fields_survive_a_save() {
        let scratch = std::env::temp_dir().join("open-tangerine-photo-camera");
        let _ = std::fs::remove_dir_all(&scratch);
        std::fs::create_dir_all(&scratch).expect("the scratch folder was made");
        let camera = photo_with_fields(&scratch, "shot.jpg", &[camera_field()]);
        let fresh = std::fs::read(&camera).expect("the picture was read");
        let original = exif_fields(&camera).expect("the block read back");
        assert!(original.iter().any(|field| field.tag == exif::Tag::Model));

        let edits = vec![(MetadataTag::Title, "Renamed".to_string())];
        let fields = recompose_exif_fields(original, &edits, false);
        let written = jpeg_with_exif(&fresh, &fields).expect("the block was laid in");
        let saved = scratch.join("shot saved.jpg");
        std::fs::write(&saved, written).expect("the copy was written");

        let carried = exif_fields(&saved).expect("the block read back");
        assert!(
            carried.iter().any(|field| field.tag == exif::Tag::Model),
            "the camera's own field is carried over"
        );
        let said = read_image_tags(&saved).expect("the words read back");
        assert_eq!(said.get(&MetadataTag::Title).map(String::as_str), Some("Renamed"));
        let _ = std::fs::remove_dir_all(&scratch);
    }

    #[test]
    fn the_places_and_names_go_when_asked() {
        let camera = camera_field();
        let artist = exif::Field {
            tag: exif::Tag::Artist,
            ifd_num: exif::In::PRIMARY,
            value: exif::Value::Ascii(vec![b"Ravi\0".to_vec()]),
        };
        let latitude = exif::Field {
            tag: exif::Tag(exif::Context::Gps, 0x0002),
            ifd_num: exif::In::PRIMARY,
            value: exif::Value::Ascii(vec![b"44; 12; 33.42\0".to_vec()]),
        };

        let kept = recompose_exif_fields(
            vec![artist.clone(), latitude, camera.clone()],
            &[],
            true,
        );
        assert!(
            !kept.iter().any(|field| field.tag.context() == exif::Context::Gps),
            "the where-it-stood facts go"
        );
        assert!(!kept.iter().any(|field| field.tag == exif::Tag::Artist));
        assert!(
            kept.iter().any(|field| field.tag == exif::Tag::Model),
            "the camera's own field is not one that carries a person's name"
        );
    }
}
