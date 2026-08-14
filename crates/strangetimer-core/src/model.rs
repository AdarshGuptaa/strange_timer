use std::path::PathBuf;

use chrono::{DateTime, Duration, Local};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Timer {
    pub name: String,
    pub buzzers: Vec<BuzzerRef>,
    pub created_at: DateTime<Local>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BuzzerRef {
    pub offset: Duration,
    pub buzzer_name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Buzzer {
    pub name: String,
    pub actions: Vec<BuzzerAction>,
    /// True for the pre-installed built-in buzzers (e.g. `default_audio`).
    /// Built-in buzzers cannot be deleted by the user.
    pub builtin: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum BuzzerAction {
    DefaultAudio,
    DefaultVideo,
    CloseAllWindows,
    /// `None` means use the built-in default sound.
    Audio(Option<PathBuf>),
    /// `None` means use the built-in default video.
    Video(Option<PathBuf>),
    Application(PathBuf),
    Url(String),
    Bash(PathBuf),
    /// Close a running application by process name (e.g. `firefox`).
    /// Destructive and gated behind `confirm-destructive` like
    /// `CloseAllWindows`; use `--close-app NAME`.
    CloseApplication(String),
    /// Bring a window matching a title substring or application name to the
    /// foreground. Non-destructive; use `--focus-window NAME`.
    FocusWindow(String),
    Llm {
        model: String,
        prompt: LlmPromptSource,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum LlmPromptSource {
    Inline(String),
    File(PathBuf),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimerRun {
    pub timer_name: String,
    pub started_at: DateTime<Local>,
    pub repetitions: RepeatMode,
    pub current_rep: u32,
    pub schedule_time: Option<DateTime<Local>>,
    pub status: TimerStatus,
    pub paused_at: Option<DateTime<Local>>,
    pub elapsed_before_pause: Duration,
    pub fired_indices: Vec<usize>,
    /// `run -u/--userinterrupt`: the run pauses at every buzzer and waits
    /// for the user to acknowledge (Enter on the attached CLI, or
    /// `strangetimer resume <name>`).
    #[serde(default)]
    pub user_interrupt: bool,
    /// Terminal window captured at `run -u` time; the daemon focuses it
    /// after non-audio buzzer actions so the user sees the prompt.
    #[serde(default)]
    pub interrupt_focus: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RepeatMode {
    Count(u32),
    Infinite,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum TimerStatus {
    Running,
    Paused,
    Scheduled,
    Completed,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DaemonState {
    pub runs: Vec<TimerRun>,
    /// True once the daemon has registered itself as an OS autostart service.
    pub registered: bool,
    /// Wall-clock time of the most recent state save. Used by restart
    /// recovery (Prompt 20) to reason about downtime; updated automatically
    /// inside `persistence::save_state`.
    pub last_saved_at: Option<DateTime<Local>>,
    /// Timer currently awaiting a user-interrupt acknowledgement
    /// (`run -u`). Persisted so a restart keeps the run paused and the
    /// attached CLI (or `resume`) can still acknowledge it.
    #[serde(default)]
    pub interrupt_pending: Option<String>,
}
