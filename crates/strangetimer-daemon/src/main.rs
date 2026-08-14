use std::sync::Arc;

mod buzzers;
mod platform;
mod scheduler;
mod state;

use anyhow::{Context, Result};
use interprocess::local_socket::traits::tokio::Listener as TokioListener;
use strangetimer_core::ipc::socket_name;
use strangetimer_core::ipc::{ClientMessage, ServerMessage};
use strangetimer_core::model::{Buzzer, BuzzerAction, TimerStatus};
use strangetimer_core::persistence::{
    load_buzzers, load_state, load_timers, save_buzzers, save_state,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::mpsc;

use state::AppState;

#[tokio::main]
async fn main() -> Result<()> {
    // 1. Load timers, buzzers, and state from persistence (or initialise
    //    fresh). The persistence layer writes default files on first call.
    let timers = load_timers().context("failed to load timers")?;
    let mut buzzers = load_buzzers().context("failed to load buzzers")?;
    let state = load_state().context("failed to load state")?;

    // 2. Seed the built-in buzzer library on first run.
    if buzzers.is_empty() {
        buzzers = builtin_buzzers();
        save_buzzers(&buzzers).context("failed to persist seeded buzzers")?;
        eprintln!("strangetimer-daemon: seeded built-in buzzer library");
    }

    eprintln!(
        "strangetimer-daemon: loaded {} timers, {} buzzers, {} active runs",
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
                eprintln!("strangetimer-daemon: registered for autostart.");
            }
            Err(e) => {
                eprintln!("strangetimer-daemon: autostart registration failed: {e:#}");
            }
        }
    }

    // 4. Buzzer channel + scheduler + buzzer dispatcher tasks.
    let (buzzer_tx, mut buzzer_rx) = mpsc::channel::<String>(100);

    let scheduler_state = Arc::clone(&app_state);
    let scheduler_tx = buzzer_tx.clone();
    tokio::spawn(async move {
        scheduler::run_scheduler(scheduler_state, scheduler_tx).await;
    });

    let fire_state = Arc::clone(&app_state);
    tokio::spawn(async move {
        while let Some(name) = buzzer_rx.recv().await {
            fire_buzzer(&name, Arc::clone(&fire_state)).await;
        }
    });

    // 5. Restart recovery (Prompt 20): runs that were active when the daemon
    //    last stopped fire any alarms that were missed during downtime, and
    //    scheduled runs whose time has passed start immediately.
    recover_runs(Arc::clone(&app_state), &buzzer_tx).await;

    // 6. Bind the IPC listener and serve until a shutdown signal arrives.
    let listener = bind_listener(&socket_name()).context("failed to bind IPC listener")?;
    eprintln!("strangetimer-daemon: listening on {}", socket_name());

    // `shutdown` lets an IPC `Shutdown` request (from `strangetimer daemon
    // stop`) tear the accept loop down just like a signal would.
    let shutdown = Arc::new(tokio::sync::Notify::new());

    tokio::select! {
        result = accept_loop(listener, Arc::clone(&app_state), Arc::clone(&shutdown)) => {
            if let Err(e) = result {
                eprintln!("strangetimer-daemon: accept loop exited with error: {e:#}");
            }
        }
        _ = shutdown_signal() => {
            eprintln!("strangetimer-daemon: shutdown signal received");
        }
    }

    // 7. Save state before exiting.
    let final_state = app_state.get_state().await;
    if let Err(e) = save_state(&final_state) {
        eprintln!("strangetimer-daemon: failed to save state on shutdown: {e:#}");
    } else {
        eprintln!("strangetimer-daemon: state saved");
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
async fn fire_buzzer(name: &str, state: Arc<AppState>) {
    match state.get_buzzer(name).await {
        Some(buzzer) => {
            eprintln!("strangetimer-daemon: BUZZ: {name}");
            for action in &buzzer.actions {
                buzzers::dispatch(&state, action).await;
            }
        }
        None => eprintln!("strangetimer-daemon: BUZZ: {name} — no such buzzer (was it deleted?)"),
    }
}

/// Fire alarms that became due while the daemon was stopped, and promote
/// Scheduled runs whose start time has already passed.
async fn recover_runs(state: Arc<AppState>, buzzer_tx: &mpsc::Sender<String>) {
    let now = chrono::Local::now();
    let mut missed: Vec<String> = Vec::new();
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
                    let elapsed = (now - run.started_at) - run.elapsed_before_pause;

                    let Some(timer) = timers.iter().find(|t| t.name == run.timer_name) else {
                        continue;
                    };

                    for (idx, buzzer_ref) in timer.buzzers.iter().enumerate() {
                        if run.fired_indices.contains(&idx) {
                            continue;
                        }
                        if elapsed >= buzzer_ref.offset {
                            run.fired_indices.push(idx);
                            missed.push(buzzer_ref.buzzer_name.clone());
                            state_changed = true;
                        }
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
                eprintln!("strangetimer-daemon: recovery failed to save state: {e:#}");
            }
        }
    }

    // Fire missed alarms immediately — dispatch happens in the fire task.
    for name in missed {
        eprintln!("strangetimer-daemon: firing missed alarm for {name:?} from downtime");
        if let Err(e) = buzzer_tx.send(name).await {
            eprintln!("strangetimer-daemon: recovery buzzer channel error: {e}");
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
                eprintln!(
                    "strangetimer-daemon: removed stale socket {}",
                    path.display()
                );
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
                        eprintln!("strangetimer-daemon: connection error: {e:#}");
                    }
                });
            }
        }
    }
}

/// Handle a single client connection: read one `ClientMessage`, apply it to
/// `AppState`, and reply with the matching `ServerMessage`.
async fn handle_connection(
    mut conn: interprocess::local_socket::tokio::Stream,
    app_state: Arc<AppState>,
    shutdown: Arc<tokio::sync::Notify>,
) -> Result<()> {
    let msg: ClientMessage = read_message_async(&mut conn)
        .await
        .context("failed to read ClientMessage")?;

    eprintln!(
        "strangetimer-daemon: received ClientMessage::{}",
        variant_name(&msg)
    );

    // Shutdown is special: it is answered before the normal handler so the
    // client gets its Ok even though the accept loop is about to stop.
    if matches!(msg, ClientMessage::Shutdown) {
        write_message_async(&mut conn, &ServerMessage::Ok)
            .await
            .context("failed to write ServerMessage")?;
        eprintln!("strangetimer-daemon: graceful shutdown requested over IPC");
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
        ClientMessage::CreateTimer { timer } => reply(state.add_timer(timer).await),
        ClientMessage::DuplicateTimer { source, new_name } => {
            match state.duplicate_timer(&source, new_name).await {
                Ok(name) => {
                    eprintln!("strangetimer-daemon: duplicated {source:?} as {name:?}");
                    ServerMessage::Ok
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
        } => match state.start_run(&name, repeat, schedule_time).await {
            Ok(run) => {
                eprintln!(
                    "strangetimer-daemon: run started for {name:?} ({:?})",
                    run.status
                );
                ServerMessage::Ok
            }
            Err(e) => ServerMessage::Error(e.to_string()),
        },
        ClientMessage::Pause { name } => reply(state.pause_run(&name).await),
        ClientMessage::PauseAll => reply(state.pause_all().await),
        ClientMessage::Resume { name } => reply(state.resume_run(&name).await),
        ClientMessage::Stop { name } => reply(state.remove_run(&name).await),
        ClientMessage::StopAll => reply(state.stop_all().await),
        ClientMessage::GetTimers => ServerMessage::TimerList {
            timers: state.get_timers().await,
            runs: state.lock().await.state.runs.clone(),
        },
        ClientMessage::GetTimer { name } => match state.get_timer(&name).await {
            Some(timer) => {
                let runs = match state.get_run(&name).await {
                    Some(run) => vec![run],
                    None => Vec::new(),
                };
                ServerMessage::TimerDetail { timer, runs }
            }
            None => ServerMessage::Error(format!("no timer named {name:?}")),
        },
        ClientMessage::GetBuzzers => ServerMessage::BuzzerList(state.get_buzzers().await),
        ClientMessage::ConfirmDestructive => {
            state.set_close_windows_confirmed().await;
            ServerMessage::Ok
        }
        ClientMessage::Ping => ServerMessage::Status {
            pid: std::process::id(),
            version: env!("CARGO_PKG_VERSION").to_string(),
        },
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
        ClientMessage::ConfirmDestructive => "ConfirmDestructive",
        ClientMessage::Ping => "Ping",
        ClientMessage::Shutdown => "Shutdown",
    }
}

/// Async version of `strangetimer_core::ipc::read_message`. Reads a
/// length-prefixed JSON frame from a tokio async reader.
async fn read_message_async<R>(reader: &mut R) -> Result<ClientMessage>
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
