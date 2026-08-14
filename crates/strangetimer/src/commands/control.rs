use anyhow::{anyhow, Result};
use chrono::{DateTime, Local, TimeZone};
use strangetimer_core::ipc::{ClientMessage, ServerMessage};
use strangetimer_core::model::{FocusSpec, RepeatMode};

use crate::cli::RunArgs;
use crate::commands::{ensure_ok, send_and_receive};
use crate::style;

/// `strangetimer run <name> [-n count | -i] [-t HH:MM] [-u]`
pub fn run(args: &RunArgs) -> Result<()> {
    let repeat = if args.infinite {
        RepeatMode::Infinite
    } else {
        RepeatMode::Count(args.count.unwrap_or(1).max(1))
    };

    let schedule_time = match &args.schedule_time {
        Some(t) => Some(parse_schedule_time(t)?),
        None => None,
    };

    // In user-interrupt mode the terminal window is captured now, so the
    // daemon can bring it back to the foreground when a buzzer fires.
    let interrupt_focus = if args.user_interrupt {
        capture_active_window()
    } else {
        None
    };

    let response = send_and_receive(&ClientMessage::RunTimer {
        name: args.name.clone(),
        repeat,
        schedule_time,
        user_interrupt: args.user_interrupt,
        interrupt_focus,
    })?;

    match response {
        ServerMessage::Ok => {
            match schedule_time {
                Some(t) => println!(
                    "Timer {} scheduled for {}.",
                    style::name(&args.name),
                    t.format("%Y-%m-%d %H:%M:%S")
                ),
                None => println!("Timer {} started.", style::name(&args.name)),
            }
            if args.user_interrupt {
                // Non-blocking: the CLI returns immediately. The daemon
                // pauses the run and loops audio at every buzzer until the
                // acknowledgement arrives via `strangetimer resume`.
                println!(
                    "{}",
                    style::dim(
                        "user-interrupt on: the timer pauses at every buzzer until acknowledged"
                    )
                );
                println!(
                    "{}",
                    style::prompt(&format!(
                        "When it pauses, acknowledge with: strangetimer resume {}",
                        args.name
                    ))
                );
            }
            Ok(())
        }
        ServerMessage::Error(e) => Err(anyhow!(e)),
        other => Err(anyhow!("unexpected daemon response: {other:?}")),
    }
}

/// External-tool override for tests (mirrors the daemon's
/// `STRANGETIMER_TEST_<NAME>` seam).
fn tool(name: &str) -> String {
    let key = format!("STRANGETIMER_TEST_{}", name.to_uppercase());
    std::env::var(&key).unwrap_or_else(|_| name.to_string())
}

/// Capture the active terminal window so the daemon can focus it after
/// buzzer actions. Best-effort: `None` when no tooling is available (e.g.
/// headless CI). The result is a JSON [`FocusSpec`] stored in the run's
/// `interrupt_focus` field.
fn capture_active_window() -> Option<String> {
    #[cfg(target_os = "linux")]
    {
        let wayland = std::env::var("XDG_SESSION_TYPE")
            .map(|t| t.eq_ignore_ascii_case("wayland"))
            .unwrap_or(false);

        // Prefer the X11 window id — titles change; ids do not.
        let window_id = std::process::Command::new(tool("xdotool"))
            .arg("getactivewindow")
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .filter(|s| !s.is_empty());
        let title = window_id.as_ref().and_then(|_| {
            std::process::Command::new(tool("xdotool"))
                .arg("getwindowname")
                .output()
                .ok()
                .filter(|o| o.status.success())
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                .filter(|s| !s.is_empty())
        });

        if window_id.is_none() && title.is_none() && !wayland {
            return None;
        }

        let spec = FocusSpec {
            window_id,
            title,
            display: std::env::var("DISPLAY").ok(),
            xauthority: std::env::var("XAUTHORITY").ok(),
            wayland,
        };
        Some(spec.encode())
    }
    #[cfg(target_os = "macos")]
    {
        let out = std::process::Command::new(tool("osascript"))
            .args([
                "-e",
                "tell application \"System Events\" to get name of first process \
                 whose frontmost is true",
            ])
            .output()
            .ok()?;
        let app = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if app.is_empty() {
            None
        } else {
            let spec = FocusSpec {
                title: Some(app),
                ..FocusSpec::default()
            };
            Some(spec.encode())
        }
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        None
    }
}

/// `strangetimer pause <name>`
pub fn pause(name: &str) -> Result<()> {
    ensure_ok(send_and_receive(&ClientMessage::Pause {
        name: name.to_string(),
    })?)?;
    println!("Paused timer {}.", style::name(name));
    Ok(())
}

/// `strangetimer pauseall`
pub fn pause_all() -> Result<()> {
    ensure_ok(send_and_receive(&ClientMessage::PauseAll)?)?;
    println!("Paused all timers.");
    Ok(())
}

/// `strangetimer resume <name>`
pub fn resume(name: &str) -> Result<()> {
    ensure_ok(send_and_receive(&ClientMessage::Resume {
        name: name.to_string(),
    })?)?;
    println!("Resumed timer {}.", style::name(name));
    Ok(())
}

/// `strangetimer stop <name>`
pub fn stop(name: &str) -> Result<()> {
    ensure_ok(send_and_receive(&ClientMessage::Stop {
        name: name.to_string(),
    })?)?;
    println!("Stopped timer {}.", style::name(name));
    Ok(())
}

/// `strangetimer stopall`
pub fn stop_all() -> Result<()> {
    ensure_ok(send_and_receive(&ClientMessage::StopAll)?)?;
    println!("Stopped all timers.");
    Ok(())
}

/// `strangetimer confirm-destructive`
pub fn confirm_destructive() -> Result<()> {
    ensure_ok(send_and_receive(&ClientMessage::ConfirmDestructive)?)?;
    println!("close_windows buzzer enabled. Use with care!");
    Ok(())
}

/// Parse a 24h clock time (`HH:MM`) into today's local date-time. If that
/// moment has already passed, the schedule rolls over to tomorrow.
fn parse_schedule_time(s: &str) -> Result<DateTime<Local>> {
    let naive_time = chrono::NaiveTime::parse_from_str(s, "%H:%M")
        .map_err(|_| anyhow!("invalid time {s:?}: expected HH:MM (24h clock)"))?;

    let today = Local::now().date_naive();
    let naive_dt = today.and_time(naive_time);
    let mut dt = Local
        .from_local_datetime(&naive_dt)
        .earliest()
        .ok_or_else(|| anyhow!("cannot resolve local time {s:?}"))?;

    if dt <= Local::now() {
        dt += chrono::Duration::days(1);
    }
    Ok(dt)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_valid_time() {
        let t = parse_schedule_time("09:30").unwrap();
        assert_eq!(t.format("%H:%M").to_string(), "09:30");
        assert!(t > Local::now() - chrono::Duration::hours(24));
    }

    #[test]
    fn past_time_rolls_to_tomorrow() {
        let now = Local::now();
        let past = now - chrono::Duration::hours(2);
        let s = past.format("%H:%M").to_string();
        let t = parse_schedule_time(&s).unwrap();
        assert!(t > now);
        assert!(t - now < chrono::Duration::hours(26));
    }

    #[test]
    fn rejects_garbage() {
        assert!(parse_schedule_time("banana").is_err());
        assert!(parse_schedule_time("25:00").is_err());
    }
}
