use std::io::Write;
use std::sync::{Arc, Mutex};

use anyhow::{anyhow, Result};
use chrono::{DateTime, Duration, Local};
use crossterm::cursor::{Hide, Show};
use crossterm::event::{poll, read, Event, KeyCode, KeyEvent, KeyModifiers};
use crossterm::queue;
use crossterm::style::Print;
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, Clear, ClearType};
use strangetimer_core::ipc::{ClientMessage, ServerMessage};
use strangetimer_core::model::{BuzzerRef, RepeatMode, Timer, TimerRun, TimerStatus};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::commands::send_and_receive;
use crate::style;

/// Characters the animated cursor cycles through (roughly one per 100ms
/// frame, so the full cycle takes ~0.3s).
const CURSOR_CYCLE: [char; 3] = ['▂', '▄', '▆'];
/// Static placeholder used when not animating (also by unit tests).
#[cfg_attr(not(test), allow(dead_code))]
const CURSOR_STATIC: char = '▄';

/// Below this width the block layout degrades to a one-line summary.
const MIN_BLOCK_WIDTH: usize = 30;
/// Below this width the overview table degrades to a one-line list.
const MIN_TABLE_WIDTH: usize = 40;
/// The progress bar is capped so a wide terminal doesn't stretch it absurdly.
const MAX_BAR_WIDTH: usize = 40;
/// The bar is hidden entirely below this many available columns.
const MIN_BAR_WIDTH: usize = 8;
/// Re-fetch the daemon snapshot every N frames in live mode (~1s at 100ms).
const REFETCH_EVERY: usize = 10;

/// A snapshot of everything `view` needs, refetched periodically in live
/// mode so fires/completions/pending states appear without exiting.
struct Snapshot {
    timers: Vec<Timer>,
    runs: Vec<TimerRun>,
    pending: Vec<String>,
}

/// `strangetimer view timers [--snapshot]` — live overview of every active
/// run plus an inactive section, or a persistent static snapshot.
pub fn view_timers(snapshot: bool) -> Result<()> {
    let snap = fetch_snapshot()?;

    if active_runs(&snap.runs).is_empty() && snap.timers.is_empty() {
        println!("No timers currently running.");
        return Ok(());
    }

    if is_tty() && !snapshot {
        let cache: Arc<Mutex<Option<Snapshot>>> = Arc::new(Mutex::new(Some(snap)));
        let render_cache = Arc::clone(&cache);
        let final_cache = Arc::clone(&cache);
        animate(
            move |frame| {
                let mut guard = render_cache.lock().expect("view cache lock");
                if frame % REFETCH_EVERY == 0 {
                    if let Ok(fresh) = fetch_snapshot() {
                        *guard = Some(fresh);
                    }
                }
                let snap = guard.as_ref().expect("snapshot present");
                render_overview(&snap.timers, &snap.runs, &snap.pending, Local::now(), true)
            },
            move || {
                let guard = final_cache.lock().expect("view cache lock");
                let snap = guard.as_ref().expect("snapshot present");
                render_overview(&snap.timers, &snap.runs, &snap.pending, Local::now(), false)
            },
        )
    } else {
        let out = render_overview(&snap.timers, &snap.runs, &snap.pending, Local::now(), false);
        println!("{out}");
        Ok(())
    }
}

/// `strangetimer view <name> [--snapshot]` — single-timer block + buzzer
/// countdown table.
pub fn view_timer(name: &str, snapshot: bool) -> Result<()> {
    let response = send_and_receive(&ClientMessage::GetTimer {
        name: name.to_string(),
    })?;

    let (timer, runs) = match response {
        ServerMessage::TimerDetail { timer, runs, .. } => (timer, runs),
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

    if is_tty() && !snapshot {
        let timer_name = name.to_string();
        let state: Arc<Mutex<Option<(Timer, TimerRun)>>> =
            Arc::new(Mutex::new(Some((timer.clone(), active[0].clone()))));
        let render_state = Arc::clone(&state);
        let final_state = Arc::clone(&state);
        animate(
            move |frame| {
                {
                    let mut guard = render_state.lock().expect("view cache lock");
                    if frame % REFETCH_EVERY == 0 {
                        // Keep the live block in sync with the daemon
                        // (fires, pause/resume, completion).
                        if let Ok(ServerMessage::TimerDetail { timer, runs, .. }) =
                            send_and_receive(&ClientMessage::GetTimer {
                                name: timer_name.clone(),
                            })
                        {
                            if let Some(run) = runs
                                .into_iter()
                                .find(|r| r.status != TimerStatus::Completed)
                            {
                                *guard = Some((timer, run));
                            }
                        }
                    }
                }
                let (timer, run) = guard_clone(&render_state);
                let (width, height) = terminal_size();
                let mut out = String::new();
                out.push_str(&render_block(
                    &timer,
                    &run,
                    Local::now(),
                    width,
                    CURSOR_CYCLE[frame % 3],
                ));
                out.push('\n');
                let block_lines = out.lines().count() + 1;
                let rows = height.saturating_sub(block_lines).max(1);
                for line in print_buzzer_table(&timer, width).lines().take(rows) {
                    out.push_str(line);
                    out.push('\n');
                }
                out
            },
            move || {
                let (timer, run) = guard_clone(&final_state);
                let (width, _) = terminal_size();
                let mut out = render_block(&timer, &run, Local::now(), width, CURSOR_STATIC);
                out.push('\n');
                out.push_str(&print_buzzer_table(&timer, width));
                out
            },
        )
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

/// Fetch the daemon's timer list + live runs + pending interrupts.
fn fetch_snapshot() -> Result<Snapshot> {
    match send_and_receive(&ClientMessage::GetTimers)? {
        ServerMessage::TimerList {
            timers,
            runs,
            pending_interrupts,
            ..
        } => Ok(Snapshot {
            timers,
            runs,
            pending: pending_interrupts,
        }),
        ServerMessage::Error(e) => Err(anyhow!(e)),
        other => Err(anyhow!("unexpected daemon response: {other:?}")),
    }
}

/// Clone the cached (timer, run) pair from the live single-timer view.
fn guard_clone(cache: &Arc<Mutex<Option<(Timer, TimerRun)>>>) -> (Timer, TimerRun) {
    let guard = cache.lock().expect("view cache lock");
    guard.as_ref().expect("snapshot present").clone()
}

/// Runs worth displaying: everything except completed terminal states.
fn active_runs(runs: &[TimerRun]) -> Vec<TimerRun> {
    runs.iter()
        .filter(|r| r.status != TimerStatus::Completed)
        .cloned()
        .collect()
}

/// Render the full `view timers` output as an exact-width bordered table:
/// every physical line is at most the terminal width, active timers span
/// two rows (details + a full-width progress line), and user-interrupt
/// timers show a PENDING marker. `blink_pending` adds a blinking ANSI
/// emphasis in live mode.
fn render_overview(
    timers: &[Timer],
    runs: &[TimerRun],
    pending: &[String],
    now: DateTime<Local>,
    blink_pending: bool,
) -> String {
    let (width, height) = terminal_size();
    if width < MIN_TABLE_WIDTH {
        return render_minimal_list(timers, runs, now, width, height);
    }

    let cols = Columns::for_width(width);
    let sep = style::rule(" │ ");
    let left = style::rule("│ ");
    let right = style::rule(" │");
    let rule = style::rule(&"─".repeat(width));

    let header_cell = |label: &str, w: usize| style::header(&pad(&truncate_vis(label, w), w));

    let mut out = String::new();
    out.push_str(&style::header("ACTIVE RUNS"));
    out.push('\n');
    out.push_str(&format!(
        "{left}{}{sep}{}{sep}{}{sep}{}{sep}{}{right}",
        header_cell("TIMER", cols.name),
        header_cell("STATUS", cols.status),
        header_cell("ELAPSED", cols.elapsed),
        header_cell("START → END", cols.time),
        header_cell("NEXT", cols.next),
    ));
    out.push('\n');
    out.push_str(&rule);
    out.push('\n');

    // Active rows first (only live, non-completed runs), sorted by the
    // soonest next buzzer.
    let mut active: Vec<&TimerRun> = runs
        .iter()
        .filter(|r| r.status != TimerStatus::Completed)
        .collect();
    active.sort_by_key(|r| next_buzzer_fire(timers, r, now).unwrap_or_else(chrono::Local::now));

    let mut used = 3usize; // section header + header row + rule
    let mut overflow = 0usize;

    for run in active {
        let is_pending = pending.iter().any(|p| p == &run.timer_name);
        let Some(timer) = timers.iter().find(|t| t.name == run.timer_name) else {
            continue;
        };
        let detail = active_detail_row(timer, run, now, &cols, is_pending, blink_pending);
        let progress = progress_row(timer, run, now, width, &cols, is_pending);
        // Active timers occupy two physical lines.
        if used + 3 > height {
            overflow += 1;
            continue;
        }
        out.push_str(&detail);
        out.push('\n');
        out.push_str(&progress);
        out.push('\n');
        used += 2;
    }

    // Inactive section for defined timers without a live run.
    let inactive: Vec<&Timer> = timers
        .iter()
        .filter(|t| {
            !runs
                .iter()
                .any(|r| r.timer_name == t.name && r.status != TimerStatus::Completed)
        })
        .collect();
    if !inactive.is_empty() {
        if used + 3 + inactive.len() > height {
            overflow += inactive.len();
        } else {
            out.push_str(&rule);
            out.push('\n');
            out.push_str(&style::header("INACTIVE TIMERS"));
            out.push('\n');
            out.push_str(&format!(
                "{left}{}{sep}{}{sep}{}{sep}{}{right}",
                header_cell("TIMER", cols.name),
                header_cell("STATUS", cols.status),
                header_cell("DURATION", cols.time),
                header_cell("BUZZERS", cols.next),
            ));
            out.push('\n');
            used += 4;
            for timer in &inactive {
                if used + 1 > height {
                    overflow += 1;
                    continue;
                }
                out.push_str(&inactive_row(timer, &cols));
                out.push('\n');
                used += 1;
            }
        }
    }

    if overflow > 0 {
        out.push_str(&style::dim(&format!("+{overflow} more timer(s) not shown")));
        out.push('\n');
    }

    out
}

/// Column widths for the detail rows, derived from the exact terminal
/// width so the joined line never wraps.
struct Columns {
    name: usize,
    status: usize,
    elapsed: usize,
    time: usize,
    next: usize,
}

impl Columns {
    fn for_width(width: usize) -> Self {
        // "│ " + a + " │ " + b + " │ " + c + " │ " + d + " │ " + e + " │"
        // = 2 borders + 4 separators(" │ ") + 2 spacing = 16 fixed columns.
        let overhead = 16usize;
        let budget = width.saturating_sub(overhead);

        let name_min = 8usize;
        let status_min = 8usize;
        let elapsed_min = 10usize;
        let time_min = 12usize;
        let next_min = 8usize;
        let mins = name_min + status_min + elapsed_min + time_min + next_min;

        if budget <= mins {
            // Narrow terminal: shrink columns proportionally (weights sum
            // to 8, with small floors) so the row still fits exactly.
            let name = (budget * 3 / 8).max(4);
            let status = (budget / 8).max(3);
            let elapsed = (budget / 8).max(3);
            let time = (budget * 2 / 8).max(5);
            let next = (budget / 8).max(3);
            return Columns {
                name,
                status,
                elapsed,
                time,
                next,
            };
        }

        let extra = budget - mins;
        // Proportionally grow columns up to their maxima.
        let name_max = 20usize;
        let status_max = 12usize;
        let elapsed_max = 12usize;
        let time_max = 22usize;
        let next_max = 18usize;
        let share = |max: usize, weight: usize| (extra * weight / 8).min(max);
        let name = name_min + share(name_max - name_min, 3);
        let status = status_min + share(status_max - status_min, 1);
        let elapsed = elapsed_min + share(elapsed_max - elapsed_min, 1);
        let time = time_min + share(time_max - time_min, 2);
        let next = next_min + share(next_max - next_min, 1);
        Columns {
            name,
            status,
            elapsed,
            time,
            next,
        }
    }
}

/// One detail row for a live run. Raw text is truncated and padded to the
/// column width *before* styling, so ANSI codes never shift the columns.
fn active_detail_row(
    timer: &Timer,
    run: &TimerRun,
    now: DateTime<Local>,
    cols: &Columns,
    is_pending: bool,
    blink_pending: bool,
) -> String {
    let total = total_duration(timer);

    let rep = match run.repetitions {
        RepeatMode::Count(n) if n > 1 => format!(" ×{n}"),
        RepeatMode::Infinite => " ∞".to_string(),
        _ => String::new(),
    };
    let status_text = if is_pending {
        "PENDING".to_string()
    } else {
        match run.status {
            TimerStatus::Running => format!("run{rep}"),
            TimerStatus::Paused => "paused".to_string(),
            TimerStatus::Scheduled => "scheduled".to_string(),
            TimerStatus::Completed => "done".to_string(),
        }
    };

    let start = run.started_at.format("%H:%M");
    let end = (run.started_at + total).format("%H:%M");
    let time = format!("{start} → {end}");

    let next = next_buzzer(timer, run, now)
        .map(|(n, _)| n)
        .unwrap_or_else(|| "—".to_string());

    let name = cell(&timer.name, cols.name);
    let status = if is_pending {
        let padded = pad(&truncate_vis(&status_text, cols.status), cols.status);
        if blink_pending {
            style::blink(&style::warn(&padded))
        } else {
            style::warn(&padded)
        }
    } else {
        cell_status(&status_text, cols.status, run.status.clone())
    };
    let time = cell(&time, cols.time);
    let next = cell(&next, cols.next);

    format!("│ {name} │ {status} │ {time} │ {next} │",)
}

/// The progress continuation row: the bar lives on its own line under the
/// details, padded to the exact terminal width.
fn progress_row(
    timer: &Timer,
    run: &TimerRun,
    now: DateTime<Local>,
    width: usize,
    _cols: &Columns,
    _is_pending: bool,
) -> String {
    let total = total_duration(timer);
    let elapsed = effective_elapsed(run, now, total);
    // "│" + "  X-<bar>-X" + pad + " │" must equal exactly `width`:
    // bar + 6 + 3 = width, so bar <= width - 9.
    let inner = width.saturating_sub(9);
    let bar_width = inner.min(MAX_BAR_WIDTH);
    let bar = render_bar(total, elapsed, &timer.buzzers, bar_width, CURSOR_STATIC);
    let line = format!("  X-{bar}-X");
    format!(
        "│{}{} │",
        line,
        " ".repeat(width.saturating_sub(3 + line.len()))
    )
}

/// One row for a defined-but-not-running timer.
fn inactive_row(timer: &Timer, cols: &Columns) -> String {
    let total = total_duration(timer);
    let time = format!("total {}", fmt_remaining(total));
    let next = match timer.buzzers.len() {
        0 => "no buzzers".to_string(),
        1 => "1 buzzer".to_string(),
        n => format!("{n} buzzers"),
    };
    let name = style::dim(&cell(&timer.name, cols.name));
    let status = style::dim(&pad("—", cols.status));
    let time = cell(&time, cols.time);
    let next = cell(&next, cols.next);
    format!("│ {name} │ {status} │ {time} │ {next} │")
}

/// The fire time of a run's next buzzer (for sorting), or `now`.
fn next_buzzer_fire(
    timers: &[Timer],
    run: &TimerRun,
    now: DateTime<Local>,
) -> Option<DateTime<Local>> {
    let timer = timers.iter().find(|t| t.name == run.timer_name)?;
    next_buzzer(timer, run, now).map(|(_, t)| t)
}

/// Minimal one-line-per-timer list for terminals narrower than the table
/// threshold. Pure; capped at `height`.
fn render_minimal_list(
    timers: &[Timer],
    runs: &[TimerRun],
    now: DateTime<Local>,
    width: usize,
    height: usize,
) -> String {
    let mut lines: Vec<String> = Vec::new();
    let mut overflow = 0usize;

    for run in runs.iter().filter(|r| r.status != TimerStatus::Completed) {
        let Some(timer) = timers.iter().find(|t| t.name == run.timer_name) else {
            continue;
        };
        if lines.len() >= height {
            overflow += 1;
            continue;
        }
        lines.push(render_minimal(timer, run, now, width, CURSOR_STATIC));
    }
    for timer in timers.iter().filter(|t| {
        !runs
            .iter()
            .any(|r| r.timer_name == t.name && r.status != TimerStatus::Completed)
    }) {
        if lines.len() >= height {
            overflow += 1;
            continue;
        }
        let total = total_duration(timer);
        // Style after truncation so ANSI never distorts the width math.
        let line = truncate_vis(
            &format!("{} total {} — inactive", timer.name, fmt_remaining(total)),
            width,
        );
        lines.push(style::dim(&line));
    }

    if overflow > 0 {
        lines.push(format!("+{overflow} more timer(s) not shown"));
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

    let elapsed_text = fmt_remaining(elapsed);
    let mut header = format!("{}  Start: {}", timer.name, start);
    if width >= 78 {
        let end = (run.started_at + total).format("%Y-%m-%d %H:%M:%S");
        header.push_str(&format!("  End: {end}"));
    }
    if width >= 68 {
        header.push_str(&format!("  Elapsed: {elapsed_text}"));
    }
    if width >= 56 {
        header.push_str(&format!("  Mult: {mult}"));
    }

    let mut out = String::new();
    out.push_str(&truncate_vis(&header, width));
    out.push('\n');

    match next_buzzer(timer, run, now) {
        Some((buzzer, fire_time)) => {
            let line = format!("Next: {buzzer}  {}", fmt_remaining(fire_time - now));
            out.push_str(&truncate_vis(&line, width));
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
    truncate_vis(&line, width)
}

/// Print the buzzer countdown table for repetition 1. Column widths adapt
/// to the terminal width; the "Time Remaining" column drops on narrow
/// terminals. Cells are padded on raw text before styling.
fn print_buzzer_table(timer: &Timer, width: usize) -> String {
    let available = width.saturating_sub(2).max(10);
    let name_w = ((available as f64 * 0.45).round() as usize).clamp(8, 20);
    let off_w = if available >= 40 { 10 } else { 6 };
    let show_remaining = available >= name_w + off_w + 14;

    let mut out = String::new();
    if show_remaining {
        out.push_str(&format!(
            " {} {} Time Remaining\n",
            style::header(&pad(&truncate_vis("Buzzer Name", name_w), name_w)),
            style::header(&pad(&truncate_vis("Offset", off_w), off_w)),
        ));
    } else {
        out.push_str(&format!(
            " {} {}\n",
            style::header(&pad(&truncate_vis("Buzzer Name", name_w), name_w)),
            style::header(&pad(&truncate_vis("Offset", off_w), off_w)),
        ));
    }
    out.push_str(&format!(" {}\n", "─".repeat(available)));

    for buzzer_ref in &timer.buzzers {
        let name = pad(&truncate_vis(&buzzer_ref.buzzer_name, name_w), name_w);
        let offset = pad(&truncate_vis(&fmt_offset(buzzer_ref.offset), off_w), off_w);
        if show_remaining {
            out.push_str(&format!(
                " {} {} {}\n",
                name,
                offset,
                fmt_remaining(buzzer_ref.offset)
            ));
        } else {
            out.push_str(&format!(" {} {}\n", name, offset));
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

/// Truncate `s` by terminal display width to at most `max` columns,
/// appending `…` when cut.
fn truncate_vis(s: &str, max: usize) -> String {
    if s.width() <= max {
        return s.to_string();
    }
    let budget = max.saturating_sub(1);
    let mut out = String::new();
    let mut w = 0usize;
    for ch in s.chars() {
        let cw = UnicodeWidthChar::width(ch).unwrap_or(0);
        if w + cw > budget {
            break;
        }
        out.push(ch);
        w += cw;
    }
    out.push('…');
    out
}

/// Pad `s` with spaces to exactly `max` terminal columns (no truncation).
fn pad(s: &str, max: usize) -> String {
    let w = s.width();
    if w >= max {
        return s.to_string();
    }
    format!("{s}{}", " ".repeat(max - w))
}

/// Pad + truncate a raw cell, then apply the given style to the padded
/// text — ANSI codes never participate in column math.
fn cell(s: &str, max: usize) -> String {
    pad(&truncate_vis(s, max), max)
}

/// Like `cell`, but with a status color.
fn cell_status(s: &str, max: usize, status: TimerStatus) -> String {
    style::status(&cell(s, max), status)
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

/// Public terminal-size accessor (used by the buzzer views).
pub fn terminal_size_pub() -> (usize, usize) {
    terminal_size()
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
/// Run an animated render loop on the **terminal alternate buffer**:
/// every frame is drawn from an explicit `(0, 0)` origin after clearing
/// the whole buffer, so updates happen strictly in place — nothing is
/// ever appended to the primary screen's scrollback. Exit with `q`,
/// Escape or Ctrl+C; arrow keys and mouse scrolling never exit. On exit
/// the alternate buffer is left cleanly and exactly one final snapshot
/// is printed to the primary screen.
fn animate<F, G>(mut render: F, final_snapshot: G) -> Result<()>
where
    F: FnMut(usize) -> String,
    G: Fn() -> String,
{
    use crossterm::cursor::MoveTo;
    use crossterm::terminal::EnterAlternateScreen;

    let mut stdout = std::io::stdout();

    enable_raw_mode().map_err(|e| anyhow!("failed to enter raw mode: {e}"))?;
    let restore = TerminalGuard;

    queue!(stdout, EnterAlternateScreen, Hide)?;
    stdout.flush()?;

    let mut frame = 0usize;
    loop {
        // Reserve one line so the final printed line never triggers a
        // scroll inside the alternate buffer.
        let (_, height) = terminal_size();
        let budget = height.saturating_sub(1).max(1);
        let body: Vec<String> = render(frame)
            .lines()
            .take(budget)
            .map(str::to_string)
            .collect();

        // Explicit origin + full clear: the previous frame is overwritten
        // in place, never appended to scrollback.
        queue!(stdout, MoveTo(0, 0), Clear(ClearType::All))?;
        for line in &body {
            queue!(stdout, Print(line), Print("\r\n"))?;
        }
        stdout.flush()?;

        if poll(std::time::Duration::from_millis(100))? {
            match read()? {
                Event::Key(KeyEvent {
                    code, modifiers, ..
                }) => {
                    let quit = matches!(code, KeyCode::Char('q') | KeyCode::Char('Q'))
                        || code == KeyCode::Esc
                        || (code == KeyCode::Char('c')
                            && modifiers.contains(KeyModifiers::CONTROL));
                    if quit {
                        break;
                    }
                    // Arrow keys / any other key: ignored — the view stays
                    // up. Mouse scrolling inside the alternate buffer does
                    // not affect the primary screen.
                }
                Event::Resize(_, _) => {}
                _ => {}
            }
        }
        frame += 1;
    }

    // Leave the alternate buffer, then print exactly one final snapshot
    // so the display persists in the primary scrollback.
    drop(restore);
    println!("{}", final_snapshot());
    Ok(())
}

/// Restore the terminal on any exit path (including panics/errors): leave
/// the alternate buffer, show the cursor, exit raw mode.
struct TerminalGuard;
impl Drop for TerminalGuard {
    fn drop(&mut self) {
        use crossterm::terminal::LeaveAlternateScreen;
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
            user_interrupt: false,
            interrupt_focus: None,
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
        assert_eq!(bar.width(), 10);
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
                    line.width() <= width,
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
        assert_eq!(bar_line.width(), 44, "{bar_line}"); // X-<40>-X
    }

    #[test]
    fn narrow_terminal_uses_minimal_layout() {
        let t = timer("t", &[60]);
        let run = running_run("t", 30);
        let block = render_block(&t, &run, Local::now(), 20, CURSOR_STATIC);
        assert_eq!(block.lines().count(), 1, "{block}");
        assert!(!block.contains("X-"), "{block}");
    }

    /// Terminal-visible width of a possibly-styled line: strips ANSI
    /// escape sequences before measuring (the parallel style tests may
    /// force color on globally).
    fn vis_width(s: &str) -> usize {
        let mut out = String::new();
        let mut chars = s.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '\u{1b}' {
                // consume CSI sequence: ESC [ params ... final byte in @-~
                if chars.peek() == Some(&'[') {
                    chars.next();
                    for c2 in chars.by_ref() {
                        if ('@'..='~').contains(&c2) {
                            break;
                        }
                    }
                    continue;
                }
            }
            out.push(c);
        }
        out.width()
    }

    #[test]
    fn overview_lines_never_exceed_the_width() {
        let timers: Vec<Timer> = (0..5).map(|i| timer(&format!("t{i}"), &[60])).collect();
        let runs: Vec<TimerRun> = (0..5).map(|i| running_run(&format!("t{i}"), 10)).collect();
        // Sweep the full practical width range, including very narrow
        // (below MIN_TABLE_WIDTH the table degrades to the minimal list,
        // so only widths >= 40 produce detail/progress rows).
        let mut widths: Vec<usize> = (40..=80).collect();
        widths.extend([96, 120, 160, 200, 240]);
        for width in widths {
            // render_overview reads the real terminal; emulate by sizing the
            // columns directly and asserting the row builder stays in bounds.
            let cols = Columns::for_width(width);
            let t = &timers[0];
            let r = &runs[0];
            // 5-column rows (TIMER|STATUS|ELAPSED|START→END|NEXT) must fit.
            assert!(
                cols.name + cols.status + cols.elapsed + cols.time + cols.next + 16 <= width,
                "column sum exceeds width {width}"
            );
            let detail = active_detail_row(t, r, Local::now(), &cols, false, false);
            assert!(
                vis_width(&detail) <= width,
                "detail {detail:?} exceeds {width}"
            );
            let progress = progress_row(t, r, Local::now(), width, &cols, false);
            assert!(
                vis_width(&progress) <= width,
                "progress {progress:?} exceeds {width}"
            );
        }
    }

    #[test]
    fn pending_status_marks_interrupt_runs() {
        let timers = vec![timer("t", &[60])];
        let runs = vec![running_run("t", 10)];
        let out = render_overview(&timers, &runs, &["t".to_string()], Local::now(), false);
        assert!(out.contains("PENDING"), "{out}");
    }

    #[test]
    fn overview_shows_inactive_section() {
        let timers = vec![timer("running_t", &[60]), timer("idle", &[60])];
        let runs = vec![running_run("running_t", 10)];
        let out = render_overview(&timers, &runs, &[], Local::now(), false);
        assert!(out.contains("ACTIVE RUNS"), "{out}");
        assert!(out.contains("INACTIVE TIMERS"), "{out}");
        assert!(out.contains("idle"), "{out}");
    }

    #[test]
    fn truncate_uses_display_width() {
        assert_eq!(truncate_vis("hello", 10), "hello");
        assert_eq!(truncate_vis("hello world", 8), "hello w…");
        // Wide characters (CJK) count as two columns.
        assert_eq!(truncate_vis("漢字x", 4), "漢…");
        assert_eq!(pad("ab", 5), "ab   ");
    }
}
