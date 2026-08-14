//! State-aware completion candidates for <Tab>.
//!
//! Candidates come from the live daemon (timer names, buzzer library
//! names) and are refreshed on every keystroke. Queries go through
//! `send_and_receive_no_autostart` so tab-completion never spawns a
//! daemon; when the daemon is down the completers silently return empty.

use std::ffi::OsStr;

use clap_complete::engine::CompletionCandidate;
use strangetimer_core::ipc::{ClientMessage, ServerMessage};
use strangetimer_core::model::Timer;

use crate::commands::send_and_receive_no_autostart;

/// Timer names with a short description ("next: <buzzer> · <offset>").
pub fn timer_names(current: &OsStr) -> Vec<CompletionCandidate> {
    let Some(current) = current.to_str() else {
        return vec![];
    };
    let Ok(ServerMessage::TimerList { timers, .. }) =
        send_and_receive_no_autostart(&ClientMessage::GetTimers)
    else {
        return vec![];
    };
    timers
        .iter()
        .filter(|t| t.name.starts_with(current))
        .map(timer_candidate)
        .collect()
}

fn timer_candidate(timer: &Timer) -> CompletionCandidate {
    let description = format!("buzzers: {}", timer.buzzers.len());
    CompletionCandidate::new(timer.name.clone()).help(Some(description.into()))
}

/// Buzzer library names with their action types.
pub fn buzzer_names(current: &OsStr) -> Vec<CompletionCandidate> {
    let Some(current) = current.to_str() else {
        return vec![];
    };
    let Ok(ServerMessage::BuzzerList(buzzers)) =
        send_and_receive_no_autostart(&ClientMessage::GetBuzzers)
    else {
        return vec![];
    };
    buzzers
        .iter()
        .filter(|b| b.name.starts_with(current))
        .map(|b| {
            let kinds: Vec<&str> = b.actions.iter().map(action_label).collect();
            CompletionCandidate::new(b.name.clone()).help(Some(kinds.join(", ").into()))
        })
        .collect()
}

/// Candidates for the variadic `(offset, [buzzer])` slots of
/// `create timer`: buzzers while the current word looks like a name, and
/// nothing when it looks like an offset (digits).
pub fn timer_slot_buzzers(current: &OsStr) -> Vec<CompletionCandidate> {
    let Some(current) = current.to_str() else {
        return vec![];
    };
    if current.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        return vec![];
    }
    buzzer_names(OsStr::new(current))
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
