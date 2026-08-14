//! `strangetimer completions <shell>` — print a shell completion script.

use anyhow::Result;
use clap::CommandFactory;
use clap_complete::Shell;

use crate::cli::{Cli, Shell as ShellArg};

/// `strangetimer completions <bash|zsh|fish|powershell>`
pub fn print_completions(shell: ShellArg) -> Result<()> {
    print!("{}", script_for(shell));
    Ok(())
}

/// The completion script for `shell`, generated from the live clap tree so
/// it always matches the installed CLI.
pub fn script_for(shell: ShellArg) -> String {
    let shell = match shell {
        ShellArg::Bash => Shell::Bash,
        ShellArg::Zsh => Shell::Zsh,
        ShellArg::Fish => Shell::Fish,
        ShellArg::Powershell => Shell::PowerShell,
    };
    let mut cmd = Cli::command();
    let name = cmd.get_name().to_string();
    let mut buf = Vec::new();
    clap_complete::generate(shell, &mut cmd, name, &mut buf);
    String::from_utf8_lossy(&buf).into_owned()
}
