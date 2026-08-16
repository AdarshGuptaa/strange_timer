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

/// Display metadata for a buzzer, computed by the daemon for the views:
/// per-action targets and media durations, plus how many timer
/// definitions and live runs reference the buzzer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BuzzerInfo {
    pub name: String,
    pub actions: Vec<BuzzerAction>,
    pub builtin: bool,
    /// Display target per action (path, URL, model, "embedded chime", ...).
    pub targets: Vec<String>,
    /// Formatted duration per action (e.g. "0.80s") or None when unknown.
    pub durations: Vec<Option<String>>,
    /// Number of timer definitions referencing this buzzer (each timer
    /// counts once, even with multiple slots).
    pub timer_count: usize,
    /// Number of live (non-completed) runs whose timer references it.
    pub live_count: usize,
}

/// Full detail for `view buzzer NAME`: the info plus every referencing
/// timer with its live-run status.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BuzzerDetail {
    pub info: BuzzerInfo,
    pub referencing_timers: Vec<(String, TimerStatus)>,
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
    /// Close a *selected* window by X11 window id or window title (e.g.
    /// `--close-window 0x0123abcd` or `--close-window 'Meeting - Zoom'`).
    /// Destructive and gated behind `confirm-destructive`.
    CloseWindow(String),
    /// Bring a window matching a title substring or application name to the
    /// foreground. Non-destructive; use `--focus-window NAME`.
    FocusWindow(String),
    Llm {
        model: String,
        prompt: LlmPromptSource,
    },
}

impl BuzzerAction {
    /// Human-readable action type, shared by the CLI (`view buzzers`,
    /// completions) and the daemon (buzzer events).
    pub fn label(&self) -> &'static str {
        match self {
            BuzzerAction::DefaultAudio | BuzzerAction::Audio(_) => "audio",
            BuzzerAction::DefaultVideo | BuzzerAction::Video(_) => "video",
            BuzzerAction::CloseAllWindows => "close_windows",
            BuzzerAction::Application(_) => "application",
            BuzzerAction::Url(_) => "url",
            BuzzerAction::Bash(_) => "bash",
            BuzzerAction::CloseApplication(_) => "close_app",
            BuzzerAction::CloseWindow(_) => "close_window",
            BuzzerAction::FocusWindow(_) => "focus_window",
            BuzzerAction::Llm { .. } => "llm",
        }
    }
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FireEvent {
    pub timer_name: String,
    pub buzzer_name: String,
    /// Index of the buzzer within the timer's definition.
    pub buzzer_index: usize,
    /// Repetition (0-based) during which the buzzer fired.
    pub repetition: u32,
}

/// A user-facing "the buzzer is ringing" notification, pushed by the
/// daemon and consumed by `strangetimer watch` (and future integrations).
/// Kept in memory only — events are transient.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BuzzerEvent {
    /// Monotonic sequence id used by watchers to resume where they left off.
    pub id: u64,
    pub timer_name: String,
    pub buzzer_name: String,
    /// Action types of the buzzer (audio, video, ...).
    pub buzzer_types: Vec<String>,
    pub fired_at: chrono::DateTime<chrono::Local>,
    pub repetition: u32,
    /// True when the run is in user-interrupt mode and is now waiting for
    /// `strangetimer resume <timer>`.
    pub requires_ack: bool,
    /// Dispatch outcome, when it was not a plain fire: e.g. blocked by
    /// confirmation, or deprecated action refused.
    #[serde(default)]
    pub outcome: Option<String>,
}

/// What the daemon needs to bring the user's terminal back to the front
/// after a buzzer fires. Captured at `run -u` time and stored (JSON) in
/// `TimerRun::interrupt_focus` so focus works even when the daemon is a
/// system service without the interactive session environment.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct FocusSpec {
    /// X11 window id (e.g. `0x0123abcd`) of the terminal that started the
    /// run — the reliable activation target.
    pub window_id: Option<String>,
    /// Window title, used as a fallback when the id is gone.
    pub title: Option<String>,
    /// `DISPLAY` of the session the run started in.
    pub display: Option<String>,
    /// `XAUTHORITY` of that session.
    pub xauthority: Option<String>,
    /// True when the session is Wayland (X11 tools cannot reliably focus;
    /// the daemon reports unsupported instead of pretending success).
    pub wayland: bool,
}

impl FocusSpec {
    /// Serialize the spec into the `interrupt_focus` storage field.
    pub fn encode(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }

    /// Parse the storage field; a plain title (pre-spec installs) parses
    /// as a legacy title-only target.
    pub fn decode(stored: &str) -> Option<FocusSpec> {
        serde_json::from_str(stored).ok()
    }
}

/// The interactive-session environment needed to launch GUI-side buzzer
/// actions (video, URL, focus). Captured fresh by the CLI on every command
/// and piggybacked onto each IPC request, so the daemon never relies on the
/// (possibly stale) environment it was started with — a reboot, a new X
/// login or a different terminal no longer breaks the openers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct SessionEnv {
    #[serde(default)]
    pub display: Option<String>,
    #[serde(default)]
    pub wayland_display: Option<String>,
    #[serde(default)]
    pub xauthority: Option<String>,
    #[serde(default)]
    pub xdg_runtime_dir: Option<String>,
    #[serde(default)]
    pub dbus_session_bus_address: Option<String>,
    #[serde(default)]
    pub path: Option<String>,
}

impl SessionEnv {
    /// Capture the current process's session environment. Values are taken
    /// only when present and non-empty, so a CLI running headless (SSH
    /// without X forwarding) sends a partial snapshot instead of nothing.
    pub fn from_process_env() -> SessionEnv {
        SessionEnv {
            display: env_opt("DISPLAY"),
            wayland_display: env_opt("WAYLAND_DISPLAY"),
            xauthority: env_opt("XAUTHORITY"),
            xdg_runtime_dir: env_opt("XDG_RUNTIME_DIR"),
            dbus_session_bus_address: env_opt("DBUS_SESSION_BUS_ADDRESS"),
            path: env_opt("PATH"),
        }
    }

    /// The snapshot is useful only when it carries at least one display
    /// hint; a fully empty snapshot must not overwrite a stored one.
    pub fn is_empty(&self) -> bool {
        self.display.is_none()
            && self.wayland_display.is_none()
            && self.xauthority.is_none()
            && self.xdg_runtime_dir.is_none()
            && self.dbus_session_bus_address.is_none()
            && self.path.is_none()
    }
}

fn env_opt(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|v| !v.is_empty())
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
    /// Latest known interactive-session environment, refreshed from every
    /// CLI request and persisted so a daemon that restarts before any new
    /// CLI contact still launches GUI buzzers against the last-known
    /// session.
    #[serde(default)]
    pub session_env: SessionEnv,
    /// Opt-in for the destructive `close_window` / `close_app` buzzers.
    /// Persisted so the opt-in survives daemon restarts; revoked with
    /// `strangetimer revoke-destructive`.
    #[serde(default)]
    pub close_windows_confirmed: bool,
    /// Legacy single-pending marker from the pre-multi-interrupt format.
    /// Kept so older state files still deserialize; folded into
    /// `pending_interrupts` on load.
    #[serde(default)]
    pub interrupt_pending: Option<String>,
    /// Timers currently awaiting a user-interrupt acknowledgement
    /// (`run -u`). Persisted so a restart keeps the runs paused and
    /// `resume` can still acknowledge them.
    #[serde(default)]
    pub pending_interrupts: Vec<String>,
    /// Fired-but-not-yet-dispatched buzzer events (the fire outbox). The
    /// scheduler persists an event here before handing it to the fire
    /// task, which removes it after dispatch — so a daemon crash between
    /// scheduling and dispatch never loses the alarm.
    #[serde(default)]
    pub pending_fires: Vec<FireEvent>,
}
