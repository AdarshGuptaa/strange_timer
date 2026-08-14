use std::process::Command;

/// Close a *selected* window by X11 window id or window title.
///
/// This is a targeted destructive action — `buzzers::dispatch` only calls
/// it after the user opted in via `strangetimer confirm-destructive`.
///
/// Linux (X11): `wmctrl -i -c <id>` when the target is an id, otherwise
/// `wmctrl -c <title>`; falls back to `xdotool windowclose <id>` /
/// `xdotool search --name <title> windowclose`. Wayland cannot be
/// controlled generically and is reported as unsupported. Windows has no
/// safe generic window-close primitive — use `--close-app` there.
pub fn fire_close_window(target: &str) {
    #[cfg(target_os = "linux")]
    {
        if let Err(e) = close_linux(target) {
            warn!("close_window {target:?} failed: {e}");
        }
    }
    #[cfg(target_os = "macos")]
    {
        // AppleScript: close the first window of the named application.
        let script = format!(
            "tell application \"System Events\"\n\
                 try\n\
                   close (first window of first process whose name contains \"{target}\")\n\
                 end try\n\
             end tell"
        );
        if let Err(e) = Command::new(crate::platform::tool("osascript"))
            .arg("-e")
            .arg(&script)
            .status()
        {
            warn!("close_window {target:?} failed: {e}");
        }
    }
    #[cfg(target_os = "windows")]
    {
        warn!(
            "close_window is not supported on Windows — use `--close-app <name>` instead \
             (target {target:?})"
        );
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        warn!("close_window is not supported on this platform (target {target:?})");
    }
}

#[cfg(target_os = "linux")]
fn close_linux(target: &str) -> anyhow::Result<()> {
    if std::env::var("XDG_SESSION_TYPE")
        .map(|t| t.eq_ignore_ascii_case("wayland"))
        .unwrap_or(false)
    {
        warn!(
            "close_window: Wayland session — X11 window tools cannot close \
             native windows; closing {target:?} was skipped. Use `--close-app` \
             for process-level closing."
        );
        return Ok(());
    }

    // A hex window id (0x...) closes directly by id; anything else is a
    // title, searched first.
    let is_id = target.starts_with("0x");
    if is_id {
        let mut cmd = Command::new(crate::platform::tool("wmctrl"));
        cmd.args(["-i", "-c"]).arg(target);
        if let Ok(status) = cmd.status() {
            if status.success() {
                return Ok(());
            }
        }
        let mut cmd = Command::new(crate::platform::tool("xdotool"));
        cmd.args(["windowclose"]).arg(target);
        if let Ok(status) = cmd.status() {
            if status.success() {
                return Ok(());
            }
        }
        warn!("close_window: failed to close window id {target:?}");
        return Ok(());
    }

    // Title target: wmctrl -c searches by title directly.
    let mut cmd = Command::new(crate::platform::tool("wmctrl"));
    cmd.args(["-c"]).arg(target);
    if let Ok(status) = cmd.status() {
        if status.success() {
            return Ok(());
        }
    }

    // Fallback: search for windows whose name matches, then close them.
    let out = Command::new(crate::platform::tool("xdotool"))
        .args(["search", "--name", target])
        .output()?;
    if out.status.success() {
        for id in String::from_utf8_lossy(&out.stdout).lines().map(str::trim) {
            if !id.is_empty() {
                let _ = Command::new(crate::platform::tool("xdotool"))
                    .args(["windowclose", id])
                    .status();
            }
        }
    } else {
        warn!(
            "close_window: no window matching {target:?} found \
             (wmctrl and xdotool both failed)"
        );
    }
    Ok(())
}
