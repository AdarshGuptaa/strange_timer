use std::path::Path;

use strangetimer_core::model::SessionEnv;

/// Execute a script non-blocking in a detached child process, under the
/// latest-known interactive session environment. Returns a short error
/// string on spawn failure (surfaced as a `BuzzerEvent::outcome`).
pub fn fire_bash(path: &Path, env: &SessionEnv) -> Option<String> {
    #[cfg(unix)]
    let mut cmd = {
        let mut c = std::process::Command::new("sh");
        c.arg("-c").arg(path);
        c
    };
    #[cfg(windows)]
    let mut cmd = {
        let mut c = std::process::Command::new("cmd");
        c.arg("/C").arg(path);
        c
    };

    crate::platform::apply_session_env(&mut cmd, env);
    match cmd.spawn() {
        Ok(_) => None,
        Err(e) => {
            warn!("failed to run script {:?}: {e}", path.display());
            Some(format!("failed to run script {:?}: {e}", path.display()))
        }
    }
}
