use anyhow::{Result, anyhow};
use chrono::{Duration, Local};
use strangetimer_core::model::{Buzzer, DaemonState, RepeatMode, Timer, TimerRun, TimerStatus};
use strangetimer_core::persistence::{save_buzzers, save_state, save_timers};
use tokio::sync::Mutex;

/// AppState manages the in-memory state of the daemon, providing thread-safe
/// access to timers, buzzers, and active timer runs. Every mutating method
/// persists the affected collection to disk before returning.
pub struct AppState {
    inner: Mutex<StateInner>,
}

pub struct StateInner {
    pub timers: Vec<Timer>,
    pub buzzers: Vec<Buzzer>,
    pub state: DaemonState,
    /// Opt-in flag for the destructive `close_windows` buzzer. Set via the
    /// `strangetimer confirm-destructive` command; not persisted.
    pub close_windows_confirmed: bool,
}

impl AppState {
    pub fn new(timers: Vec<Timer>, buzzers: Vec<Buzzer>, state: DaemonState) -> Self {
        Self {
            inner: Mutex::new(StateInner {
                timers,
                buzzers,
                state,
                close_windows_confirmed: false,
            }),
        }
    }

    pub async fn lock(&self) -> tokio::sync::MutexGuard<'_, StateInner> {
        self.inner.lock().await
    }

    // --- Timers ---

    /// Add a timer definition. Refuses duplicate names (timer names are the
    /// identity key used by every other command).
    pub async fn add_timer(&self, timer: Timer) -> Result<()> {
        let mut inner = self.inner.lock().await;
        if inner.timers.iter().any(|t| t.name == timer.name) {
            return Err(anyhow!("a timer named {:?} already exists", timer.name));
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
            return Err(anyhow!("{:?} is a built-in buzzer and cannot be deleted", name));
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
            run.elapsed_before_pause = run.elapsed_before_pause + pause_duration;
        }

        run.status = TimerStatus::Running;
        run.paused_at = None;

        save_state(&inner.state)?;
        Ok(())
    }

    pub async fn stop_all(&self) -> Result<()> {
        let mut inner = self.inner.lock().await;
        if !inner.state.runs.is_empty() {
            inner.state.runs.clear();
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
}
