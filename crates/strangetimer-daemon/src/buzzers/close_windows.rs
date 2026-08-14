use std::process::Command;

/// Close every visible window belonging to processes other than the daemon.
///
/// This is destructive — it is only ever called after the user has opted in
/// via `strangetimer confirm-destructive` (checked in `buzzers::dispatch`).
pub fn fire_close_windows(daemon_pid: u32) {
    #[cfg(target_os = "linux")]
    {
        if let Err(e) = close_linux(daemon_pid) {
            eprintln!("strangetimer-daemon: close_windows failed: {e}");
        }
    }
    #[cfg(target_os = "macos")]
    {
        // AppleScript: close every window of every process whose unix id is
        // not the daemon's.
        let script = format!(
            r#"tell application "System Events"
    repeat with proc in (every process whose unix id is not {pid})
        try
            close every window of proc
        end try
    end repeat
end tell"#,
            pid = daemon_pid
        );
        if let Err(e) = Command::new("osascript").arg("-e").arg(&script).status() {
            eprintln!("strangetimer-daemon: osascript failed: {e}");
        }
    }
    #[cfg(target_os = "windows")]
    {
        // taskkill kills every RUNNING process except the daemon.
        let filter_pid = format!("PID ne {daemon_pid}");
        let res = Command::new("taskkill")
            .args(["/F", "/FI"])
            .arg(&filter_pid)
            .args(["/FI", "STATUS eq RUNNING"])
            .status();
        if let Err(e) = res {
            eprintln!("strangetimer-daemon: taskkill failed: {e}");
        }
    }
}

/// Linux (X11): prefer `wmctrl`, fall back to `xdotool`.
#[cfg(target_os = "linux")]
fn close_linux(daemon_pid: u32) -> anyhow::Result<()> {
    if wmctrl_available() {
        close_wmctrl(daemon_pid)
    } else if xdotool_available() {
        close_xdotool(daemon_pid)
    } else {
        eprintln!(
            "strangetimer-daemon: neither wmctrl nor xdotool found; \
             cannot close windows. Install one of them to use close_windows."
        );
        Ok(())
    }
}

#[cfg(target_os = "linux")]
fn wmctrl_available() -> bool {
    Command::new("wmctrl")
        .arg("-m")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[cfg(target_os = "linux")]
fn xdotool_available() -> bool {
    Command::new("xdotool")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Use `wmctrl -lp` (window id + PID per line) and close every window whose
/// PID does not match the daemon, so the timer's own terminal survives.
#[cfg(target_os = "linux")]
fn close_wmctrl(daemon_pid: u32) -> anyhow::Result<()> {
    let out = Command::new("wmctrl").arg("-lp").output()?;
    if !out.status.success() {
        anyhow::bail!("wmctrl -lp exited with {}", out.status);
    }
    let text = String::from_utf8_lossy(&out.stdout);
    for line in text.lines() {
        let mut fields = line.split_whitespace();
        let id = fields.next().unwrap_or("");
        let pid = fields.next().and_then(|p| p.parse::<u32>().ok());
        if id.is_empty() || id == "0x0" || id == "0x00000000" {
            continue;
        }
        if pid == Some(daemon_pid) {
            continue; // keep the terminal that runs the timer
        }
        let _ = Command::new("wmctrl").args(["-ic", id]).status();
    }
    Ok(())
}

/// Fallback: `xdotool search` for visible windows and close each one whose
/// PID does not match the daemon.
#[cfg(target_os = "linux")]
fn close_xdotool(daemon_pid: u32) -> anyhow::Result<()> {
    let out = Command::new("xdotool")
        .args(["search", "--onlyvisible", "--name", ""])
        .output()?;
    if !out.status.success() {
        anyhow::bail!("xdotool search exited with {}", out.status);
    }
    let text = String::from_utf8_lossy(&out.stdout);
    for id in text.lines().map(str::trim).filter(|s| !s.is_empty()) {
        let pid = Command::new("xdotool")
            .args(["getwindowpid", id])
            .output()
            .ok()
            .and_then(|o| String::from_utf8_lossy(&o.stdout).trim().parse::<u32>().ok());
        if pid == Some(daemon_pid) {
            continue;
        }
        let _ = Command::new("xdotool").args(["windowclose", id]).status();
    }
    Ok(())
}
