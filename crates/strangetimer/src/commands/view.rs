use std::io::Write;

use anyhow::{Result, anyhow};
use chrono::{DateTime, Duration, Local};
use crossterm::cursor::{Hide, MoveTo, Show};
use crossterm::event::{Event, KeyEvent, poll, read};
use crossterm::queue;
use crossterm::style::Print;
use crossterm::terminal::{
    Clear, ClearType, disable_raw_mode, enable_raw_mode,
};
use strangetimer_core::ipc::{ClientMessage, ServerMessage};
use strangetimer_core::model::{BuzzerRef, RepeatMode, Timer, TimerRun, TimerStatus};

use crate::commands::send_and_receive;

/// Characters the animated cursor cycles through (roughly one per 300ms
/// frame, so the full cycle takes ~1 second).
const CURSOR_CYCLE: [char; 3] = ['▂', '▄', '▆'];
/// Static placeholder used when not animating (also by unit tests).
#[cfg_attr(not(test), allow(dead_code))]
const CURSOR_STATIC: char = '▄';

/// `strangetimer view timers` — live-animated overview of every active run.
/// Falls back to a static snapshot when stdout is not a terminal.
pub fn view_timers() -> Result<()> {
    let (timers, runs) = fetch_snapshot()?;

    let active_runs = active_runs(&runs);
    if active_runs.is_empty() {
        println!("No timers currently running.");
        return Ok(());
    }

    let width = terminal_width();

    if is_tty() {
        animate(
            |frame| {
                render_overview(&timers, &active_runs, Local::now(), width, CURSOR_CYCLE[frame % 3])
            },
            "view timers",
        )
    } else {
        println!(
            "{}",
            render_overview(&timers, &active_runs, Local::now(), width, CURSOR_STATIC)
        );
        Ok(())
    }
}

/// `strangetimer view <name>` — single-timer progress block + buzzer table.
pub fn view_timer(name: &str) -> Result<()> {
    let response = send_and_receive(&ClientMessage::GetTimer {
        name: name.to_string(),
    })?;

    let (timer, runs) = match response {
        ServerMessage::TimerDetail { timer, runs } => (timer, runs),
        ServerMessage::Error(e) => return Err(anyhow!(e)),
        other => return Err(anyhow!("unexpected daemon response: {other:?}")),
    };

    let active = active_runs(&runs);
    if active.is_empty() {
        println!("Timer {name:?} has no active run.");
        println!();
        print!("{}", print_buzzer_table(&timer));
        return Ok(());
    }

    let width = terminal_width();

    if is_tty() {
        animate(
            |frame| {
                let mut out = String::new();
                out.push_str(&render_block(&timer, &active[0], Local::now(), width, CURSOR_CYCLE[frame % 3]));
                out.push('\n');
                out.push_str(&print_buzzer_table(&timer));
                out
            },
            &format!("view {name}"),
        )
    } else {
        println!(
            "{}",
            render_block(&timer, &active[0], Local::now(), width, CURSOR_STATIC)
        );
        println!();
        print!("{}", print_buzzer_table(&timer));
        Ok(())
    }
}

/// Fetch the daemon's timer list + live runs once. All animation motion is
/// computed locally from this snapshot (never re-fetched per frame).
fn fetch_snapshot() -> Result<(Vec<Timer>, Vec<TimerRun>)> {
    match send_and_receive(&ClientMessage::GetTimers)? {
        ServerMessage::TimerList { timers, runs } => Ok((timers, runs)),
        ServerMessage::Error(e) => Err(anyhow!(e)),
        other => Err(anyhow!("unexpected daemon response: {other:?}")),
    }
}

/// Runs worth displaying: everything except completed terminal states.
fn active_runs(runs: &[TimerRun]) -> Vec<TimerRun> {
    runs.iter()
        .filter(|r| r.status != TimerStatus::Completed)
        .cloned()
        .collect()
}

/// Render the full `view timers` output for a set of runs (with blank lines
/// between blocks). Pure — called by both static and animated paths.
fn render_overview(
    timers: &[Timer],
    runs: &[TimerRun],
    now: DateTime<Local>,
    width: usize,
    cursor: char,
) -> String {
    let mut out = String::new();
    for run in runs {
        let Some(timer) = timers.iter().find(|t| t.name == run.timer_name) else {
            continue;
        };
        out.push_str(&render_block(timer, run, now, width, cursor));
        out.push('\n');
    }
    while out.ends_with('\n') {
        out.pop();
    }
    out
}

/// Render a single timer's progress block:
///
/// ```text
/// <name>  Start: <datetime>  End: <datetime>  Mult: <n>
/// Next: <buzzer_name>  <remaining>
/// X-<bar>-X
/// ```
fn render_block(
    timer: &Timer,
    run: &TimerRun,
    now: DateTime<Local>,
    width: usize,
    cursor: char,
) -> String {
    let total = total_duration(timer);
    let elapsed = effective_elapsed(run, now, total);

    let mult = match run.repetitions {
        RepeatMode::Count(n) => n.to_string(),
        RepeatMode::Infinite => "∞".to_string(),
    };

    let start = run.started_at.format("%Y-%m-%d %H:%M:%S");
    let end = (run.started_at + total).format("%Y-%m-%d %H:%M:%S");

    let mut out = format!(
        "{name}  Start: {start}  End: {end}  Mult: {mult}\n",
        name = timer.name,
        start = start,
        end = end,
        mult = mult,
    );

    match next_buzzer(timer, run, now) {
        Some((buzzer, fire_time)) => {
            let remaining = fire_time - now;
            out.push_str(&format!(
                "Next: {buzzer}  {remaining}\n",
                buzzer = buzzer,
                remaining = fmt_remaining(remaining),
            ));
        }
        None => out.push_str("Next: —\n"),
    }

    out.push_str(&format!(
        "X-{}-X",
        render_bar(total, elapsed, &timer.buzzers, width, cursor)
    ));
    out
}

/// Print the buzzer countdown table for repetition 1:
///
/// ```text
///  Buzzer Name       Offset     Time Remaining
///  ─────────────────────────────────────────────
///  paymentBuzzer     1W         6D 22:41:07
/// ```
fn print_buzzer_table(timer: &Timer) -> String {
    let mut out = String::new();
    out.push_str(" Buzzer Name       Offset     Time Remaining\n");
    out.push_str(&format!(" {}\n", "─".repeat(45)));
    for buzzer_ref in &timer.buzzers {
        out.push_str(&format!(
            " {:<17} {:<10} {}\n",
            buzzer_ref.buzzer_name,
            fmt_offset(buzzer_ref.offset),
            fmt_remaining(buzzer_ref.offset),
        ));
    }
    out
}

/// Total duration of one repetition: the largest buzzer offset.
fn total_duration(timer: &Timer) -> Duration {
    timer
        .buzzers
        .iter()
        .map(|b| b.offset)
        .max()
        .unwrap_or(Duration::zero())
}

/// Elapsed time on the run's own timeline (pause-shifted). Clamped to
/// `[0, total]` so progress bars never overflow.
fn effective_elapsed(run: &TimerRun, now: DateTime<Local>, total: Duration) -> Duration {
    let elapsed = match run.status {
        TimerStatus::Running => (now - run.started_at) - run.elapsed_before_pause,
        TimerStatus::Paused => {
            (run.paused_at.unwrap_or(now) - run.started_at) - run.elapsed_before_pause
        }
        TimerStatus::Scheduled => Duration::zero(),
        TimerStatus::Completed => total,
    };
    elapsed.clamp(Duration::zero(), total.max(Duration::zero()))
}

/// The earliest unfired buzzer of the current repetition, plus its absolute
/// fire time. `None` when every buzzer of the repetition has fired.
fn next_buzzer(
    timer: &Timer,
    run: &TimerRun,
    _now: DateTime<Local>,
) -> Option<(String, DateTime<Local>)> {
    let mut earliest: Option<(usize, DateTime<Local>)> = None;
    for (idx, buzzer_ref) in timer.buzzers.iter().enumerate() {
        if run.fired_indices.contains(&idx) {
            continue;
        }
        let fire_time = run.started_at + run.elapsed_before_pause + buzzer_ref.offset;
        match earliest {
            Some((_, t)) if t <= fire_time => {}
            _ => earliest = Some((idx, fire_time)),
        }
    }
    earliest.map(|(idx, fire_time)| (timer.buzzers[idx].buzzer_name.clone(), fire_time))
}

/// Render the progress bar. Each cell is a proportional slice of `total`;
/// `▓` marks buzzer positions, `cursor` marks the current position.
fn render_bar(
    total: Duration,
    elapsed: Duration,
    buzzers: &[BuzzerRef],
    width: usize,
    cursor: char,
) -> String {
    let width = width.max(1);
    let mut cells = vec!['█'; width];

    let frac = |d: Duration| {
        let total_ms = total.num_milliseconds().max(1);
        (d.num_milliseconds() as f64 / total_ms as f64).clamp(0.0, 1.0)
    };

    for buzzer_ref in buzzers {
        let idx = (frac(buzzer_ref.offset) * (width - 1) as f64).round() as usize;
        cells[idx] = '▓';
    }

    let cursor_idx = (frac(elapsed) * (width - 1) as f64).round() as usize;
    cells[cursor_idx] = cursor;

    cells.into_iter().collect()
}

/// Format a duration as `HH:MM:SS`, or `XD HH:MM:SS` from one day onward.
fn fmt_remaining(d: Duration) -> String {
    let secs = d.num_seconds().max(0);
    let days = secs / 86400;
    let h = (secs % 86400) / 3600;
    let m = (secs % 3600) / 60;
    let s = secs % 60;
    if days > 0 {
        format!("{days}D {h:02}:{m:02}:{s:02}")
    } else {
        format!("{h:02}:{m:02}:{s:02}")
    }
}

/// Compact offset display for the table (mirrors the CLI input grammar):
/// whole units only — `30s`, `5m`, `2h`, `1D`, `1W`.
fn fmt_offset(d: Duration) -> String {
    let secs = d.num_seconds();
    if secs >= 7 * 86400 && secs % (7 * 86400) == 0 {
        format!("{}W", secs / (7 * 86400))
    } else if secs >= 86400 && secs % 86400 == 0 {
        format!("{}D", secs / 86400)
    } else if secs >= 3600 && secs % 3600 == 0 {
        format!("{}h", secs / 3600)
    } else if secs >= 60 && secs % 60 == 0 {
        format!("{}m", secs / 60)
    } else {
        format!("{secs}s")
    }
}

fn terminal_width() -> usize {
    crossterm::terminal::size()
        .map(|(cols, _)| (cols.saturating_sub(4)).max(10) as usize)
        .unwrap_or(76)
}

/// Whether stdout is attached to a real terminal (animation-capable).
fn is_tty() -> bool {
    use crossterm::tty::IsTty;
    std::io::stdout().is_tty()
}

/// Run a full-screen render loop: re-render `render` every 300ms from a
/// single snapshot, cycling the cursor character, until the user presses any
/// key. Always restores the terminal on exit.
fn animate<F>(render: F, label: &str) -> Result<()>
where
    F: Fn(usize) -> String,
{
    let mut stdout = std::io::stdout();
    let origin = MoveTo(0, 0);

    enable_raw_mode().map_err(|e| anyhow!("failed to enter raw mode: {e}"))?;
    let _restore = TerminalGuard;

    queue!(stdout, Hide, origin, Clear(ClearType::FromCursorDown))?;
    stdout.flush()?;

    let mut frame = 0usize;
    loop {
        queue!(
            stdout,
            origin,
            Clear(ClearType::FromCursorDown),
            Print(render(frame)),
        )?;
        stdout.flush()?;

        // Any key exits; Ctrl+C arrives as a normal keypress in raw mode.
        if poll(std::time::Duration::from_millis(300))? {
            match read()? {
                Event::Key(KeyEvent { .. }) | Event::Resize(_, _) => break,
                _ => {}
            }
        }
        frame += 1;
    }

    // Leave the screen in a clean state.
    queue!(
        stdout,
        origin,
        Clear(ClearType::FromCursorDown),
        Print(format!("{label}: press any key to return\n")),
        Show,
    )?;
    stdout.flush()?;
    drop(_restore);

    Ok(())
}

/// Restore the terminal on any exit path (including panics/errors).
struct TerminalGuard;
impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let mut stdout = std::io::stdout();
        let _ = queue!(stdout, Show);
        let _ = stdout.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use strangetimer_core::model::Timer;

    fn timer(name: &str, offsets_secs: &[i64]) -> Timer {
        Timer {
            name: name.to_string(),
            buzzers: offsets_secs
                .iter()
                .map(|s| BuzzerRef {
                    offset: Duration::seconds(*s),
                    buzzer_name: "default_audio".to_string(),
                })
                .collect(),
            created_at: Local::now(),
        }
    }

    fn running_run(name: &str, started_secs_ago: i64) -> TimerRun {
        TimerRun {
            timer_name: name.to_string(),
            started_at: Local::now() - Duration::seconds(started_secs_ago),
            repetitions: RepeatMode::Count(1),
            current_rep: 0,
            schedule_time: None,
            status: TimerStatus::Running,
            paused_at: None,
            elapsed_before_pause: Duration::zero(),
            fired_indices: vec![],
        }
    }

    #[test]
    fn fmt_remaining_basic() {
        assert_eq!(fmt_remaining(Duration::seconds(872)), "00:14:32");
        assert_eq!(
            fmt_remaining(Duration::days(6) + Duration::hours(22) + Duration::minutes(41) + Duration::seconds(7)),
            "6D 22:41:07"
        );
    }

    #[test]
    fn fmt_offset_units() {
        assert_eq!(fmt_offset(Duration::seconds(30)), "30s");
        assert_eq!(fmt_offset(Duration::minutes(5)), "5m");
        assert_eq!(fmt_offset(Duration::hours(2)), "2h");
        assert_eq!(fmt_offset(Duration::days(1)), "1D");
        assert_eq!(fmt_offset(Duration::weeks(1)), "1W");
    }

    #[test]
    fn bar_has_buzzer_markers() {
        let t = timer("t", &[10, 20]);
        let bar = render_bar(Duration::seconds(20), Duration::seconds(0), &t.buzzers, 10, CURSOR_STATIC);
        assert_eq!(bar.chars().count(), 10);
        assert_eq!(bar.chars().filter(|c| *c == '▓').count(), 2);
    }

    #[test]
    fn bar_cursor_tracks_elapsed() {
        let bar = render_bar(Duration::seconds(100), Duration::seconds(50), &[], 10, '▄');
        // Halfway through a 10-cell bar → position 5 (0-indexed).
        assert_eq!(bar.chars().nth(5), Some('▄'));
    }

    #[test]
    fn elapsed_freezes_while_paused() {
        let now = Local::now();
        let mut run = running_run("t", 30);
        run.status = TimerStatus::Paused;
        run.paused_at = Some(now - Duration::seconds(10));
        let elapsed = effective_elapsed(&run, now, Duration::seconds(100));
        // Allow clock-skew between the two Local::now() calls above.
        assert!((elapsed - Duration::seconds(20)).num_milliseconds().abs() < 1000);
    }

    #[test]
    fn next_buzzer_is_earliest_unfired() {
        let t = timer("t", &[10, 20, 30]);
        let mut run = running_run("t", 12);
        // 10s buzzer already fired; the 20s one is next.
        run.fired_indices = vec![0];
        let (name, fire_time) = next_buzzer(&t, &run, Local::now()).unwrap();
        assert_eq!(name, "default_audio");
        assert!((fire_time - Local::now()).num_seconds().abs() <= 8);
    }

    #[test]
    fn completed_run_shows_full_bar() {
        let t = timer("t", &[60]);
        let mut run = running_run("t", 60);
        run.status = TimerStatus::Completed;
        let block = render_block(&t, &run, Local::now(), 10, CURSOR_STATIC);
        // Cursor sits at the very end of a completed run.
        assert!(block.contains("X-█████████▄-X"));
    }
}
