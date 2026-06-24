use std::sync::Arc;
use tokio::sync::Mutex;
use anyhow::{Result, anyhow};
use chrono::Local;
use strangetimer_core::model::{Timer, Buzzer, TimerRun, TimerStatus, DaemonState};
use strangetimer_core::persistence::{save_timers, save_buzzers, save_state};

/// AppState manages the in-memory state of the daemon, providing thread-safe
/// access to timers, buzzers, and active timer runs.
pub struct AppState {
    inner: Mutex<StateInner>,
}

struct StateInner {
    timers: Vec<Timer>,
    buzzers: Vec<Buzzer>,
    state: DaemonState,
}

impl AppState {
    pub fn new(timers: Vec<Timer>, buzzers: Vec<Buzzer>, state: DaemonState) -> Self {
        Self {
            inner: Mutex::new(StateInner {
                timers,
                buzzers,
                state,
            }),
        }
    }

    // --- Timers ---

    pub async fn add_timer(&self, timer: Timer) -> Result<()> {
        let mut inner = self.inner.lock().await;
        inner.timers.push(timer);
        save_timers(&inner.timers)?;
        Ok(())
    }

    pub async fn remove_timer(&self, name: &str) -> Result<()> {
        let mut inner = self.inner.lock().await;
        inner.timers.retain(|t| t.name != name);
        save_timers(&inner.timers)?;
        Ok(())
    }

    pub async fn get_timer(&self, name: &str) -> Option<Timer> {
        let inner = self.inner.lock().await;
        inner.timers.iter().find(|t| t.name == name).cloned()
    }

    // --- Buzzers ---

    pub async fn add_buzzer(&self, buzzer: Buzzer) -> Result<()> {
        let mut inner = self.inner.lock().await;
        inner.buzzers.push(buzzer);
        save_buzzers(&inner.buzzers)?;
        Ok(())
    }

    pub async fn remove_buzzer(&self, name: &str) -> Result<()> {
        let mut inner = self.inner.lock().await;
        inner.buzzers.retain(|b| b.name != name);
        save_buzzers(&inner.buzzers)?;
        Ok(())
    }

    pub async fn get_buzzer(&self, name: &str) -> Option<Buzzer> {
        let inner = self.inner.lock().await;
        inner.buzzers.iter().find(|b| b.name == name).cloned()
    }

    // --- Runs ---

    pub async fn add_run(&self, run: TimerRun) -> Result<()> {
        let mut inner = self.inner.lock().await;
        inner.state.runs.push(run);
        save_state(&inner.state)?;
        Ok(())
    }

    pub async fn remove_run(&self, timer_name: &str) -> Result<()> {
        let mut inner = self.inner.lock().await;
        inner.state.runs.retain(|r| r.timer_name != timer_name);
        save_state(&inner.state)?;
        Ok(())
    }

    pub async fn get_run(&self, timer_name: &str) -> Option<TimerRun> {
        let inner = self.inner.lock().await;
        inner.state.runs.iter().find(|r| r.timer_name == timer_name).cloned()
    }

    pub async fn pause_run(&self, timer_name: &str) -> Result<()> {
        let mut inner = self.inner.lock().await;
        let run = inner.state.runs.iter_mut()
            .find(|r| r.timer_name == timer_name)
            .ok_or_else(|| anyhow!("no active run found for timer: {timer_name}"))?;

        run.status = TimerStatus::Paused;
        run.paused_at = Some(Local::now());

        save_state(&inner.state)?;
        Ok(())
    }

    pub async fn resume_run(&self, timer_name: &str) -> Result<()> {
        let mut inner = self.inner.lock().await;
        let run = inner.state.runs.iter_mut()
            .find(|r| r.timer_name == timer_name)
            .ok_or_else(|| anyhow!("no active run found for timer: {timer_name}"))?;

        if let Some(paused_at) = run.paused_at {
            let pause_duration = Local::now() - paused_at;
            run.elapsed_before_pause = run.elapsed_before_pause + pause_duration;
        }

        run.status = TimerStatus::Running;
        run.paused_at = None;

        save_state(&inner.state)?;
        Ok(())
    }

    /// Returns a clone of the current daemon state.
    pub async fn get_state(&self) -> DaemonState {
        let inner = self.inner.lock().await;
        inner.state.clone()
    }
}
