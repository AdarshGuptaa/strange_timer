//! Platform-specific integrations: media-player window focus and OS
//! autostart registration.

#[cfg(target_os = "macos")]
use std::path::PathBuf;
use std::process::Command;

use anyhow::{Context, Result};

/// Bring the media player window into focus after a buzzer launches one.
///
/// TODO: implement per-OS — `xdotool`/wmctrl on X11, `osascript` on macOS,
/// `SetForegroundWindow` (winapi) on Windows. The `open` crate already gives
/// us default-player launching; explicit focus is a follow-up enhancement.
pub fn focus_media_window() {
    // Intentionally a no-op for now (see TODO above).
}

/// Open `target` (a file path or URL) with the OS default handler.
///
/// Test seam: when `STRANGETIMER_TEST_OPENER` is set, that binary is run
/// with the target as its single argument instead of the system opener —
/// tests use this to record/verify opens without launching a GUI.
pub fn open_target(target: &str) {
    if let Some(override_path) = std::env::var_os("STRANGETIMER_TEST_OPENER") {
        let _ = Command::new(override_path).arg(target).status();
        return;
    }
    if let Err(e) = open::that(target) {
        warn!("failed to open {target:?}: {e}");
    }
}

/// Resolve an external tool name, honouring a test override:
/// `STRANGETIMER_TEST_<NAME>` (e.g. `STRANGETIMER_TEST_PKILL`) points at a
/// recording script instead of the real binary, so e2e tests can verify
/// command construction without touching the desktop.
pub fn tool(name: &str) -> String {
    let key = format!("STRANGETIMER_TEST_{}", name.to_uppercase());
    std::env::var(&key).unwrap_or_else(|_| name.to_string())
}

/// Register the daemon binary as an OS autostart service so it comes back up
/// after a reboot and can resume persisted runs.
pub fn register_autostart() -> Result<()> {
    let daemon_path = std::env::current_exe().context("cannot resolve daemon binary path")?;

    #[cfg(target_os = "linux")]
    {
        let unit_dir = dirs::config_dir()
            .context("cannot resolve config dir")?
            .join("systemd")
            .join("user");
        std::fs::create_dir_all(&unit_dir)
            .with_context(|| format!("failed to create systemd user dir {}", unit_dir.display()))?;

        let unit_path = unit_dir.join("strangetimer.service");
        // Carry the interactive session's GUI environment into the unit so
        // buzzer actions (window focus, media playback) work even when the
        // service starts outside the session.
        let mut env_lines = String::new();
        for var in [
            "DISPLAY",
            "WAYLAND_DISPLAY",
            "XAUTHORITY",
            "DBUS_SESSION_BUS_ADDRESS",
            "XDG_RUNTIME_DIR",
        ] {
            if let Ok(value) = std::env::var(var) {
                if !value.is_empty() {
                    env_lines.push_str(&format!("Environment={var}={value}\n"));
                }
            }
        }
        let unit = format!(
            "[Unit]\n\
             Description=StrangeTimer Daemon\n\
             After=network.target\n\
             \n\
             [Service]\n\
             ExecStart={}\n\
             Restart=on-failure\n\
             {env_lines}\
             [Install]\n\
             WantedBy=default.target\n",
            daemon_path.display()
        );
        std::fs::write(&unit_path, unit)
            .with_context(|| format!("failed to write {}", unit_path.display()))?;

        run("systemctl", &["--user", "daemon-reload"])?;
        // Enable only — NOT `--now`. Starting is the job of the CLI's
        // `strangetimer daemon start`, which prefers the systemd service
        // once the unit exists. Starting here would race the CLI-spawned
        // daemon for the IPC socket.
        run("systemctl", &["--user", "enable", "strangetimer"])?;
        Ok(())
    }

    #[cfg(target_os = "macos")]
    {
        let launch_dir = home_dir()
            .context("cannot resolve home dir")?
            .join("Library")
            .join("LaunchAgents");
        std::fs::create_dir_all(&launch_dir)
            .with_context(|| format!("failed to create {}", launch_dir.display()))?;

        let plist_path = launch_dir.join("com.strangetimer.daemon.plist");
        let plist = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
             <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \
             \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
             <plist version=\"1.0\">\n\
             <dict>\n\
             \t<key>Label</key>\n\
             \t<string>com.strangetimer.daemon</string>\n\
             \t<key>ProgramArguments</key>\n\
             \t<array>\n\
             \t\t<string>{}</string>\n\
             \t</array>\n\
             \t<key>RunAtLoad</key>\n\
             \t<true/>\n\
             \t<key>KeepAlive</key>\n\
             \t<true/>\n\
             </dict>\n\
             </plist>\n",
            daemon_path.display()
        );
        std::fs::write(&plist_path, plist)
            .with_context(|| format!("failed to write {}", plist_path.display()))?;

        // Write the plist only — do NOT `launchctl load` here. Loading
        // starts the job (RunAtLoad), racing the already-running daemon for
        // the socket. `strangetimer daemon start` starts it via
        // `launchctl kickstart` once the plist exists.
        Ok(())
    }

    #[cfg(target_os = "windows")]
    {
        run(
            "schtasks",
            &[
                "/Create",
                "/TN",
                "StrangeTimerDaemon",
                "/SC",
                "ONLOGON",
                "/TR",
                &daemon_path.to_string_lossy(),
                "/RL",
                "HIGHEST",
                "/F",
            ],
        )?;
        Ok(())
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        Err(anyhow::anyhow!(
            "autostart registration is not supported on this platform"
        ))
    }
}

#[cfg(target_os = "macos")]
fn home_dir() -> Option<PathBuf> {
    dirs::home_dir()
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
fn run(program: &str, args: &[&str]) -> Result<()> {
    let status = Command::new(program)
        .args(args)
        .status()
        .with_context(|| format!("failed to run {program}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(anyhow::anyhow!("{program} exited with {}", status))
    }
}
