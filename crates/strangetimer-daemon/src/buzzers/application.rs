use std::path::Path;

use strangetimer_core::model::SessionEnv;

/// Launch an application in a detached child process, under the latest-known
/// interactive session environment (so GUI apps land on the user's current
/// display, not whatever terminal the daemon was started from). Returns a
/// short error string on spawn failure (surfaced as a `BuzzerEvent::outcome`).
pub fn fire_application(path: &Path, env: &SessionEnv) -> Option<String> {
    let mut cmd = std::process::Command::new(path);
    crate::platform::apply_session_env(&mut cmd, env);
    match cmd.spawn() {
        Ok(_) => None,
        Err(e) => {
            warn!("failed to launch application {:?}: {e}", path.display());
            Some(format!(
                "failed to launch application {:?}: {e}",
                path.display()
            ))
        }
    }
}
