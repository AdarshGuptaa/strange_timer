//! Daemon lifecycle management: `strangetimer daemon start|stop|status|restart`.
//!
//! The daemon is a single-instance service: a second copy refuses to bind
//! the IPC socket. These commands are the supported way to manage it — no
//! more `&`, `kill`, or background-process bookkeeping by hand.

use std::fs;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use strangetimer_core::ipc::{ClientMessage, ServerMessage};
use strangetimer_core::persistence::data_dir;

use crate::cli::DaemonCommand;
use crate::commands::send_and_receive;

/// `strangetimer daemon <start|stop|status|restart>`
pub fn run(cmd: &DaemonCommand) -> Result<()> {
    match cmd {
        DaemonCommand::Start => start(),
        DaemonCommand::Stop => stop(),
        DaemonCommand::Status => status(),
        DaemonCommand::Restart => {
            // A stop failure (e.g. timeout) should not silently skip the
            // start — report it but continue.
            if let Err(e) = stop() {
                eprintln!("warning: {e:#}");
            }
            start()
        }
    }
}

/// `strangetimer daemon status`
fn status() -> Result<()> {
    match daemon_status() {
        Some((pid, version)) => {
            println!("strangetimer-daemon is running (pid {pid}, version {version}).");
        }
        None => {
            println!("strangetimer-daemon is not running.");
        }
    }
    Ok(())
}

/// `strangetimer daemon start`
fn start() -> Result<()> {
    if let Some((pid, version)) = daemon_status() {
        println!("strangetimer-daemon is already running (pid {pid}, version {version}).");
        return Ok(());
    }

    let daemon = find_daemon_binary().context(
        "could not locate the strangetimer-daemon binary — expected it next to \
         this binary or on PATH (set STRANGETIMER_DAEMON to its path)",
    )?;

    let log_path = data_dir().join("daemon.log");
    let log = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .with_context(|| format!("failed to open daemon log {}", log_path.display()))?;

    let mut cmd = std::process::Command::new(&daemon);
    cmd.stdin(Stdio::null())
        .stdout(Stdio::from(
            log.try_clone().context("failed to clone log file")?,
        ))
        .stderr(Stdio::from(log));
    #[cfg(unix)]
    {
        // New process group: Ctrl+C on the CLI must not propagate to the
        // daemon.
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    #[cfg(windows)]
    {
        // DETACHED_PROCESS: no console window, survives the CLI's exit.
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0000_0008);
    }

    let child = cmd
        .spawn()
        .with_context(|| format!("failed to spawn {}", daemon.display()))?;

    // Wait for the listener to come up. Slow machines / first-run seeding can
    // take a couple of seconds.
    const READY_TIMEOUT: Duration = Duration::from_secs(10);
    if wait_until(|s| s.is_some(), READY_TIMEOUT) {
        println!(
            "Started strangetimer-daemon (pid {}). Log: {}",
            child.id(),
            log_path.display()
        );
        Ok(())
    } else {
        Err(anyhow!(
            "strangetimer-daemon (pid {}) did not start listening within {:?}. \
             Check the log: {}",
            child.id(),
            READY_TIMEOUT,
            log_path.display()
        ))
    }
}

/// `strangetimer daemon stop`
fn stop() -> Result<()> {
    if daemon_status().is_none() {
        println!("strangetimer-daemon is not running.");
        return Ok(());
    }

    match send_and_receive(&ClientMessage::Shutdown) {
        Ok(_) => {}
        Err(e) => eprintln!("warning: shutdown request failed: {e:#}"),
    }

    const STOP_TIMEOUT: Duration = Duration::from_secs(10);
    if wait_until(|s| s.is_none(), STOP_TIMEOUT) {
        println!("Stopped strangetimer-daemon.");
        Ok(())
    } else {
        Err(anyhow!(
            "strangetimer-daemon did not exit within {:?} after a shutdown request",
            STOP_TIMEOUT
        ))
    }
}

/// Query the daemon over IPC. `Some((pid, version))` when it is running.
fn daemon_status() -> Option<(u32, String)> {
    match send_and_receive(&ClientMessage::Ping) {
        Ok(ServerMessage::Status { pid, version }) => Some((pid, version)),
        _ => None,
    }
}

/// Poll `probe` every 100 ms until it returns true or `timeout` elapses.
fn wait_until(probe: impl Fn(Option<(u32, String)>) -> bool, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if probe(daemon_status()) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    false
}

/// Locate the daemon binary: same directory as the running CLI, then PATH,
/// then the `STRANGETIMER_DAEMON` environment override.
fn find_daemon_binary() -> Result<PathBuf> {
    let exe_suffix = std::env::consts::EXE_SUFFIX;
    let daemon_name = format!("strangetimer-daemon{exe_suffix}");

    if let Some(override_path) = std::env::var_os("STRANGETIMER_DAEMON") {
        let path = PathBuf::from(override_path);
        if path.is_file() {
            return Ok(path);
        }
    }

    if let Ok(exe) = std::env::current_exe() {
        let sibling = exe
            .parent()
            .unwrap_or(std::path::Path::new("."))
            .join(&daemon_name);
        if sibling.is_file() {
            return Ok(sibling);
        }
    }

    if let Some(paths) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&paths) {
            let candidate = dir.join(&daemon_name);
            if candidate.is_file() {
                return Ok(candidate);
            }
        }
    }

    Err(anyhow!("daemon binary {:?} not found", daemon_name))
}
