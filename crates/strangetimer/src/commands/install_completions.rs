//! `strangetimer install-completions [--shell <shell>]` — install shell
//! completions so <Tab> completes commands, flags and names.
//!
//! Targets the standard per-user locations so no rc-file editing is needed:
//! - bash: `~/.local/share/bash-completion/completions/` (auto-loaded)
//! - fish: `~/.config/fish/completions/` (auto-loaded)
//! - zsh:  `~/.zfunc/` (needs `fpath` wiring — prints the line if missing)
//! - powershell: prints the `$PROFILE` line (never edits the profile)

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};

use crate::cli::Shell;
use crate::commands::completions::script_for;

/// `strangetimer install-completions`
pub fn install_completions(shell: Option<Shell>) -> Result<()> {
    let shell = match shell {
        Some(s) => s,
        None => detect_shell().ok_or_else(|| {
            anyhow!(
                "cannot detect your shell from $SHELL — pass `--shell bash|zsh|fish|powershell`"
            )
        })?,
    };

    let home = dirs::home_dir().ok_or_else(|| anyhow!("cannot resolve your home directory"))?;
    let script = script_for(shell);
    install_to(shell, &home, &script)
}

/// Install the completion script under `home`. Split out for testing.
fn install_to(shell: Shell, home: &Path, script: &str) -> Result<()> {
    match shell {
        Shell::Bash => {
            let dir = home.join(".local/share/bash-completion/completions");
            let path = dir.join("strangetimer");
            write_script(&dir, &path, script)?;
            println!(
                "Installed bash completions to {}. They are picked up by \
                 new shell sessions automatically.",
                path.display()
            );
        }
        Shell::Fish => {
            let dir = home.join(".config/fish/completions");
            let path = dir.join("strangetimer.fish");
            write_script(&dir, &path, script)?;
            println!(
                "Installed fish completions to {}. They are picked up by \
                 new shell sessions automatically.",
                path.display()
            );
        }
        Shell::Zsh => {
            let dir = home.join(".zfunc");
            let path = dir.join("_strangetimer");
            write_script(&dir, &path, script)?;
            println!("Installed zsh completions to {}.", path.display());
            if !zshrc_wired_for_zfunc(home) {
                println!(
                    "Add this to your ~/.zshrc so zsh finds them:\n\
                     \n  fpath=(~/.zfunc $fpath)\n  autoload -Uz compinit && compinit"
                );
            }
        }
        Shell::Powershell => {
            println!(
                "PowerShell completions are installed via your profile.\n\
                 Add this line to your $PROFILE:\n\
                 \n  strangetimer completions powershell | Out-String | Invoke-Expression"
            );
        }
    }
    Ok(())
}

fn write_script(dir: &Path, path: &Path, script: &str) -> Result<()> {
    fs::create_dir_all(dir).with_context(|| format!("failed to create {}", dir.display()))?;
    fs::write(path, script).with_context(|| format!("failed to write {}", path.display()))?;
    Ok(())
}

/// Whether `~/.zshrc` already points at `~/.zfunc` in its fpath.
fn zshrc_wired_for_zfunc(home: &Path) -> bool {
    let zshrc = home.join(".zshrc");
    match fs::read_to_string(zshrc) {
        Ok(content) => content.contains(".zfunc"),
        Err(_) => false,
    }
}

/// Map `$SHELL`'s basename to a supported shell.
fn detect_shell() -> Option<Shell> {
    let shell = std::env::var("SHELL").ok()?;
    let base = PathBuf::from(shell)
        .file_name()?
        .to_str()?
        .to_ascii_lowercase();
    match base.as_str() {
        "bash" => Some(Shell::Bash),
        "zsh" => Some(Shell::Zsh),
        "fish" => Some(Shell::Fish),
        "pwsh" | "powershell" => Some(Shell::Powershell),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_home(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("st-install-{label}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn bash_installs_to_bash_completion_dir() {
        let home = tmp_home("bash");
        install_to(Shell::Bash, &home, "canned-script").unwrap();
        let path = home.join(".local/share/bash-completion/completions/strangetimer");
        assert!(path.exists(), "missing {}", path.display());
        let content = fs::read_to_string(&path).unwrap();
        assert_eq!(content, "canned-script");
    }

    #[test]
    fn fish_installs_to_fish_completions_dir() {
        let home = tmp_home("fish");
        install_to(Shell::Fish, &home, "canned-script").unwrap();
        let path = home.join(".config/fish/completions/strangetimer.fish");
        assert!(path.exists(), "missing {}", path.display());
    }

    #[test]
    fn zsh_writes_script_and_reports_rc_wiring() {
        let home = tmp_home("zsh");
        install_to(Shell::Zsh, &home, "canned-script").unwrap();
        assert!(home.join(".zfunc/_strangetimer").exists());
        assert!(!zshrc_wired_for_zfunc(&home));
        fs::write(home.join(".zshrc"), "fpath=(~/.zfunc $fpath)\n").unwrap();
        assert!(zshrc_wired_for_zfunc(&home));
    }

    #[test]
    fn detect_shell_maps_common_shells() {
        std::env::set_var("SHELL", "/usr/bin/zsh");
        assert_eq!(detect_shell(), Some(Shell::Zsh));
        std::env::set_var("SHELL", "/bin/bash");
        assert_eq!(detect_shell(), Some(Shell::Bash));
    }
}
