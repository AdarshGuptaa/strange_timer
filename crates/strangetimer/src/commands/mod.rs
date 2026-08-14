pub mod buzzers;
pub mod control;
pub mod timers;
pub mod view;

use anyhow::{Context, Result};
use strangetimer_core::ipc::{ClientMessage, ServerMessage, socket_name};

/// Open a connection to the daemon's IPC endpoint and exchange a single
/// message pair (request → response). The connection is short-lived: the
/// daemon accepts, handles, and closes each one.
pub fn send_and_receive(msg: &ClientMessage) -> Result<ServerMessage> {
    let mut conn = connect()?;
    strangetimer_core::ipc::write_message(&mut conn, msg)
        .context("failed to write IPC message")?;
    let response = strangetimer_core::ipc::read_message::<ServerMessage>(&mut conn)
        .context("failed to read IPC response")?;
    Ok(response)
}

/// Connect to the daemon over the platform-appropriate IPC primitive.
pub fn connect() -> Result<interprocess::local_socket::Stream> {
    #[cfg(unix)]
    {
        use interprocess::local_socket::traits::Stream as _;
        use interprocess::local_socket::{GenericFilePath, Stream, ToFsName};
        let name = socket_name()
            .to_fs_name::<GenericFilePath>()
            .with_context(|| format!("invalid socket name: {}", socket_name()))?;
        Stream::connect(name).with_context(daemon_hint)
    }
    #[cfg(windows)]
    {
        use interprocess::local_socket::traits::Stream as _;
        use interprocess::local_socket::{GenericNamespaced, Stream, ToNsName};
        let name = socket_name()
            .to_ns_name::<GenericNamespaced>()
            .with_context(|| format!("invalid pipe name: {}", socket_name()))?;
        Stream::connect(name).with_context(daemon_hint)
    }
}

fn daemon_hint() -> String {
    format!(
        "failed to connect to the StrangeTimer daemon at {} — \
         is it running? (start it with `strangetimer-daemon`)",
        socket_name()
    )
}

/// Unwrap a `ServerMessage`, turning `Error(e)` into a CLI error.
pub fn ensure_ok(response: ServerMessage) -> Result<()> {
    match response {
        ServerMessage::Ok => Ok(()),
        ServerMessage::Error(e) => Err(anyhow::anyhow!(e)),
        other => Err(anyhow::anyhow!(
            "unexpected daemon response: {:?}",
            variant_of(&other)
        )),
    }
}

fn variant_of(msg: &ServerMessage) -> &'static str {
    match msg {
        ServerMessage::Ok => "Ok",
        ServerMessage::Error(_) => "Error",
        ServerMessage::TimerList { .. } => "TimerList",
        ServerMessage::TimerDetail { .. } => "TimerDetail",
        ServerMessage::BuzzerList(_) => "BuzzerList",
    }
}
