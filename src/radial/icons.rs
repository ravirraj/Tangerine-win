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

/// The path of the picture for a kind of choice.
pub fn path_for(kind: IconKind) -> &'static str {
    match kind {
        IconKind::Picture => PICTURE,
        IconKind::Document => DOCUMENT,
        IconKind::Film => FILM,
        IconKind::Archive => ARCHIVE,
        IconKind::Folder => FOLDER,
        IconKind::Copy => COPY,
        IconKind::Pencil => PENCIL,
        IconKind::Merge => MERGE,
        IconKind::Scissors => SCISSORS,
        IconKind::Compress => COMPRESS,
        IconKind::Crop => CROP,
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
            IconKind::Archive,
            IconKind::Folder,
            IconKind::Copy,
            IconKind::Pencil,
            IconKind::Merge,
            IconKind::Scissors,
            IconKind::Compress,
            IconKind::Crop,
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
            IconKind::Archive,
            IconKind::Folder,
            IconKind::Copy,
            IconKind::Pencil,
            IconKind::Merge,
            IconKind::Scissors,
            IconKind::Compress,
            IconKind::Crop,
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
