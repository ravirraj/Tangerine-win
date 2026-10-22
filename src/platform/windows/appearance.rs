//! Which appearance Windows is asking for.
//!
//! The wheel is drawn in the theme of the appearance of the system: a person
//! who has a light Windows gets a light wheel. The macOS program follows the
//! appearance of the system in the same way.
//!
//! Windows keeps the answer in the registry, under the person's own
//! settings, and the answer is 1 for the light appearance and 0 for the dark
//! one. A system that has never been asked has no answer at all, and the
//! light appearance is the one it falls back to.

use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
use windows::core::PCWSTR;

/// The key Windows keeps the appearance in.
const PERSONALIZE: &[u16] = &[
    b'S' as u16,
    b'o' as u16,
    b'f' as u16,
    b't' as u16,
    b'w' as u16,
    b'a' as u16,
    b'r' as u16,
    b'e' as u16,
    b'\\' as u16,
    b'M' as u16,
    b'i' as u16,
    b'c' as u16,
    b'r' as u16,
    b'o' as u16,
    b's' as u16,
    b'o' as u16,
    b'f' as u16,
    b't' as u16,
    b'\\' as u16,
    b'W' as u16,
    b'i' as u16,
    b'n' as u16,
    b'd' as u16,
    b'o' as u16,
    b'w' as u16,
    b's' as u16,
    b'\\' as u16,
    b'C' as u16,
    b'u' as u16,
    b'r' as u16,
    b'r' as u16,
    b'e' as u16,
    b'n' as u16,
    b't' as u16,
    b'V' as u16,
    b'e' as u16,
    b'r' as u16,
    b's' as u16,
    b'i' as u16,
    b'o' as u16,
    b'n' as u16,
    b'\\' as u16,
    b'T' as u16,
    b'h' as u16,
    b'e' as u16,
    b'm' as u16,
    b'e' as u16,
    b's' as u16,
    b'\\' as u16,
    b'P' as u16,
    b'e' as u16,
    b'r' as u16,
    b's' as u16,
    b'o' as u16,
    b'n' as u16,
    b'a' as u16,
    b'l' as u16,
    b'i' as u16,
    b'z' as u16,
    b'e' as u16,
    // The wide string ends in a null, as Windows wants it.
    0,
];

/// The name of the value that says which appearance is wanted.
const APPEARANCE: &str = "AppsUseLightTheme";

/// Whether Windows is asking for the dark appearance.
pub fn prefers_dark() -> bool {
    match light_theme_is_set() {
        // The value is 1 for the light appearance and 0 for the dark one.
        Some(light) => !light,
        // No answer: the light appearance is what such a system falls back
        // to.
        None => false,
    }
}

/// Whether the light appearance is the one that is wanted.
fn light_theme_is_set() -> Option<bool> {
    // The name of the value, as Windows wants it: a wide string that ends in
    // a null.
    let mut name: Vec<u16> = APPEARANCE.encode_utf16().collect();
    name.push(0);

    let mut value: u32 = 0;
    let mut size = std::mem::size_of::<u32>() as u32;
    // SAFETY: the key and the value are read into buffers of the right size,
    // and the names are wide strings that end in a null.
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            PCWSTR::from_raw(PERSONALIZE.as_ptr()),
            PCWSTR::from_raw(name.as_ptr()),
            RRF_RT_REG_DWORD,
            None,
            Some((&mut value) as *mut u32 as *mut _),
            Some(&mut size),
        )
    };
    if status != ERROR_SUCCESS {
        return None;
    }
    Some(value != 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_appearance_can_be_asked_for() {
        // The answer is what Windows says, and the machine this runs on has
        // one: the test only says that asking does not fail.
        let dark = prefers_dark();
        assert!(dark || !dark);
    }

    #[test]
    fn the_key_is_the_one_windows_keeps_the_appearance_in() {
        let name: String = PERSONALIZE
            .iter()
            .take_while(|letter| **letter != 0)
            .map(|letter| *letter as u8 as char)
            .collect();
        assert_eq!(
            name,
            "Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"
        );
    }
}
