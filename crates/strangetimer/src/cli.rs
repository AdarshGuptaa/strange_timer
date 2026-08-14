use clap::{Args, Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(
    name = "strangetimer",
    version,
    about = "StrangeTimer — CLI timer application"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Create a new timer definition or buzzer.
    Create {
        #[command(subcommand)]
        kind: CreateKind,
    },
    /// Duplicate an existing timer definition.
    Duplicate {
        #[command(subcommand)]
        kind: DuplicateKind,
    },
    /// Delete a timer or buzzer definition.
    Delete {
        #[command(subcommand)]
        kind: DeleteKind,
    },
    /// Run a timer.
    Run(RunArgs),
    /// Pause a running timer.
    Pause(PauseTarget),
    /// Pause every running timer.
    Pauseall,
    /// Resume a paused timer.
    Resume(ResumeTarget),
    /// Stop a running timer (keeps the definition).
    Stop(StopTarget),
    /// Stop every running timer.
    Stopall,
    /// Opt in to the destructive `close_windows` buzzer (closes ALL other
    /// windows when it fires).
    ConfirmDestructive,
    /// Manage the background daemon process.
    Daemon {
        #[command(subcommand)]
        cmd: DaemonCommand,
    },
    /// Show or install example buzzers demonstrating every action type.
    Examples {
        /// Create the example buzzers in the daemon's library (skips any
        /// that already exist).
        #[arg(long)]
        install: bool,
    },
    /// Print a shell completion script for this CLI.
    Completions {
        /// Which shell to generate completions for.
        #[arg(value_enum)]
        shell: Shell,
    },
    /// View timers, buzzers, or a single timer's details.
    View {
        /// `timers` for all runs, `buzzers` for the buzzer library, or a
        /// timer name to show that timer's progress block + buzzer table.
        name: String,
    },
}

#[derive(Subcommand, Debug)]
pub enum DaemonCommand {
    /// Start the daemon in the background (no-op if it is already running).
    Start,
    /// Stop the running daemon gracefully (it saves state and exits).
    Stop,
    /// Report whether the daemon is running and its PID/version.
    Status,
    /// Stop the daemon, then start it again.
    Restart,
}

#[derive(clap::ValueEnum, Debug, Clone, Copy)]
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
    /// Layout: `strangetimer create timer <name> <offset> [<buzzer_name>]
    /// [<offset> [<buzzer_name>]] ...` — pairs of (offset, optional buzzer
    /// name) repeat. A bare offset uses the default buzzer.
    Timer {
        name: String,
        /// Variadic (offset, buzzer_name?) pairs. Captured positionally.
        #[arg(required = true, num_args = 1..)]
        rest: Vec<String>,
    },
    /// Create a custom buzzer. Multiple `--` flags chain actions.
    Buzzer(CreateBuzzerArgs),
}

#[derive(Subcommand, Debug)]
pub enum DuplicateKind {
    /// Duplicate a timer definition.
    Timer {
        source: String,
        /// Defaults to `<source>_copy` (or `_copy_2`, etc.) when omitted.
        new_name: Option<String>,
    },
}

#[derive(Subcommand, Debug)]
pub enum DeleteKind {
    /// Delete a timer definition.
    Timer { name: String },
    /// Delete a buzzer.
    Buzzer { name: String },
}

#[derive(Args, Debug)]
pub struct RunArgs {
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
}

#[derive(Args, Debug)]
pub struct PauseTarget {
    pub name: String,
}

#[derive(Args, Debug)]
pub struct ResumeTarget {
    pub name: String,
}

#[derive(Args, Debug)]
pub struct StopTarget {
    pub name: String,
}

#[derive(Args, Debug)]
pub struct CreateBuzzerArgs {
    pub name: String,
    /// Play an audio file (omit path to use the built-in default sound).
    #[arg(long, num_args = 0..=1, default_missing_value = "")]
    pub audio: Option<String>,
    /// Play a video file (omit path to use the built-in default video).
    #[arg(long, num_args = 0..=1, default_missing_value = "")]
    pub video: Option<String>,
    /// Launch an application from the given path.
    #[arg(long)]
    pub application: Option<String>,
    /// Open a URL.
    #[arg(long)]
    pub url: Option<String>,
    /// Run a bash script from the given path.
    #[arg(long)]
    pub bash: Option<String>,
    /// Close a running application by process name (e.g. `firefox`).
    /// Destructive — requires `strangetimer confirm-destructive` first.
    #[arg(long)]
    pub close_app: Option<String>,
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
