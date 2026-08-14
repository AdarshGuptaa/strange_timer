use std::process::Command;

use strangetimer_core::model::FocusSpec;

/// Bring a window matching `target` to the foreground. `target` is either
/// a JSON [`FocusSpec`] (captured at `run -u` time) or a legacy plain
/// title. Non-destructive: never closes anything, only activates.
pub fn fire_focus_window(target: &str) {
    if let Some(spec) = FocusSpec::decode(target) {
        focus_spec(&spec);
    } else {
        focus_title(target);
    }
}

/// Focus with retries: a freshly launched player/browser/app may not have
/// its window ready yet, so a single activation can race it. Called after
/// interrupt buzzer actions; retries are short and never block the
/// scheduler.
pub async fn fire_focus_window_retry(target: &str) {
    for delay_ms in [0u64, 300, 1000] {
        if delay_ms > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
        }
        fire_focus_window(target);
    }
}

/// Focus a structured spec: X11 window id first (with the session's
/// DISPLAY/XAUTHORITY), title fallback; Wayland is reported unsupported
/// rather than silently failing.
fn focus_spec(spec: &FocusSpec) {
    #[cfg(target_os = "linux")]
    {
        if spec.wayland {
            warn!(
                "focus_window: Wayland session — X11 focus tools cannot reliably \
                 activate windows; skipping terminal focus (run the daemon \
                 inside the graphical session for best results)"
            );
            return;
        }
        if let Some(window_id) = &spec.window_id {
            let mut cmd = Command::new(crate::platform::tool("wmctrl"));
            cmd.args(["-i", "-a"]).arg(window_id);
            if let Some(display) = &spec.display {
                cmd.env("DISPLAY", display);
            }
            if let Some(xauth) = &spec.xauthority {
                cmd.env("XAUTHORITY", xauth);
            }
            match cmd.status() {
                Ok(s) if s.success() => return,
                _ => {}
            }
            let mut cmd = Command::new(crate::platform::tool("xdotool"));
            cmd.args(["windowactivate", "--sync"]).arg(window_id);
            if let Some(display) = &spec.display {
                cmd.env("DISPLAY", display);
            }
            if let Some(xauth) = &spec.xauthority {
                cmd.env("XAUTHORITY", xauth);
            }
            if let Ok(s) = cmd.status() {
                if s.success() {
                    return;
                }
            }
        }
        if let Some(title) = &spec.title {
            focus_title(title);
        }
    }
    #[cfg(target_os = "macos")]
    {
        if let Some(title) = &spec.title {
            let script = format!("tell application \"{title}\" to activate");
            if let Err(e) = Command::new(crate::platform::tool("osascript"))
                .arg("-e")
                .arg(&script)
                .status()
            {
                warn!("focus_window {title:?} failed: {e}");
            }
        }
    }
    #[cfg(target_os = "windows")]
    {
        if let Some(title) = &spec.title {
            let script = format!("(New-Object -ComObject WScript.Shell).AppActivate('{title}')");
            if let Err(e) = Command::new(crate::platform::tool("powershell"))
                .args(["-NoProfile", "-Command"])
                .arg(&script)
                .status()
            {
                warn!("focus_window {title:?} failed: {e}");
            }
        }
    }
}

/// Legacy title-only target.
fn focus_title(name: &str) {
    #[cfg(target_os = "linux")]
    {
        if let Err(e) = focus_linux_title(name) {
            warn!("focus_window {name:?} failed: {e}");
        }
    }
    #[cfg(target_os = "macos")]
    {
        let script = format!("tell application \"{name}\" to activate");
        if let Err(e) = Command::new(crate::platform::tool("osascript"))
            .arg("-e")
            .arg(&script)
            .status()
        {
            warn!("focus_window {name:?} failed: {e}");
        }
    }
    #[cfg(target_os = "windows")]
    {
        let script = format!("(New-Object -ComObject WScript.Shell).AppActivate('{name}')");
        if let Err(e) = Command::new(crate::platform::tool("powershell"))
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
fn focus_linux_title(name: &str) -> anyhow::Result<()> {
    let wmctrl = Command::new(crate::platform::tool("wmctrl"))
        .args(["-a", name])
        .status();
    if let Ok(status) = wmctrl {
        if status.success() {
            return Ok(());
        }
    }

    let out = Command::new(crate::platform::tool("xdotool"))
        .args(["search", "--name", name])
        .output()?;
    if !out.status.success() {
        warn!("no window matching {name:?} found (wmctrl and xdotool both failed)");
        return Ok(());
    }
    for id in String::from_utf8_lossy(&out.stdout).lines().map(str::trim) {
        if !id.is_empty() {
            let _ = Command::new(crate::platform::tool("xdotool"))
                .args(["windowactivate", "--sync", id])
                .status();
        }
    }
    Ok(())
}
