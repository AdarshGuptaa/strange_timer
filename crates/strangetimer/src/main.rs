mod cli;

use clap::Parser;
use cli::{Cli, Command, CreateKind, DeleteKind, DuplicateKind, PauseTarget, ResumeTarget, RunArgs, StopTarget};

fn main() {
    let cli = Cli::parse();
    match cli.command {
        Command::Create { kind } => match kind {
            CreateKind::Timer { name, rest } => {
                not_implemented(&format!("create timer {name} {}", rest.join(" ")))
            }
            CreateKind::Buzzer(args) => not_implemented(&format!("create buzzer {}", args.name)),
        },
        Command::Duplicate { kind } => match kind {
            DuplicateKind::Timer { source, new_name } => {
                not_implemented(&match new_name {
                    Some(n) => format!("duplicate timer {source} {n}"),
                    None => format!("duplicate timer {source}"),
                })
            }
        },
        Command::Delete { kind } => match kind {
            DeleteKind::Timer { name } => not_implemented(&format!("delete timer {name}")),
            DeleteKind::Buzzer { name } => not_implemented(&format!("delete buzzer {name}")),
        },
        Command::Run(args) => run_branch(args),
        Command::Pause(PauseTarget { name }) => not_implemented(&format!("pause {name}")),
        Command::Pauseall => not_implemented("pauseall"),
        Command::Resume(ResumeTarget { name }) => not_implemented(&format!("resume {name}")),
        Command::Stop(StopTarget { name }) => not_implemented(&format!("stop {name}")),
        Command::Stopall => not_implemented("stopall"),
        Command::View { name } => match name.as_str() {
            "timers" => not_implemented("view timers"),
            "buzzers" => not_implemented("view buzzers"),
            _ => not_implemented(&format!("view {name}")),
        },
    }
}

fn run_branch(args: RunArgs) {
    let mut parts = vec![format!("run {}", args.name)];
    if let Some(n) = args.count {
        parts.push(format!("-n {n}"));
    }
    if args.infinite {
        parts.push("-i".to_string());
    }
    if let Some(t) = args.schedule_time {
        parts.push(format!("-t {t}"));
    }
    not_implemented(&parts.join(" "));
}

fn not_implemented(cmd: &str) {
    println!("not yet implemented: {cmd}");
}
