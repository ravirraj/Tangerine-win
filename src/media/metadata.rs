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
    if let Some(tools) = tools {
        let tags = probe_format_tags(tools, path);
        for (key, val) in tags {
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
pub fn read_tags(path: &Path, tools: Option<&Tools>) -> HashMap<MetadataTag, String> {
    let mut result = HashMap::new();
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

/// Formats a system time into a human-readable string.
fn format_system_time(time: SystemTime) -> String {
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
}
