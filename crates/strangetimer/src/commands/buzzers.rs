use std::path::PathBuf;

use anyhow::{anyhow, Result};
use strangetimer_core::ipc::{ClientMessage, ServerMessage};
use strangetimer_core::model::{Buzzer, BuzzerAction, LlmPromptSource};

use crate::cli::CreateBuzzerArgs;
use crate::commands::{ensure_ok, send_and_receive};

/// `strangetimer create buzzer <name> [--audio [path]] [--video [path]]
/// [--application path] [--url url] [--bash path] [--llm model prompt]`
pub fn create_buzzer(args: &CreateBuzzerArgs) -> Result<()> {
    let actions = build_actions(args)?;

    let buzzer = Buzzer {
        name: args.name.clone(),
        actions,
        builtin: false,
    };

    match send_and_receive(&ClientMessage::CreateBuzzer { buzzer })? {
        ServerMessage::Ok => println!("Created buzzer {:?}.", args.name),
        ServerMessage::Error(e) => return Err(anyhow!(e)),
        other => return Err(anyhow!("unexpected daemon response: {other:?}")),
    }
    Ok(())
}

/// `strangetimer delete buzzer <name>`
pub fn delete_buzzer(name: &str) -> Result<()> {
    ensure_ok(send_and_receive(&ClientMessage::DeleteBuzzer {
        name: name.to_string(),
    })?)?;
    println!("Deleted buzzer {name:?}.");
    Ok(())
}

/// `strangetimer view buzzers`
pub fn view_buzzers() -> Result<()> {
    let response = send_and_receive(&ClientMessage::GetBuzzers)?;
    let buzzers = match response {
        ServerMessage::BuzzerList(b) => b,
        ServerMessage::Error(e) => return Err(anyhow!(e)),
        other => return Err(anyhow!("unexpected daemon response: {other:?}")),
    };

    if buzzers.is_empty() {
        println!("No buzzers defined.");
        return Ok(());
    }

    println!("{:<24} {:<32} Built-in", "Name", "Type(s)");
    println!("{}", "─".repeat(78));
    for buzzer in &buzzers {
        let kinds = buzzer
            .actions
            .iter()
            .map(action_label)
            .collect::<Vec<_>>()
            .join(", ");
        let tag = if buzzer.builtin { "[built-in]" } else { "" };
        println!("{:<24} {:<32} {}", buzzer.name, kinds, tag);
    }

    if buzzers.iter().any(|b| {
        b.actions.iter().any(|a| {
            matches!(
                a,
                BuzzerAction::CloseAllWindows | BuzzerAction::CloseApplication(_)
            )
        })
    }) {
        println!();
        println!(
            "WARNING: the close_windows / close_app buzzers close windows \
             when they fire.\nRun `strangetimer confirm-destructive` to \
             enable them."
        );
    }

    Ok(())
}

/// Translate each `--flag` into a `BuzzerAction`, in command-line order.
fn build_actions(args: &CreateBuzzerArgs) -> Result<Vec<BuzzerAction>> {
    let mut actions: Vec<BuzzerAction> = Vec::new();

    if let Some(audio) = &args.audio {
        if audio.is_empty() {
            actions.push(BuzzerAction::DefaultAudio);
        } else {
            actions.push(BuzzerAction::Audio(Some(PathBuf::from(audio))));
        }
    }
    if let Some(video) = &args.video {
        if video.is_empty() {
            actions.push(BuzzerAction::DefaultVideo);
        } else {
            actions.push(BuzzerAction::Video(Some(PathBuf::from(video))));
        }
    }
    if let Some(app) = &args.application {
        actions.push(BuzzerAction::Application(PathBuf::from(app)));
    }
    if let Some(url) = &args.url {
        actions.push(BuzzerAction::Url(url.clone()));
    }
    if let Some(bash) = &args.bash {
        actions.push(BuzzerAction::Bash(PathBuf::from(bash)));
    }
    if let Some(app) = &args.close_app {
        actions.push(BuzzerAction::CloseApplication(app.clone()));
    }
    if let Some(window) = &args.focus_window {
        actions.push(BuzzerAction::FocusWindow(window.clone()));
    }
    if let Some(llm) = &args.llm {
        let model = llm[0].clone();
        let prompt_or_file = llm[1].clone();
        let prompt = if std::path::Path::new(&prompt_or_file).exists() {
            LlmPromptSource::File(PathBuf::from(prompt_or_file))
        } else {
            LlmPromptSource::Inline(prompt_or_file)
        };
        actions.push(BuzzerAction::Llm { model, prompt });
    }

    if actions.is_empty() {
        return Err(anyhow!(
            "a buzzer needs at least one action — pass one or more of \
             --audio, --video, --application, --url, --bash, --close-app, \
             --focus-window, --llm"
        ));
    }

    Ok(actions)
}

fn action_label(action: &BuzzerAction) -> String {
    match action {
        BuzzerAction::DefaultAudio | BuzzerAction::Audio(_) => "audio".to_string(),
        BuzzerAction::DefaultVideo | BuzzerAction::Video(_) => "video".to_string(),
        BuzzerAction::CloseAllWindows => "close_windows".to_string(),
        BuzzerAction::Application(_) => "application".to_string(),
        BuzzerAction::Url(_) => "url".to_string(),
        BuzzerAction::Bash(_) => "bash".to_string(),
        BuzzerAction::CloseApplication(_) => "close_app".to_string(),
        BuzzerAction::FocusWindow(_) => "focus_window".to_string(),
        BuzzerAction::Llm { .. } => "llm".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args() -> CreateBuzzerArgs {
        CreateBuzzerArgs {
            name: "t".to_string(),
            audio: None,
            video: None,
            application: None,
            url: None,
            bash: None,
            close_app: None,
            focus_window: None,
            llm: None,
        }
    }

    #[test]
    fn audio_without_path_is_default_audio() {
        let mut a = args();
        a.audio = Some(String::new());
        let actions = build_actions(&a).unwrap();
        assert!(matches!(actions[0], BuzzerAction::DefaultAudio));
    }

    #[test]
    fn audio_with_path_is_custom_audio() {
        let mut a = args();
        a.audio = Some("/tmp/x.wav".to_string());
        let actions = build_actions(&a).unwrap();
        assert!(
            matches!(&actions[0], BuzzerAction::Audio(Some(p)) if p == std::path::Path::new("/tmp/x.wav"))
        );
    }

    #[test]
    fn multiple_flags_chain_actions() {
        let mut a = args();
        a.audio = Some(String::new());
        a.url = Some("https://example.com".to_string());
        let actions = build_actions(&a).unwrap();
        assert_eq!(actions.len(), 2);
    }

    #[test]
    fn no_flags_is_an_error() {
        assert!(build_actions(&args()).is_err());
    }

    #[test]
    fn close_app_and_focus_window_map_to_actions() {
        let mut a = args();
        a.close_app = Some("firefox".to_string());
        a.focus_window = Some("Slack".to_string());
        let actions = build_actions(&a).unwrap();
        assert_eq!(actions.len(), 2);
        assert!(matches!(&actions[0], BuzzerAction::CloseApplication(n) if n == "firefox"));
        assert!(matches!(&actions[1], BuzzerAction::FocusWindow(n) if n == "Slack"));
    }

    #[test]
    fn action_labels_cover_new_actions() {
        assert_eq!(
            action_label(&BuzzerAction::CloseApplication("x".to_string())),
            "close_app"
        );
        assert_eq!(
            action_label(&BuzzerAction::FocusWindow("x".to_string())),
            "focus_window"
        );
    }
}
