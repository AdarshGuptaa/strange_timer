//! Daemon lifecycle management: `strangetimer daemon start|stop|status|restart`.
//!
//! The daemon is a single-instance service: a second copy refuses to bind
//! the IPC socket. These commands are the supported way to manage it — no
//! more `&`, `kill`, or background-process bookkeeping by hand.
//!
//! Liveness is probed in two steps: a raw socket connect decides whether
//! anything is *listening*, and a `Ping` round-trip decides whether it is a
//! *compatible* daemon. A listener that cannot answer `Ping` is an older or
//! foreign binary — probing it as "not running" would make `daemon start`
//! spawn a second instance that then fails with "Address already in use".

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use strangetimer_core::ipc::socket_name;
use strangetimer_core::ipc::{ClientMessage, ServerMessage};
use strangetimer_core::persistence::data_dir;

use crate::cli::DaemonCommand;
use crate::commands::{send_and_receive, try_connect};

/// How long to wait for the daemon to appear / disappear after a lifecycle
/// action, and between polls.
const WAIT_TIMEOUT: Duration = Duration::from_secs(10);
const POLL_INTERVAL: Duration = Duration::from_millis(100);

/// State of the daemon endpoint as seen by the probe.
#[derive(Debug, Clone, PartialEq)]
pub enum Probe {
    /// A compatible daemon answered `Ping`.
    Running { pid: u32, version: String },
    /// Something is listening but cannot answer `Ping` (older binary or a
    /// foreign process on our socket).
    Incompatible,
    /// Nothing is listening.
    NotRunning,
}

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
    match probe() {
        Probe::Running { pid, version } => {
            println!("strangetimer-daemon is running (pid {pid}, version {version}).");
        }
        Probe::Incompatible => {
            println!(
                "something is listening on {} but it is not a compatible \
                 strangetimer-daemon (is an older version running?)",
                socket_name()
            );
        }
        Probe::NotRunning => {
            println!("strangetimer-daemon is not running.");
        }
    }
    Ok(())
}

/// `strangetimer daemon start`
fn start() -> Result<()> {
    start_daemon(true).map(|_| ())
}

/// Ensure a daemon is running, starting one if needed. When `announce` is
/// false the routine stays quiet except for what it actually did (used by
/// the auto-start path, which prints its own notice).
pub fn ensure_started(announce: bool) -> Result<()> {
    match probe() {
        Probe::Running { .. } => {
            if announce {
                println!("strangetimer-daemon is already running.");
            }
            Ok(())
        }
        Probe::Incompatible => Err(incompatible_error()),
        Probe::NotRunning => {
            start_daemon(announce)?;
            Ok(())
        }
    }
}

/// The shared start routine: probes, locates the binary, tries the OS
/// service manager first (systemd/launchd/schtasks), falls back to a direct
/// detached spawn, and waits for readiness.
fn start_daemon(verbose: bool) -> Result<Probe> {
    match probe() {
        Probe::Running { pid, version } => {
            if verbose {
                println!("strangetimer-daemon is already running (pid {pid}, version {version}).");
            }
            return Ok(Probe::Running { pid, version });
        }
        Probe::Incompatible => return Err(incompatible_error()),
        Probe::NotRunning => {}
    }

    let daemon = find_daemon_binary().context(
        "could not locate the strangetimer-daemon binary — expected it next to \
         this binary or on PATH (set STRANGETIMER_DAEMON to its path)",
    )?;

    #[cfg(target_os = "linux")]
    if let Some(outcome) = try_systemd_start(&daemon)? {
        return Ok(outcome);
    }
    #[cfg(target_os = "macos")]
    if let Some(outcome) = try_launchd_start(&daemon)? {
        return Ok(outcome);
    }
    #[cfg(target_os = "windows")]
    if let Some(outcome) = try_schtasks_start()? {
        return Ok(outcome);
    }

    spawn_detached(&daemon)
}

/// Spawn the daemon as a detached background process with its logs going to
/// `daemon.log` in the data dir, then wait for it to serve.
fn spawn_detached(daemon: &Path) -> Result<Probe> {
    let log_path = data_dir().join("daemon.log");
    let log = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .with_context(|| format!("failed to open daemon log {}", log_path.display()))?;

    let mut cmd = Command::new(daemon);
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
    println!(
        "Started strangetimer-daemon (pid {}). Log: {}",
        child.id(),
        log_path.display()
    );

    wait_for_running().map_err(|e| {
        anyhow!(
            "strangetimer-daemon (pid {}) {e} Check the log: {}",
            child.id(),
            log_path.display()
        )
    })
}

/// Wait until the probe reports a compatible running daemon.
fn wait_for_running() -> Result<Probe> {
    let deadline = Instant::now() + WAIT_TIMEOUT;
    loop {
        match probe() {
            probe @ Probe::Running { .. } => return Ok(probe),
            Probe::Incompatible => return Err(incompatible_error()),
            Probe::NotRunning => {
                if Instant::now() >= deadline {
                    return Err(anyhow!("did not start listening within {:?}", WAIT_TIMEOUT));
                }
                std::thread::sleep(POLL_INTERVAL);
            }
        }
    }
}

/// Wait until nothing is listening (graceful shutdown finished). An
/// incompatible listener counts as gone — the daemon we asked to exit is
/// no longer answering.
fn wait_for_gone() -> Result<()> {
    let deadline = Instant::now() + WAIT_TIMEOUT;
    loop {
        if !matches!(probe(), Probe::Running { .. }) {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(anyhow!("did not exit within {:?}", WAIT_TIMEOUT));
        }
        std::thread::sleep(POLL_INTERVAL);
    }
}

/// `strangetimer daemon stop`
fn stop() -> Result<()> {
    match probe() {
        Probe::NotRunning => {
            println!("strangetimer-daemon is not running.");
            return Ok(());
        }
        Probe::Incompatible => {
            eprintln!(
                "warning: the listener on {} cannot answer IPC; forcing a stop",
                socket_name()
            );
            force_kill()?;
            return wait_for_gone().map(|_| println!("Stopped strangetimer-daemon."));
        }
        Probe::Running { .. } => {}
    }

    // If the OS service manager owns this daemon, let it stop the process —
    // otherwise systemd would restart a pkill'd one.
    #[cfg(target_os = "linux")]
    if systemd_unit_active() {
        let ok = run_quiet("systemctl", &["--user", "stop", "strangetimer"]);
        if ok && wait_for_gone().is_ok() {
            println!("Stopped strangetimer-daemon (systemd service).");
            return Ok(());
        }
    }
    #[cfg(target_os = "macos")]
    if launchd_plist_exists() {
        let ok = run_quiet("launchctl", &["stop", "com.strangetimer.daemon"]);
        if ok && wait_for_gone().is_ok() {
            println!("Stopped strangetimer-daemon (launchd service).");
            return Ok(());
        }
    }

    match send_and_receive(&ClientMessage::Shutdown) {
        Ok(_) => {}
        Err(e) => eprintln!("warning: shutdown request failed: {e:#}"),
    }

    match wait_for_gone() {
        Ok(()) => {
            println!("Stopped strangetimer-daemon.");
            Ok(())
        }
        Err(e) => {
            eprintln!("warning: {e:#}; forcing a stop");
            force_kill()?;
            wait_for_gone().map(|_| println!("Stopped strangetimer-daemon."))
        }
    }
}

/// Hard-kill the daemon by process name (used only when IPC is unavailable).
fn force_kill() -> Result<()> {
    #[cfg(unix)]
    {
        let name = "strangetimer-daemon";
        let status = Command::new("pkill").args(["-x", name]).status();
        match status {
            Ok(s) if s.success() => Ok(()),
            _ => Err(anyhow!(
                "failed to stop {name} — kill it manually (e.g. `pkill -x {name}`)"
            )),
        }
    }
    #[cfg(windows)]
    {
        let name = "strangetimer-daemon.exe";
        let status = Command::new("taskkill").args(["/F", "/IM", name]).status();
        match status {
            Ok(s) if s.success() => Ok(()),
            _ => Err(anyhow!(
                "failed to stop {name} — kill it manually (`taskkill /F /IM {name}`)"
            )),
        }
    }
}

/// The two-step probe: connect decides *listening*, Ping decides *compatible*.
/// The Ping is exchanged on the same connection so the probe never leaves a
/// dangling empty connection behind (which would log a spurious warning in
/// the daemon).
pub fn probe() -> Probe {
    let mut conn = match try_connect() {
        Ok(c) => c,
        Err(_) => return Probe::NotRunning,
    };
    if strangetimer_core::ipc::write_message(&mut conn, &ClientMessage::Ping).is_err() {
        return Probe::Incompatible;
    }
    match strangetimer_core::ipc::read_message::<ServerMessage>(&mut conn) {
        Ok(ServerMessage::Status { pid, version }) => Probe::Running { pid, version },
        _ => Probe::Incompatible,
    }
}

fn incompatible_error() -> anyhow::Error {
    anyhow!(
        "the listener on {} is not a compatible strangetimer-daemon — an older \
         version is probably running. Stop it with `strangetimer daemon stop` \
         (it will force-kill if IPC fails) or kill it manually.",
        socket_name()
    )
}

/// Locate the daemon binary: `STRANGETIMER_DAEMON` override, same directory
/// as the running CLI, then PATH.
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

// --- OS service manager integration -------------------------------------
//
// When the daemon has registered an OS service unit, start/stop go through
// the service manager instead of spawning a second process that would race
// for the socket. The isolation env vars (tests / custom setups) always
// take the direct path — a spawned daemon inherits them, systemd would not.

/// True when STRANGETIMER_SOCKET or STRANGETIMER_DATA_DIR is set (isolated
/// setups that must never touch the system service manager).
fn isolated_env() -> bool {
    std::env::var_os("STRANGETIMER_SOCKET").is_some()
        || std::env::var_os("STRANGETIMER_DATA_DIR").is_some()
}

fn run_quiet(program: &str, args: &[&str]) -> bool {
    Command::new(program)
        .args(args)
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

#[cfg(target_os = "linux")]
fn systemd_unit_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("systemd/user/strangetimer.service"))
}

#[cfg(target_os = "linux")]
fn systemd_unit_active() -> bool {
    if isolated_env() {
        return false;
    }
    let Some(unit) = systemd_unit_path() else {
        return false;
    };
    unit.exists()
        && run_quiet(
            "systemctl",
            &["--user", "is-active", "--quiet", "strangetimer"],
        )
}

/// Start via systemd when the user unit exists. Returns:
/// - `Ok(Some(probe))` — systemd handled it (ready, or the attempt failed
///   in a way worth surfacing),
/// - `Ok(None)` — fall back to a direct spawn (no unit / no systemctl /
///   unit failed to start).
#[cfg(target_os = "linux")]
fn try_systemd_start(daemon: &Path) -> Result<Option<Probe>> {
    if isolated_env() {
        return Ok(None);
    }
    let Some(unit) = systemd_unit_path() else {
        return Ok(None);
    };
    if !unit.exists() || Command::new("systemctl").arg("--version").output().is_err() {
        return Ok(None);
    }

    heal_systemd_unit(&unit, daemon)?;

    if !run_quiet("systemctl", &["--user", "start", "strangetimer"]) {
        // e.g. unit enabled but broken; fall back to a direct spawn.
        return Ok(None);
    }
    match wait_for_running() {
        Ok(probe) => {
            println!("Started strangetimer-daemon (systemd service).");
            Ok(Some(probe))
        }
        Err(e) => Err(anyhow!(
            "{e:#} — the systemd service was started but the daemon did not \
             come up; check `journalctl --user -u strangetimer -n 50`"
        )),
    }
}

/// Keep the unit's ExecStart pointing at the current daemon binary. Dev
/// builds move between target/debug and target/release; a stale path makes
/// autostart start a binary that no longer exists.
#[cfg(target_os = "linux")]
fn heal_systemd_unit(unit: &Path, daemon: &Path) -> Result<()> {
    let content =
        fs::read_to_string(unit).with_context(|| format!("failed to read {}", unit.display()))?;
    let want = format!("ExecStart={}", daemon.display());
    if content.lines().any(|l| l == want) {
        return Ok(());
    }

    let healed: String = content
        .lines()
        .map(|l| {
            if l.starts_with("ExecStart=") {
                want.as_str()
            } else {
                l
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    fs::write(unit, healed).with_context(|| format!("failed to heal {}", unit.display()))?;
    run_quiet("systemctl", &["--user", "daemon-reload"]);
    eprintln!(
        "strangetimer: healed systemd unit {} to point at {}",
        unit.display(),
        daemon.display()
    );
    Ok(())
}

#[cfg(target_os = "macos")]
fn launchd_plist_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("Library/LaunchAgents/com.strangetimer.daemon.plist")
}

#[cfg(target_os = "macos")]
fn launchd_plist_exists() -> bool {
    !isolated_env() && launchd_plist_path().exists()
}

/// Start via launchd when the plist exists (`launchctl kickstart` starts
/// the job without re-loading).
#[cfg(target_os = "macos")]
fn try_launchd_start(_daemon: &Path) -> Result<Option<Probe>> {
    if !launchd_plist_exists() {
        return Ok(None);
    }
    let uid = Command::new("id")
        .arg("-u")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();
    let target = format!("gui/{uid}/com.strangetimer.daemon");
    if !run_quiet("launchctl", &["kickstart", "-k", &target]) {
        return Ok(None);
    }
    match wait_for_running() {
        Ok(probe) => {
            println!("Started strangetimer-daemon (launchd service).");
            Ok(Some(probe))
        }
        Err(e) => Err(anyhow!(
            "{e:#} — launchd job started but the daemon did not come up"
        )),
    }
}

/// Start via the scheduled task when it exists.
#[cfg(target_os = "windows")]
fn try_schtasks_start() -> Result<Option<Probe>> {
    if isolated_env() {
        return Ok(None);
    }
    let exists = Command::new("schtasks")
        .args(["/Query", "/TN", "StrangeTimerDaemon"])
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if !exists {
        return Ok(None);
    }
    if !run_quiet("schtasks", &["/Run", "/TN", "StrangeTimerDaemon"]) {
        return Ok(None);
    }
    match wait_for_running() {
        Ok(probe) => {
            println!("Started strangetimer-daemon (scheduled task).");
            Ok(Some(probe))
        }
        Err(e) => Err(anyhow!(
            "{e:#} — scheduled task started but the daemon did not come up"
        )),
    }
}
