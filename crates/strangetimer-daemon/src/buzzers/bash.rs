use std::path::Path;

/// Execute a script non-blocking in a detached child process.
pub fn fire_bash(path: &Path) {
    #[cfg(unix)]
    let mut cmd = {
        let mut c = std::process::Command::new("sh");
        c.arg("-c").arg(path);
        c
    };
    #[cfg(windows)]
    let mut cmd = {
        let mut c = std::process::Command::new("cmd");
        c.args(["/C", path]);
        c
    };

    match cmd.spawn() {
        Ok(_) => {}
        Err(e) => eprintln!(
            "strangetimer-daemon: failed to run script {:?}: {e}",
            path.display()
        ),
    }
}
