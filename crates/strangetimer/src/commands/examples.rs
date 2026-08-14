//! `strangetimer examples [--install]` — ready-to-use example buzzers
//! covering every action type. See docs/BUZZER_EXAMPLES.md for the full
//! guide with worked timers.

use anyhow::{anyhow, Result};
use strangetimer_core::ipc::{ClientMessage, ServerMessage};
use strangetimer_core::model::{Buzzer, BuzzerAction, LlmPromptSource};

use crate::commands::send_and_receive;
use crate::style;

/// The project's GitHub page, used by the URL example buzzer.
const PROJECT_URL: &str = "https://github.com/AdarshGuptaa/strange_timer";

/// An example buzzer: what it demonstrates, the exact command to create it,
/// and (for examples with no user-specific files) the actions to install.
struct ExampleBuzzer {
    name: &'static str,
    description: &'static str,
    command: &'static str,
    /// `Some` when the example can be installed without user-specific
    /// files (paths differ per machine), `None` for docs-only examples.
    installable_actions: Option<Vec<BuzzerAction>>,
}

fn examples() -> Vec<ExampleBuzzer> {
    vec![
        ExampleBuzzer {
            name: "exampleAudio",
            description: "plays the built-in chime sound",
            command: "strangetimer create buzzer exampleAudio --audio",
            installable_actions: Some(vec![BuzzerAction::DefaultAudio]),
        },
        ExampleBuzzer {
            name: "exampleAudioFile",
            description: "plays a custom audio file of your choice",
            command: "strangetimer create buzzer exampleAudioFile --audio ~/Music/alert.wav",
            installable_actions: None,
        },
        ExampleBuzzer {
            name: "exampleVideo",
            description: "opens the built-in default video clip",
            command: "strangetimer create buzzer exampleVideo --video",
            installable_actions: Some(vec![BuzzerAction::DefaultVideo]),
        },
        ExampleBuzzer {
            name: "exampleUrl",
            description: "opens this project's GitHub page in your browser",
            command: "strangetimer create buzzer exampleUrl --url https://github.com/AdarshGuptaa/strange_timer",
            installable_actions: Some(vec![BuzzerAction::Url(PROJECT_URL.to_string())]),
        },
        ExampleBuzzer {
            name: "exampleApplication",
            description: "launches an application (use a path from your system)",
            command: "strangetimer create buzzer exampleApplication --application /usr/bin/gnome-calculator",
            installable_actions: None,
        },
        ExampleBuzzer {
            name: "exampleBash",
            description: "runs a shell script (point it at your own script)",
            command: "strangetimer create buzzer exampleBash --bash ~/notify.sh",
            installable_actions: None,
        },
        ExampleBuzzer {
            name: "exampleChain",
            description: "chains audio and a URL — actions fire in sequence",
            command: "strangetimer create buzzer exampleChain --audio --url https://github.com/AdarshGuptaa/strange_timer",
            installable_actions: Some(vec![
                BuzzerAction::DefaultAudio,
                BuzzerAction::Url(PROJECT_URL.to_string()),
            ]),
        },
        ExampleBuzzer {
            name: "exampleLlm",
            description: "asks a local Ollama model to announce the timer (falls back to the chime if Ollama is down)",
            command: "strangetimer create buzzer exampleLlm --llm llama3 \"Announce that the timer finished and suggest a 5 minute break.\"",
            installable_actions: Some(vec![BuzzerAction::Llm {
                model: "llama3".to_string(),
                prompt: LlmPromptSource::Inline(
                    "Announce that the timer finished and suggest a 5 minute break.".to_string(),
                ),
            }]),
        },
    ]
}

/// `strangetimer examples` — print the table of examples.
pub fn list_examples() -> Result<()> {
    println!(
        "{}",
        style::header("Example buzzers — one per action type, plus a chained combo:")
    );
    println!();
    for ex in examples() {
        println!(
            "  {} {}",
            style::name(&format!("{:<18}", ex.name)),
            ex.description
        );
    }
    println!();
    println!(
        "{}",
        style::dim(
            "Create any of them with the commands below, or install the file-free \
         ones automatically with `strangetimer examples --install`."
        )
    );
    println!();
    for ex in examples() {
        println!("{}", style::accent(&ex.to_string()));
        println!("  {}", ex.command);
        println!();
    }
    println!(
        "{}",
        style::dim("See docs/BUZZER_EXAMPLES.md for a full guide with worked timers.")
    );
    Ok(())
}

/// `strangetimer examples --install` — create the file-free examples in the
/// daemon's buzzer library (skipping any that already exist).
pub fn install_examples() -> Result<()> {
    let mut created = 0;
    let mut skipped = 0;
    for ex in examples()
        .into_iter()
        .filter(|e| e.installable_actions.is_some())
    {
        let buzzer = Buzzer {
            name: ex.name.to_string(),
            actions: ex.installable_actions.clone().expect("filtered above"),
            builtin: false,
        };
        match send_and_receive(&ClientMessage::CreateBuzzer {
            buzzer: buzzer.clone(),
        }) {
            Ok(ServerMessage::Ok) => {
                println!("Created example buzzer {:?}.", buzzer.name);
                created += 1;
            }
            Ok(ServerMessage::Error(e)) if e.contains("already exists") => {
                println!("Skipped {:?} (already exists).", buzzer.name);
                skipped += 1;
            }
            Ok(ServerMessage::Error(e)) => return Err(anyhow!(e)),
            Ok(other) => return Err(anyhow!("unexpected daemon response: {other:?}")),
            Err(e) => return Err(e),
        }
    }

    println!(
        "Done: {created} created, {skipped} skipped. See `strangetimer examples` \
         and docs/BUZZER_EXAMPLES.md for the file-based examples."
    );
    Ok(())
}

impl std::fmt::Display for ExampleBuzzer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} — {}", self.name, self.description)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_action_type_has_an_example() {
        let examples = examples();
        let kinds: Vec<&str> = examples
            .iter()
            .flat_map(|e| e.installable_actions.clone().unwrap_or_default())
            .map(|a| action_kind(&a))
            .collect();
        for kind in ["audio", "video", "url", "llm"] {
            assert!(kinds.contains(&kind), "missing {kind} example");
        }
        // Chaining is demonstrated by a multi-action example.
        assert!(
            examples
                .iter()
                .any(|e| e.installable_actions.as_ref().map_or(0, Vec::len) > 1),
            "missing chained multi-action example"
        );
        // File-based examples are docs-only but still listed.
        assert!(examples.iter().any(|e| e.installable_actions.is_none()));
    }

    #[test]
    fn installable_examples_are_named_consistently() {
        for ex in examples() {
            assert!(ex.command.contains(&format!("create buzzer {}", ex.name)));
        }
    }

    fn action_kind(a: &BuzzerAction) -> &'static str {
        match a {
            BuzzerAction::DefaultAudio => "audio",
            BuzzerAction::DefaultVideo => "video",
            BuzzerAction::Url(_) => "url",
            BuzzerAction::Llm { .. } => "llm",
            _ => "other",
        }
    }
}
