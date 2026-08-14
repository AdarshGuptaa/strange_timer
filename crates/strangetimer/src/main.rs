mod cli;
mod commands;
mod style;

use anyhow::Result;
use clap::CommandFactory;
use clap::Parser;
use cli::{Cli, Command, CreateKind, DeleteKind, DuplicateKind};

fn main() {
    // State-aware completion engine: answers COMPLETE=<shell> <bin> …
    // invocations (tab completion) and exits; a no-op for normal runs.
    clap_complete::CompleteEnv::with_factory(Cli::command).complete();

    let cli = Cli::parse();
    if let Err(e) = run(cli) {
        eprintln!("{}", style::err(&format!("error: {e:#}")));
        std::process::exit(1);
    }
}

fn run(cli: Cli) -> Result<()> {
    match cli.command {
        Command::Create { kind } => match kind {
            CreateKind::Timer { name, rest } => commands::timers::create_timer(&name, &rest),
            CreateKind::Buzzer(args) => commands::buzzers::create_buzzer(&args),
        },
        Command::Duplicate { kind } => match kind {
            DuplicateKind::Timer { source, new_name } => {
                commands::timers::duplicate_timer(&source, new_name)
            }
        },
        Command::Delete { kind } => match kind {
            DeleteKind::Timer { name } => commands::timers::delete_timer(&name),
            DeleteKind::Buzzer { name } => commands::buzzers::delete_buzzer(&name),
        },
        Command::Run(args) => commands::control::run(&args),
        Command::Pause(args) => commands::control::pause(&args.name),
        Command::Pauseall => commands::control::pause_all(),
        Command::Resume(args) => commands::control::resume(&args.name),
        Command::Stop(args) => commands::control::stop(&args.name),
        Command::Stopall => commands::control::stop_all(),
        Command::ConfirmDestructive => commands::control::confirm_destructive(),
        Command::Daemon { cmd } => commands::daemon::run(&cmd),
        Command::Examples { install } => {
            if install {
                commands::examples::install_examples()
            } else {
                commands::examples::list_examples()
            }
        }
        Command::Completions { shell } => commands::completions::print_completions(shell),
        Command::InstallCompletions { shell } => {
            commands::install_completions::install_completions(shell)
        }
        Command::View { name, snapshot } => match name.as_str() {
            "timers" => commands::view::view_timers(snapshot),
            "buzzers" => commands::buzzers::view_buzzers(),
            _ => commands::view::view_timer(&name, snapshot),
        },
    }
}
