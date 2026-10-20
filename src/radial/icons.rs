//! The little pictures drawn on the segments.
//!
//! Every path is drawn in a 24 by 24 box and stroked rather than filled, which
//! keeps the set visually consistent and lets the colour follow the state of
//! the segment.

use super::actions::IconKind;

/// The outline of a picture.
const PICTURE: &str = "M 3 5 H 21 V 19 H 3 Z M 3 16 L 9 10 L 13 14 L 16 11 L 21 16 \
                       M 15 8.5 A 1.6 1.6 0 1 0 11.8 8.5 A 1.6 1.6 0 1 0 15 8.5";

/// The outline of a page of paper.
const DOCUMENT: &str = "M 6 3 H 14 L 19 8 V 21 H 6 Z M 14 3 V 8 H 19 \
                        M 9 12 H 16 M 9 15.5 H 16 M 9 19 H 13";

/// The outline of a strip of film.
const FILM: &str = "M 4 5 H 20 V 19 H 4 Z M 4 9 H 20 M 4 15 H 20 \
                    M 7 5 V 9 M 7 15 V 19 M 10.5 10 L 15 12 L 10.5 14 Z";

/// The outline of an archive.
const ARCHIVE: &str = "M 3 5 H 21 V 9 H 3 Z M 5 9 V 20 H 19 V 9 M 10 13 H 14";

/// The outline of a folder.
const FOLDER: &str = "M 3 6 H 9 L 11 9 H 21 V 19 H 3 Z";

/// Three lines of text.
const LINES: &str = "M 4 7 H 20 M 4 12 H 20 M 4 17 H 13";

/// Two sheets of paper, one on top of the other.
const COPY: &str = "M 9 9 H 20 V 20 H 9 Z M 15 5 H 4 V 16 H 9";

/// A pencil.
const PENCIL: &str = "M 4 20 L 5.5 15.5 L 15.5 5.5 L 18.5 8.5 L 8.5 18.5 Z \
                      M 13.5 7.5 L 16.5 10.5";

/// Two lines coming together.
const MERGE: &str = "M 4 5 L 12 13 L 20 5 M 12 13 V 20 M 9 17 L 12 20 L 15 17";

/// The marks a cropper leaves behind.
const CROP: &str = "M 7 3 V 17 H 21 M 3 7 H 17 V 21";

/// Two lines and an arrow between them.
const GAUGE: &str = "M 4 6 H 20 M 4 19 H 20 M 12 9 V 15 M 9 12 L 12 15 L 15 12";

/// The path of the picture for a kind of choice.
pub fn path_for(kind: IconKind) -> &'static str {
    match kind {
        IconKind::Picture => PICTURE,
        IconKind::Document => DOCUMENT,
        IconKind::Film => FILM,
        IconKind::Archive => ARCHIVE,
        IconKind::Folder => FOLDER,
        IconKind::Lines => LINES,
        IconKind::Copy => COPY,
        IconKind::Pencil => PENCIL,
        IconKind::Merge => MERGE,
        IconKind::Scissors => CROP,
        IconKind::Gauge => GAUGE,
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
            IconKind::Lines,
            IconKind::Copy,
            IconKind::Pencil,
            IconKind::Merge,
            IconKind::Scissors,
            IconKind::Gauge,
        ] {
            let path = path_for(kind);
            assert!(path.starts_with('M'), "{kind:?} has no path");
            assert!(path.len() > 10);
        }
    }

    #[test]
    fn the_icons_stay_inside_their_box() {
        for kind in [IconKind::Picture, IconKind::Document, IconKind::Film] {
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
