use std::path::PathBuf;

use chrono::{DateTime, Duration, Local};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Timer {
    pub name: String,
    pub buzzers: Vec<BuzzerRef>,
    pub created_at: DateTime<Local>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuzzerRef {
    pub offset: Duration,
    pub buzzer_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Buzzer {
    pub name: String,
    pub actions: Vec<BuzzerAction>,
    /// True for the pre-installed built-in buzzers (e.g. `default_audio`).
    /// Built-in buzzers cannot be deleted by the user.
    pub builtin: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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
    Llm {
        model: String,
        prompt: LlmPromptSource,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RepeatMode {
    Count(u32),
    Infinite,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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
}
