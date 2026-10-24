//! The little pictures drawn on the segments.
//!
//! Every path is drawn in a 24 by 24 box and stroked rather than filled, which
//! keeps the set visually consistent and lets the colour follow the state of
//! the segment.

use super::actions::IconKind;

/// A picture icon.
///
/// A camera/viewfinder silhouette in a square frame.
const PICTURE: &str = "M 3 5 H 21 V 19 H 3 Z M 5 7 H 19 V 17 H 5 Z M 8 10 L 12 13 L 16 11";

/// A document icon.
///
/// A page with text lines.
const DOCUMENT: &str =
    "M 6 3 H 14 L 19 8 V 21 H 6 Z M 14 3 V 8 H 19 M 6 7 H 14 M 6 10 H 12 M 6 13 H 16";

/// A film strip icon.
///
/// Perforated strip with frame in the middle.
const FILM: &str =
    "M 4 5 H 20 V 19 H 4 Z M 4 9 H 20 M 4 15 H 20 M 7 5 V 9 M 7 15 V 19 M 12 10 L 15 12 L 12 14";

/// A music icon.
///
/// Two notes joined by a beam.
const MUSIC: &str = "M 6.5 17.5 A 2 2 0 1 0 6.5 17.51 M 8.5 17.5 V 7.5 L 16.5 5.5 V 15.5 M 16.5 15.5 A 2 2 0 1 0 16.5 15.51 M 8.5 11.5 L 16.5 9.5";

/// An archive icon.
///
/// A box with a label strip.
const ARCHIVE: &str = "M 3 5 H 21 V 9 H 3 Z M 5 9 V 20 H 19 V 9 M 8 13 H 14";

/// A folder icon.
///
/// Tab folder shape.
const FOLDER: &str = "M 3 6 H 9 L 11 9 H 21 V 19 H 3 Z";

/// A copy icon.
///
/// Two overlapping sheets of paper.
const COPY: &str = "M 9 9 H 20 V 20 H 9 Z M 15 5 H 4 V 16 H 9";

/// A pencil icon.
///
/// Angled pencil with tip.
const PENCIL: &str = "M 4 20 L 5.5 15.5 L 15.5 5.5 L 18.5 8.5 L 8.5 18.5 Z M 13.5 7.5 L 16.5 10.5";

/// A merge icon.
///
/// Three arrows converging from top, one going down.
const MERGE: &str = "M 4 5 L 12 13 L 20 5 M 12 13 V 20 M 9 17 L 12 20 L 15 17";

/// A scissors icon.
///
/// Crossed scissor blades.
const SCISSORS: &str = "M 7 3 V 17 H 21 M 3 7 H 17 V 21";

/// A compress icon.
///
/// Box with arrows pointing inward.
const COMPRESS: &str = "M 4 4 H 20 V 20 H 4 Z M 8 8 H 16 V 16 H 8 Z M 4 12 L 8 12 M 20 12 L 16 12 M 12 4 L 12 8 M 12 20 L 12 16";

/// A crop icon.
///
/// Corner brackets at each corner.
const CROP: &str = "M 4 4 H 10 M 4 4 V 10 M 20 4 H 14 M 20 4 V 10 M 4 20 H 10 M 4 20 V 14 M 20 20 H 14 M 20 20 V 14";

/// A path icon.
///
/// A curved path line.
const PATH: &str = "M 4 4 L 12 12 L 20 4 M 12 12 L 16 8 M 12 12 L 16 16";

/// A rotate icon.
///
/// A curved arrow.
const ROTATE: &str = "M 4 12 L 10 4 L 12 12 L 10 20 M 12 12 L 20 12 M 20 12 L 16 8 M 20 12 L 16 16";

/// A resize icon.
///
/// Diagonal arrows pointing outward.
const RESIZE: &str = "M 6 6 L 10 10 M 6 10 L 10 6 M 14 14 L 18 18 M 14 18 L 18 14";

/// A terminal icon.
///
/// A command prompt symbol.
const TERMINAL: &str = "M 4 4 H 20 V 20 H 4 Z M 8 8 H 16 V 8 M 8 10 H 16 V 10 M 8 12 H 16 V 12 M 8 14 H 16 V 14 M 8 16 H 16 V 16";

/// An extract icon.
///
/// An arrow pointing out of a box.
const EXTRACT: &str = "M 4 4 H 20 V 20 H 4 Z M 8 12 L 12 8 L 16 12 M 12 8 L 12 4";

/// The path of the picture for a kind of choice.
pub fn path_for(kind: IconKind) -> &'static str {
    match kind {
        IconKind::Picture => PICTURE,
        IconKind::Document => DOCUMENT,
        IconKind::Film => FILM,
        IconKind::Music => MUSIC,
        IconKind::Archive => ARCHIVE,
        IconKind::Folder => FOLDER,
        IconKind::Copy => COPY,
        IconKind::Pencil => PENCIL,
        IconKind::Merge => MERGE,
        IconKind::Scissors => SCISSORS,
        IconKind::Compress => COMPRESS,
        IconKind::Crop => CROP,
        IconKind::Rotate => ROTATE,
        IconKind::Resize => RESIZE,
        IconKind::Terminal => TERMINAL,
        IconKind::Extract => EXTRACT,
        IconKind::Path => PATH,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_icon_has_a_path() {
        for kind in [
            IconKind::Picture,
            IconKind::Document,
            IconKind::Film,
            IconKind::Music,
            IconKind::Archive,
            IconKind::Folder,
            IconKind::Copy,
            IconKind::Pencil,
            IconKind::Merge,
            IconKind::Scissors,
            IconKind::Compress,
            IconKind::Crop,
            IconKind::Rotate,
            IconKind::Resize,
            IconKind::Terminal,
            IconKind::Extract,
            IconKind::Path,
        ] {
            let path = path_for(kind);
            assert!(path.starts_with('M'), "{kind:?} has no path");
            assert!(path.len() > 10);
        }
    }

    #[test]
    fn the_icons_stay_inside_their_box() {
        for kind in [
            IconKind::Picture,
            IconKind::Document,
            IconKind::Film,
            IconKind::Music,
            IconKind::Archive,
            IconKind::Folder,
            IconKind::Copy,
            IconKind::Pencil,
            IconKind::Merge,
            IconKind::Scissors,
            IconKind::Compress,
            IconKind::Crop,
            IconKind::Rotate,
            IconKind::Resize,
            IconKind::Terminal,
            IconKind::Extract,
            IconKind::Path,
        ] {
            let path = path_for(kind);
            let numbers: Vec<f32> = path
                .split_whitespace()
                .filter_map(|word| word.parse::<f32>().ok())
                .collect();
            for value in numbers {
                assert!(
                    (-24.0..=24.0).contains(&value),
                    "{kind:?} has {value} outside the box"
                );
            }
        }
    }
}
