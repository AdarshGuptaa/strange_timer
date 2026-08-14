use std::collections::VecDeque;

use anyhow::{anyhow, Result};
use chrono::{Duration, Local};
use strangetimer_core::model::{
    Buzzer, BuzzerEvent, DaemonState, RepeatMode, Timer, TimerRun, TimerStatus,
};
use strangetimer_core::persistence::{save_buzzers, save_state, save_timers};
use tokio::sync::Mutex;

/// How many ringing events are kept for `strangetimer watch` (memory-only).
const MAX_EVENTS: usize = 100;

/// AppState manages the in-memory state of the daemon, providing thread-safe
/// access to timers, buzzers, and active timer runs. Every mutating method
/// persists the affected collection to disk before returning.
pub struct AppState {
    inner: Mutex<StateInner>,
    /// Synchronous mirror of `DaemonState::pending_interrupts`, readable
    /// from background threads (the looping audio playback) without async.
    pending: std::sync::Mutex<Vec<String>>,
}

pub struct StateInner {
    pub timers: Vec<Timer>,
    pub buzzers: Vec<Buzzer>,
    pub state: DaemonState,
    /// Opt-in flag for the destructive `close_windows` buzzer. Set via the
    /// `strangetimer confirm-destructive` command; not persisted.
    pub close_windows_confirmed: bool,
    /// Ringing-event log for `strangetimer watch` (memory-only, bounded).
    pub events: VecDeque<BuzzerEvent>,
    pub next_event_id: u64,
}

impl AppState {
    pub fn new(timers: Vec<Timer>, buzzers: Vec<Buzzer>, state: DaemonState) -> Self {
        Self {
            inner: Mutex::new(StateInner {
                timers,
                buzzers,
                state,
                close_windows_confirmed: false,
                events: VecDeque::new(),
                next_event_id: 0,
            }),
            pending: std::sync::Mutex::new(Vec::new()),
        }
    }

    pub async fn lock(&self) -> tokio::sync::MutexGuard<'_, StateInner> {
        self.inner.lock().await
    }

    // --- Timers ---

    /// Add a timer definition. Refuses duplicate names (timer names are the
    /// identity key used by every other command).
    pub async fn add_timer(&self, timer: Timer) -> Result<()> {
        validate_name(&timer.name)?;
        let mut inner = self.inner.lock().await;
        if inner.timers.iter().any(|t| t.name == timer.name) {
            return Err(anyhow!("a timer named {:?} already exists", timer.name));
        }
        // Reject unknown buzzer references up front instead of silently
        // swallowing the alarm at fire time.
        for buzzer_ref in &timer.buzzers {
            if !inner
                .buzzers
                .iter()
                .any(|b| b.name == buzzer_ref.buzzer_name)
            {
                return Err(anyhow!(
                    "no buzzer named {:?} — create it first with                      `strangetimer create buzzer`",
                    buzzer_ref.buzzer_name
                ));
            }
        }
        inner.timers.push(timer);
        save_timers(&inner.timers)?;
        Ok(())
    }

    /// Remove a timer definition. Refuses while a run for that timer exists
    /// (the plan's `delete timer` contract: "does not stop a running
    /// instance" — you must stop it first).
    pub async fn remove_timer(&self, name: &str) -> Result<()> {
        let mut inner = self.inner.lock().await;
        if inner.state.runs.iter().any(|r| r.timer_name == name) {
            return Err(anyhow!(
                "timer {name:?} has an active run — stop it before deleting"
            ));
        }
        inner.timers.retain(|t| t.name != name);
        save_timers(&inner.timers)?;
        Ok(())
    }

    pub async fn get_timer(&self, name: &str) -> Option<Timer> {
        let inner = self.inner.lock().await;
        inner.timers.iter().find(|t| t.name == name).cloned()
    }

    pub async fn get_timers(&self) -> Vec<Timer> {
        let inner = self.inner.lock().await;
        inner.timers.clone()
    }

    /// Clone a timer under a new name. `new_name` defaults to
    /// `<source>_copy`, incrementing the suffix (`_copy_2`, `_copy_3`, …)
    /// while the name is taken. Returns the final name.
    pub async fn duplicate_timer(&self, source: &str, new_name: Option<String>) -> Result<String> {
        let mut inner = self.inner.lock().await;
        let timer = inner
            .timers
            .iter()
            .find(|t| t.name == source)
            .cloned()
            .ok_or_else(|| anyhow!("no timer named {source:?}"))?;

        let base = new_name.unwrap_or_else(|| format!("{source}_copy"));
        let mut name = base.clone();
        let mut suffix = 2;
        while inner.timers.iter().any(|t| t.name == name) {
            name = format!("{base}_{suffix}");
            suffix += 1;
        }

        let mut copy = timer;
        copy.name = name.clone();
        inner.timers.push(copy);
        save_timers(&inner.timers)?;
        Ok(name)
    }

    // --- Buzzers ---

    /// Add a buzzer library entry. Refuses duplicate names (built-in or
    /// custom).
    pub async fn add_buzzer(&self, buzzer: Buzzer) -> Result<()> {
        validate_name(&buzzer.name)?;
        let mut inner = self.inner.lock().await;
        if inner.buzzers.iter().any(|b| b.name == buzzer.name) {
            return Err(anyhow!("a buzzer named {:?} already exists", buzzer.name));
        }
        inner.buzzers.push(buzzer);
        save_buzzers(&inner.buzzers)?;
        Ok(())
    }

    /// Remove a buzzer library entry. Refuses for built-in buzzers and for
    /// buzzers referenced by any timer definition.
    pub async fn remove_buzzer(&self, name: &str) -> Result<()> {
        let mut inner = self.inner.lock().await;
        let buzzer = inner
            .buzzers
            .iter()
            .find(|b| b.name == name)
            .ok_or_else(|| anyhow!("no buzzer named {name:?}"))?;
        if buzzer.builtin {
            return Err(anyhow!(
                "{:?} is a built-in buzzer and cannot be deleted",
                name
            ));
        }
        if inner
            .timers
            .iter()
            .any(|t| t.buzzers.iter().any(|b| b.buzzer_name == name))
        {
            return Err(anyhow!(
                "{name:?} is referenced by a timer definition and cannot be deleted"
            ));
        }
        inner.buzzers.retain(|b| b.name != name);
        save_buzzers(&inner.buzzers)?;
        Ok(())
    }

    pub async fn get_buzzer(&self, name: &str) -> Option<Buzzer> {
        let inner = self.inner.lock().await;
        inner.buzzers.iter().find(|b| b.name == name).cloned()
    }

    pub async fn get_buzzers(&self) -> Vec<Buzzer> {
        let inner = self.inner.lock().await;
        inner.buzzers.clone()
    }

    // --- Runs ---

    /// Start a run for `timer_name`. Replaces any existing run for the same
    /// timer. With `schedule_time` the run is created Scheduled (it flips to
    /// Running when the clock passes the target time); otherwise it starts
    /// Running immediately.
    pub async fn start_run(
        &self,
        timer_name: &str,
        repetitions: RepeatMode,
        schedule_time: Option<chrono::DateTime<Local>>,
        user_interrupt: bool,
        interrupt_focus: Option<String>,
    ) -> Result<TimerRun> {
        let mut inner = self.inner.lock().await;
        let timer = inner
            .timers
            .iter()
            .find(|t| t.name == timer_name)
            .cloned()
            .ok_or_else(|| anyhow!("no timer named {timer_name:?}"))?;

        let now = Local::now();
        let run = TimerRun {
            timer_name: timer.name.clone(),
            started_at: schedule_time.unwrap_or(now),
            repetitions,
            current_rep: 0,
            schedule_time,
            status: if schedule_time.is_some() {
                TimerStatus::Scheduled
            } else {
                TimerStatus::Running
            },
            paused_at: None,
            elapsed_before_pause: Duration::zero(),
            fired_indices: Vec::new(),
            user_interrupt,
            interrupt_focus,
        };

        // A timer has at most one live run: replace any previous one.
        inner.state.runs.retain(|r| r.timer_name != timer.name);
        inner.state.runs.push(run.clone());
        save_state(&inner.state)?;
        Ok(run)
    }

    pub async fn remove_run(&self, timer_name: &str) -> Result<()> {
        let mut inner = self.inner.lock().await;
        inner.state.runs.retain(|r| r.timer_name != timer_name);
        inner.state.pending_interrupts.retain(|p| p != timer_name);
        self.pending
            .lock()
            .expect("pending lock")
            .retain(|p| p != timer_name);
        save_state(&inner.state)?;
        Ok(())
    }

    pub async fn get_run(&self, timer_name: &str) -> Option<TimerRun> {
        let inner = self.inner.lock().await;
        inner
            .state
            .runs
            .iter()
            .find(|r| r.timer_name == timer_name)
            .cloned()
    }

    pub async fn pause_run(&self, timer_name: &str) -> Result<()> {
        let mut inner = self.inner.lock().await;
        let run = inner
            .state
            .runs
            .iter_mut()
            .find(|r| r.timer_name == timer_name)
            .ok_or_else(|| anyhow!("no active run found for timer: {timer_name}"))?;

        match run.status {
            TimerStatus::Running => {
                run.status = TimerStatus::Paused;
                run.paused_at = Some(Local::now());
            }
            TimerStatus::Paused => {
                return Err(anyhow!("timer {timer_name:?} is already paused"));
            }
            _ => {
                return Err(anyhow!(
                    "timer {timer_name:?} cannot be paused (status {:?})",
                    run.status
                ));
            }
        }

        save_state(&inner.state)?;
        Ok(())
    }

    pub async fn pause_all(&self) -> Result<()> {
        let mut inner = self.inner.lock().await;
        let now = Local::now();
        let mut changed = false;
        for run in inner.state.runs.iter_mut() {
            if run.status == TimerStatus::Running {
                run.status = TimerStatus::Paused;
                run.paused_at = Some(now);
                changed = true;
            }
        }
        if changed {
            save_state(&inner.state)?;
        }
        Ok(())
    }

    pub async fn resume_run(&self, timer_name: &str) -> Result<()> {
        let mut inner = self.inner.lock().await;
        let run = inner
            .state
            .runs
            .iter_mut()
            .find(|r| r.timer_name == timer_name)
            .ok_or_else(|| anyhow!("no active run found for timer: {timer_name}"))?;

        if run.status == TimerStatus::Running {
            return Err(anyhow!("timer {timer_name:?} is already running"));
        }
        if run.status != TimerStatus::Paused {
            return Err(anyhow!(
                "timer {timer_name:?} cannot be resumed (status {:?})",
                run.status
            ));
        }

        if let Some(paused_at) = run.paused_at {
            let pause_duration = Local::now() - paused_at;
            run.elapsed_before_pause += pause_duration;
        }

        run.status = TimerStatus::Running;
        run.paused_at = None;

        // Resume doubles as the user-interrupt acknowledgement: clear the
        // pending marker so any looping audio stops.
        inner.state.pending_interrupts.retain(|p| p != timer_name);
        self.pending
            .lock()
            .expect("pending lock")
            .retain(|p| p != timer_name);

        save_state(&inner.state)?;
        Ok(())
    }

    pub async fn stop_all(&self) -> Result<()> {
        let mut inner = self.inner.lock().await;
        if !inner.state.runs.is_empty() {
            inner.state.runs.clear();
            inner.state.pending_interrupts.clear();
            self.pending.lock().expect("pending lock").clear();
            save_state(&inner.state)?;
        }
        Ok(())
    }

    pub async fn get_state(&self) -> DaemonState {
        let inner = self.inner.lock().await;
        inner.state.clone()
    }

    /// Provides a way to atomically update the daemon state and persist it.
    pub async fn update_state<F>(&self, f: F) -> Result<()>
    where
        F: FnOnce(&mut DaemonState),
    {
        let mut inner = self.inner.lock().await;
        f(&mut inner.state);
        save_state(&inner.state)?;
        Ok(())
    }

    // --- Destructive-buzzer opt-in ---

    pub async fn is_close_windows_confirmed(&self) -> bool {
        let inner = self.inner.lock().await;
        inner.close_windows_confirmed
    }

    pub async fn set_close_windows_confirmed(&self) {
        let mut inner = self.inner.lock().await;
        inner.close_windows_confirmed = true;
    }

    // --- Ringing events (`strangetimer watch`) ---

    /// Push a ringing event. Bounded: the oldest events are dropped so a
    /// slow watcher never grows memory unboundedly.
    pub async fn push_event(&self, mut event: BuzzerEvent) {
        let mut inner = self.inner.lock().await;
        event.id = inner.next_event_id;
        inner.next_event_id += 1;
        inner.events.push_back(event);
        while inner.events.len() > MAX_EVENTS {
            inner.events.pop_front();
        }
    }

    /// Events with id > `after_id` (all when None), oldest first.
    pub async fn events_after(&self, after_id: Option<u64>) -> Vec<BuzzerEvent> {
        let inner = self.inner.lock().await;
        match after_id {
            Some(id) => inner.events.iter().filter(|e| e.id > id).cloned().collect(),
            None => inner.events.iter().cloned().collect(),
        }
    }

    // --- User-interrupt (`run -u`) ---

    /// Pause the run and mark it as awaiting acknowledgement. Called before
    /// the buzzer actions dispatch, so a `resume` arriving mid-dispatch
    /// clears the marker and stops any looping audio.
    pub async fn begin_interrupt(&self, timer_name: &str) -> Result<()> {
        let mut inner = self.inner.lock().await;
        let run = inner
            .state
            .runs
            .iter_mut()
            .find(|r| r.timer_name == timer_name)
            .ok_or_else(|| anyhow!("no active run found for timer: {timer_name}"))?;

        run.status = TimerStatus::Paused;
        run.paused_at = Some(Local::now());
        if !inner
            .state
            .pending_interrupts
            .iter()
            .any(|p| p == timer_name)
        {
            inner.state.pending_interrupts.push(timer_name.to_string());
        }
        let mut pending = self.pending.lock().expect("pending lock");
        if !pending.iter().any(|p| p == timer_name) {
            pending.push(timer_name.to_string());
        }
        drop(pending);
        save_state(&inner.state)?;
        Ok(())
    }

    /// Timers awaiting a user-interrupt acknowledgement.
    pub async fn interrupt_pending(&self) -> Vec<String> {
        let inner = self.inner.lock().await;
        inner.state.pending_interrupts.clone()
    }

    /// Whether `timer_name` is currently awaiting acknowledgement.
    pub async fn pending_contains(&self, timer_name: &str) -> bool {
        self.interrupt_pending()
            .await
            .iter()
            .any(|p| p == timer_name)
    }

    /// Synchronous pending query for background threads (looping audio).
    pub fn interrupt_pending_sync(&self) -> Vec<String> {
        self.pending.lock().expect("pending lock").clone()
    }
}

/// Reject names that would corrupt terminal rendering or shell quoting.
fn validate_name(name: &str) -> Result<()> {
    if name.is_empty() {
        return Err(anyhow!("name must not be empty"));
    }
    if name.chars().any(|c| c.is_control()) {
        return Err(anyhow!(
            "name {:?} contains control characters — not allowed",
            name
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use strangetimer_core::model::{BuzzerAction, BuzzerRef, RepeatMode, TimerStatus};

    fn fresh_state() -> AppState {
        crate::test_util::init();
        AppState::new(Vec::new(), Vec::new(), DaemonState::default())
    }

    fn timer(name: &str) -> Timer {
        Timer {
            name: name.to_string(),
            buzzers: vec![],
            created_at: Local::now(),
        }
    }

    fn buzzer(name: &str, builtin: bool) -> Buzzer {
        Buzzer {
            name: name.to_string(),
            actions: vec![BuzzerAction::DefaultAudio],
            builtin,
        }
    }

    #[tokio::test]
    async fn add_and_get_timer() {
        let s = fresh_state();
        s.add_timer(timer("workAndFun")).await.unwrap();
        assert_eq!(s.get_timer("workAndFun").await.unwrap().name, "workAndFun");
        assert!(s.get_timer("nope").await.is_none());
        assert_eq!(s.get_timers().await.len(), 1);
    }

    #[tokio::test]
    async fn add_timer_refuses_duplicates() {
        let s = fresh_state();
        s.add_timer(timer("t")).await.unwrap();
        assert!(s.add_timer(timer("t")).await.is_err());
    }

    #[tokio::test]
    async fn remove_timer_refused_while_run_active() {
        let s = fresh_state();
        s.add_timer(timer("t")).await.unwrap();
        s.start_run("t", RepeatMode::Count(1), None, false, None)
            .await
            .unwrap();
        assert!(s.remove_timer("t").await.is_err());
        s.remove_run("t").await.unwrap();
        s.remove_timer("t").await.unwrap();
        assert!(s.get_timer("t").await.is_none());
    }

    #[tokio::test]
    async fn duplicate_timer_default_and_suffix_names() {
        let s = fresh_state();
        s.add_timer(timer("t")).await.unwrap();
        let n1 = s.duplicate_timer("t", None).await.unwrap();
        assert_eq!(n1, "t_copy");
        let n2 = s.duplicate_timer("t", None).await.unwrap();
        assert_eq!(n2, "t_copy_2");
        let n3 = s
            .duplicate_timer("t", Some("custom".to_string()))
            .await
            .unwrap();
        assert_eq!(n3, "custom");
        assert!(s.duplicate_timer("missing", None).await.is_err());
    }

    #[tokio::test]
    async fn buzzer_guards() {
        let s = fresh_state();
        s.add_buzzer(buzzer("builtin_a", true)).await.unwrap();
        s.add_buzzer(buzzer("custom_b", false)).await.unwrap();

        // Duplicate names refused (built-in or not).
        assert!(s.add_buzzer(buzzer("builtin_a", false)).await.is_err());
        assert!(s.add_buzzer(buzzer("custom_b", false)).await.is_err());

        // Built-ins cannot be deleted.
        assert!(s.remove_buzzer("builtin_a").await.is_err());

        // Buzzers referenced by a timer cannot be deleted.
        let mut t = timer("uses_b");
        t.buzzers.push(BuzzerRef {
            offset: Duration::minutes(1),
            buzzer_name: "custom_b".to_string(),
        });
        s.add_timer(t).await.unwrap();
        assert!(s.remove_buzzer("custom_b").await.is_err());

        // Free buzzers can be deleted.
        s.add_buzzer(buzzer("freebie", false)).await.unwrap();
        s.remove_buzzer("freebie").await.unwrap();
        assert!(s.get_buzzer("freebie").await.is_none());
    }

    #[tokio::test]
    async fn start_run_immediate_and_scheduled() {
        let s = fresh_state();
        s.add_timer(timer("t")).await.unwrap();

        let run = s
            .start_run("t", RepeatMode::Count(1), None, false, None)
            .await
            .unwrap();
        assert_eq!(run.status, TimerStatus::Running);
        assert_eq!(run.current_rep, 0);
        assert!(run.fired_indices.is_empty());

        let future = Local::now() + Duration::hours(1);
        let run = s
            .start_run("t", RepeatMode::Infinite, Some(future), false, None)
            .await
            .unwrap();
        assert_eq!(run.status, TimerStatus::Scheduled);
        assert_eq!(run.schedule_time, Some(future));
        assert_eq!(s.lock().await.state.runs.len(), 1);

        assert!(s
            .start_run("missing", RepeatMode::Count(1), None, false, None)
            .await
            .is_err());
    }

    #[tokio::test]
    async fn start_run_replaces_existing_run() {
        let s = fresh_state();
        s.add_timer(timer("t")).await.unwrap();
        s.start_run("t", RepeatMode::Count(1), None, false, None)
            .await
            .unwrap();
        s.start_run("t", RepeatMode::Count(5), None, false, None)
            .await
            .unwrap();
        let runs = s.lock().await.state.runs.clone();
        assert_eq!(runs.len(), 1);
        assert!(matches!(runs[0].repetitions, RepeatMode::Count(5)));
    }

    #[tokio::test]
    async fn pause_resume_accounts_elapsed() {
        let s = fresh_state();
        s.add_timer(timer("t")).await.unwrap();
        s.start_run("t", RepeatMode::Count(1), None, false, None)
            .await
            .unwrap();

        s.pause_run("t").await.unwrap();
        let paused = s.get_run("t").await.unwrap();
        assert_eq!(paused.status, TimerStatus::Paused);
        assert!(paused.paused_at.is_some());

        // Double pause is an error.
        assert!(s.pause_run("t").await.is_err());

        // Resume adds the pause duration to elapsed_before_pause.
        std::thread::sleep(std::time::Duration::from_millis(1100));
        s.resume_run("t").await.unwrap();
        let resumed = s.get_run("t").await.unwrap();
        assert_eq!(resumed.status, TimerStatus::Running);
        assert!(resumed.paused_at.is_none());
        assert!(resumed.elapsed_before_pause >= Duration::seconds(1));

        // Resuming a running run is an error.
        assert!(s.resume_run("t").await.is_err());
    }

    #[tokio::test]
    async fn pause_all_and_stop_all() {
        let s = fresh_state();
        s.add_timer(timer("a")).await.unwrap();
        s.add_timer(timer("b")).await.unwrap();
        s.start_run("a", RepeatMode::Count(1), None, false, None)
            .await
            .unwrap();
        s.start_run("b", RepeatMode::Count(1), None, false, None)
            .await
            .unwrap();

        s.pause_all().await.unwrap();
        for run in s.lock().await.state.runs.clone() {
            assert_eq!(run.status, TimerStatus::Paused);
        }

        s.stop_all().await.unwrap();
        assert!(s.lock().await.state.runs.is_empty());
    }

    #[tokio::test]
    async fn close_windows_confirmation_flag() {
        let s = fresh_state();
        assert!(!s.is_close_windows_confirmed().await);
        s.set_close_windows_confirmed().await;
        assert!(s.is_close_windows_confirmed().await);
    }

    #[tokio::test]
    async fn add_timer_validates_buzzer_names() {
        let s = fresh_state();
        let mut t = timer("t");
        t.buzzers.push(BuzzerRef {
            offset: Duration::minutes(1),
            buzzer_name: "no_such_buzzer".to_string(),
        });
        let err = s.add_timer(t).await.unwrap_err().to_string();
        assert!(err.contains("no buzzer named"), "{err}");
    }

    #[tokio::test]
    async fn names_with_control_characters_are_rejected() {
        let s = fresh_state();
        let t = timer("bad\nname");
        let err = s.add_timer(t.clone()).await.unwrap_err().to_string();
        assert!(err.contains("control characters"), "{err}");
        let err = s
            .add_buzzer(Buzzer {
                name: "bad\x1b[31m".to_string(),
                actions: vec![BuzzerAction::DefaultAudio],
                builtin: false,
            })
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("control characters"), "{err}");
    }

    #[tokio::test]
    async fn user_interrupt_begin_pauses_and_resume_clears() {
        let s = fresh_state();
        s.add_timer(timer("t")).await.unwrap();
        s.start_run("t", RepeatMode::Count(1), None, true, Some("term".into()))
            .await
            .unwrap();

        assert!(s.interrupt_pending().await.is_empty());
        s.begin_interrupt("t").await.unwrap();

        let run = s.get_run("t").await.unwrap();
        assert_eq!(run.status, TimerStatus::Paused);
        assert_eq!(s.interrupt_pending().await, vec!["t".to_string()]);
        assert_eq!(s.interrupt_pending_sync(), vec!["t".to_string()]);
        assert!(s.pending_contains("t").await);

        // Resume doubles as the acknowledgement.
        s.resume_run("t").await.unwrap();
        assert!(s.interrupt_pending().await.is_empty());
        assert!(s.interrupt_pending_sync().is_empty());
        assert!(!s.pending_contains("t").await);
        assert_eq!(s.get_run("t").await.unwrap().status, TimerStatus::Running);
    }

    #[tokio::test]
    async fn stop_and_remove_run_clear_pending() {
        let s = fresh_state();
        s.add_timer(timer("t")).await.unwrap();
        s.start_run("t", RepeatMode::Count(1), None, true, None)
            .await
            .unwrap();
        s.begin_interrupt("t").await.unwrap();

        s.remove_run("t").await.unwrap();
        assert!(s.interrupt_pending().await.is_empty());

        s.start_run("t", RepeatMode::Count(1), None, true, None)
            .await
            .unwrap();
        s.begin_interrupt("t").await.unwrap();
        s.stop_all().await.unwrap();
        assert!(s.interrupt_pending().await.is_empty());
    }
}
