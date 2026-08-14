use anyhow::{Result, anyhow};
use chrono::{DateTime, Local, TimeZone};
use strangetimer_core::ipc::{ClientMessage, ServerMessage};
use strangetimer_core::model::RepeatMode;

use crate::cli::RunArgs;
use crate::commands::{ensure_ok, send_and_receive};

/// `strangetimer run <name> [-n count | -i] [-t HH:MM]`
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

    let response = send_and_receive(&ClientMessage::RunTimer {
        name: args.name.clone(),
        repeat,
        schedule_time,
    })?;

    match response {
        ServerMessage::Ok => {
            match schedule_time {
                Some(t) => println!(
                    "Timer {:?} scheduled for {}.",
                    args.name,
                    t.format("%Y-%m-%d %H:%M:%S")
                ),
                None => println!("Timer {:?} started.", args.name),
            }
            Ok(())
        }
        ServerMessage::Error(e) => Err(anyhow!(e)),
        other => Err(anyhow!("unexpected daemon response: {other:?}")),
    }
}

/// `strangetimer pause <name>`
pub fn pause(name: &str) -> Result<()> {
    ensure_ok(send_and_receive(&ClientMessage::Pause {
        name: name.to_string(),
    })?)?;
    println!("Paused timer {name:?}.");
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
    println!("Resumed timer {name:?}.");
    Ok(())
}

/// `strangetimer stop <name>`
pub fn stop(name: &str) -> Result<()> {
    ensure_ok(send_and_receive(&ClientMessage::Stop {
        name: name.to_string(),
    })?)?;
    println!("Stopped timer {name:?}.");
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
