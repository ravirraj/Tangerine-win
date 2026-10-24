//! Clipboard access for copy-path file action.

use std::io::Write;
use std::process::Command;

/// Set the clipboard text to the given string using the `clip.exe` utility.
pub fn set_text(text: &str) -> Result<(), Box<dyn std::error::Error>> {
    let mut child = Command::new("clip.exe")
        .stdin(std::process::Stdio::piped())
        .spawn()
        .map_err(|_| "failed to start clip.exe")?;
    child
        .stdin
        .take()
        .ok_or("failed to open clipboard stdin")?
        .write_all(text.as_bytes())
        .map_err(|_| "failed to write to clipboard")?;
    child.wait().map_err(|_| "failed to wait for clip.exe")?;
    Ok(())
}
