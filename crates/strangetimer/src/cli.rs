use clap::builder::{Styles, ValueHint};
use clap::{Args, Parser, Subcommand};
use clap_complete::engine::ArgValueCompleter;

use crate::commands::candidates;

/// Muted "Cosmic"-like theme for `--help` output: soft teal headers,
/// cyan literals, dim placeholders — nothing bright.
fn muted_styles() -> Styles {
    use clap::builder::styling::{AnsiColor, Effects, RgbColor};
    let teal = RgbColor(72, 185, 199); // Pop!_OS muted teal accent
    Styles::styled()
        .header(teal.on_default() | Effects::BOLD)
        .usage(teal.on_default() | Effects::BOLD)
        .literal(teal.on_default())
        .placeholder(AnsiColor::BrightBlack.on_default())
        .error(AnsiColor::Red.on_default() | Effects::BOLD)
        .valid(AnsiColor::Green.on_default())
        .invalid(AnsiColor::Yellow.on_default())
}

#[derive(Parser, Debug)]
#[command(
    name = "strangetimer",
    version,
    about = "StrangeTimer — CLI timer application",
    styles = muted_styles(),
    after_help = "Getting started:\n\
    \n\
      strangetimer create timer workAndFun 45min 15min\n\
      strangetimer run workAndFun -n 3\n\
      strangetimer view timers\n\
    \n\
    The daemon starts automatically on first use (STRANGETIMER_AUTO_START=0\n\
    disables that). Full docs: https://github.com/AdarshGuptaa/strange_timer"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Create a new timer definition or buzzer.
    #[command(after_help = "Examples:\n\
    \n\
      strangetimer create timer focus 25min\n\
      strangetimer create timer workAndFun 45min 15min\n\
      strangetimer create timer weekly 1W paymentBuzzer\n\
      strangetimer create timer focus 25min 30min breakBuzzer\n\
      strangetimer create buzzer alarm --audio\n\
      strangetimer create buzzer meeting --url https://meet.example.com/standup\n\
      strangetimer create buzzer tidy --close-app firefox --focus-window Slack\n\
    \n\
    Timers take (offset, optional buzzer) pairs; a bare offset uses the\n\
    built-in default_audio buzzer. Buzzers chain their --flags in order.")]
    Create {
        #[command(subcommand)]
        kind: CreateKind,
    },
    /// Duplicate an existing timer definition.
    #[command(after_help = "Examples:\n\
    \n\
      strangetimer duplicate timer focus\n\
      strangetimer duplicate timer focus morning_focus\n\
    \n\
    Without a new name the copy gets <source>_copy (or _copy_2, …).")]
    Duplicate {
        #[command(subcommand)]
        kind: DuplicateKind,
    },
    /// Delete a timer or buzzer definition.
    #[command(after_help = "Examples:\n\
    \n\
      strangetimer delete timer focus\n\
      strangetimer delete buzzer meeting\n\
    \n\
    Deleting a timer is refused while it has an active run — stop it first.\n\
    Built-in buzzers (default_audio, default_video, close_windows) cannot\n\
    be deleted.")]
    Delete {
        #[command(subcommand)]
        kind: DeleteKind,
    },
    /// Run a timer.
    #[command(after_help = "Examples:\n\
    \n\
      strangetimer run focus\n\
      strangetimer run focus -n 3\n\
      strangetimer run focus -i\n\
      strangetimer run focus -t 09:00\n\
      strangetimer run focus -u\n\
      strangetimer run focus -n 4 -t 14:30\n\
    \n\
    -u / --userinterrupt pauses the timer at every buzzer and loops audio\n\
    until you press Enter (the CLI stays attached and prompts you).\n\
    `strangetimer resume <name>` acknowledges too.")]
    Run(RunArgs),
    /// Pause a running timer.
    #[command(after_help = "Examples:\n\
    \n\
      strangetimer pause focus\n\
    \n\
    Pausing freezes the countdown; `strangetimer resume <name>` continues\n\
    from exactly where it stopped.")]
    Pause(PauseTarget),
    /// Pause every running timer.
    #[command(after_help = "Examples:\n\
    \n\
      strangetimer pauseall\n\
    \n\
    Equivalent to pausing each running timer individually.")]
    Pauseall,
    /// Resume a paused timer.
    #[command(after_help = "Examples:\n\
    \n\
      strangetimer resume focus\n\
    \n\
    Also acknowledges a user-interrupt prompt (`run -u`).")]
    Resume(ResumeTarget),
    /// Stop a running timer (keeps the definition).
    #[command(after_help = "Examples:\n\
    \n\
      strangetimer stop focus\n\
    \n\
    Cancels the live run; the timer definition stays for later runs.")]
    Stop(StopTarget),
    /// Stop every running timer.
    #[command(after_help = "Examples:\n\
    \n\
      strangetimer stopall")]
    Stopall,
    /// Opt in to the destructive `close_windows` buzzer (closes ALL other
    /// windows when it fires).
    #[command(after_help = "Examples:\n\
    \n\
      strangetimer confirm-destructive\n\
    \n\
    Required once per daemon session before any close_windows or close_app\n\
    buzzer will run. The opt-in resets when the daemon restarts.")]
    ConfirmDestructive,
    /// Manage the background daemon process.
    #[command(after_help = "Examples:\n\
    \n\
      strangetimer daemon start\n\
      strangetimer daemon status\n\
      strangetimer daemon stop\n\
      strangetimer daemon restart\n\
    \n\
    Most commands auto-start the daemon, so `daemon start` is only needed\n\
    for explicit control or scripting.")]
    Daemon {
        #[command(subcommand)]
        cmd: DaemonCommand,
    },
    /// Show or install example buzzers demonstrating every action type.
    #[command(after_help = "Examples:\n\
    \n\
      strangetimer examples\n\
      strangetimer examples --install\n\
    \n\
    --install creates the file-free examples (audio, video, url, chain,\n\
    llm) in your buzzer library, skipping any that already exist.")]
    Examples {
        /// Create the example buzzers in the daemon's library (skips any
        /// that already exist).
        #[arg(long)]
        install: bool,
    },
    /// Print a shell completion script for this CLI.
    #[command(after_help = "Examples:\n\
    \n\
      strangetimer completions bash\n\
      strangetimer completions zsh\n\
      strangetimer completions fish\n\
      strangetimer completions powershell\n\
      strangetimer completions --doctor\n\
    \n\
    Prefer `strangetimer install-completions`, which places the right\n\
    script in the right location for you. `--doctor` diagnoses missing\n\
    tab suggestions (stale scripts, wrong binary in $PATH).")]
    Completions {
        /// Which shell to generate completions for (omit with `--doctor`).
        #[arg(value_enum)]
        shell: Option<Shell>,
        /// Diagnose why <Tab> suggestions may be missing.
        #[arg(long)]
        doctor: bool,
    },
    /// Install shell completions so <Tab> works in your terminal.
    #[command(after_help = "Examples:\n\
    \n\
      strangetimer install-completions\n\
      strangetimer install-completions --shell zsh\n\
    \n\
    Detects your shell from $SHELL. bash and fish need no rc-file edits;\n\
    zsh prints the one line to add to ~/.zshrc; powershell prints the\n\
    $PROFILE line to paste.")]
    InstallCompletions {
        /// Shell to install for; defaults to the shell in `$SHELL`.
        #[arg(long, value_enum)]
        shell: Option<Shell>,
    },
    /// Watch buzzer-ringing notifications as they fire.
    #[command(after_help = "Examples:\n\
    \n\
      strangetimer watch\n\
    \n\
    Prints one line per fired buzzer:\n\
      <timer> ringing | <type> | <time>\n\
    plus `strangetimer resume <timer>` when the run is in user-interrupt\n\
    mode. Ctrl+C stops the watcher.")]
    Watch,
    /// View timers, buzzers, or a single timer's details.
    #[command(after_help = "Examples:\n\
    \n\
      strangetimer view timers\n\
      strangetimer view timers --snapshot\n\
      strangetimer view buzzers\n\
      strangetimer view focus\n\
      strangetimer view focus --snapshot\n\
    \n\
    `view timers` shows every live run (active section) plus defined timers\n\
    without a run (inactive section) in a table; press q or Ctrl+C to exit.\n\
    By default it runs as a live dashboard; `--snapshot` prints a static,\n\
    persistent view that stays in the terminal scrollback.")]
    View {
        /// `timers` for the live overview table, `buzzers` for the buzzer
        /// library, or a timer name for its progress block + countdown
        /// table.
        #[arg(add = ArgValueCompleter::new(candidates::view_targets))]
        name: String,
        /// Print a static snapshot instead of the live dashboard; the
        /// output stays in the terminal scrollback.
        #[arg(long)]
        snapshot: bool,
    },
}

#[derive(Subcommand, Debug)]
pub enum DaemonCommand {
    /// Start the daemon in the background (no-op if it is already running).
    #[command(after_help = "Examples:\n\
    \n\
      strangetimer daemon start\n\
    \n\
    Uses the registered system service (systemd/launchd/schtasks) when one\n\
    exists, otherwise spawns the daemon detached. First run also registers\n\
    the daemon for autostart.")]
    Start,
    /// Stop the running daemon gracefully (it saves state and exits).
    #[command(after_help = "Examples:\n\
    \n\
      strangetimer daemon stop\n\
    \n\
    Sends a graceful shutdown request; force-kills only if the listener\n\
    cannot answer IPC (e.g. an older daemon binary).")]
    Stop,
    /// Report whether the daemon is running and its PID/version.
    #[command(after_help = "Examples:\n\
    \n\
      strangetimer daemon status\n\
    \n\
    Reports running (pid, version), an incompatible listener, or not\n\
    running.")]
    Status,
    /// Stop the daemon, then start it again.
    #[command(after_help = "Examples:\n\
    \n\
      strangetimer daemon restart\n\
    \n\
    Equivalent to `daemon stop` followed by `daemon start`.")]
    Restart,
}

#[derive(clap::ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shell {
    Bash,
    Zsh,
    Fish,
    Powershell,
}

#[derive(Subcommand, Debug)]
pub enum CreateKind {
    /// Create a timer definition.
    ///
    /// Timers take (offset, optional buzzer) pairs — `OFFSET [BUZZER]` —
    /// that repeat. A bare offset uses the built-in default_audio buzzer.
    #[command(after_help = "Examples:\n\
    \n\
      strangetimer create timer focus 25min\n\
      strangetimer create timer workAndFun 45min 15min\n\
      strangetimer create timer weekly 1W paymentBuzzer\n\
    \n\
    Offsets: 30s, 5m/5min, 2h, 1D, 1W, and compounds like 1h30m.")]
    Timer {
        name: String,
        /// Variadic (offset, optional buzzer) pairs. Captured positionally.
        #[arg(
            value_name = "OFFSET [BUZZER]",
            required = true,
            num_args = 1..,
            add = ArgValueCompleter::new(candidates::CreateTimerCompleter)
        )]
        rest: Vec<String>,
    },
    /// Create a custom buzzer. Multiple `--` flags chain actions.
    #[command(after_help = "Examples:\n\
    \n\
      strangetimer create buzzer alarm --audio\n\
      strangetimer create buzzer alarm --audio ~/Music/alert.wav\n\
      strangetimer create buzzer break --video\n\
      strangetimer create buzzer github --url https://github.com/AdarshGuptaa/strange_timer\n\
      strangetimer create buzzer calc --application /usr/bin/gnome-calculator\n\
      strangetimer create buzzer notify --bash ~/notify.sh\n\
      strangetimer create buzzer quitBrowser --close-app firefox\n\
      strangetimer create buzzer chat --focus-window Slack\n\
      strangetimer create buzzer pepTalk --llm llama3 \"a one-line pep talk\"\n\
      strangetimer create buzzer dayEnd --audio --url https://github.com/AdarshGuptaa/strange_timer\n\
    \n\
    Flags chain in order. --close-app requires `confirm-destructive` first.")]
    Buzzer(CreateBuzzerArgs),
}

#[derive(Subcommand, Debug)]
pub enum DuplicateKind {
    /// Duplicate a timer definition.
    #[command(after_help = "Examples:\n\
    \n\
      strangetimer duplicate timer focus\n\
      strangetimer duplicate timer focus morning_focus\n\
    \n\
    Without a new name the copy gets <source>_copy (or _copy_2, …).")]
    Timer {
        #[arg(add = ArgValueCompleter::new(candidates::timer_names))]
        source: String,
        /// Defaults to `<source>_copy` (or `_copy_2`, etc.) when omitted.
        new_name: Option<String>,
    },
}

#[derive(Subcommand, Debug)]
pub enum DeleteKind {
    /// Delete a timer definition.
    #[command(after_help = "Examples:\n\
    \n\
      strangetimer delete timer focus\n\
    \n\
    Refused while the timer has an active run — stop it first.")]
    Timer {
        #[arg(add = ArgValueCompleter::new(candidates::timer_names))]
        name: String,
    },
    /// Delete a buzzer.
    #[command(after_help = "Examples:\n\
    \n\
      strangetimer delete buzzer meeting\n\
    \n\
    Built-in buzzers cannot be deleted.")]
    Buzzer {
        #[arg(add = ArgValueCompleter::new(candidates::deletable_buzzer_names))]
        name: String,
    },
}

#[derive(Args, Debug)]
pub struct RunArgs {
    #[arg(add = ArgValueCompleter::new(candidates::timer_names))]
    pub name: String,
    /// Number of repetitions. Mutually exclusive with `-i`.
    #[arg(short = 'n', long, conflicts_with = "infinite")]
    pub count: Option<u32>,
    /// Repeat infinitely. Mutually exclusive with `-n`.
    #[arg(short = 'i', long, conflicts_with = "count")]
    pub infinite: bool,
    /// 24h clock time at which the first run begins (e.g. `09:00`).
    #[arg(short = 't', long)]
    pub schedule_time: Option<String>,
    /// User-interrupt mode: the timer pauses at every buzzer and audio
    /// buzzers loop until acknowledged with `strangetimer resume <name>`.
    /// The CLI returns immediately after starting.
    #[arg(short = 'u', long)]
    pub user_interrupt: bool,
}

#[derive(Args, Debug)]
pub struct PauseTarget {
    #[arg(add = ArgValueCompleter::new(candidates::running_timer_names))]
    pub name: String,
}

#[derive(Args, Debug)]
pub struct ResumeTarget {
    #[arg(add = ArgValueCompleter::new(candidates::paused_timer_names))]
    pub name: String,
}

#[derive(Args, Debug)]
pub struct StopTarget {
    #[arg(add = ArgValueCompleter::new(candidates::active_run_timer_names))]
    pub name: String,
}

#[derive(Args, Debug)]
pub struct CreateBuzzerArgs {
    pub name: String,
    /// Play an audio file (omit path to use the built-in default sound).
    #[arg(long, num_args = 0..=1, default_missing_value = "", value_hint = ValueHint::AnyPath)]
    pub audio: Option<String>,
    /// Play a video file (omit path to use the built-in default video).
    #[arg(long, num_args = 0..=1, default_missing_value = "", value_hint = ValueHint::AnyPath)]
    pub video: Option<String>,
    /// Launch an application from the given path.
    #[arg(long, value_hint = ValueHint::ExecutablePath)]
    pub application: Option<String>,
    /// Open a URL.
    #[arg(long)]
    pub url: Option<String>,
    /// Run a bash script from the given path.
    #[arg(long, value_hint = ValueHint::AnyPath)]
    pub bash: Option<String>,
    /// Close a running application by process name (e.g. `firefox`).
    /// Destructive — requires `strangetimer confirm-destructive` first.
    #[arg(long)]
    pub close_app: Option<String>,
    /// Close a selected window by X11 window id or window title.
    /// Destructive — requires `strangetimer confirm-destructive` first.
    #[arg(long)]
    pub close_window: Option<String>,
    /// Bring a window matching the given title or application name to the
    /// foreground (e.g. `--focus-window firefox`).
    #[arg(long)]
    pub focus_window: Option<String>,
    /// Invoke an LLM via Ollama. Arg form: `<model> <prompt_or_file>`.
    /// `prompt_or_file` is inline if it parses as text, otherwise read as a
    /// file path at fire time.
    #[arg(long, num_args = 2, value_names = ["MODEL", "PROMPT_OR_FILE"])]
    pub llm: Option<Vec<String>>,
}
