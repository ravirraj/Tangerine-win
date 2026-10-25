//! Compiles the Slint markup into Rust before the program is built.
//!
//! The revision the tree is at is put in the program, so that any copy of it
//! can say where it came from. A program that says which revision it is
//! cannot be mistaken for one that was built later, which is the whole
//! trouble with a folder of executables.

use std::path::PathBuf;
use std::process::Command;

fn main() {
    let path = "src/ui/wheel.slint";
    println!("cargo:rerun-if-changed={path}");
    println!("cargo:rerun-if-changed=src/ui/tool_window.slint");
    // The revision is part of what the program is, so the program is made
    // again when the revision moves. The head of the repository points at the
    // branch, and the branch is what moves.
    println!(
        "cargo:rerun-if-changed={}",
        git_directory().join("HEAD").display()
    );
    if let Some(reference) = reference_file() {
        println!("cargo:rerun-if-changed={}", reference.display());
    }
    // A caller can say which revision the program is of, and the program is
    // then made again when that answer changes. This is how the packaging
    // script asks for a program of the tree it is packaging.
    println!("cargo:rerun-if-env-changed=OPEN_TANGERINE_REVISION");
    println!("cargo:rustc-env=OPEN_TANGERINE_REVISION={}", revision());
    slint_build::compile(path).expect("the wheel markup could not be compiled");
}

/// The revision the tree is at, and whether anything has been changed since.
///
/// When there is no git, or no answer from it, the revision is unknown. A
/// program that says so is more useful than one that names a revision that
/// may be wrong.
fn revision() -> String {
    // What the caller says comes first: the packaging script knows the
    // revision of the tree it is packaging, and asking it means the program
    // is made again when the tree moves.
    if let Ok(given) = std::env::var("OPEN_TANGERINE_REVISION")
        && !given.trim().is_empty()
    {
        return given.trim().to_string();
    }
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

/// The directory the repository is in.
fn git_directory() -> PathBuf {
    match git(&["rev-parse", "--absolute-git-dir"]) {
        Some(directory) => PathBuf::from(directory),
        None => PathBuf::from(".git"),
    }
}

/// The file the head of the repository points at.
///
/// The head is a name of a branch, and the branch is a file whose contents
/// change when the branch moves. Watching that file is what makes the program
/// be made again after a commit.
fn reference_file() -> Option<PathBuf> {
    let head = git_directory().join("HEAD");
    let contents = std::fs::read_to_string(&head).ok()?;
    let reference = contents.trim().strip_prefix("ref: ")?;
    Some(git_directory().join(reference))
}
