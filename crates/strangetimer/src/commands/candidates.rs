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

/// Timers that can actually be resumed (for `resume`): paused runs,
/// which includes user-interrupt runs awaiting acknowledgement.
pub fn paused_timer_names(current: &OsStr) -> Vec<CompletionCandidate> {
    let Some(current) = current.to_str() else {
        return vec![];
    };
    run_candidates(current, |r| r.status == TimerStatus::Paused)
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
    let kinds: Vec<&str> = b
        .actions
        .iter()
        .map(action_label)
        .chain(BUILTIN_BUZZERS.iter().filter_map(|(name, label)| {
            (name == &b.name && b.actions.is_empty()).then_some(*label)
        }))
        .collect();
    CompletionCandidate::new(b.name).help(Some(kinds.join(", ").into()))
}

/// Candidates for the variadic `(offset, [buzzer])` slots of `create
/// timer`.
///
/// The grammar is ambiguous at each position (offsets may repeat without
/// buzzer names), so position parity is not reliable — the current token's
/// *content* decides the slot:
///
/// - a token starting with a digit is an offset slot: suggest example
///   offsets, and when it is already a complete offset (`1m<Tab>`) also
///   suggest `1m <buzzer>` expansions so the offset is preserved;
/// - anything else (including empty) is a buzzer slot: suggest the buzzer
///   library, plus example offsets on an empty token so both continuations
///   are offered.
pub struct CreateTimerCompleter;

impl ValueCompleter for CreateTimerCompleter {
    fn complete_at(&self, _arg_index: usize, current: &OsStr) -> Vec<CompletionCandidate> {
        let Some(current) = current.to_str() else {
            return vec![];
        };

        if current.starts_with(|c: char| c.is_ascii_digit()) {
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
            return candidates;
        }

        // Buzzer slot (or the very first slot before any offset): offer
        // buzzers; an empty token additionally offers example offsets.
        let mut candidates = buzzer_names(OsStr::new(current));
        if current.is_empty() {
            candidates.extend(offset_candidates(""));
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
    state_snapshot()
        .map(|(timers, _)| timers)
        .unwrap_or_else(|| load_timers().unwrap_or_default())
}

fn runs() -> Vec<TimerRun> {
    state_snapshot()
        .map(|(_, runs)| runs)
        .unwrap_or_else(|| load_state().map(|s| s.runs).unwrap_or_default())
}

/// One consistent daemon snapshot for the whole completion request.
fn state_snapshot() -> Option<(Vec<Timer>, Vec<TimerRun>)> {
    match send_and_receive_no_autostart(&ClientMessage::GetTimers) {
        Ok(ServerMessage::TimerList { timers, runs, .. }) => Some((timers, runs)),
        _ => None,
    }
}

fn buzzer_defs() -> Vec<Buzzer> {
    let mut buzzers = match send_and_receive_no_autostart(&ClientMessage::GetBuzzers) {
        Ok(ServerMessage::BuzzerList(buzzers)) => buzzers,
        _ => load_buzzers().unwrap_or_default(),
    };
    // Ensure the built-ins always appear, even before a daemon has seeded
    // buzzers.json — deterministic suggestions on fresh installs.
    for builtin in BUILTIN_BUZZERS {
        if !buzzers.iter().any(|b| b.name == builtin.0) {
            buzzers.push(Buzzer {
                name: builtin.0.to_string(),
                actions: vec![],
                builtin: true,
            });
        }
    }
    buzzers
}

/// (name, action label) for the built-in buzzer library.
const BUILTIN_BUZZERS: [(&str, &str); 3] = [
    ("default_audio", "audio"),
    ("default_video", "video"),
    ("close_windows", "close_windows"),
];

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
        A::CloseWindow(_) => "close_window",
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
