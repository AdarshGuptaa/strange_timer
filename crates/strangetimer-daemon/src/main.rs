use std::sync::Arc;

mod state;
use state::AppState;

use anyhow::{Context, Result};
use interprocess::local_socket::traits::tokio::Listener as TokioListener;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use strangetimer_core::ipc::{ClientMessage, ServerMessage, SOCKET_NAME};
use strangetimer_core::persistence::{load_buzzers, load_state, load_timers, save_state};

#[tokio::main]
async fn main() -> Result<()> {
    // 1. Load timers, buzzers, and state from persistence (or initialise fresh).
    //    The persistence layer writes default files on first call, so the
    //    data dir is guaranteed to exist after this block.
    let timers = load_timers().context("failed to load timers")?;
    let buzzers = load_buzzers().context("failed to load buzzers")?;
    let state = load_state().context("failed to load state")?;
    eprintln!(
        "strangetimer-daemon: loaded {} timers, {} buzzers, {} active runs",
        timers.len(),
        buzzers.len(),
        state.runs.len(),
    );

    let app_state = Arc::new(AppState::new(timers, buzzers, state));

    // 2. Bind an interprocess listener on SOCKET_NAME.
    let listener = bind_listener(SOCKET_NAME).context("failed to bind IPC listener")?;
    eprintln!("strangetimer-daemon: listening on {SOCKET_NAME}");

    // 3. Accept connections in a loop; each connection is handled in a spawned task.
    //    Race the accept loop against a shutdown signal so Ctrl+C / SIGTERM
    //    breaks us out cleanly.
    tokio::select! {
        result = accept_loop(listener, Arc::clone(&app_state)) => {
            if let Err(e) = result {
                eprintln!("strangetimer-daemon: accept loop exited with error: {e:#}");
            }
        }
        _ = shutdown_signal() => {
            eprintln!("strangetimer-daemon: shutdown signal received");
        }
    }

    // 4. Save state before exiting.
    let final_state = app_state.get_state().await;
    if let Err(e) = save_state(&final_state) {
        eprintln!("strangetimer-daemon: failed to save state on shutdown: {e:#}");
    } else {
        eprintln!("strangetimer-daemon: state saved");
    }

    Ok(())
}

/// Bind a tokio-flavoured interprocess `Listener` on `name`.
///
/// On Unix this wraps a filesystem path; on Windows it's a named pipe name.
fn bind_listener(name: &str) -> Result<interprocess::local_socket::tokio::Listener> {
    #[cfg(unix)]
    use interprocess::local_socket::{GenericFilePath, ListenerOptions, ToFsName};
    #[cfg(windows)]
    use interprocess::local_socket::{GenericNamespaced, ListenerOptions, ToNsName};

    // The two name flavours resolve to the right OS primitive per platform.
    // We try namespaced first (Unix abstract namespace / Windows pipe name)
    // and fall back to filesystem path on Unix.
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

async fn accept_loop(listener: interprocess::local_socket::tokio::Listener, app_state: Arc<AppState>) -> Result<()> {
    loop {
        let conn = listener.accept().await.context("accept failed")?;
        let state = Arc::clone(&app_state);
        tokio::spawn(async move {
            if let Err(e) = handle_connection(conn, state).await {
                eprintln!("strangetimer-daemon: connection error: {e:#}");
            }
        });
    }
}

/// Handle a single client connection: read one `ClientMessage`, log its
/// variant, and respond with `ServerMessage::Ok`. Stub — no real logic.
async fn handle_connection(
    mut conn: interprocess::local_socket::tokio::Stream,
    _app_state: Arc<AppState>,
) -> Result<()> {
    let msg: ClientMessage = read_message_async(&mut conn)
        .await
        .context("failed to read ClientMessage")?;

    eprintln!(
        "strangetimer-daemon: received ClientMessage::{}",
        variant_name(&msg)
    );

    write_message_async(&mut conn, &ServerMessage::Ok)
        .await
        .context("failed to write ServerMessage::Ok")?;

    Ok(())
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
        let mut sigint = signal(SignalKind::interrupt())
            .expect("failed to install SIGINT handler");
        let mut sigterm = signal(SignalKind::terminate())
            .expect("failed to install SIGTERM handler");
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
