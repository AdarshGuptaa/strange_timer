use anyhow::{anyhow, Result};
use chrono::Local;
use strangetimer_core::duration_parse::parse_offset;
use strangetimer_core::ipc::{ClientMessage, ServerMessage};
use strangetimer_core::model::{BuzzerRef, Timer};

use crate::commands::{ensure_ok, send_and_receive};
use crate::style;

/// `strangetimer create timer <name> <offset> [<buzzer> [<offset> [<buzzer>]]...]`
pub fn create_timer(name: &str, rest: &[String]) -> Result<()> {
    let buzzers = parse_buzzer_refs(rest)?;

    let timer = Timer {
        name: name.to_string(),
        buzzers,
        created_at: Local::now(),
    };

    match send_and_receive(&ClientMessage::CreateTimer { timer })? {
        ServerMessage::Ok => println!("Created timer {}.{}", style::name(name), style::dim(".")),
        ServerMessage::Error(e) => return Err(anyhow!(e)),
        other => return Err(anyhow!("unexpected daemon response: {other:?}")),
    }
    Ok(())
}

/// `strangetimer duplicate timer <source> [<new_name>]`
pub fn duplicate_timer(source: &str, new_name: Option<String>) -> Result<()> {
    let response = send_and_receive(&ClientMessage::DuplicateTimer {
        source: source.to_string(),
        new_name: new_name.clone(),
    })?;

    match response {
        ServerMessage::DuplicateTimerOk { name } => println!(
            "Duplicated timer {} as {}.",
            style::name(source),
            style::name(&name)
        ),
        ServerMessage::Error(e) => return Err(anyhow!(e)),
        other => return Err(anyhow!("unexpected daemon response: {other:?}")),
    }
    Ok(())
}

/// `strangetimer delete timer <name>`
pub fn delete_timer(name: &str) -> Result<()> {
    ensure_ok(send_and_receive(&ClientMessage::DeleteTimer {
        name: name.to_string(),
    })?)?;
    println!("Deleted timer {}.", style::name(name));
    Ok(())
}

/// Parse the variadic `(offset, optional buzzer)` pairs of `create timer`.
///
/// A token that parses as an offset opens a new slot; the next token that
/// does *not* parse as an offset is taken as that slot's buzzer name. A slot
/// left without a buzzer name falls back to the built-in `default_audio`.
fn parse_buzzer_refs(rest: &[String]) -> Result<Vec<BuzzerRef>> {
    let mut refs: Vec<BuzzerRef> = Vec::new();
    let mut pending: Option<chrono::Duration> = None;

    for token in rest {
        if let Ok(offset) = parse_offset(token) {
            // Close out the previous slot with the default buzzer if it had
            // none (e.g. `create timer t 5m 10m`).
            if let Some(prev) = pending.take() {
                refs.push(BuzzerRef {
                    offset: prev,
                    buzzer_name: "default_audio".to_string(),
                });
            }
            pending = Some(offset);
        } else {
            let offset = pending.take().ok_or_else(|| {
                anyhow!(
                    "unexpected token {token:?}: expected an offset, or a \
                     buzzer name following an offset"
                )
            })?;
            refs.push(BuzzerRef {
                offset,
                buzzer_name: token.clone(),
            });
        }
    }

    if let Some(offset) = pending.take() {
        refs.push(BuzzerRef {
            offset,
            buzzer_name: "default_audio".to_string(),
        });
    }

    if refs.is_empty() {
        return Err(anyhow!("no offsets given — a timer needs at least one"));
    }

    Ok(refs)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(refs: &[BuzzerRef]) -> Vec<(&str, i64)> {
        refs.iter()
            .map(|r| (r.buzzer_name.as_str(), r.offset.num_seconds()))
            .collect()
    }

    #[test]
    fn bare_offsets_default_to_default_audio() {
        let refs = parse_buzzer_refs(&["45min".into(), "15min".into()]).unwrap();
        assert_eq!(
            names(&refs),
            vec![("default_audio", 2700), ("default_audio", 900)]
        );
    }

    #[test]
    fn offset_then_buzzer_name() {
        let refs = parse_buzzer_refs(&["1W".into(), "paymentBuzzer".into()]).unwrap();
        assert_eq!(names(&refs), vec![("paymentBuzzer", 604800)]);
    }

    #[test]
    fn mixed_pairs() {
        let refs =
            parse_buzzer_refs(&["5m".into(), "alarmA".into(), "10m".into(), "alarmB".into()])
                .unwrap();
        assert_eq!(names(&refs), vec![("alarmA", 300), ("alarmB", 600)]);
    }

    #[test]
    fn rejects_buzzer_without_preceding_offset() {
        assert!(parse_buzzer_refs(&["alarmA".into(), "5m".into()]).is_err());
    }
}
