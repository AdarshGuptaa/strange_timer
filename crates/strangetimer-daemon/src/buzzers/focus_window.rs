use std::process::Command;

/// Bring a window matching `name` (title substring or application name) to
/// the foreground. Non-destructive: never closes anything, only activates.
pub fn fire_focus_window(name: &str) {
    #[cfg(target_os = "linux")]
    {
        if let Err(e) = focus_linux(name) {
            warn!("focus_window {name:?} failed: {e}");
        }
    }
    #[cfg(target_os = "macos")]
    {
        let script = format!("tell application \"{name}\" to activate");
        if let Err(e) = Command::new("osascript").arg("-e").arg(&script).status() {
            warn!("focus_window {name:?} failed: {e}");
        }
    }
    #[cfg(target_os = "windows")]
    {
        let script = format!("(New-Object -ComObject WScript.Shell).AppActivate('{name}')");
        if let Err(e) = Command::new("powershell")
            .args(["-NoProfile", "-Command"])
            .arg(&script)
            .status()
        {
            warn!("focus_window {name:?} failed: {e}");
        }
    }
}

/// Linux (X11): prefer `wmctrl -a` (search by title/name), fall back to
/// `xdotool search --name ... windowactivate`.
#[cfg(target_os = "linux")]
fn focus_linux(name: &str) -> anyhow::Result<()> {
    let wmctrl = Command::new("wmctrl").args(["-a", name]).status();
    if let Ok(status) = wmctrl {
        if status.success() {
            return Ok(());
        }
    }

    let out = Command::new("xdotool")
        .args(["search", "--name", name])
        .output()?;
    if !out.status.success() {
        warn!(
            "no window matching {name:?} found \
             (wmctrl and xdotool both failed)"
        );
        return Ok(());
    }
    for id in String::from_utf8_lossy(&out.stdout).lines().map(str::trim) {
        if !id.is_empty() {
            let _ = Command::new("xdotool")
                .args(["windowactivate", id])
                .status();
        }
    }
    Ok(())
}
