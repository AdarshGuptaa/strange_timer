use std::path::Path;

/// Launch an application in a detached child process.
pub fn fire_application(path: &Path) {
    match std::process::Command::new(path).spawn() {
        Ok(_) => {}
        Err(e) => eprintln!(
            "strangetimer-daemon: failed to launch application {:?}: {e}",
            path.display()
        ),
    }
}
