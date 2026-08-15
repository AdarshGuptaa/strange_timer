//! `strangetimer doctor` — report installation health and optional
//! capability status so users know what works and what to install.

use std::process::Command;

use anyhow::{Context, Result};
use strangetimer_core::ipc::{ClientMessage, ServerMessage};

use crate::commands::send_and_receive;
use crate::style;

/// `strangetimer doctor`
pub fn doctor() -> Result<()> {
    println!("{}", style::header("StrangeTimer doctor"));
    println!();

    let cli = std::env::current_exe().context("cannot resolve CLI path")?;
    println!("{} {}", style::dim("CLI binary:"), cli.display());
    println!(
        "{} {}",
        style::dim("CLI version:"),
        env!("CARGO_PKG_VERSION")
    );

    let daemon = match send_and_receive(&ClientMessage::Ping) {
        Ok(ServerMessage::Status {
            pid,
            version,
            protocol,
        }) => {
            println!(
                "{} (pid {}, version {}, protocol {})",
                style::name("Daemon: running"),
                pid,
                version,
                protocol
            );
            format!("pid {pid} ({version})")
        }
        Ok(_) => {
            println!("{}", style::warn("Daemon: running but unexpected reply"));
            "unknown".to_string()
        }
        Err(e) => {
            println!("{}", style::err(&format!("Daemon: not reachable ({e:#})")));
            "not reachable".to_string()
        }
    };
    let _ = daemon;

    let session = std::env::var("XDG_SESSION_TYPE").unwrap_or_default();
    if !session.is_empty() {
        println!(
            "{} {}",
            style::dim("Session type:"),
            if session == "wayland" {
                "wayland (window focus/closing via X11 tools is not supported; --close-app works)"
                    .to_string()
            } else {
                session.clone()
            }
        );
    }

    println!();
    println!("{}", style::header("Optional capabilities"));

    let tool_ok = |name: &str| Command::new(name).arg("--version").output().is_ok();
    for (name, purpose) in [
        ("wmctrl", "window focus / close (X11)"),
        ("xdotool", "window focus / close (X11)"),
        ("pkill", "close_app buzzers"),
    ] {
        if tool_ok(name) {
            println!("  [ok] {name} — {purpose}");
        } else {
            println!("  [--] {name} — {purpose} (not installed)");
        }
    }

    let ollama =
        std::env::var("OLLAMA_HOST").unwrap_or_else(|_| "http://localhost:11434".to_string());
    println!(
        "  {} --llm buzzers expect Ollama at {}",
        if tool_ok("ollama") { "[ok]" } else { "[--]" },
        ollama
    );

    println!();
    println!("{}", style::dim("Fix hints:"));
    println!(
        "{}",
        style::dim(
            "  sudo apt install wmctrl xdotool   (X11 focus/closing, Debian/Ubuntu)\n  \
             ollama serve                       (local LLM buzzers)"
        )
    );
    Ok(())
}
