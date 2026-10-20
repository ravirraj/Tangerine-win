//! The Windows platform layer.
//!
//! Everything that talks to Win32 lives under `windows`, so the rest of the
//! program never sees an `HWND` or a virtual key code.

pub mod windows;
