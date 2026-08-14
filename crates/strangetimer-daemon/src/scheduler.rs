use std::sync::Arc;

use chrono::Local;
use strangetimer_core::model::{RepeatMode, TimerStatus};
use strangetimer_core::persistence::save_state;
use tokio::sync::mpsc::Sender;
use tokio::time::{Duration, interval};

use crate::state::AppState;

/// Run the scheduler forever: every 500ms, advance all Running runs and
/// forward any buzzer names that became due to `buzzer_tx`.
pub async fn run_scheduler(state: Arc<AppState>, buzzer_tx: Sender<String>) {
    let mut ticker = interval(Duration::from_millis(500));
    loop {
        ticker.tick().await;
        let fired = tick(Arc::clone(&state)).await;
        for name in fired {
            if buzzer_tx.send(name).await.is_err() {
                return; // receiver gone; nothing left to do
            }
        }
    }
}

/// Advance the state by one scheduler pass.
///
/// Returns the names of the buzzers whose fire_time has passed in this pass
/// (deduplicated by name, in index order). Extracted from the loop so unit
/// tests can drive the scheduler deterministically.
pub async fn tick(state: Arc<AppState>) -> Vec<String> {
    let mut fired_this_tick: Vec<String> = Vec::new();
    let mut state_changed = false;

    let mut inner = state.lock().await;
    let now = Local::now();

    let mut updates: Vec<(usize, strangetimer_core::model::TimerRun)> = Vec::new();

    for i in 0..inner.state.runs.len() {
        let mut run = inner.state.runs[i].clone();

        // A run that was scheduled for the past starts now.
        if run.status == TimerStatus::Scheduled {
            if let Some(schedule_time) = run.schedule_time {
                if now >= schedule_time {
                    run.status = TimerStatus::Running;
                    state_changed = true;
                    updates.push((i, run));
                }
            }
            continue;
        }

        if run.status != TimerStatus::Running {
            continue;
        }

        let timer_name = run.timer_name.clone();
        let Some(timer) = inner.timers.iter().find(|t| t.name == timer_name) else {
            continue;
        };

        // Fire every buzzer whose time has passed in this repetition.
        let mut fired_now = Vec::new();
        for (idx, buzzer_ref) in timer.buzzers.iter().enumerate() {
            if run.fired_indices.contains(&idx) {
                continue;
            }
            let fire_time = run.started_at + run.elapsed_before_pause + buzzer_ref.offset;
            if now >= fire_time {
                run.fired_indices.push(idx);
                fired_now.push(buzzer_ref.buzzer_name.clone());
            }
        }

        // When every buzzer of the current repetition has fired, advance to
        // the next repetition or complete the run.
        if run.fired_indices.len() == timer.buzzers.len() {
            match run.repetitions {
                RepeatMode::Count(count) if run.current_rep + 1 < count => {
                    run.current_rep += 1;
                    run.fired_indices.clear();
                    run.started_at = now;
                }
                RepeatMode::Infinite => {
                    run.current_rep += 1;
                    run.fired_indices.clear();
                    run.started_at = now;
                }
                _ => {
                    run.status = TimerStatus::Completed;
                }
            }
        }

        if !fired_now.is_empty()
            || run.fired_indices != inner.state.runs[i].fired_indices
            || run.status != inner.state.runs[i].status
            || run.current_rep != inner.state.runs[i].current_rep
        {
            state_changed = true;
            fired_this_tick.extend(fired_now);
            updates.push((i, run));
        }
    }

    for (i, updated_run) in updates {
        inner.state.runs[i] = updated_run;
    }

    if state_changed {
        if let Err(e) = save_state(&inner.state) {
            eprintln!("strangetimer-daemon: scheduler failed to save state: {e:#}");
        }
    }

    fired_this_tick
}
