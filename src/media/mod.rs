//! Working with media through an external program.
//!
//! The program is FFmpeg, run as a child process. Nothing in this module
//! knows how FFmpeg works inside: the file `ffmpeg/components.txt` says
//! which parts of it the build has, and the binary itself is asked what it
//! can do.
//!
//! The parts are:
//!
//! - [`manifest`] reads the component manifest, so that the names of the
//!   parts and the way they fit together are in one place;
//! - [`locator`] finds the program, and the program that goes with it;
//! - [`runner`] runs a child process, behind a trait so that the tests can
//!   stand in for it;
//! - [`probe`] asks the program what it can do;
//! - [`command`] builds the argument lists;
//! - [`animation`] decides whether a file holds more than one frame;
//! - [`publish`] puts a finished file at its final name.

pub mod animation;
pub mod command;
pub mod convert;
pub mod locator;
pub mod manifest;
pub mod probe;
pub mod publish;
pub mod runner;
