use std::path::PathBuf;

use anyhow::{anyhow, Result};
use strangetimer_core::ipc::{ClientMessage, ServerMessage};
use strangetimer_core::model::{Buzzer, BuzzerAction, LlmPromptSource};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::cli::CreateBuzzerArgs;
use crate::commands::{confirm, ensure_ok, send_and_receive};
use crate::style;

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
        ServerMessage::Ok => println!("Created buzzer {}.", style::name(&args.name)),
        ServerMessage::Error(e) => return Err(anyhow!(e)),
        other => return Err(anyhow!("unexpected daemon response: {other:?}")),
    }

    // Show the created definition unless disabled.
    if !args.no_preview {
        view_buzzer(&args.name)?;
    }
    Ok(())
}

/// `strangetimer delete buzzer <name> [--cascade] [--yes]`
pub fn delete_buzzer(name: &str, cascade: bool, yes: bool) -> Result<()> {
    if cascade {
        // Fetch the referencing timers so the user can confirm the blast
        // radius before anything is deleted.
        let detail = match send_and_receive(&ClientMessage::GetBuzzerDetail {
            name: name.to_string(),
        })? {
            ServerMessage::BuzzerDetailInfo(d) => d,
            ServerMessage::Error(e) => return Err(anyhow!(e)),
            other => return Err(anyhow!("unexpected daemon response: {other:?}")),
        };
        if detail.referencing_timers.is_empty() {
            ensure_ok(send_and_receive(&ClientMessage::DeleteBuzzer {
                name: name.to_string(),
            })?)?;
            println!("Deleted buzzer {}.", style::name(name));
            return Ok(());
        }
        let names: Vec<&str> = detail
            .referencing_timers
            .iter()
            .map(|(n, _)| n.as_str())
            .collect();
        let ok = confirm(
            &format!(
                "Buzzer {name:?} is used by timers: {}. Delete the buzzer and these \
                 timer definitions?",
                names.join(", ")
            ),
            yes,
        )?;
        if !ok {
            println!("Aborted — nothing was deleted.");
            return Ok(());
        }
        ensure_ok(send_and_receive(&ClientMessage::DeleteBuzzerCascade {
            name: name.to_string(),
        })?)?;
        println!(
            "Deleted buzzer {} and its {} referencing timer definition(s).",
            style::name(name),
            detail.referencing_timers.len()
        );
        return Ok(());
    }

    let result = send_and_receive(&ClientMessage::DeleteBuzzer {
        name: name.to_string(),
    });
    if let Err(e) = result.and_then(ensure_ok) {
        if e.to_string().contains("referenced") {
            return Err(anyhow!(
                "{} — use `delete buzzer {name} --cascade` to also delete the \
                 timers using it",
                e
            ));
        }
        return Err(e);
    }
    println!("Deleted buzzer {}.", style::name(name));
    Ok(())
}

/// `strangetimer view buzzers` — summary table with targets, durations and
/// reference counts.
pub fn view_buzzers() -> Result<()> {
    let response = send_and_receive(&ClientMessage::GetBuzzerInfo)?;
    let buzzers = match response {
        ServerMessage::BuzzerInfoList(b) => b,
        ServerMessage::Error(e) => return Err(anyhow!(e)),
        other => return Err(anyhow!("unexpected daemon response: {other:?}")),
    };

    if buzzers.is_empty() {
        println!("No buzzers defined.");
        return Ok(());
    }

    let (width, _) = crate::commands::view::terminal_size_pub();
    let header = |label: &str, w: usize| style::header(&pad(&truncate(label, w), w));

    // Column widths derived from the terminal width (muted, readable).
    let name_w = (width * 24 / 100).clamp(12, 28);
    let actions_w = (width * 18 / 100).clamp(10, 20);
    let target_w = (width * 30 / 100).clamp(12, 40);
    let duration_w = 9usize;
    let timers_w = 7usize;
    let live_w = 5usize;
    let builtin_w = 9usize;

    let sep = style::rule(" │ ");
    let left = style::rule("│ ");
    let right = style::rule(" │");
    let rule = style::rule(&"─".repeat(width));

    println!(
        "{left}{}{sep}{}{sep}{}{sep}{}{sep}{}{sep}{}{sep}{}{right}",
        header("NAME", name_w),
        header("ACTIONS", actions_w),
        header("TARGET/PATH", target_w),
        header("DURATION", duration_w),
        header("TIMERS", timers_w),
        header("LIVE", live_w),
        header("BUILTIN", builtin_w),
    );
    println!("{rule}");

    for b in &buzzers {
        let kinds: Vec<String> = b.actions.iter().map(action_label).collect();
        let target = b
            .targets
            .first()
            .cloned()
            .unwrap_or_else(|| "—".to_string());
        let duration = b
            .durations
            .first()
            .and_then(|d| d.clone())
            .unwrap_or_else(|| "—".to_string());
        let timer_s = format!("{}", b.timer_count);
        let live_s = format!("{}", b.live_count);
        let builtin = if b.builtin {
            style::builtin("[built-in]")
        } else {
            String::new()
        };
        println!(
            "{left}{}{sep}{}{sep}{}{sep}{}{sep}{}{sep}{}{sep}{}{right}",
            style::name(&pad(&truncate(&b.name, name_w), name_w)),
            pad(&truncate(&kinds.join(", "), actions_w), actions_w),
            pad(&truncate_target(&target, target_w), target_w),
            pad(&truncate(&duration, duration_w), duration_w),
            pad(&timer_s, timers_w),
            pad(&live_s, live_w),
            builtin,
        );
    }

    if buzzers.iter().any(|b| {
        b.actions.iter().any(|a| {
            matches!(
                a,
                BuzzerAction::CloseAllWindows
                    | BuzzerAction::CloseApplication(_)
                    | BuzzerAction::CloseWindow(_)
            )
        })
    }) {
        println!();
        println!(
            "{}",
            style::warn(
                "WARNING: close_windows / close_app / close_window buzzers close \
                 windows when they fire.\nRun `strangetimer confirm-destructive` \
                 to enable them."
            )
        );
    }

    Ok(())
}

/// `strangetimer view buzzer <name>` — detailed view of one buzzer: every
/// action with its target and duration, plus referencing timers.
pub fn view_buzzer(name: &str) -> Result<()> {
    let response = send_and_receive(&ClientMessage::GetBuzzerDetail {
        name: name.to_string(),
    })?;
    let detail = match response {
        ServerMessage::BuzzerDetailInfo(d) => d,
        ServerMessage::Error(e) => return Err(anyhow!(e)),
        other => return Err(anyhow!("unexpected daemon response: {other:?}")),
    };

    println!(
        "{} {}",
        style::header("Buzzer:"),
        style::name(&detail.info.name)
    );
    println!(
        "{} {}    {} {}    {} {}",
        style::dim("Timers using it:"),
        detail.info.timer_count,
        style::dim("Live runs:"),
        detail.info.live_count,
        style::dim("Built-in:"),
        if detail.info.builtin { "yes" } else { "no" },
    );
    println!();

    let (width, _) = crate::commands::view::terminal_size_pub();
    let type_w = 12usize;
    let target_w = (width * 60 / 100).clamp(16, 60);
    let duration_w = 10usize;
    let sep = style::rule(" │ ");
    let left = style::rule("│ ");
    let right = style::rule(" │");
    let rule = style::rule(&"─".repeat(width));
    println!(
        "{left}{}{sep}{}{sep}{}{right}",
        style::header("TYPE"),
        style::header("TARGET/PATH"),
        style::header("DURATION"),
    );
    println!("{rule}");
    for (i, action) in detail.info.actions.iter().enumerate() {
        let ty = action_label(action);
        let target = detail.info.targets.get(i).cloned().unwrap_or_default();
        let duration = detail
            .info
            .durations
            .get(i)
            .and_then(|d| d.clone())
            .unwrap_or_else(|| "—".to_string());
        println!(
            "{left}{}{sep}{}{sep}{}{right}",
            pad(&truncate(&ty, type_w), type_w),
            pad(&truncate_target(&target, target_w), target_w),
            pad(&truncate(&duration, duration_w), duration_w),
        );
    }

    if !detail.referencing_timers.is_empty() {
        println!();
        println!("{}", style::header("Referencing timers:"));
        for (timer, status) in &detail.referencing_timers {
            println!("  {} ({})", style::name(timer), status_label(status));
        }
    }

    Ok(())
}

fn status_label(status: &strangetimer_core::model::TimerStatus) -> String {
    match status {
        strangetimer_core::model::TimerStatus::Running => "running".to_string(),
        strangetimer_core::model::TimerStatus::Paused => "paused".to_string(),
        strangetimer_core::model::TimerStatus::Scheduled => "scheduled".to_string(),
        strangetimer_core::model::TimerStatus::Completed => "inactive".to_string(),
    }
}

/// Truncate a target that looks like a path, keeping the filename tail so
/// the useful part stays visible.
fn truncate_target(s: &str, max: usize) -> String {
    if s.width() <= max || !s.contains('/') {
        return truncate(s, max);
    }
    let budget = max.saturating_sub(1);
    let tail: String = s
        .chars()
        .rev()
        .take(budget)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    format!("…{tail}")
}

/// Truncate by terminal display width, appending `…` when cut.
fn truncate(s: &str, max: usize) -> String {
    if s.width() <= max {
        return s.to_string();
    }
    let budget = max.saturating_sub(1);
    let mut out = String::new();
    let mut w = 0usize;
    for ch in s.chars() {
        let cw = UnicodeWidthChar::width(ch).unwrap_or(0);
        if w + cw > budget {
            break;
        }
        out.push(ch);
        w += cw;
    }
    out.push('…');
    out
}

/// Pad by terminal display width.
fn pad(s: &str, max: usize) -> String {
    let w = s.width();
    if w >= max {
        return s.to_string();
    }
    format!("{s}{}", " ".repeat(max - w))
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
    if let Some(window) = &args.close_window {
        actions.push(BuzzerAction::CloseWindow(window.clone()));
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
             --close-window, --focus-window, --llm"
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
        BuzzerAction::CloseWindow(_) => "close_window".to_string(),
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
            close_window: None,
            focus_window: None,
            llm: None,
            no_preview: true,
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
