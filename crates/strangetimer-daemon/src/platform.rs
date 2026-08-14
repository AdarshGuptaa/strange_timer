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
        std::fs::create_dir_all(&unit_dir).with_context(|| {
            format!("failed to create systemd user dir {}", unit_dir.display())
        })?;

        let unit_path = unit_dir.join("strangetimer.service");
        let unit = format!(
            "[Unit]\n\
             Description=StrangeTimer Daemon\n\
             After=network.target\n\
             \n\
             [Service]\n\
             ExecStart={}\n\
             Restart=on-failure\n\
             \n\
             [Install]\n\
             WantedBy=default.target\n",
            daemon_path.display()
        );
        std::fs::write(&unit_path, unit)
            .with_context(|| format!("failed to write {}", unit_path.display()))?;

        run("systemctl", &["--user", "daemon-reload"])?;
        run("systemctl", &["--user", "enable", "--now", "strangetimer"])?;
        Ok(())
    }

    #[cfg(target_os = "macos")]
    {
        let launch_dir = home_dir().context("cannot resolve home dir")?.join("Library").join("LaunchAgents");
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

        run("launchctl", &["load", plist_path.to_str().unwrap_or_default()])?;
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

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
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
