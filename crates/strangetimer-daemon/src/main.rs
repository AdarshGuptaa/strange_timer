use std::sync::Arc;

#[macro_use]
mod log;
mod buzzers;
mod platform;
mod scheduler;
mod state;

use anyhow::{Context, Result};
use interprocess::local_socket::traits::tokio::Listener as TokioListener;
use strangetimer_core::ipc::socket_name;
use strangetimer_core::ipc::{ClientMessage, ClientRequest, ServerMessage};
use strangetimer_core::model::{
    Buzzer, BuzzerAction, BuzzerEvent, FireEvent, RepeatMode, TimerStatus,
};
use strangetimer_core::persistence::{
    load_buzzers, load_state, load_timers, save_buzzers, save_state,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::mpsc;

use state::AppState;

#[tokio::main]
async fn main() -> Result<()> {
    log::init();

    // 1. Load timers, buzzers, and state from persistence (or initialise
    //    fresh). The persistence layer writes default files on first call.
    let timers = load_timers().context("failed to load timers")?;
    let mut buzzers = load_buzzers().context("failed to load buzzers")?;
    let state = load_state().context("failed to load state")?;

    // 2. Seed the built-in buzzer library on first run.
    if buzzers.is_empty() {
        buzzers = builtin_buzzers();
        save_buzzers(&buzzers).context("failed to persist seeded buzzers")?;
        info!("seeded built-in buzzer library");
    }

    info!(
        "loaded {} timers, {} buzzers, {} active runs",
        timers.len(),
        buzzers.len(),
        state.runs.len(),
    );

    let app_state = Arc::new(AppState::new(timers, buzzers, state));

    // 3. Autostart registration (Prompt 21): register once, remember forever.
    if !app_state.get_state().await.registered {
        match platform::register_autostart() {
            Ok(()) => {
                app_state
                    .update_state(|s| s.registered = true)
                    .await
                    .context("failed to persist autostart flag")?;
                info!("registered for autostart.");
            }
            Err(e) => {
                warn!("autostart registration failed: {e:#}");
            }
        }
    }

    // 4. Buzzer channel + scheduler + buzzer dispatcher tasks. The channel
    //    carries structured FireEvents — the run context and repetition
    //    matter for `run -u` handling and for tests.
    let (buzzer_tx, mut buzzer_rx) = mpsc::channel::<FireEvent>(100);

    let scheduler_state = Arc::clone(&app_state);
    let scheduler_tx = buzzer_tx.clone();
    tokio::spawn(async move {
        scheduler::run_scheduler(scheduler_state, scheduler_tx).await;
    });

    let fire_state = Arc::clone(&app_state);
    tokio::spawn(async move {
        while let Some(event) = buzzer_rx.recv().await {
            fire_buzzer(&event, Arc::clone(&fire_state)).await;
            // Acknowledge the outbox entry so a restart never replays an
            // already-dispatched alarm.
            fire_state.remove_pending_fire(&event).await;
        }
    });

    // 5. Restart recovery (Prompt 20): runs that were active when the daemon
    //    last stopped fire any alarms that were missed during downtime, and
    //    scheduled runs whose time has passed start immediately.
    recover_runs(Arc::clone(&app_state), &buzzer_tx).await;

    // Replay any fired-but-undispatched events from the durable outbox
    // (a crash between scheduling and dispatch must not lose an alarm).
    for event in app_state.pending_fires().await {
        info!("replaying pending fire for {:?}", event.buzzer_name);
        if buzzer_tx.send(event).await.is_err() {
            break;
        }
    }

    // 6. Bind the IPC listener and serve until a shutdown signal arrives.
    let listener = bind_listener(&socket_name()).context("failed to bind IPC listener")?;
    info!("listening on {}", socket_name());

    // `shutdown` lets an IPC `Shutdown` request (from `strangetimer daemon
    // stop`) tear the accept loop down just like a signal would.
    let shutdown = Arc::new(tokio::sync::Notify::new());

    tokio::select! {
        result = accept_loop(listener, Arc::clone(&app_state), Arc::clone(&shutdown)) => {
            if let Err(e) = result {
                warn!("accept loop exited with error: {e:#}");
            }
        }
        _ = shutdown_signal() => {
            info!("shutdown signal received");
        }
    }

    // 7. Save state before exiting.
    let final_state = app_state.get_state().await;
    if let Err(e) = save_state(&final_state) {
        warn!("failed to save state on shutdown: {e:#}");
    } else {
        info!("state saved");
    }

    Ok(())
}

/// The three built-in buzzers shipped with every install (Prompt 19).
fn builtin_buzzers() -> Vec<Buzzer> {
    vec![
        Buzzer {
            name: "default_audio".to_string(),
            actions: vec![BuzzerAction::DefaultAudio],
            builtin: true,
        },
        Buzzer {
            name: "default_video".to_string(),
            actions: vec![BuzzerAction::DefaultVideo],
            builtin: true,
        },
        Buzzer {
            name: "close_windows".to_string(),
            actions: vec![BuzzerAction::CloseAllWindows],
            builtin: true,
        },
    ]
}

/// Look up a buzzer by name and fire every action it chains. Unknown names
/// degrade to a logged beep so silent failures are easy to spot.
///
/// For `run -u` (user-interrupt) runs the fire path changes: the run is
/// paused and marked pending *before* the actions dispatch, audio actions
/// loop until the acknowledgement (`resume`), and afterwards the terminal
/// captured at run time is focused.
async fn fire_buzzer(event: &FireEvent, state: Arc<AppState>) {
    let name = &event.buzzer_name;
    let timer_name = &event.timer_name;
    let Some(buzzer) = state.get_buzzer(name).await else {
        warn!("BUZZ: {name} — no such buzzer (was it deleted?)");
        return;
    };

    // A user-interrupt run that is already awaiting acknowledgement skips
    // further buzzers fired at the same moment (they were marked fired).
    if state.pending_contains(timer_name).await {
        info!("BUZZ: {name} — run {timer_name:?} is awaiting user interrupt; skipping");
        return;
    }

    info!("BUZZ: {name} (rep {})", event.repetition);

    let run = state.get_run(timer_name).await;
    let requires_ack = run.as_ref().is_some_and(|r| r.user_interrupt);
    let mut dispatch_issues: Vec<String> = Vec::new();
    if requires_ack {
        if let Err(e) = state.begin_interrupt(timer_name).await {
            warn!("failed to begin user interrupt for {timer_name:?}: {e:#}");
        }
        buzzers::dispatch_interrupt(&state, &buzzer.actions, timer_name).await;
    } else {
        for action in &buzzer.actions {
            if let Some(issue) = buzzers::dispatch(&state, action).await {
                dispatch_issues.push(issue);
            }
        }
    }

    // Announce the ringing so `strangetimer watch` can print it, including
    // why an action was blocked or failed to launch.
    let types: Vec<String> = buzzer
        .actions
        .iter()
        .map(|a| a.label().to_string())
        .collect();
    let mut outcome = {
        let confirmed = state.is_close_windows_confirmed().await;
        let has_legacy = buzzer
            .actions
            .iter()
            .any(|a| matches!(a, BuzzerAction::CloseAllWindows));
        let has_close = buzzer.actions.iter().any(|a| {
            matches!(
                a,
                BuzzerAction::CloseWindow(_) | BuzzerAction::CloseApplication(_)
            )
        });
        if has_legacy {
            Some(
                "blocked: close_windows is deprecated — use --close-window or --close-app"
                    .to_string(),
            )
        } else if has_close && !confirmed {
            Some("blocked: requires `strangetimer confirm-destructive`".to_string())
        } else if requires_ack {
            Some(format!(
                "awaiting acknowledgement: strangetimer resume {timer_name}"
            ))
        } else {
            None
        }
    };
    // A launch failure (e.g. the opener could not reach the session's
    // display) must not vanish into the log: surface it to `watch`.
    if outcome.is_none() && !dispatch_issues.is_empty() {
        outcome = Some(dispatch_issues.join("; "));
    }
    state
        .push_event(BuzzerEvent {
            id: 0, // assigned by the daemon
            timer_name: timer_name.to_string(),
            buzzer_name: name.clone(),
            buzzer_types: types,
            fired_at: chrono::Local::now(),
            repetition: event.repetition,
            requires_ack,
            outcome,
        })
        .await;
}

/// Fire alarms that became due while the daemon was stopped, and promote
/// Scheduled runs whose start time has already passed.
async fn recover_runs(state: Arc<AppState>, buzzer_tx: &mpsc::Sender<FireEvent>) {
    let now = chrono::Local::now();
    let mut missed: Vec<FireEvent> = Vec::new();
    let mut state_changed = false;

    {
        let mut inner = state.lock().await;

        // Snapshot timer definitions first so we don't borrow `inner`
        // immutably while iterating its runs mutably.
        let timers: Vec<strangetimer_core::model::Timer> = inner.timers.clone();

        for run in inner.state.runs.iter_mut() {
            match run.status {
                TimerStatus::Running => {
                    // Effective elapsed time (pause-shifted timeline).
                    let mut elapsed = (now - run.started_at) - run.elapsed_before_pause;

                    let Some(timer) = timers.iter().find(|t| t.name == run.timer_name) else {
                        continue;
                    };

                    let rep_len = timer
                        .buzzers
                        .iter()
                        .map(|b| b.offset)
                        .max()
                        .unwrap_or(chrono::Duration::zero());

                    // Catch up every fully-elapsed repetition, then mark
                    // the buzzers due in the current one. A Count(3) timer
                    // down for three periods fires all three alarms.
                    loop {
                        let all_fired = run.fired_indices.len() == timer.buzzers.len();
                        let rep_done = rep_len > chrono::Duration::zero()
                            && elapsed >= rep_len
                            && all_fired
                            && (matches!(run.repetitions, RepeatMode::Infinite)
                                || match run.repetitions {
                                    RepeatMode::Count(n) => run.current_rep + 1 < n,
                                    RepeatMode::Infinite => true,
                                });
                        if rep_done {
                            run.current_rep += 1;
                            run.fired_indices.clear();
                            run.elapsed_before_pause = chrono::Duration::zero();
                            elapsed -= rep_len;
                            state_changed = true;
                            continue;
                        }

                        for (idx, buzzer_ref) in timer.buzzers.iter().enumerate() {
                            if run.fired_indices.contains(&idx) {
                                continue;
                            }
                            if elapsed >= buzzer_ref.offset {
                                run.fired_indices.push(idx);
                                missed.push(FireEvent {
                                    timer_name: run.timer_name.clone(),
                                    buzzer_name: buzzer_ref.buzzer_name.clone(),
                                    buzzer_index: idx,
                                    repetition: run.current_rep,
                                });
                                state_changed = true;
                            }
                        }
                        break;
                    }
                }
                TimerStatus::Scheduled => {
                    if let Some(schedule_time) = run.schedule_time {
                        if now >= schedule_time {
                            run.status = TimerStatus::Running;
                            state_changed = true;
                        }
                    }
                }
                _ => {}
            }
        }

        if state_changed {
            if let Err(e) = save_state(&inner.state) {
                warn!("recovery failed to save state: {e:#}");
            }
        }
    }

    // Fire missed alarms immediately — dispatch happens in the fire task.
    // Persist each event to the durable outbox first (mirroring the
    // scheduler path), so a crash between this send and the fire task's
    // dispatch replays the alarm on the next boot instead of losing it.
    for event in missed {
        info!(
            "firing missed alarm for {:?} ({:?}) from downtime",
            event.buzzer_name, event.timer_name
        );
        state.add_pending_fire(event.clone()).await;
        if let Err(e) = buzzer_tx.send(event).await {
            warn!("recovery buzzer channel error: {e}");
            break;
        }
    }
}

/// Bind a tokio-flavoured interprocess `Listener` on `name`.
///
/// On Unix this wraps a filesystem path; on Windows it's a named pipe name.
fn bind_listener(name: &str) -> Result<interprocess::local_socket::tokio::Listener> {
    #[cfg(unix)]
    use interprocess::local_socket::{GenericFilePath, ListenerOptions, ToFsName};
    #[cfg(windows)]
    use interprocess::local_socket::{GenericNamespaced, ListenerOptions, ToNsName};

    #[cfg(unix)]
    {
        // A Unix-domain socket leaves a stale file behind when the daemon
        // dies uncleanly (e.g. SIGKILL). If nothing is actually listening
        // there, remove the file so we can rebind; if something IS listening,
        // keep the error below — another daemon owns the endpoint.
        let path = std::path::Path::new(name);
        if path.exists() {
            let alive = std::os::unix::net::UnixStream::connect(path).is_ok();
            if !alive {
                std::fs::remove_file(path)
                    .with_context(|| format!("failed to remove stale socket {}", path.display()))?;
                info!("removed stale socket {}", path.display());
            }
        }
    }

    #[cfg(unix)]
    let name = name
        .to_fs_name::<GenericFilePath>()
        .with_context(|| format!("invalid socket name: {name}"))?;
    #[cfg(windows)]
    let name = name
        .to_ns_name::<GenericNamespaced>()
        .with_context(|| format!("invalid pipe name: {name}"))?;

    ListenerOptions::new()
        .name(name)
        .create_tokio()
        .context("failed to create tokio listener")
}

async fn accept_loop(
    listener: interprocess::local_socket::tokio::Listener,
    app_state: Arc<AppState>,
    shutdown: Arc<tokio::sync::Notify>,
) -> Result<()> {
    loop {
        tokio::select! {
            _ = shutdown.notified() => {
                // An IPC Shutdown request wants the daemon to stop serving.
                // Return so `main` runs the normal save-and-exit teardown.
                return Ok(());
            }
            conn = listener.accept() => {
                let conn = conn.context("accept failed")?;
                let state = Arc::clone(&app_state);
                let shutdown = Arc::clone(&shutdown);
                tokio::spawn(async move {
                    if let Err(e) = handle_connection(conn, state, shutdown).await {
                        warn!("connection error: {e:#}");
                    }
                });
            }
        }
    }
}

/// Handle a single client connection: read one `ClientRequest` (the wire
/// envelope carrying the sender's session environment plus the command),
/// apply it to `AppState`, and reply with the matching `ServerMessage`.
async fn handle_connection(
    mut conn: interprocess::local_socket::tokio::Stream,
    app_state: Arc<AppState>,
    shutdown: Arc<tokio::sync::Notify>,
) -> Result<()> {
    let request: ClientRequest = read_message_async(&mut conn)
        .await
        .context("failed to read ClientRequest")?;

    // Refresh the launch environment from the sender's current session so
    // GUI-side buzzer actions (video, URL, focus) never run against a stale
    // display/session. Done before dispatch; ignored when the sender is
    // headless (empty snapshot).
    app_state.set_session_env(request.env).await;

    let msg = request.msg;
    debug!("received ClientMessage::{}", variant_name(&msg));

    // Shutdown is special: it is answered before the normal handler so the
    // client gets its Ok even though the accept loop is about to stop.
    if matches!(msg, ClientMessage::Shutdown) {
        write_message_async(&mut conn, &ServerMessage::Ok)
            .await
            .context("failed to write ServerMessage")?;
        info!("graceful shutdown requested over IPC");
        shutdown.notify_one();
        return Ok(());
    }

    let response = handle_message(msg, Arc::clone(&app_state)).await;

    write_message_async(&mut conn, &response)
        .await
        .context("failed to write ServerMessage")?;

    Ok(())
}

/// Apply a client message to the daemon state and produce the response.
async fn handle_message(msg: ClientMessage, state: Arc<AppState>) -> ServerMessage {
    match msg {
        ClientMessage::CreateTimer {
            timer,
            replace,
            stop_running,
        } => reply(state.add_timer_options(timer, replace, stop_running).await),
        ClientMessage::DuplicateTimer { source, new_name } => {
            match state.duplicate_timer(&source, new_name).await {
                Ok(name) => {
                    info!("duplicated {source:?} as {name:?}");
                    ServerMessage::DuplicateTimerOk { name }
                }
                Err(e) => ServerMessage::Error(e.to_string()),
            }
        }
        ClientMessage::DeleteTimer { name } => reply(state.remove_timer(&name).await),
        ClientMessage::CreateBuzzer { buzzer } => reply(state.add_buzzer(buzzer).await),
        ClientMessage::DeleteBuzzer { name } => reply(state.remove_buzzer(&name).await),
        ClientMessage::RunTimer {
            name,
            repeat,
            schedule_time,
            user_interrupt,
            interrupt_focus,
        } => match state
            .start_run(
                &name,
                repeat,
                schedule_time,
                user_interrupt,
                interrupt_focus,
            )
            .await
        {
            Ok(run) => {
                info!("run started for {name:?} ({:?})", run.status);
                ServerMessage::Ok
            }
            Err(e) => ServerMessage::Error(e.to_string()),
        },
        ClientMessage::Pause { name } => reply(state.pause_run(&name).await),
        ClientMessage::PauseAll => reply(state.pause_all().await),
        ClientMessage::Resume { name } => reply(state.resume_run(&name).await),
        ClientMessage::Stop { name } => reply(state.remove_run(&name).await),
        ClientMessage::StopAll => reply(state.stop_all().await),
        ClientMessage::GetTimers => {
            let pending = state.interrupt_pending().await;
            ServerMessage::TimerList {
                timers: state.get_timers().await,
                runs: state.lock().await.state.runs.clone(),
                interrupt_pending: pending.first().cloned(),
                pending_interrupts: pending,
            }
        }
        ClientMessage::GetTimer { name } => match state.get_timer(&name).await {
            Some(timer) => {
                let runs = match state.get_run(&name).await {
                    Some(run) => vec![run],
                    None => Vec::new(),
                };
                let pending = state.interrupt_pending().await;
                ServerMessage::TimerDetail {
                    timer,
                    runs,
                    interrupt_pending: pending.first().cloned(),
                    pending_interrupts: pending,
                }
            }
            None => ServerMessage::Error(format!("no timer named {name:?}")),
        },
        ClientMessage::GetBuzzers => ServerMessage::BuzzerList(state.get_buzzers().await),
        ClientMessage::GetBuzzerInfo => ServerMessage::BuzzerInfoList(state.buzzer_infos().await),
        ClientMessage::GetBuzzerDetail { name } => match state.buzzer_detail(&name).await {
            Some(detail) => ServerMessage::BuzzerDetailInfo(detail),
            None => ServerMessage::Error(format!("no buzzer named {name:?}")),
        },
        ClientMessage::DeleteBuzzerCascade { name } => {
            match state.delete_buzzer_cascade(&name).await {
                Ok(timers) => {
                    info!(
                        "cascade-deleted buzzer {name:?} and {} timers",
                        timers.len()
                    );
                    ServerMessage::Ok
                }
                Err(e) => ServerMessage::Error(e.to_string()),
            }
        }
        ClientMessage::ConfirmDestructive => {
            state.set_close_windows_confirmed().await;
            ServerMessage::Ok
        }
        ClientMessage::RevokeDestructive => match state.revoke_close_windows_confirmed().await {
            Ok(()) => {
                info!("destructive-buzzer opt-in revoked");
                ServerMessage::Ok
            }
            Err(e) => ServerMessage::Error(e.to_string()),
        },
        ClientMessage::EnableAutostart => match platform::register_autostart() {
            Ok(()) => {
                state
                    .update_state(|s| s.registered = true)
                    .await
                    .unwrap_or_else(|e| warn!("failed to persist autostart flag: {e:#}"));
                ServerMessage::Ok
            }
            Err(e) => ServerMessage::Error(e.to_string()),
        },
        ClientMessage::DisableAutostart => match platform::disable_autostart() {
            Ok(()) => {
                state
                    .update_state(|s| s.registered = false)
                    .await
                    .unwrap_or_else(|e| warn!("failed to persist autostart flag: {e:#}"));
                ServerMessage::Ok
            }
            Err(e) => ServerMessage::Error(e.to_string()),
        },
        ClientMessage::UninstallService => {
            let _ = platform::disable_autostart();
            state
                .update_state(|s| s.registered = false)
                .await
                .unwrap_or_else(|e| warn!("failed to persist autostart flag: {e:#}"));
            ServerMessage::Ok
        }
        ClientMessage::Ping => ServerMessage::Status {
            pid: std::process::id(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            protocol: strangetimer_core::ipc::IPC_PROTOCOL_VERSION,
        },
        ClientMessage::GetEvents { after_id } => {
            ServerMessage::BuzzerEvents(state.events_after(after_id).await)
        }
        ClientMessage::Shutdown => {
            // Handled in `handle_connection` before dispatch; this arm is
            // unreachable but kept exhaustive.
            unreachable!("Shutdown is intercepted in handle_connection")
        }
    }
}

/// Map a state-operation result onto the wire protocol.
fn reply(result: Result<()>) -> ServerMessage {
    match result {
        Ok(()) => ServerMessage::Ok,
        Err(e) => ServerMessage::Error(e.to_string()),
    }
}

/// Return a short, human-readable name for the `ClientMessage` variant.
fn variant_name(msg: &ClientMessage) -> &'static str {
    match msg {
        ClientMessage::CreateTimer { .. } => "CreateTimer",
        ClientMessage::DuplicateTimer { .. } => "DuplicateTimer",
        ClientMessage::DeleteTimer { .. } => "DeleteTimer",
        ClientMessage::CreateBuzzer { .. } => "CreateBuzzer",
        ClientMessage::DeleteBuzzer { .. } => "DeleteBuzzer",
        ClientMessage::RunTimer { .. } => "RunTimer",
        ClientMessage::Pause { .. } => "Pause",
        ClientMessage::PauseAll => "PauseAll",
        ClientMessage::Resume { .. } => "Resume",
        ClientMessage::Stop { .. } => "Stop",
        ClientMessage::StopAll => "StopAll",
        ClientMessage::GetTimers => "GetTimers",
        ClientMessage::GetTimer { .. } => "GetTimer",
        ClientMessage::GetBuzzers => "GetBuzzers",
        ClientMessage::GetBuzzerInfo => "GetBuzzerInfo",
        ClientMessage::GetBuzzerDetail { .. } => "GetBuzzerDetail",
        ClientMessage::DeleteBuzzerCascade { .. } => "DeleteBuzzerCascade",
        ClientMessage::ConfirmDestructive => "ConfirmDestructive",
        ClientMessage::RevokeDestructive => "RevokeDestructive",
        ClientMessage::EnableAutostart => "EnableAutostart",
        ClientMessage::DisableAutostart => "DisableAutostart",
        ClientMessage::UninstallService => "UninstallService",
        ClientMessage::Ping => "Ping",
        ClientMessage::Shutdown => "Shutdown",
        ClientMessage::GetEvents { .. } => "GetEvents",
    }
}

/// Async version of `strangetimer_core::ipc::read_message`. Reads a
/// length-prefixed JSON frame from a tokio async reader.
async fn read_message_async<R>(reader: &mut R) -> Result<ClientRequest>
where
    R: AsyncReadExt + Unpin,
{
    let mut len_bytes = [0u8; 4];
    reader
        .read_exact(&mut len_bytes)
        .await
        .context("failed to read IPC length prefix")?;
    let len = u32::from_be_bytes(len_bytes) as usize;

    // Mirror the sanity cap from the sync version.
    const MAX_PAYLOAD: usize = 64 * 1024 * 1024;
    if len > MAX_PAYLOAD {
        anyhow::bail!("IPC payload length {len} exceeds sanity cap of {MAX_PAYLOAD} bytes");
    }

    let mut payload = vec![0u8; len];
    reader
        .read_exact(&mut payload)
        .await
        .with_context(|| format!("failed to read IPC payload of {len} bytes"))?;

    serde_json::from_slice(&payload)
        .with_context(|| format!("failed to parse IPC payload of {len} bytes"))
}

/// Async version of `strangetimer_core::ipc::write_message`. Writes a
/// length-prefixed JSON frame to a tokio async writer.
async fn write_message_async<W, T>(writer: &mut W, msg: &T) -> Result<()>
where
    W: AsyncWriteExt + Unpin,
    T: serde::Serialize,
{
    let payload = serde_json::to_vec(msg).context("failed to serialise IPC message")?;
    let len = u32::try_from(payload.len()).context("IPC payload exceeds u32::MAX bytes")?;
    writer
        .write_all(&len.to_be_bytes())
        .await
        .context("failed to write IPC length prefix")?;
    writer
        .write_all(&payload)
        .await
        .context("failed to write IPC payload")?;
    writer.flush().await.context("failed to flush IPC stream")?;
    Ok(())
}

/// Wait for a shutdown signal: SIGINT / SIGTERM on unix, Ctrl+C on Windows.
async fn shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        let mut sigint = signal(SignalKind::interrupt()).expect("failed to install SIGINT handler");
        let mut sigterm =
            signal(SignalKind::terminate()).expect("failed to install SIGTERM handler");
        tokio::select! {
            _ = sigint.recv() => {}
            _ = sigterm.recv() => {}
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

#[cfg(test)]
mod test_util {
    use std::sync::Once;

    /// Point `STRANGETIMER_DATA_DIR` at one process-unique temp directory,
    /// shared by every test module so they never race each other.
    static ONCE: Once = Once::new();

    pub fn init() {
        ONCE.call_once(|| {
            let dir = std::env::temp_dir()
                .join(format!("strangetimer-daemon-test-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            std::env::set_var("STRANGETIMER_DATA_DIR", &dir);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use strangetimer_core::model::{BuzzerRef, DaemonState, Timer, TimerRun, TimerStatus};

    /// Build a state with one timer whose buzzer fires at `offset_secs`
    /// and a Running run that started `started_secs_ago` seconds ago.
    fn overdue_state(offset_secs: i64, started_secs_ago: i64) -> Arc<AppState> {
        crate::test_util::init();
        Arc::new(AppState::new(
            vec![Timer {
                name: "t".to_string(),
                buzzers: vec![BuzzerRef {
                    offset: chrono::Duration::seconds(offset_secs),
                    buzzer_name: "b".to_string(),
                }],
                created_at: chrono::Local::now(),
            }],
            vec![Buzzer {
                name: "b".to_string(),
                actions: vec![BuzzerAction::DefaultAudio],
                builtin: false,
            }],
            DaemonState {
                runs: vec![TimerRun {
                    timer_name: "t".to_string(),
                    started_at: chrono::Local::now() - chrono::Duration::seconds(started_secs_ago),
                    repetitions: RepeatMode::Count(1),
                    current_rep: 0,
                    schedule_time: None,
                    status: TimerStatus::Running,
                    paused_at: None,
                    elapsed_before_pause: chrono::Duration::zero(),
                    fired_indices: Vec::new(),
                    user_interrupt: false,
                    interrupt_focus: None,
                }],
                registered: true,
                last_saved_at: None,
                interrupt_pending: None,
                pending_interrupts: Vec::new(),
                pending_fires: Vec::new(),
                session_env: Default::default(),
                close_windows_confirmed: false,
            },
        ))
    }

    /// Recovery must persist each missed alarm to the durable outbox
    /// BEFORE handing it to the fire channel — a crash between recovery
    /// and dispatch replays the alarm on the next boot instead of losing
    /// it (the scheduler path already does this; recovery now matches).
    #[tokio::test]
    async fn recovery_persists_missed_events_to_outbox() {
        let state = overdue_state(5, 10); // buzzer at 5s, run started 10s ago
        let (tx, mut rx) = mpsc::channel(4);

        recover_runs(Arc::clone(&state), &tx).await;

        // The event reached the fire channel…
        let received = rx.try_recv().expect("missed alarm was not sent");
        assert_eq!(received.buzzer_name, "b");
        assert_eq!(received.timer_name, "t");

        // …and is durably recorded in the outbox at the same moment.
        let pending = state.pending_fires().await;
        assert_eq!(pending.len(), 1, "recovery must persist to the outbox");
        assert_eq!(pending[0].buzzer_name, "b");

        // The run's fired_indices advanced so the scheduler does not
        // re-fire the same alarm after dispatch.
        let run = state.get_run("t").await.unwrap();
        assert_eq!(run.fired_indices, vec![0]);
    }
}
