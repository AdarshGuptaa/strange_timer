//! Central styling helpers for CLI output — a muted, minimalistic
//! "Cosmic"-like theme (soft teal accents, dim borders, nothing bright).
//!
//! Color is enabled only when stdout is a TTY and `NO_COLOR` is unset;
//! `STRANGETIMER_COLOR=always` forces it on (used by tests). Every helper
//! degrades to a plain passthrough when color is disabled, so alignment
//! and scripted output are never affected by escape codes.

use crossterm::style::{Attribute, Color, Stylize};
use strangetimer_core::model::TimerStatus;

/// Whether ANSI colors should be emitted on stdout.
pub fn color_enabled() -> bool {
    if std::env::var_os("STRANGETIMER_COLOR").is_some() {
        return std::env::var("STRANGETIMER_COLOR")
            .map(|v| !v.is_empty() && v != "0")
            .unwrap_or(false);
    }
    if std::env::var_os("NO_COLOR").is_some() {
        return false;
    }
    use crossterm::tty::IsTty;
    std::io::stdout().is_tty()
}

fn paint(s: &str, color: Color, bold: bool) -> String {
    if !color_enabled() {
        return s.to_string();
    }
    let styled = s.with(color);
    if bold {
        styled.attribute(Attribute::Bold).to_string()
    } else {
        styled.to_string()
    }
}

/// Section headers and table titles (DarkCyan, bold).
pub fn header(s: &str) -> String {
    paint(s, Color::DarkCyan, true)
}

/// Table borders and rules (DarkGrey).
pub fn rule(s: &str) -> String {
    paint(s, Color::DarkGrey, false)
}

/// Accent text (Cyan) — prompts, focus cues, highlights.
pub fn accent(s: &str) -> String {
    paint(s, Color::Cyan, false)
}

/// The primary accent used for interactive prompts (Green, bold).
pub fn prompt(s: &str) -> String {
    paint(s, Color::Green, true)
}

/// Dimmed text (DarkGrey) — secondary info.
pub fn dim(s: &str) -> String {
    paint(s, Color::DarkGrey, false)
}

/// Warnings (Yellow).
pub fn warn(s: &str) -> String {
    paint(s, Color::Yellow, false)
}

/// Errors (Red, bold).
pub fn err(s: &str) -> String {
    paint(s, Color::Red, true)
}

/// The built-in tag in library tables (Magenta, dim).
pub fn builtin(s: &str) -> String {
    paint(s, Color::Magenta, false)
}

/// A timer name, highlighted with the accent.
pub fn name(s: &str) -> String {
    paint(s, Color::Cyan, false)
}

/// A live run's status, colored by state:
/// running green, paused yellow, scheduled cyan, completed grey.
pub fn status(s: &str, status: TimerStatus) -> String {
    let color = match status {
        TimerStatus::Running => Color::Green,
        TimerStatus::Paused => Color::Yellow,
        TimerStatus::Scheduled => Color::Cyan,
        TimerStatus::Completed => Color::DarkGrey,
    };
    paint(s, color, false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn color_switch_controls_ansi_emission() {
        // Single test, sequential env mutations — tests run in parallel and
        // the environment is process-global.
        std::env::set_var("STRANGETIMER_COLOR", "always");
        assert!(color_enabled());
        assert!(header("Header").contains("\u{1b}["), "expected ANSI");

        std::env::set_var("STRANGETIMER_COLOR", "0");
        assert!(!color_enabled());
        assert_eq!(header("H"), "H");
        assert_eq!(name("N"), "N");
        assert_eq!(err("E"), "E");
        assert_eq!(status("s", TimerStatus::Running), "s");
    }
}
