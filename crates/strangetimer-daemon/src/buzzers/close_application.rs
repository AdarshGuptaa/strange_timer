use std::process::Command;

/// Close a single application by process name, e.g. `firefox`.
///
/// This is a destructive action — `buzzers::dispatch` only calls it after
/// the user has opted in via `strangetimer confirm-destructive`.
pub fn fire_close_application(name: &str) {
    #[cfg(target_os = "linux")]
    {
        if let Err(e) = close_linux(name) {
            eprintln!("strangetimer-daemon: close_app {name:?} failed: {e}");
        }
    }
    #[cfg(target_os = "macos")]
    {
        // Prefer a graceful `quit`; force-kill only if nothing to quit.
        let script = format!("tell application \"{name}\" to quit");
        let quit_ok = Command::new("osascript")
            .arg("-e")
            .arg(&script)
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if quit_ok {
            return;
        }
        if let Err(e) = pkill_exact(name) {
            eprintln!("strangetimer-daemon: close_app {name:?} failed: {e}");
        }
    }
    #[cfg(target_os = "windows")]
    {
        // taskkill by image name; try a graceful stop first, force as fallback.
        let res = Command::new("taskkill").args(["/IM", name]).status();
        match res {
            Ok(_) => {}
            Err(_) => {
                let _ = Command::new("taskkill").args(["/F", "/IM", name]).status();
            }
        }
    }
}

/// Linux: exact process-name match (`pkill -x`), falling back to a full
/// command-line match (`pkill -f`) for names that are not the process name.
#[cfg(target_os = "linux")]
fn close_linux(name: &str) -> anyhow::Result<()> {
    let exact = Command::new("pkill").args(["-x", name]).status()?;
    if exact.success() {
        return Ok(());
    }
    let full = Command::new("pkill").args(["-f", name]).status()?;
    if full.success() {
        Ok(())
    } else {
        eprintln!("strangetimer-daemon: no process matching {name:?} was running");
        Ok(())
    }
}

#[cfg(target_os = "macos")]
fn pkill_exact(name: &str) -> anyhow::Result<()> {
    Command::new("pkill").args(["-x", name]).status()?;
    Ok(())
}
