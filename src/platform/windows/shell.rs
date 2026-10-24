//! Shell integration for reveal-in-Explorer file action.

use std::path::Path;
use std::process::Command;

/// Reveal the given path in Windows Explorer.
pub fn reveal_in_explorer(path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let path_str = path.to_string_lossy();
    Command::new("explorer.exe")
        .args(["/select,", path_str.as_ref()])
        .spawn()
        .map_err(|_| "failed to start explorer.exe")?;
    Ok(())
}
