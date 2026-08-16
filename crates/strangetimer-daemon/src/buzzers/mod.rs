pub mod application;
pub mod audio;
pub mod bash;
pub mod close_application;
pub mod close_window;
pub mod focus_window;
pub mod llm;
pub mod url;
pub mod video;

use strangetimer_core::model::BuzzerAction;

use crate::state::AppState;

/// Dispatch a single buzzer action. Multiple actions on one buzzer are fired
/// sequentially by the caller (`fire_buzzer` in main.rs).
///
/// Returns a short outcome string when the action was blocked or failed
/// (`None` on a clean fire). The caller surfaces it as a
/// `BuzzerEvent::outcome` so `strangetimer watch` reports why an alarm did
/// not run instead of letting failures vanish into `daemon.log`.
///
/// GUI-launching actions (video, URL, application, script) run under the
/// latest-known interactive-session environment, so they never target a
/// stale display from the daemon's own startup environment.
pub async fn dispatch(state: &AppState, action: &BuzzerAction) -> Option<String> {
    match action {
        BuzzerAction::DefaultAudio => {
            audio::fire_audio(None);
            None
        }
        BuzzerAction::Audio(path) => {
            audio::fire_audio(path.as_deref());
            None
        }
        BuzzerAction::DefaultVideo => {
            let env = state.session_env_sync();
            video::fire_video(None, &env)
        }
        BuzzerAction::Video(path) => {
            let env = state.session_env_sync();
            video::fire_video(path.as_deref(), &env)
        }
        BuzzerAction::Application(path) => {
            let env = state.session_env_sync();
            application::fire_application(path, &env)
        }
        BuzzerAction::Url(url) => {
            let env = state.session_env_sync();
            url::fire_url(url, &env)
        }
        BuzzerAction::Bash(path) => {
            let env = state.session_env_sync();
            bash::fire_bash(path, &env)
        }
        BuzzerAction::CloseAllWindows => {
            // Deprecated (Prompt 48): closing the *entire* desktop is too
            // destructive. Refuse with a migration hint instead.
            warn!(
                "the close_windows buzzer (close ALL windows) is deprecated and \
                 will not run. Use `--close-window <id-or-title>` to close a \
                 selected window, or `--close-app <name>` to close an application."
            );
            Some(
                "blocked: close_windows is deprecated — use --close-window or --close-app"
                    .to_string(),
            )
        }
        BuzzerAction::CloseApplication(name) => {
            let confirmed = state.is_close_windows_confirmed().await;
            if !confirmed {
                warn!(
                    "WARNING: close_app buzzer will close the {:?} application.\n\
                     Run `strangetimer confirm-destructive` to enable it.",
                    name
                );
                return Some("blocked: requires `strangetimer confirm-destructive`".to_string());
            }
            close_application::fire_close_application(name);
            None
        }
        BuzzerAction::CloseWindow(target) => {
            let confirmed = state.is_close_windows_confirmed().await;
            if !confirmed {
                warn!(
                    "WARNING: close_window buzzer will close {target:?}.\n\
                     Run `strangetimer confirm-destructive` to enable it."
                );
                return Some("blocked: requires `strangetimer confirm-destructive`".to_string());
            }
            close_window::fire_close_window(target);
            None
        }
        BuzzerAction::FocusWindow(name) => {
            focus_window::fire_focus_window(name);
            None
        }
        BuzzerAction::Llm { model, prompt } => {
            llm::fire_llm(model, prompt).await;
            None
        }
    }
}

/// Dispatch the actions of a buzzer that belongs to a `run -u` run, which
/// has already been paused and marked pending by `begin_interrupt`.
///
/// - Audio actions **loop** until the acknowledgement arrives (`resume`
///   clears the pending marker), replaying roughly once per playback. Each
///   loop runs in its own task so a waiting interrupt never blocks the
///   dispatch of other timers.
/// - All other actions fire once, as usual.
/// - After every action, the terminal window captured at run time is
///   focused so the user lands back on the CLI prompt.
pub async fn dispatch_interrupt(
    state: &std::sync::Arc<AppState>,
    actions: &[BuzzerAction],
    timer_name: &str,
) {
    for action in actions {
        match action {
            BuzzerAction::DefaultAudio | BuzzerAction::Audio(_) => {
                let state = std::sync::Arc::clone(state);
                let timer_name = timer_name.to_string();
                let path = match action {
                    BuzzerAction::Audio(p) => p.as_deref().map(std::path::PathBuf::from),
                    _ => None,
                };
                tokio::spawn(async move {
                    audio::fire_audio_until(path.as_deref(), move || {
                        pending_is(&state, &timer_name)
                    })
                    .await;
                });
            }
            _ => {
                let _ = dispatch(state, action).await;
            }
        }
    }

    // Return focus to the terminal the user ran `run -u` from.
    let focus = {
        let run = state.get_run(timer_name).await;
        run.and_then(|r| r.interrupt_focus)
    };
    if let Some(window) = focus {
        info!("user interrupt: focusing terminal");
        focus_window::fire_focus_window_retry(&window).await;
    }
}

/// Whether `timer_name` is still awaiting the interrupt acknowledgement.
fn pending_is(state: &AppState, timer_name: &str) -> bool {
    state
        .interrupt_pending_sync()
        .iter()
        .any(|p| p == timer_name)
}
