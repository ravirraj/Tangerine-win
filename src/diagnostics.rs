//! Structured diagnostics.
//!
//! Nothing here ever writes file names, file contents or anything else that
//! could carry private data: only counts, state names and short labels.

use std::io::Write;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// How much is worth saying.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Debug = 0,
    Info = 1,
    Warn = 2,
    Error = 3,
}

impl Level {
    fn name(self) -> &'static str {
        match self {
            Level::Debug => "DEBUG",
            Level::Info => "INFO ",
            Level::Warn => "WARN ",
            Level::Error => "ERROR",
        }
    }

    fn parse(text: &str) -> Option<Level> {
        match text.trim().to_ascii_lowercase().as_str() {
            "debug" | "trace" => Some(Level::Debug),
            "info" => Some(Level::Info),
            "warn" | "warning" => Some(Level::Warn),
            "error" => Some(Level::Error),
            _ => None,
        }
    }
}

static LEVEL: AtomicU8 = AtomicU8::new(Level::Info as u8);

/// Turns the log up or down.
///
/// The level comes from `OPEN_TANGERINE_LOG` (`debug`, `info`, `warn`,
/// `error`). Without it the log stays at `info`, which is quiet enough to run
/// in the background all day.
pub fn set_level_from_env() {
    if let Ok(value) = std::env::var("OPEN_TANGERINE_LOG")
        && let Some(level) = Level::parse(&value)
    {
        LEVEL.store(level as u8, Ordering::Relaxed);
    }
}

pub fn level() -> Level {
    match LEVEL.load(Ordering::Relaxed) {
        0 => Level::Debug,
        1 => Level::Info,
        2 => Level::Warn,
        _ => Level::Error,
    }
}

pub fn enabled(threshold: Level) -> bool {
    threshold >= level()
}

fn timestamp() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let seconds = now.as_secs();
    let millis = now.subsec_millis();
    // A small civil-time conversion keeps the log readable without a date
    // library: days since the epoch, then the usual h:m:s.
    let days = (seconds / 86_400) as i64;
    let seconds_of_day = seconds % 86_400;
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{millis:03}Z",
        seconds_of_day / 3600,
        (seconds_of_day % 3600) / 60,
        seconds_of_day % 60
    )
}

/// Days since 1970-01-01 to a civil date (Howard Hinnant's algorithm).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let mp = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if month <= 2 { year + 1 } else { year }, month, day)
}

/// Writes one line to the log.
pub fn log(level: Level, category: &str, message: &str) {
    if !enabled(level) {
        return;
    }
    let line = format!(
        "{} {} [{}] {}\n",
        timestamp(),
        level.name(),
        category,
        message
    );
    let mut stderr = std::io::stderr().lock();
    let _ = stderr.write_all(line.as_bytes());
    let _ = stderr.flush();
}

/// Logs a state change.
pub fn state_change(from: &str, to: &str, reason: &str) {
    log(Level::Info, "state", &format!("{from} -> {to} ({reason})"));
}

/// Logs an error with a short description.
pub fn error(category: &str, message: &str) {
    log(Level::Error, category, message);
}

/// A one-off guard that logs a message when it is dropped.
pub struct ScopedTimer {
    category: &'static str,
    started: std::time::Instant,
}

impl ScopedTimer {
    pub fn new(category: &'static str) -> Self {
        Self {
            category,
            started: std::time::Instant::now(),
        }
    }

    /// How long the timer has been running.
    pub fn elapsed(&self) -> std::time::Duration {
        self.started.elapsed()
    }
}

impl Drop for ScopedTimer {
    fn drop(&mut self) {
        log(
            Level::Debug,
            self.category,
            &format!("took {:?}", self.started.elapsed()),
        );
    }
}

static START: OnceLock<std::time::Instant> = OnceLock::new();

/// The moment the process started, for measuring startup cost.
pub fn since_start() -> std::time::Duration {
    START.get_or_init(std::time::Instant::now).elapsed()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_civil_date_conversion_is_right() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(19_723), (2024, 1, 1));
        assert_eq!(civil_from_days(19_782), (2024, 2, 29));
    }

    #[test]
    fn levels_are_ordered() {
        assert!(Level::Debug < Level::Info);
        assert!(Level::Info < Level::Warn);
        assert!(Level::Warn < Level::Error);
    }
}
