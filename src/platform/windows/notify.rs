//! System notifications and audio feedback.
//!
//! Plays audio chime when operations finish, and displays Windows notifications.

use crate::settings::AppSettings;

#[link(name = "user32")]
unsafe extern "system" {
    fn MessageBeep(uType: u32) -> i32;
}

const MB_OK: u32 = 0x00000000;

/// Plays completion chime if audio notifications are enabled in settings.
pub fn play_completion_chime() {
    let settings = AppSettings::load();
    if settings.sound_enabled {
        unsafe {
            let _ = MessageBeep(MB_OK);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn play_chime_runs_without_panic() {
        play_completion_chime();
    }
}
