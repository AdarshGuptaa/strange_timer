pub mod buzzers;
pub mod candidates;
pub mod completions;
pub mod control;
pub mod daemon;
pub mod doctor;
pub mod examples;
pub mod install_completions;
pub mod timers;
pub mod view;
pub mod watch;

use anyhow::{Context, Result};
use strangetimer_core::ipc::{socket_name, ClientMessage, ServerMessage};

/// Open a connection to the daemon's IPC endpoint and exchange a single
/// message pair (request → response). The connection is short-lived: the
/// daemon accepts, handles, and closes each one.
///
/// When nothing is listening, most commands transparently start the daemon
/// and retry once (see [`auto_start`]). `daemon status` / `daemon stop`
/// never auto-start — they must be able to report "not running".
pub fn send_and_receive(msg: &ClientMessage) -> Result<ServerMessage> {
    send_and_receive_opt(msg, true)
}

/// Like [`send_and_receive`] but never auto-starts the daemon — used by
/// completion candidates, which must stay silent when the daemon is down.
pub fn send_and_receive_no_autostart(msg: &ClientMessage) -> Result<ServerMessage> {
    send_and_receive_opt(msg, false)
}

fn send_and_receive_opt(msg: &ClientMessage, auto_start: bool) -> Result<ServerMessage> {
    match try_connect() {
        Ok(conn) => exchange_on(conn, msg),
        Err(first_err) => {
            if auto_start && auto_start_enabled() && !is_lifecycle_command(msg) {
                eprintln!(
                    "{}",
                    crate::style::dim("StrangeTimer daemon not running — starting it…")
                );
                daemon::ensure_started(false)?;
                let conn = try_connect().with_context(|| {
                    format!(
                        "failed to connect to the StrangeTimer daemon at {} \
                         after starting it",
                        socket_name()
                    )
                })?;
                exchange_on(conn, msg)
            } else {
                Err(first_err).with_context(daemon_hint)
            }
        }
    }
}

/// `Ping` (daemon status) and `Shutdown` (daemon stop) must never trigger
/// an auto-start — they report on the daemon's absence instead.
fn is_lifecycle_command(msg: &ClientMessage) -> bool {
    matches!(msg, ClientMessage::Ping | ClientMessage::Shutdown)
}

/// Auto-start is on unless the user opts out with `STRANGETIMER_AUTO_START=0`.
fn auto_start_enabled() -> bool {
    !matches!(std::env::var("STRANGETIMER_AUTO_START").as_deref(), Ok("0"))
}

fn exchange_on(
    mut conn: interprocess::local_socket::Stream,
    msg: &ClientMessage,
) -> Result<ServerMessage> {
    strangetimer_core::ipc::write_message(&mut conn, msg).context("failed to write IPC message")?;
    let response = strangetimer_core::ipc::read_message::<ServerMessage>(&mut conn)
        .context("failed to read IPC response")?;
    Ok(response)
}

/// Raw connect attempt without the "is the daemon running?" hint context —
/// used by the lifecycle probe to distinguish "not listening" from
/// "listening".
pub fn try_connect() -> Result<interprocess::local_socket::Stream> {
    #[cfg(unix)]
    {
        use interprocess::local_socket::traits::Stream as _;
        use interprocess::local_socket::{GenericFilePath, Stream, ToFsName};
        let name = socket_name()
            .to_fs_name::<GenericFilePath>()
            .with_context(|| format!("invalid socket name: {}", socket_name()))?;
        Stream::connect(name).map_err(Into::into)
    }
    #[cfg(windows)]
    {
        use interprocess::local_socket::traits::Stream as _;
        use interprocess::local_socket::{GenericNamespaced, Stream, ToNsName};
        let name = socket_name()
            .to_ns_name::<GenericNamespaced>()
            .with_context(|| format!("invalid pipe name: {}", socket_name()))?;
        Stream::connect(name).map_err(Into::into)
    }
}

fn daemon_hint() -> String {
    format!(
        "failed to connect to the StrangeTimer daemon at {} — \
         is it running? (start it with `strangetimer daemon start`)",
        socket_name()
    )
}

/// Ask the user to confirm an action with `[y/N]`. `assume_yes` skips the
/// prompt. Without a terminal, confirmation fails safely unless `--yes`
/// was given (so scripts never hang).
pub fn confirm(prompt: &str, assume_yes: bool) -> Result<bool> {
    use std::io::{BufRead, Write};
    if assume_yes {
        return Ok(true);
    }
    use crossterm::tty::IsTty;
    if !std::io::stdin().is_tty() {
        return Err(anyhow::anyhow!(
            "confirmation required — pass --yes to accept non-interactively"
        ));
    }
    print!("{prompt} [y/N] ");
    std::io::stdout().flush()?;
    let mut line = String::new();
    let read = std::io::stdin().lock().read_line(&mut line)?;
    if read == 0 {
        return Ok(false); // EOF counts as "no"
    }
    Ok(matches!(
        line.trim().to_ascii_lowercase().as_str(),
        "y" | "yes"
    ))
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
        ServerMessage::BuzzerInfoList(_) => "BuzzerInfoList",
        ServerMessage::BuzzerDetailInfo(_) => "BuzzerDetailInfo",
        ServerMessage::Status { .. } => "Status",
        ServerMessage::BuzzerEvents(_) => "BuzzerEvents",
        ServerMessage::DuplicateTimerOk { .. } => "DuplicateTimerOk",
    }
}
