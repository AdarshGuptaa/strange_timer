//! Tiny dependency-free logger for the daemon.
//!
//! - Every message is appended to `daemon.log` in the data dir.
//! - The terminal (stderr) only receives messages at or above the
//!   configured level, defaulting to `warn` — so a foreground daemon stays
//!   quiet instead of interleaving IPC chatter with the user's typing.
//! - `STRANGETIMER_LOG=debug|info|warn` raises/lowers the terminal level;
//!   the log file always receives everything.
//!
//! Exposes the `debug!`, `info!` and `warn!` macros (crate-wide via
//! `#[macro_use] mod log;` in main.rs).

use std::fmt;
use std::fs::OpenOptions;
use std::io::Write;
use std::sync::{Mutex, OnceLock};

use strangetimer_core::persistence::data_dir;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Debug,
    Info,
    Warn,
}

impl Level {
    fn label(self) -> &'static str {
        match self {
            Level::Debug => "DEBUG",
            Level::Info => "INFO",
            Level::Warn => "WARN",
        }
    }
}

static FILE: OnceLock<Mutex<Option<std::fs::File>>> = OnceLock::new();

/// Open `daemon.log` for appending. Called once at startup; a failure here
/// degrades the logger to terminal-only rather than killing the daemon.
pub fn init() {
    let file = match OpenOptions::new()
        .create(true)
        .append(true)
        .open(data_dir().join("daemon.log"))
    {
        Ok(f) => Some(f),
        Err(e) => {
            eprintln!("strangetimer-daemon: failed to open daemon.log: {e}");
            None
        }
    };
    let _ = FILE.set(Mutex::new(file));
}

/// The level at which messages also appear on stderr.
fn terminal_level() -> Level {
    match std::env::var("STRANGETIMER_LOG").as_deref() {
        Ok("debug") => Level::Debug,
        Ok("info") => Level::Info,
        _ => Level::Warn,
    }
}

/// Record a message. The file copy is timestamped; the terminal copy is
/// filtered by `terminal_level()`.
pub fn log(level: Level, args: fmt::Arguments<'_>) {
    if let Some(mtx) = FILE.get() {
        if let Ok(mut guard) = mtx.lock() {
            if let Some(file) = guard.as_mut() {
                let ts = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
                let _ = writeln!(file, "{ts} [{:>5}] {args}", level.label());
                let _ = file.flush();
            }
        }
    }

    if level >= terminal_level() {
        eprintln!("strangetimer-daemon: {args}");
    }
}

/// `debug!(...)` — message recorded to the log file only.
#[macro_export]
macro_rules! debug {
    ($($arg:tt)*) => {
        $crate::log::log($crate::log::Level::Debug, format_args!($($arg)*))
    };
}

/// `info!(...)` — recorded to the log file; shown on stderr only with
/// `STRANGETIMER_LOG=info` (or debug).
#[macro_export]
macro_rules! info {
    ($($arg:tt)*) => {
        $crate::log::log($crate::log::Level::Info, format_args!($($arg)*))
    };
}

/// `warn!(...)` — recorded and shown on stderr by default.
#[macro_export]
macro_rules! warn {
    ($($arg:tt)*) => {
        $crate::log::log($crate::log::Level::Warn, format_args!($($arg)*))
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_level_follows_env_with_warn_default() {
        // One test, sequential env mutations — tests run in parallel and
        // the environment is process-global.
        std::env::remove_var("STRANGETIMER_LOG");
        assert_eq!(terminal_level(), Level::Warn);
        std::env::set_var("STRANGETIMER_LOG", "debug");
        assert_eq!(terminal_level(), Level::Debug);
        std::env::set_var("STRANGETIMER_LOG", "info");
        assert_eq!(terminal_level(), Level::Info);
        std::env::set_var("STRANGETIMER_LOG", "banana");
        assert_eq!(terminal_level(), Level::Warn);
    }
}
