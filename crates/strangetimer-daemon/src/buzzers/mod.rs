pub mod application;
pub mod audio;
pub mod bash;
pub mod close_application;
pub mod close_windows;
pub mod focus_window;
pub mod llm;
pub mod url;
pub mod video;

use strangetimer_core::model::BuzzerAction;

use crate::state::AppState;

/// Dispatch a single buzzer action. Multiple actions on one buzzer are fired
/// sequentially by the caller (`fire_buzzer` in main.rs).
pub async fn dispatch(state: &AppState, action: &BuzzerAction) {
    match action {
        BuzzerAction::DefaultAudio => audio::fire_audio(None),
        BuzzerAction::Audio(path) => audio::fire_audio(path.as_deref()),
        BuzzerAction::DefaultVideo => video::fire_video(None),
        BuzzerAction::Video(path) => video::fire_video(path.as_deref()),
        BuzzerAction::Application(path) => application::fire_application(path),
        BuzzerAction::Url(url) => url::fire_url(url),
        BuzzerAction::Bash(path) => bash::fire_bash(path),
        BuzzerAction::CloseAllWindows => {
            let confirmed = state.is_close_windows_confirmed().await;
            if !confirmed {
                eprintln!(
                    "WARNING: close_windows buzzer will close ALL open windows.\n\
                     Run `strangetimer confirm-destructive` to enable it."
                );
                return;
            }
            close_windows::fire_close_windows(std::process::id());
        }
        BuzzerAction::CloseApplication(name) => {
            let confirmed = state.is_close_windows_confirmed().await;
            if !confirmed {
                eprintln!(
                    "WARNING: close_app buzzer will close the {:?} application.\n\
                     Run `strangetimer confirm-destructive` to enable it.",
                    name
                );
                return;
            }
            close_application::fire_close_application(name);
        }
        BuzzerAction::FocusWindow(name) => focus_window::fire_focus_window(name),
        BuzzerAction::Llm { model, prompt } => {
            llm::fire_llm(model, prompt).await;
        }
    }
}
