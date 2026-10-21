//! Putting a finished file at its final name, on Windows.
//!
//! The move is made with `MoveFileEx` and without the flag that allows
//! overwriting, so the move fails when something is already at the name.
//! The file is therefore never written over, and it either arrives whole
//! or does not arrive at all.

use std::os::windows::ffi::OsStrExt;
use std::path::Path;

use windows::Win32::Foundation::ERROR_ALREADY_EXISTS;
use windows::Win32::Storage::FileSystem::{MOVE_FILE_FLAGS, MoveFileExW};
use windows::core::{HRESULT, PCWSTR};

use crate::media::publish::{PublishError, Publisher};

/// Puts files at their final names, on Windows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WindowsPublisher;

impl Publisher for WindowsPublisher {
    fn publish(&self, temporary: &Path, destination: &Path) -> Result<(), PublishError> {
        let source = wide(temporary);
        let target = wide(destination);

        // SAFETY: both strings are null-terminated and live until the call
        // returns. The flags are zero, so nothing is overwritten and the
        // move is not allowed to be undone by a reboot.
        let moved = unsafe {
            MoveFileExW(
                PCWSTR(source.as_ptr()),
                PCWSTR(target.as_ptr()),
                MOVE_FILE_FLAGS(0),
            )
        };

        match moved {
            Ok(()) => Ok(()),
            Err(error) => {
                if error.code() == HRESULT::from_win32(ERROR_ALREADY_EXISTS.0) {
                    Err(PublishError::DestinationExists)
                } else {
                    Err(PublishError::CouldNotMove)
                }
            }
        }
    }
}

/// A path as the wide string the system wants.
fn wide(path: &Path) -> Vec<u16> {
    path.as_os_str().encode_wide().chain(Some(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_move_to_a_name_that_is_free_succeeds() {
        let folder = std::env::temp_dir().join("open-tangerine-publish-free");
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).expect("the folder could be made");
        let temporary = folder.join("photo.part");
        let destination = folder.join("photo.jpeg");
        std::fs::write(&temporary, b"picture").expect("the file could be made");

        let publisher = WindowsPublisher;
        assert_eq!(
            publisher.publish(&temporary, &destination),
            Ok(()),
            "the file was not moved"
        );
        assert_eq!(
            std::fs::read(&destination).expect("the file is there"),
            b"picture"
        );
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn a_move_to_a_name_that_is_taken_does_not_overwrite() {
        let folder = std::env::temp_dir().join("open-tangerine-publish-taken");
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).expect("the folder could be made");
        let temporary = folder.join("photo.part");
        let destination = folder.join("photo.jpeg");
        std::fs::write(&temporary, b"new picture").expect("the file could be made");
        std::fs::write(&destination, b"the old picture").expect("the file could be made");

        let publisher = WindowsPublisher;
        assert_eq!(
            publisher.publish(&temporary, &destination),
            Err(PublishError::DestinationExists)
        );
        assert_eq!(
            std::fs::read(&destination).expect("the file is there"),
            b"the old picture",
            "the file that was there was overwritten"
        );
        let _ = std::fs::remove_dir_all(&folder);
    }
}
