//! Windows Explorer context menu integration.
//!
//! Registers "Open with Tangerine" in the Windows shell context menu for all files
//! under `HKCU\Software\Classes\*\shell\OpenTangerine`.

use std::env;
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::System::Registry::{
    RegCloseKey, RegCreateKeyExW, RegDeleteTreeW, RegOpenKeyExW, RegSetValueExW, HKEY,
    HKEY_CURRENT_USER, KEY_ALL_ACCESS, KEY_READ, REG_OPTION_NON_VOLATILE, REG_SZ,
};

use crate::platform::windows::windowhost::to_wide;

const SHELL_KEY_PATH: PCWSTR = w!(r"Software\Classes\*\shell\OpenTangerine");
#[allow(dead_code)]
const COMMAND_SUBKEY: PCWSTR = w!("command");

pub fn is_shell_menu_registered() -> bool {
    unsafe {
        let mut key = HKEY::default();
        let status = RegOpenKeyExW(
            HKEY_CURRENT_USER,
            SHELL_KEY_PATH,
            Some(0),
            KEY_READ,
            &mut key,
        );
        if status == ERROR_SUCCESS {
            let _ = RegCloseKey(key);
            true
        } else {
            false
        }
    }
}

#[allow(dead_code)]
pub fn register_shell_menu() -> Result<(), String> {
    let current_exe = env::current_exe().map_err(|e| e.to_string())?;
    let exe_str = current_exe.to_string_lossy().into_owned();
    let cmd_str = format!("\"{}\" \"%1\"", exe_str);

    let verb_label = to_wide("Open with Tangerine");
    let cmd_wide = to_wide(&cmd_str);
    let icon_wide = to_wide(&exe_str);

    unsafe {
        let mut key = HKEY::default();
        let status = RegCreateKeyExW(
            HKEY_CURRENT_USER,
            SHELL_KEY_PATH,
            Some(0),
            PCWSTR::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_ALL_ACCESS,
            None,
            &mut key,
            None,
        );
        if status != ERROR_SUCCESS {
            return Err("Failed to create registry key".to_string());
        }

        // Set default value (menu label)
        let _ = RegSetValueExW(
            key,
            PCWSTR::null(),
            Some(0),
            REG_SZ,
            Some(std::slice::from_raw_parts(
                verb_label.as_ptr() as *const u8,
                verb_label.len() * 2,
            )),
        );

        // Set Icon value
        let _ = RegSetValueExW(
            key,
            w!("Icon"),
            Some(0),
            REG_SZ,
            Some(std::slice::from_raw_parts(
                icon_wide.as_ptr() as *const u8,
                icon_wide.len() * 2,
            )),
        );

        // Create command subkey
        let mut cmd_key = HKEY::default();
        let cmd_status = RegCreateKeyExW(
            key,
            COMMAND_SUBKEY,
            Some(0),
            PCWSTR::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_ALL_ACCESS,
            None,
            &mut cmd_key,
            None,
        );
        if cmd_status == ERROR_SUCCESS {
            let _ = RegSetValueExW(
                cmd_key,
                PCWSTR::null(),
                Some(0),
                REG_SZ,
                Some(std::slice::from_raw_parts(
                    cmd_wide.as_ptr() as *const u8,
                    cmd_wide.len() * 2,
                )),
            );
            let _ = RegCloseKey(cmd_key);
        }

        let _ = RegCloseKey(key);
        Ok(())
    }
}

#[allow(dead_code)]
pub fn unregister_shell_menu() -> Result<(), String> {
    unsafe {
        let status = RegDeleteTreeW(HKEY_CURRENT_USER, SHELL_KEY_PATH);
        if status == ERROR_SUCCESS {
            Ok(())
        } else {
            Err("Registry key not found or could not be removed".to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_shell_menu_registration_does_not_panic() {
        let _ = is_shell_menu_registered();
    }
}
