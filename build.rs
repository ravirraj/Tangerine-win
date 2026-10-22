//! Compiles the Slint markup into Rust before the program is built.
//!
//! The revision the tree is at is put in the program, so that any copy of it
//! can say where it came from. A program that says which revision it is
//! cannot be mistaken for one that was built later, which is the whole
//! trouble with a folder of executables.

use std::process::Command;

fn main() {
    let path = "src/ui/wheel.slint";
    println!("cargo:rerun-if-changed={path}");
    // The revision is part of what the program is, so the program is made
    // again when the revision moves.
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rustc-env=OPEN_TANGERINE_REVISION={}", revision());
    slint_build::compile(path).expect("the wheel markup could not be compiled");
}

/// The revision the tree is at, and whether anything has been changed since.
///
/// When there is no git, or no answer from it, the revision is unknown. A
/// program that says so is more useful than one that names a revision that
/// may be wrong.
fn revision() -> String {
    let Some(head) = git(&["rev-parse", "--short", "HEAD"]) else {
        return "unknown".to_string();
    };
    let dirty = git(&["status", "--porcelain"]).is_some_and(|out| !out.trim().is_empty());
    if dirty { format!("{head}-dirty") } else { head }
}

/// Asks git something, and gives nothing back when it cannot answer.
fn git(arguments: &[&str]) -> Option<String> {
    let output = Command::new("git").args(arguments).output().ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
}
