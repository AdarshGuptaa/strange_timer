use std::sync::Arc;

use chrono::Local;
use strangetimer_core::model::{FireEvent, RepeatMode, TimerStatus};
use strangetimer_core::persistence::save_state;
use tokio::sync::mpsc::Sender;
use tokio::time::{interval, Duration};

use crate::state::AppState;

/// Run the scheduler forever: every 500ms, advance all Running runs and
/// forward any [`FireEvent`]s that became due to `buzzer_tx`.
pub async fn run_scheduler(state: Arc<AppState>, buzzer_tx: Sender<FireEvent>) {
    let mut ticker = interval(Duration::from_millis(500));
    loop {
        ticker.tick().await;
        let fired = tick(Arc::clone(&state)).await;
        for event in fired {
            if buzzer_tx.send(event).await.is_err() {
                return; // receiver gone; nothing left to do
            }
        }
    }
}

/// Advance the state by one scheduler pass.
///
/// Returns the [`FireEvent`]s whose fire_time has passed in this pass (in
/// index order). Extracted from the loop so unit tests can drive the
/// scheduler deterministically.
pub async fn tick(state: Arc<AppState>) -> Vec<FireEvent> {
    let mut fired_this_tick: Vec<FireEvent> = Vec::new();
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
                fired_now.push(FireEvent {
                    timer_name: timer_name.clone(),
                    buzzer_name: buzzer_ref.buzzer_name.clone(),
                    buzzer_index: idx,
                    repetition: run.current_rep,
                });
            }
        }

        // When every buzzer of the current repetition has fired, advance to
        // the next repetition or complete the run. `elapsed_before_pause`
        // is reset so pause offsets never leak into later repetitions.
        if run.fired_indices.len() == timer.buzzers.len() {
            match run.repetitions {
                RepeatMode::Count(count) if run.current_rep + 1 < count => {
                    run.current_rep += 1;
                    run.fired_indices.clear();
                    run.started_at = now;
                    run.elapsed_before_pause = chrono::Duration::zero();
                }
                RepeatMode::Infinite => {
                    run.current_rep += 1;
                    run.fired_indices.clear();
                    run.started_at = now;
                    run.elapsed_before_pause = chrono::Duration::zero();
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
            warn!("scheduler failed to save state: {e:#}");
        }
    }

    fired_this_tick
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::AppState;
    use std::sync::Arc;

    use chrono::Duration;
    use strangetimer_core::model::{
        BuzzerRef, DaemonState, RepeatMode, Timer, TimerRun, TimerStatus,
    };

    fn state_with(timers: Vec<Timer>, runs: Vec<TimerRun>) -> Arc<AppState> {
        crate::test_util::init();
        Arc::new(AppState::new(
            timers,
            Vec::new(),
            DaemonState {
                runs,
                registered: true,
                last_saved_at: None,
                interrupt_pending: None,
                pending_interrupts: Vec::new(),
            },
        ))
    }

    fn timer_with_offsets(name: &str, offsets_secs: &[i64]) -> Timer {
        Timer {
            name: name.to_string(),
            buzzers: offsets_secs
                .iter()
                .map(|s| BuzzerRef {
                    offset: Duration::seconds(*s),
                    buzzer_name: "buzz".to_string(),
                })
                .collect(),
            created_at: Local::now(),
        }
    }

    fn run(
        name: &str,
        status: TimerStatus,
        started_secs_ago: i64,
        rep: u32,
        fired: &[usize],
    ) -> TimerRun {
        TimerRun {
            timer_name: name.to_string(),
            started_at: Local::now() - Duration::seconds(started_secs_ago),
            repetitions: strangetimer_core::model::RepeatMode::Count(1),
            current_rep: rep,
            schedule_time: None,
            status,
            paused_at: None,
            elapsed_before_pause: Duration::zero(),
            fired_indices: fired.to_vec(),
            user_interrupt: false,
            interrupt_focus: None,
        }
    }

    #[tokio::test]
    async fn fires_due_buzzers_only() {
        // 5s buzzer is due (run started 10s ago), 30s buzzer is not.
        let s = state_with(
            vec![timer_with_offsets("t", &[5, 30])],
            vec![run("t", TimerStatus::Running, 10, 0, &[])],
        );
        let fired = tick(Arc::clone(&s)).await;
        assert_eq!(fired.len(), 1);
        assert_eq!(fired[0].buzzer_name, "buzz");
        assert_eq!(fired[0].timer_name, "t");
        let run = s.get_run("t").await.unwrap();
        assert_eq!(run.fired_indices, vec![0]);
        assert_eq!(run.status, TimerStatus::Running);
    }

    #[tokio::test]
    async fn completes_run_when_all_fired_and_count_done() {
        let s = state_with(
            vec![timer_with_offsets("t", &[5])],
            vec![run("t", TimerStatus::Running, 10, 0, &[])],
        );
        let fired = tick(Arc::clone(&s)).await;
        assert_eq!(fired.len(), 1);
        assert_eq!(fired[0].buzzer_name, "buzz");
        assert_eq!(fired[0].timer_name, "t");
        let run = s.get_run("t").await.unwrap();
        assert_eq!(run.status, TimerStatus::Completed);
    }

    #[tokio::test]
    async fn advances_to_next_repetition_for_count() {
        let mut run = run("t", TimerStatus::Running, 10, 0, &[]);
        run.repetitions = RepeatMode::Count(2);
        let s = state_with(vec![timer_with_offsets("t", &[5])], vec![run]);
        // Rep 0 fires and advances to rep 1.
        tick(Arc::clone(&s)).await;
        let mut run = s.get_run("t").await.unwrap();
        assert_eq!(run.current_rep, 1);
        assert_eq!(run.status, TimerStatus::Running);
        assert!(run.fired_indices.is_empty());

        // Push rep 1's clock into the past and tick again → completed.
        run.started_at = Local::now() - Duration::seconds(10);
        s.update_state(|st| {
            st.runs[0] = run.clone();
        })
        .await
        .unwrap();
        tick(Arc::clone(&s)).await;
        let run = s.get_run("t").await.unwrap();
        assert_eq!(run.status, TimerStatus::Completed);
    }

    #[tokio::test]
    async fn infinite_repetitions_never_complete() {
        let mut run = run("t", TimerStatus::Running, 10, 0, &[]);
        run.repetitions = RepeatMode::Infinite;
        let s = state_with(vec![timer_with_offsets("t", &[5])], vec![run]);
        tick(Arc::clone(&s)).await;
        let run = s.get_run("t").await.unwrap();
        assert_eq!(run.current_rep, 1);
        assert_eq!(run.status, TimerStatus::Running);
    }

    #[tokio::test]
    async fn count_run_fires_one_event_per_repetition() {
        // A Count(3) timer whose 1s buzzer fires on every tick: across
        // enough ticks it must fire exactly three events, one per
        // repetition, with increasing repetition numbers.
        let mut run = run("t", TimerStatus::Running, 100, 0, &[]);
        run.repetitions = RepeatMode::Count(3);
        let s = state_with(vec![timer_with_offsets("t", &[0])], vec![run]);
        let mut events = Vec::new();
        for _ in 0..12 {
            let fired = tick(Arc::clone(&s)).await;
            events.extend(fired);
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert_eq!(
            events.len(),
            3,
            "expected three fire events, got {events:?}"
        );
        let reps: Vec<u32> = events.iter().map(|e| e.repetition).collect();
        assert_eq!(reps, vec![0, 1, 2], "{events:?}");
    }

    #[tokio::test]
    async fn repetition_advance_resets_pause_shift() {
        let mut run = run("t", TimerStatus::Running, 100, 0, &[]);
        run.repetitions = RepeatMode::Count(2);
        run.elapsed_before_pause = chrono::Duration::seconds(90);
        let s = state_with(vec![timer_with_offsets("t", &[0])], vec![run]);
        tick(Arc::clone(&s)).await;
        let run = s.get_run("t").await.unwrap();
        assert_eq!(run.current_rep, 1);
        assert_eq!(
            run.elapsed_before_pause,
            chrono::Duration::zero(),
            "pause offset must not leak into the next repetition"
        );
    }

    #[tokio::test]
    async fn skips_paused_runs() {
        let s = state_with(
            vec![timer_with_offsets("t", &[5])],
            vec![run("t", TimerStatus::Paused, 10, 0, &[])],
        );
        let fired = tick(Arc::clone(&s)).await;
        assert!(fired.is_empty());
    }

    #[tokio::test]
    async fn scheduled_run_starts_when_time_passes() {
        let s = state_with(vec![timer_with_offsets("t", &[5])], vec![]);

        // Not yet due.
        let future_run = {
            let mut r = run("t", TimerStatus::Scheduled, 0, 0, &[]);
            r.schedule_time = Some(Local::now() + Duration::hours(1));
            r
        };
        s.update_state(|st| st.runs.push(future_run)).await.unwrap();
        tick(Arc::clone(&s)).await;
        assert_eq!(s.get_run("t").await.unwrap().status, TimerStatus::Scheduled);

        // Now due: flips to Running in the same tick.
        s.update_state(|st| {
            st.runs[0].schedule_time = Some(Local::now() - Duration::seconds(1));
        })
        .await
        .unwrap();
        tick(Arc::clone(&s)).await;
        let run = s.get_run("t").await.unwrap();
        assert_eq!(run.status, TimerStatus::Running);
        // Offsets count from the scheduled start, so nothing fires yet.
        assert!(run.fired_indices.is_empty());
    }

    #[tokio::test]
    async fn fires_all_due_buzzers_in_one_pass() {
        // Run started 10s ago: the 5s and 8s buzzers are due, 30s is not.
        let s = state_with(
            vec![timer_with_offsets("t", &[5, 8, 30])],
            vec![run("t", TimerStatus::Running, 10, 0, &[])],
        );
        let fired = tick(Arc::clone(&s)).await;
        assert_eq!(fired.len(), 2);
        assert!(fired.iter().all(|e| e.buzzer_name == "buzz"));
        let run = s.get_run("t").await.unwrap();
        assert_eq!(run.fired_indices, vec![0, 1]);
    }
}
