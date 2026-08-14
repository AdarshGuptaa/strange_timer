//! State-aware completion candidates for <Tab>.
//!
//! Candidates come from the live daemon (timer names, buzzer library
//! names, run states) and are refreshed on every keystroke. Queries go
//! through `send_and_receive_no_autostart` so tab-completion never spawns
//! a daemon; when the daemon is down the completers fall back to the
//! persisted state files so suggestions still work.

use std::ffi::OsStr;

use clap_complete::engine::{CompletionCandidate, ValueCompleter};
use strangetimer_core::duration_parse::parse_offset;
use strangetimer_core::ipc::{ClientMessage, ServerMessage};
use strangetimer_core::model::{Buzzer, Timer, TimerRun, TimerStatus};
use strangetimer_core::persistence::{load_buzzers, load_state, load_timers};

use crate::commands::send_and_receive_no_autostart;

/// All defined timer names (any state) with a short description.
pub fn timer_names(current: &OsStr) -> Vec<CompletionCandidate> {
    let Some(current) = current.to_str() else {
        return vec![];
    };
    timer_defs()
        .into_iter()
        .filter(|t| t.name.starts_with(current))
        .map(|t| {
            CompletionCandidate::new(t.name)
                .help(Some(format!("buzzers: {}", t.buzzers.len()).into()))
        })
        .collect()
}

/// Timers with a live RUNNING run (for `pause`).
pub fn running_timer_names(current: &OsStr) -> Vec<CompletionCandidate> {
    let Some(current) = current.to_str() else {
        return vec![];
    };
    run_candidates(current, |r| r.status == TimerStatus::Running)
}

/// Timers paused or awaiting a user-interrupt acknowledgement (for
/// `resume`).
pub fn paused_timer_names(current: &OsStr) -> Vec<CompletionCandidate> {
    let Some(current) = current.to_str() else {
        return vec![];
    };
    run_candidates(current, |r| {
        r.status == TimerStatus::Paused || r.user_interrupt
    })
}

/// Timers with any live run (for `stop`).
pub fn active_run_timer_names(current: &OsStr) -> Vec<CompletionCandidate> {
    let Some(current) = current.to_str() else {
        return vec![];
    };
    run_candidates(current, |r| r.status != TimerStatus::Completed)
}

/// Timers with a live run matching `filter`.
fn run_candidates(current: &str, filter: impl Fn(&TimerRun) -> bool) -> Vec<CompletionCandidate> {
    let runs = runs();
    let timers = timer_defs();
    timers
        .into_iter()
        .filter(|t| {
            t.name.starts_with(current) && runs.iter().any(|r| r.timer_name == t.name && filter(r))
        })
        .map(|t| CompletionCandidate::new(t.name).help(Some("live run".into())))
        .collect()
}

/// Buzzer library names (all, including built-ins) with their action types.
pub fn buzzer_names(current: &OsStr) -> Vec<CompletionCandidate> {
    let Some(current) = current.to_str() else {
        return vec![];
    };
    buzzer_defs()
        .into_iter()
        .filter(|b| b.name.starts_with(current))
        .map(buzzer_candidate)
        .collect()
}

/// Deletable (non-built-in) buzzer library names for `delete buzzer`.
pub fn deletable_buzzer_names(current: &OsStr) -> Vec<CompletionCandidate> {
    let Some(current) = current.to_str() else {
        return vec![];
    };
    buzzer_defs()
        .into_iter()
        .filter(|b| !b.builtin && b.name.starts_with(current))
        .map(buzzer_candidate)
        .collect()
}

fn buzzer_candidate(b: Buzzer) -> CompletionCandidate {
    let kinds: Vec<&str> = b.actions.iter().map(action_label).collect();
    CompletionCandidate::new(b.name).help(Some(kinds.join(", ").into()))
}

/// Candidates for the variadic `(offset, [buzzer])` slots of `create
/// timer`. `complete_at` sees each token's position: even positions are
/// offset slots, odd positions are buzzer slots. When the current token is
/// already a complete offset, the suggestion appends the buzzer so `1m<Tab>`
/// expands to `1m <buzzer>` instead of replacing the offset.
pub struct CreateTimerCompleter;

impl ValueCompleter for CreateTimerCompleter {
    fn complete_at(&self, arg_index: usize, current: &OsStr) -> Vec<CompletionCandidate> {
        let Some(current) = current.to_str() else {
            return vec![];
        };
        if arg_index % 2 == 1 {
            return buzzer_names(OsStr::new(current));
        }
        let mut candidates = offset_candidates(current);
        if parse_offset(current).is_ok() {
            for b in buzzer_defs() {
                candidates.push(
                    CompletionCandidate::new(format!("{current} {}", b.name)).help(Some(
                        b.actions
                            .iter()
                            .map(action_label)
                            .collect::<Vec<_>>()
                            .join(", ")
                            .into(),
                    )),
                );
            }
        }
        candidates
    }

    fn complete(&self, current: &OsStr) -> Vec<CompletionCandidate> {
        self.complete_at(0, current)
    }
}

/// Example offsets for the offset slots of `create timer`.
fn offset_candidates(current: &str) -> Vec<CompletionCandidate> {
    ["30s", "1m", "5m", "15m", "1h", "1D", "1W"]
        .into_iter()
        .filter(|o| o.starts_with(current))
        .map(|o| CompletionCandidate::new(o).help(Some("offset".into())))
        .collect()
}

/// `view` targets: the two special views plus every timer name.
pub fn view_targets(current: &OsStr) -> Vec<CompletionCandidate> {
    let Some(current) = current.to_str() else {
        return vec![];
    };
    let mut candidates = vec![];
    for value in ["timers", "buzzers"] {
        if value.starts_with(current) {
            candidates.push(CompletionCandidate::new(value).help(Some("special view".into())));
        }
    }
    candidates.extend(timer_names(OsStr::new(current)));
    candidates
}

// --- Data sources: live daemon first, persisted files as fallback ------

fn timer_defs() -> Vec<Timer> {
    match send_and_receive_no_autostart(&ClientMessage::GetTimers) {
        Ok(ServerMessage::TimerList { timers, .. }) => timers,
        _ => load_timers().unwrap_or_default(),
    }
}

fn buzzer_defs() -> Vec<Buzzer> {
    match send_and_receive_no_autostart(&ClientMessage::GetBuzzers) {
        Ok(ServerMessage::BuzzerList(buzzers)) => buzzers,
        _ => load_buzzers().unwrap_or_default(),
    }
}

fn runs() -> Vec<TimerRun> {
    match send_and_receive_no_autostart(&ClientMessage::GetTimers) {
        Ok(ServerMessage::TimerList { runs, .. }) => runs,
        _ => load_state().map(|s| s.runs).unwrap_or_default(),
    }
}

fn action_label(action: &strangetimer_core::model::BuzzerAction) -> &'static str {
    use strangetimer_core::model::BuzzerAction as A;
    match action {
        A::DefaultAudio | A::Audio(_) => "audio",
        A::DefaultVideo | A::Video(_) => "video",
        A::CloseAllWindows => "close_windows",
        A::Application(_) => "application",
        A::Url(_) => "url",
        A::Bash(_) => "bash",
        A::CloseApplication(_) => "close_app",
        A::FocusWindow(_) => "focus_window",
        A::Llm { .. } => "llm",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offset_slot_suggests_offsets() {
        let c = CreateTimerCompleter;
        let out = c.complete_at(0, OsStr::new(""));
        let values: Vec<&str> = out
            .iter()
            .map(|c| c.get_value().to_str().unwrap())
            .collect();
        for expected in ["30s", "1m", "5m", "15m", "1h", "1D", "1W"] {
            assert!(values.contains(&expected), "missing {expected}: {values:?}");
        }
    }

    #[test]
    fn completed_offset_appends_buzzers() {
        let c = CreateTimerCompleter;
        let out = c.complete_at(0, OsStr::new("1m"));
        let values: Vec<&str> = out
            .iter()
            .map(|c| c.get_value().to_str().unwrap())
            .collect();
        assert!(
            values.iter().any(|v| v.starts_with("1m default_audio")),
            "expected `1m <buzzer>` suggestions, got {values:?}"
        );
    }

    #[test]
    fn buzzer_slot_suggests_library() {
        let c = CreateTimerCompleter;
        let out = c.complete_at(1, OsStr::new("default"));
        let values: Vec<&str> = out
            .iter()
            .map(|c| c.get_value().to_str().unwrap())
            .collect();
        assert!(values.contains(&"default_audio"), "{values:?}");
    }

    #[test]
    fn view_targets_include_specials() {
        let out = view_targets(OsStr::new(""));
        let values: Vec<&str> = out
            .iter()
            .map(|c| c.get_value().to_str().unwrap())
            .collect();
        assert!(values.contains(&"timers"));
        assert!(values.contains(&"buzzers"));
    }
}
