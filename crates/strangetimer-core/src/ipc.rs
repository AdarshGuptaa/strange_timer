use std::io::{Read, Write};

use anyhow::{Context, Result};
use serde::{de::DeserializeOwned, Deserialize, Serialize};

use crate::model::{Buzzer, RepeatMode, Timer, TimerRun};

/// Platform-specific name used when binding / connecting to the daemon's
/// IPC endpoint.
///
/// On Unix this is a filesystem path suitable for a Unix-domain socket.
/// On Windows this is a bare pipe name (no leading `\\.\pipe\`) that
/// `interprocess` accepts as a `NamedPipeServerOptions` name; callers
/// prepend the `\\.\pipe\` prefix as needed.
pub const SOCKET_NAME: &str = if cfg!(windows) {
    "strangetimer"
} else {
    "/tmp/strangetimer.sock"
};

/// Resolve the IPC endpoint name, honouring the `STRANGETIMER_SOCKET`
/// environment override (used by the test-suite to run isolated daemons).
pub fn socket_name() -> String {
    std::env::var("STRANGETIMER_SOCKET").unwrap_or_else(|_| SOCKET_NAME.to_string())
}

/// Commands sent from the CLI to the daemon.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ClientMessage {
    CreateTimer {
        timer: Timer,
    },
    DuplicateTimer {
        source: String,
        new_name: Option<String>,
    },
    DeleteTimer {
        name: String,
    },
    CreateBuzzer {
        buzzer: Buzzer,
    },
    DeleteBuzzer {
        name: String,
    },
    RunTimer {
        name: String,
        repeat: RepeatMode,
        schedule_time: Option<chrono::DateTime<chrono::Local>>,
        /// `run -u/--userinterrupt`: pause at every buzzer until acked.
        #[serde(default)]
        user_interrupt: bool,
        /// Terminal window captured at `run -u` time; the daemon focuses it
        /// after buzzer actions so the user sees the prompt.
        #[serde(default)]
        interrupt_focus: Option<String>,
    },
    Pause {
        name: String,
    },
    PauseAll,
    Resume {
        name: String,
    },
    Stop {
        name: String,
    },
    StopAll,
    GetTimers,
    GetTimer {
        name: String,
    },
    GetBuzzers,
    /// Opt-in acknowledgement that the `close_windows` buzzer is allowed to
    /// close all other windows. Guarded by `state.close_windows_confirmed`.
    ConfirmDestructive,
    /// Liveness probe: the daemon answers with `ServerMessage::Status`.
    /// Used by `strangetimer daemon status/start` to detect a running daemon.
    Ping,
    /// Graceful shutdown request. The daemon replies `Ok`, saves state and
    /// exits — the same teardown path as SIGINT/SIGTERM. Used by
    /// `strangetimer daemon stop/restart`.
    Shutdown,
    /// Fetch buzzer-ringing events newer than `after_id` (None = all).
    /// Used by `strangetimer watch`.
    GetEvents {
        after_id: Option<u64>,
    },
}

/// Responses sent from the daemon to the CLI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ServerMessage {
    Ok,
    Error(String),
    /// Timer definitions plus every live run. Runs are needed by the `view`
    /// commands to render progress blocks; the daemon owns all live state.
    TimerList {
        timers: Vec<Timer>,
        runs: Vec<TimerRun>,
        /// Legacy single-pending marker (kept for older CLI binaries).
        #[serde(default)]
        interrupt_pending: Option<String>,
        /// Timers currently awaiting a user-interrupt acknowledgement.
        #[serde(default)]
        pending_interrupts: Vec<String>,
    },
    TimerDetail {
        timer: Timer,
        runs: Vec<TimerRun>,
        /// Legacy single-pending marker (kept for older CLI binaries).
        #[serde(default)]
        interrupt_pending: Option<String>,
        #[serde(default)]
        pending_interrupts: Vec<String>,
    },
    BuzzerList(Vec<Buzzer>),
    /// Reply to `ClientMessage::Ping`: the daemon's process id and version.
    Status {
        pid: u32,
        version: String,
    },
    /// Reply to `ClientMessage::GetEvents`.
    BuzzerEvents(Vec<crate::model::BuzzerEvent>),
}

/// Write a single length-prefixed JSON message to `stream`.
///
/// Frame layout: `[u32 BE length][JSON payload]`.
pub fn write_message<T: Serialize>(stream: &mut impl Write, msg: &T) -> Result<()> {
    let payload = serde_json::to_vec(msg).context("failed to serialise IPC message")?;
    let len = u32::try_from(payload.len()).context("IPC payload exceeds u32::MAX bytes")?;
    stream
        .write_all(&len.to_be_bytes())
        .context("failed to write IPC length prefix")?;
    stream
        .write_all(&payload)
        .context("failed to write IPC payload")?;
    stream.flush().context("failed to flush IPC stream")?;
    Ok(())
}

/// Read a single length-prefixed JSON message from `stream`.
///
/// Returns an error if the length prefix would not fit in a `usize` on
/// this platform, or if the declared length exceeds the sanity cap below.
pub fn read_message<T: DeserializeOwned>(stream: &mut impl Read) -> Result<T> {
    let mut len_bytes = [0u8; 4];
    stream
        .read_exact(&mut len_bytes)
        .context("failed to read IPC length prefix")?;
    let len = u32::from_be_bytes(len_bytes) as usize;

    // 64 MiB sanity cap. A real Timer/Buzzer/TimerRun is well under 1 KiB;
    // this guards against a malicious or corrupt peer sending an
    // enormous length that would force us to allocate gigabytes.
    const MAX_PAYLOAD: usize = 64 * 1024 * 1024;
    if len > MAX_PAYLOAD {
        anyhow::bail!("IPC payload length {len} exceeds sanity cap of {MAX_PAYLOAD} bytes");
    }

    let mut payload = vec![0u8; len];
    stream
        .read_exact(&mut payload)
        .with_context(|| format!("failed to read IPC payload of {len} bytes"))?;

    serde_json::from_slice(&payload)
        .with_context(|| format!("failed to parse IPC payload of {len} bytes"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn socket_name_is_non_empty() {
        assert!(!SOCKET_NAME.is_empty());
    }

    #[test]
    fn roundtrip_client_message() {
        let original = ClientMessage::RunTimer {
            name: "workAndFun".to_string(),
            repeat: RepeatMode::Infinite,
            schedule_time: None,
            user_interrupt: true,
            interrupt_focus: Some("term".to_string()),
        };
        let mut buf: Vec<u8> = Vec::new();
        write_message(&mut buf, &original).unwrap();
        let decoded: ClientMessage = read_message(&mut buf.as_slice()).unwrap();
        match decoded {
            ClientMessage::RunTimer {
                name,
                repeat,
                schedule_time,
                user_interrupt,
                interrupt_focus,
            } => {
                assert_eq!(name, "workAndFun");
                assert!(matches!(repeat, RepeatMode::Infinite));
                assert!(schedule_time.is_none());
                assert!(user_interrupt);
                assert_eq!(interrupt_focus.as_deref(), Some("term"));
            }
            other => panic!("unexpected variant: {:?}", other),
        }
    }

    #[test]
    fn roundtrip_server_message() {
        let original = ServerMessage::TimerDetail {
            timer: Timer {
                name: "t".to_string(),
                buzzers: vec![],
                created_at: chrono::Local::now(),
            },
            runs: vec![],
            interrupt_pending: None,
            pending_interrupts: vec![],
        };
        let mut buf: Vec<u8> = Vec::new();
        write_message(&mut buf, &original).unwrap();
        let decoded: ServerMessage = read_message(&mut buf.as_slice()).unwrap();
        match decoded {
            ServerMessage::TimerDetail { timer, runs, .. } => {
                assert_eq!(timer.name, "t");
                assert!(runs.is_empty());
            }
            other => panic!("unexpected variant: {:?}", other),
        }
    }

    #[test]
    fn write_message_emits_length_prefix() {
        let original = ClientMessage::GetBuzzers;
        let mut buf: Vec<u8> = Vec::new();
        write_message(&mut buf, &original).unwrap();
        // First 4 bytes: big-endian length of the JSON payload.
        let len = u32::from_be_bytes([buf[0], buf[1], buf[2], buf[3]]) as usize;
        assert_eq!(len, buf.len() - 4);
    }
}
