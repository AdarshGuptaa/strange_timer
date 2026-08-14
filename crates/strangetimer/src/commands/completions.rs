//! `strangetimer completions <shell>` — print a shell completion script.

use anyhow::Result;
use clap::CommandFactory;
use clap_complete::Shell;

use crate::cli::{Cli, Shell as ShellArg};

/// `strangetimer completions <bash|zsh|fish|powershell>`
pub fn print_completions(shell: ShellArg) -> Result<()> {
    let shell = match shell {
        ShellArg::Bash => Shell::Bash,
        ShellArg::Zsh => Shell::Zsh,
        ShellArg::Fish => Shell::Fish,
        ShellArg::Powershell => Shell::PowerShell,
    };
    let mut cmd = Cli::command();
    let name = cmd.get_name().to_string();
    clap_complete::generate(shell, &mut cmd, name, &mut std::io::stdout());
    Ok(())
}
