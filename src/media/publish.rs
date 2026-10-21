//! Putting a finished file at its final name.
//!
//! The file is made in the folder it will live in and is then moved to its
//! name. The move never overwrites: if something is already at that name,
//! the move fails and the caller is told, and the caller picks another
//! name. Nothing is ever written over the user's file.
//!
//! The way the move is made belongs to the platform, so it is behind a
//! small interface. Everything that decides *what* the name should be is
//! ordinary code and knows nothing about the platform.

use std::path::Path;

/// Why a file could not be put at its name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PublishError {
    /// Something is already at that name.
    ///
    /// The file has not been touched. Another name has to be chosen.
    DestinationExists,
    /// The file could not be moved for some other reason.
    CouldNotMove,
}

/// Puts a finished file at its final name.
pub trait Publisher {
    /// Moves the file from where it was made to where it belongs.
    ///
    /// The two paths are on the same place, so the move is a rename and not
    /// a copy: the file either arrives whole or does not arrive at all.
    fn publish(&self, temporary: &Path, destination: &Path) -> Result<(), PublishError>;
}

/// The publisher of this platform.
pub use crate::platform::windows::publish::WindowsPublisher as PlatformPublisher;

/// The name a file should have, when the one that was asked for is taken.
///
/// The name is the name of the original with another extension: `photo.png`
/// becomes `photo.jpeg`. When that name is taken, a number is put in front
/// of the extension, the way a file manager does it: `photo (2).jpeg`.
pub fn name_for(original: &Path, extension: &str) -> std::path::PathBuf {
    let stem = original
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_else(|| "converted".to_string());
    original.with_file_name(format!("{stem}.{extension}"))
}

/// The names a file may have, in the order they are tried.
///
/// The first is the plain name; the rest carry a number.
pub fn names_for(original: &Path, extension: &str) -> Vec<std::path::PathBuf> {
    let first = name_for(original, extension);
    let mut names = vec![first.clone()];
    let stem = first
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_else(|| "converted".to_string());
    let parent = first.parent().map(|parent| parent.to_path_buf());
    for number in 2..1000 {
        let candidate = match &parent {
            Some(parent) => parent.join(format!("{stem} ({number}).{extension}")),
            None => std::path::PathBuf::from(format!("{stem} ({number}).{extension}")),
        };
        names.push(candidate);
    }
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_name_keeps_the_original_and_changes_the_extension() {
        let name = name_for(Path::new("C:\\pictures\\photo.png"), "jpeg");
        assert_eq!(name, Path::new("C:\\pictures\\photo.jpeg"));
    }

    #[test]
    fn a_taken_name_gets_a_number() {
        let names = names_for(Path::new("C:\\pictures\\photo.png"), "jpeg");
        assert_eq!(names[0], Path::new("C:\\pictures\\photo.jpeg"));
        assert_eq!(names[1], Path::new("C:\\pictures\\photo (2).jpeg"));
        assert_eq!(names[2], Path::new("C:\\pictures\\photo (3).jpeg"));
    }

    #[test]
    fn a_name_with_no_stem_still_has_one() {
        // A file whose name is only an extension has no stem, and the name
        // it is given keeps what is there.
        let name = name_for(Path::new("C:\\pictures\\.png"), "jpeg");
        assert_eq!(name, Path::new("C:\\pictures\\.png.jpeg"));
    }
}
