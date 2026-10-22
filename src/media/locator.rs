//! Finding the program.
//!
//! The program is looked for beside the application first, because that is
//! where a release puts it and where a developer puts it. A build that is
//! not a release build may also look along the `PATH`, but a release build
//! never does: a released program uses the program it was given, and not
//! one that happens to be installed on the machine.

use std::path::{Path, PathBuf};

use crate::media::manifest::Manifest;

/// The programs the conversion needs.
///
/// They are found together, so that the program that does the work and the
/// program that inspects the file are always the same build.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Tools {
    /// The program that converts.
    pub ffmpeg: PathBuf,
    /// The program that inspects.
    pub ffprobe: PathBuf,
}

impl Tools {
    /// The two programs, if they are both there.
    pub fn new(ffmpeg: PathBuf, ffprobe: PathBuf) -> Option<Tools> {
        if ffmpeg.is_file() && ffprobe.is_file() {
            Some(Tools { ffmpeg, ffprobe })
        } else {
            None
        }
    }
}

/// The names of the programs, as the manifest gives them.
///
/// The manifest says which programs the build makes, and the program looks
/// for those and no others.
fn program_names() -> (&'static str, &'static str) {
    let manifest = Manifest::get();
    let ffmpeg = manifest
        .programs
        .first()
        .map(String::as_str)
        .unwrap_or("ffmpeg");
    let ffprobe = manifest
        .programs
        .get(1)
        .map(String::as_str)
        .unwrap_or("ffprobe");
    (ffmpeg, ffprobe)
}

/// The ending a program has on this platform.
fn program_ending() -> &'static str {
    if cfg!(windows) { ".exe" } else { "" }
}

/// Finds the programs, if they are there.
pub fn find() -> Option<Tools> {
    // Beside the application, in the folder a release puts them in.
    if let Some(executable) = current_executable()
        && let Some(folder) = executable.parent()
    {
        if let Some(tools) = in_folder(&folder.join("ffmpeg")) {
            return Some(tools);
        }
        // A developer keeps a build in the repository.
        if let Some(tools) = in_folder(&folder.join("vendor").join("ffmpeg")) {
            return Some(tools);
        }
        // A program built by cargo sits in target/debug or target/release, so
        // the repository is two folders above it.
        if folder
            .file_name()
            .is_some_and(|name| name == "debug" || name == "release")
            && let Some(root) = folder.parent().and_then(Path::parent)
            && let Some(tools) = in_folder(&root.join("vendor").join("ffmpeg"))
        {
            return Some(tools);
        }
    }

    // Along the PATH, but only where a build is not a release build. A
    // released program does not go looking for another one.
    if cfg!(debug_assertions)
        && let Some(tools) = along_the_path()
    {
        return Some(tools);
    }

    None
}

/// The two programs in a folder, if they are both there.
fn in_folder(folder: &Path) -> Option<Tools> {
    let (ffmpeg_name, probe_name) = program_names();
    let ffmpeg = folder.join(format!("{ffmpeg_name}{}", program_ending()));
    let ffprobe = folder.join(format!("{probe_name}{}", program_ending()));
    Tools::new(ffmpeg, ffprobe)
}

/// The two programs along the `PATH`.
fn along_the_path() -> Option<Tools> {
    let path = std::env::var_os("PATH")?;
    for folder in std::env::split_paths(&path) {
        if let Some(tools) = in_folder(&folder) {
            return Some(tools);
        }
    }
    None
}

/// The path of this program.
fn current_executable() -> Option<PathBuf> {
    std::env::current_exe().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_folder_without_the_programs_has_none() {
        let folder = std::env::temp_dir().join("open-tangerine-no-tools-here");
        let _ = std::fs::create_dir_all(&folder);
        assert!(in_folder(&folder).is_none());
    }

    #[test]
    fn a_folder_with_only_one_of_them_has_none() {
        let folder = std::env::temp_dir().join("open-tangerine-half-tools");
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).expect("the folder could be made");
        std::fs::write(
            folder.join(format!("{}{}", program_names().0, program_ending())),
            b"x",
        )
        .expect("the file could be made");
        assert!(in_folder(&folder).is_none());
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn a_folder_with_both_of_them_has_them() {
        let folder = std::env::temp_dir().join("open-tangerine-both-tools");
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).expect("the folder could be made");
        let (ffmpeg_name, probe_name) = program_names();
        for name in [ffmpeg_name, probe_name] {
            std::fs::write(folder.join(format!("{name}{}", program_ending())), b"x")
                .expect("the file could be made");
        }
        let tools = in_folder(&folder).expect("both are there");
        assert!(tools.ffmpeg.is_file());
        assert!(tools.ffprobe.is_file());
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn the_manifest_says_which_programs_are_built() {
        let manifest = Manifest::get();
        assert!(manifest.programs.contains(&"ffmpeg".to_string()));
        assert!(manifest.programs.contains(&"ffprobe".to_string()));
    }
}
