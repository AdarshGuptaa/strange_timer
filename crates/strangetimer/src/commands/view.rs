use std::io::Write;

use anyhow::{anyhow, Result};
use chrono::{DateTime, Duration, Local};
use crossterm::cursor::{Hide, MoveTo, Show};
use crossterm::event::{poll, read, Event, KeyEvent};
use crossterm::queue;
use crossterm::style::Print;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen,
};
use strangetimer_core::ipc::{ClientMessage, ServerMessage};
use strangetimer_core::model::{BuzzerRef, RepeatMode, Timer, TimerRun, TimerStatus};

use crate::commands::send_and_receive;

/// Characters the animated cursor cycles through (roughly one per 100ms
/// frame, so the full cycle takes ~0.3s).
const CURSOR_CYCLE: [char; 3] = ['▂', '▄', '▆'];
/// Static placeholder used when not animating (also by unit tests).
#[cfg_attr(not(test), allow(dead_code))]
const CURSOR_STATIC: char = '▄';

/// Below this width the block layout degrades to a one-line summary.
const MIN_BLOCK_WIDTH: usize = 30;
/// The progress bar is capped so a wide terminal doesn't stretch it absurdly.
const MAX_BAR_WIDTH: usize = 40;
/// The bar is hidden entirely below this many available columns.
const MIN_BAR_WIDTH: usize = 8;

/// `strangetimer view timers` — live-animated overview of every active run.
/// Falls back to a static snapshot when stdout is not a terminal.
pub fn view_timers() -> Result<()> {
    let (timers, runs) = fetch_snapshot()?;

    let active_runs = active_runs(&runs);
    if active_runs.is_empty() {
        println!("No timers currently running.");
        return Ok(());
    }

    if is_tty() {
        animate(|frame| {
            let (width, height) = terminal_size();
            render_overview(
                &timers,
                &active_runs,
                Local::now(),
                width,
                height,
                CURSOR_CYCLE[frame % 3],
            )
        })
    } else {
        let (width, _) = terminal_size();
        println!(
            "{}",
            render_overview(
                &timers,
                &active_runs,
                Local::now(),
                width,
                usize::MAX,
                CURSOR_STATIC
            )
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
        let (width, _) = terminal_size();
        print!("{}", print_buzzer_table(&timer, width));
        return Ok(());
    }

    if is_tty() {
        animate(|frame| {
            let (width, height) = terminal_size();
            let mut out = String::new();
            out.push_str(&render_block(
                &timer,
                &active[0],
                Local::now(),
                width,
                CURSOR_CYCLE[frame % 3],
            ));
            out.push('\n');
            // The table must not run past the bottom of the screen.
            let block_lines = out.lines().count() + 1;
            let rows = height.saturating_sub(block_lines).max(1);
            for line in print_buzzer_table(&timer, width).lines().take(rows) {
                out.push_str(line);
                out.push('\n');
            }
            out
        })
    } else {
        let (width, _) = terminal_size();
        println!(
            "{}",
            render_block(&timer, &active[0], Local::now(), width, CURSOR_STATIC)
        );
        println!();
        print!("{}", print_buzzer_table(&timer, width));
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
/// between blocks), capped at `height` lines. Pure — called by both static
/// and animated paths.
fn render_overview(
    timers: &[Timer],
    runs: &[TimerRun],
    now: DateTime<Local>,
    width: usize,
    height: usize,
    cursor: char,
) -> String {
    let mut lines: Vec<String> = Vec::new();
    let mut overflow = 0usize;

    for run in runs {
        let Some(timer) = timers.iter().find(|t| t.name == run.timer_name) else {
            continue;
        };
        let block = render_block(timer, run, now, width, cursor);
        let block_lines: Vec<String> = block.lines().map(str::to_string).collect();
        let spacer = if lines.is_empty() { 0 } else { 1 };

        if lines.len() + spacer + block_lines.len() > height {
            overflow += 1;
            continue;
        }
        if spacer > 0 {
            lines.push(String::new());
        }
        lines.extend(block_lines);
    }

    if overflow > 0 {
        lines.push(format!(
            "+{overflow} more timer(s) not shown (resize the terminal)"
        ));
    }

    lines.join("\n")
}

/// Render a single timer's progress block. Layout adapts to `width`:
///
/// ```text
/// <name>  Start: <datetime>  End: <datetime>  Mult: <n>   (wide terminals)
/// <name>  Start: <datetime>  Mult: <n>                    (medium)
/// <name>  Start: HH:MM                                    (narrow)
/// <name> <elapsed>/<total> next: <buzzer> <cursor>        (below MIN_BLOCK_WIDTH)
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
    if width < MIN_BLOCK_WIDTH {
        return render_minimal(timer, run, now, width, cursor);
    }

    let total = total_duration(timer);
    let elapsed = effective_elapsed(run, now, total);

    let mult = match run.repetitions {
        RepeatMode::Count(n) => n.to_string(),
        RepeatMode::Infinite => "∞".to_string(),
    };

    // Timestamps shrink with the available width; optional fields drop out.
    let start_fmt = if width >= 78 {
        "%Y-%m-%d %H:%M:%S"
    } else if width >= 56 {
        "%Y-%m-%d %H:%M"
    } else {
        "%H:%M"
    };
    let start = run.started_at.format(start_fmt);

    let mut header = format!("{}  Start: {}", timer.name, start);
    if width >= 78 {
        let end = (run.started_at + total).format("%Y-%m-%d %H:%M:%S");
        header.push_str(&format!("  End: {end}"));
    }
    if width >= 56 {
        header.push_str(&format!("  Mult: {mult}"));
    }

    let mut out = String::new();
    out.push_str(&truncate(&header, width));
    out.push('\n');

    match next_buzzer(timer, run, now) {
        Some((buzzer, fire_time)) => {
            let line = format!("Next: {buzzer}  {}", fmt_remaining(fire_time - now));
            out.push_str(&truncate(&line, width));
        }
        None => out.push_str("Next: —"),
    }

    let available = width.saturating_sub(4);
    if available >= MIN_BAR_WIDTH {
        let bar_width = available.min(MAX_BAR_WIDTH);
        out.push('\n');
        out.push_str(&format!(
            "X-{}-X",
            render_bar(total, elapsed, &timer.buzzers, bar_width, cursor)
        ));
    }

    out
}

/// One-line fallback for very narrow terminals.
fn render_minimal(
    timer: &Timer,
    run: &TimerRun,
    now: DateTime<Local>,
    width: usize,
    cursor: char,
) -> String {
    let total = total_duration(timer);
    let elapsed = effective_elapsed(run, now, total);
    let next = next_buzzer(timer, run, now)
        .map(|(name, _)| name)
        .unwrap_or_else(|| "—".to_string());
    let line = format!(
        "{} {} / {} next: {} {}",
        timer.name,
        fmt_remaining(elapsed),
        fmt_remaining(total),
        next,
        cursor
    );
    truncate(&line, width)
}

/// Print the buzzer countdown table for repetition 1. Column widths adapt
/// to the terminal width; the "Time Remaining" column drops on narrow
/// terminals.
///
/// ```text
///  Buzzer Name       Offset     Time Remaining
///  ─────────────────────────────────────────────
///  paymentBuzzer     1W         6D 22:41:07
/// ```
fn print_buzzer_table(timer: &Timer, width: usize) -> String {
    let available = width.saturating_sub(2).max(10);
    let name_w = ((available as f64 * 0.45).round() as usize).clamp(8, 20);
    let off_w = if available >= 40 { 10 } else { 6 };
    let show_remaining = available >= name_w + off_w + 12;

    let mut out = String::new();
    if show_remaining {
        out.push_str(&format!(
            " {:<name_w$} {:<off_w$} Time Remaining\n",
            "Buzzer Name", "Offset"
        ));
    } else {
        out.push_str(&format!(
            " {:<name_w$} {:<off_w$}\n",
            "Buzzer Name", "Offset"
        ));
    }
    out.push_str(&format!(" {}\n", "─".repeat(available)));

    for buzzer_ref in &timer.buzzers {
        let name = truncate(&buzzer_ref.buzzer_name, name_w);
        let offset = truncate(&fmt_offset(buzzer_ref.offset), off_w);
        if show_remaining {
            out.push_str(&format!(
                " {:<name_w$} {:<off_w$} {}\n",
                name,
                offset,
                fmt_remaining(buzzer_ref.offset)
            ));
        } else {
            out.push_str(&format!(" {:<name_w$} {:<off_w$}\n", name, offset));
        }
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

/// Truncate a string to `max` characters, appending `…` when cut.
fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
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

/// Query the terminal size, falling back to 76x24 when it cannot be read.
fn terminal_size() -> (usize, usize) {
    crossterm::terminal::size()
        .map(|(cols, rows)| (cols as usize, rows as usize))
        .unwrap_or((76, 24))
}

/// Whether stdout is attached to a real terminal (animation-capable).
fn is_tty() -> bool {
    use crossterm::tty::IsTty;
    std::io::stdout().is_tty()
}

/// Run a full-screen render loop on the alternate screen: re-render every
/// 100ms from a single snapshot, cycling the cursor character, until the
/// user presses any key. Terminal size is re-queried on every frame and a
/// resize merely re-renders — it never exits the view. Always restores the
/// terminal on exit.
fn animate<F>(render: F) -> Result<()>
where
    F: Fn(usize) -> String,
{
    let mut stdout = std::io::stdout();

    enable_raw_mode().map_err(|e| anyhow!("failed to enter raw mode: {e}"))?;
    let _restore = TerminalGuard;

    queue!(stdout, EnterAlternateScreen, Hide)?;
    stdout.flush()?;

    let mut frame = 0usize;
    loop {
        queue!(
            stdout,
            MoveTo(0, 0),
            Clear(ClearType::All),
            Print(render(frame)),
        )?;
        stdout.flush()?;

        // Any key exits; Ctrl+C arrives as a normal keypress in raw mode.
        // A resize just falls through and the next frame re-lays out.
        if poll(std::time::Duration::from_millis(100))? {
            match read()? {
                Event::Key(KeyEvent { .. }) => break,
                Event::Resize(_, _) => {}
                _ => {}
            }
        }
        frame += 1;
    }

    Ok(())
}

/// Restore the terminal on any exit path (including panics/errors): leave
/// the alternate screen, show the cursor, exit raw mode.
struct TerminalGuard;
impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let mut stdout = std::io::stdout();
        let _ = queue!(stdout, Show, LeaveAlternateScreen);
        let _ = stdout.flush();
        let _ = disable_raw_mode();
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
            fmt_remaining(
                Duration::days(6)
                    + Duration::hours(22)
                    + Duration::minutes(41)
                    + Duration::seconds(7)
            ),
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
        let bar = render_bar(
            Duration::seconds(20),
            Duration::seconds(0),
            &t.buzzers,
            10,
            CURSOR_STATIC,
        );
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
        // 40 columns → bar is min(40-4, 40) = 36 cells; cursor at the end.
        let block = render_block(&t, &run, Local::now(), 40, CURSOR_STATIC);
        assert!(
            block.contains(&format!("X-{}▄-X", "█".repeat(35))),
            "{block}"
        );
    }

    #[test]
    fn block_lines_never_exceed_the_width() {
        let t = timer("a_very_long_timer_name", &[10, 20, 300]);
        let run = running_run("a_very_long_timer_name", 30);
        for width in [30usize, 50, 80, 200] {
            let block = render_block(&t, &run, Local::now(), width, CURSOR_STATIC);
            for line in block.lines() {
                assert!(
                    line.chars().count() <= width,
                    "line {line:?} exceeds width {width}: {block:?}"
                );
            }
        }
    }

    #[test]
    fn bar_is_capped_on_very_wide_terminals() {
        let t = timer("t", &[60]);
        let run = running_run("t", 30);
        // 200 columns → bar capped at MAX_BAR_WIDTH (40) cells.
        let block = render_block(&t, &run, Local::now(), 200, CURSOR_STATIC);
        let bar_line = block.lines().last().unwrap();
        assert_eq!(bar_line.chars().count(), 44, "{bar_line}"); // X-<40>-X
    }

    #[test]
    fn narrow_terminal_uses_minimal_layout() {
        let t = timer("t", &[60]);
        let run = running_run("t", 30);
        let block = render_block(&t, &run, Local::now(), 20, CURSOR_STATIC);
        assert_eq!(block.lines().count(), 1, "{block}");
        assert!(!block.contains("X-"), "{block}");
    }

    #[test]
    fn overview_caps_height_with_more_indicator() {
        let timers: Vec<Timer> = (0..5).map(|i| timer(&format!("t{i}"), &[60])).collect();
        let runs: Vec<TimerRun> = (0..5).map(|i| running_run(&format!("t{i}"), 10)).collect();
        let out = render_overview(&timers, &runs, Local::now(), 80, 5, CURSOR_STATIC);
        assert!(out.contains("more timer(s) not shown"), "{out}");
        assert!(out.lines().count() <= 5, "{out}");
    }

    #[test]
    fn buzzer_table_adapts_to_width() {
        let mut t = timer("t", &[60, 300]);
        t.buzzers[0].buzzer_name = "a_very_long_buzzer_name".to_string();
        for width in [30usize, 100] {
            let table = print_buzzer_table(&t, width);
            for line in table.lines() {
                assert!(line.chars().count() <= width, "{line:?} > {width}");
            }
        }
        // Wide: full header with the remaining column.
        let wide = print_buzzer_table(&t, 100);
        assert!(wide.contains("Buzzer Name"), "{wide}");
        assert!(wide.contains("Time Remaining"), "{wide}");
        // Narrow: remaining column dropped, names truncated.
        let narrow = print_buzzer_table(&t, 30);
        assert!(narrow.contains("a_very_long_…"), "{narrow}");
        assert!(!narrow.contains("Time Remaining"), "{narrow}");
    }

    #[test]
    fn truncate_appends_ellipsis() {
        assert_eq!(truncate("hello", 10), "hello");
        assert_eq!(truncate("hello world", 8), "hello w…");
        assert_eq!(truncate("hello", 1), "…");
    }
}
