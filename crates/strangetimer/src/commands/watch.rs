//! `strangetimer watch` — print buzzer-ringing notifications as they fire.
//!
//! The daemon pushes a `BuzzerEvent` every time a buzzer fires; this
//! command polls for new events and prints, for each:
//!
//! ```text
//! Timer <name> ringing | <types> | <time> -> strangetimer resume <name>
//! ```
//!
//! The `resume` hint is only shown for user-interrupt runs that are now
//! paused awaiting acknowledgement. Ctrl+C stops the watcher.

use std::time::Duration;

use anyhow::{anyhow, Result};
use strangetimer_core::ipc::{ClientMessage, ServerMessage};
use strangetimer_core::model::BuzzerEvent;

use crate::commands::send_and_receive;
use crate::style;

const POLL_INTERVAL: Duration = Duration::from_millis(400);

/// `strangetimer watch` — print each ringing buzzer until Ctrl+C.
pub fn watch() -> Result<()> {
    let mut last_id: Option<u64> = None;
    loop {
        let response = send_and_receive(&ClientMessage::GetEvents { after_id: last_id })?;
        let events = match response {
            ServerMessage::BuzzerEvents(events) => events,
            ServerMessage::Error(e) => return Err(anyhow!(e)),
            other => return Err(anyhow!("unexpected daemon response: {other:?}")),
        };

        for event in &events {
            print_event(event);
            last_id = Some(event.id);
        }

        std::thread::sleep(POLL_INTERVAL);
    }
}

fn print_event(event: &BuzzerEvent) {
    let kinds = event.buzzer_types.join(", ");
    let when = event.fired_at.format("%Y-%m-%d %H:%M:%S");
    println!(
        "{} {} | {} | {}",
        style::name(&event.timer_name),
        style::accent("ringing"),
        kinds,
        when,
    );
    if event.requires_ack {
        println!(
            "{}",
            style::prompt(&format!(
                "-> strangetimer resume {} to resume!",
                event.timer_name
            ))
        );
    }
    if let Some(outcome) = &event.outcome {
        println!("{}", style::warn(&format!("   ({outcome})")));
    }
}
