//! Open Tangerine for Windows.
//!
//! The program is a small system utility: it waits in the background, notices
//! a file drag coming out of Explorer, and puts a radial wheel around the
//! pointer when the user holds a modifier down.
//!
//! The branch still stands parts up whose wiring has not come: a settings
//! window, a completion chime, the shell menu question, helpers the capture
//! tests alone read. In a development build the tests speak for them and no
//! one may be quieted; in a release build nothing does, so this one note says
//! they are known and meant rather than lost.
#![cfg_attr(not(debug_assertions), allow(dead_code))]
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod diagnostics;
mod lifecycle;
mod media;
mod platform;
mod radial;
mod session;
pub mod settings;
mod ui;

fn main() {
    diagnostics::set_level_from_env();

    let arguments: Vec<String> = std::env::args().collect();
    let options = match app::parse_options(&arguments) {
        Ok(options) => options,
        Err(message) => {
            eprintln!("{message}");
            // Asking for help or for the version is not a failure, so the
            // program says what was asked and stops with a code that means
            // it worked.
            let asked = arguments
                .iter()
                .any(|a| matches!(a.as_str(), "--help" | "-h" | "--version"));
            std::process::exit(if asked { 0 } else { 2 });
        }
    };

    if let Err(error) = app::run(options) {
        diagnostics::error("start", &error);
        eprintln!("Open Tangerine could not start: {error}");
        std::process::exit(1);
    }
}
