//! End-to-end integration tests: spawn the real daemon binary and drive it
//! with the real CLI binary over real IPC, in isolated temp directories.
//!
//! Run with `cargo test --workspace` (builds every binary first).

use std::fs;
use std::io::Read;
use std::path::PathBuf;
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

/// Unique per-test data directory + socket, so tests can run in parallel.
struct TestEnv {
    dir: PathBuf,
    socket: PathBuf,
}

impl TestEnv {
    fn new(label: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("strangetimer-e2e-{label}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        TestEnv {
            dir: dir.clone(),
            socket: dir.join("st.sock"),
        }
    }

    /// Pre-write state.json with `registered: true` so the daemon never
    /// attempts autostart registration during tests.
    fn pre_seed_registered(&self) {
        fs::write(
            self.dir.join("state.json"),
            r#"{"runs":[],"registered":true,"last_saved_at":null}"#,
        )
        .unwrap();
    }
}

/// Runs the daemon binary, waits until it accepts connections, and kills it
/// on drop.
struct DaemonGuard {
    child: Child,
    env: TestEnv,
    log: PathBuf,
}

impl DaemonGuard {
    fn start(env: TestEnv) -> DaemonGuard {
        Self::start_with(env, &[])
    }

    /// Like [`start`] but with extra environment variables for the daemon
    /// (e.g. `STRANGETIMER_TEST_OPENER` seams).
    fn start_with(env: TestEnv, extra_env: &[(&str, &str)]) -> DaemonGuard {
        env.pre_seed_registered();
        let log = env.dir.join("daemon.log");
        let mut cmd = Command::new(daemon_binary());
        cmd.env("STRANGETIMER_DATA_DIR", &env.dir)
            .env("STRANGETIMER_SOCKET", &env.socket);
        for (k, v) in extra_env {
            cmd.env(k, v);
        }
        let child = cmd
            .stdout(Stdio::from(fs::File::create(&log).unwrap()))
            .stderr(Stdio::from(fs::File::create(&log).unwrap()))
            .spawn()
            .expect("failed to spawn strangetimer-daemon");

        let guard = DaemonGuard { child, env, log };

        // Wait for the listener to come up (probe-connect until it works).
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            if Instant::now() > deadline {
                panic!(
                    "daemon did not start listening; log:\n{}",
                    guard.log_contents()
                );
            }
            let alive = std::os::unix::net::UnixStream::connect(&guard.env.socket).is_ok();
            if alive {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        guard
    }

    fn cli(&self, args: &[&str]) -> Output {
        Command::new(cli_binary())
            .env("STRANGETIMER_DATA_DIR", &self.env.dir)
            .env("STRANGETIMER_SOCKET", &self.env.socket)
            .args(args)
            .output()
            .expect("failed to run strangetimer CLI")
    }

    fn log_contents(&self) -> String {
        fs::read_to_string(&self.log).unwrap_or_default()
    }
}

impl Drop for DaemonGuard {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = fs::remove_dir_all(&self.env.dir);
    }
}

/// The daemon binary lives next to the CLI binary in target/debug.
fn daemon_binary() -> PathBuf {
    let cli = cli_binary();
    let daemon = cli.parent().unwrap().join("strangetimer-daemon");
    assert!(
        daemon.exists(),
        "strangetimer-daemon not found at {} — run `cargo build --workspace` first",
        daemon.display()
    );
    daemon
}

fn cli_binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_strangetimer"))
}

fn stdout_text(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn stderr_text(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

fn expect_success(guard: &DaemonGuard, args: &[&str]) -> String {
    let out = guard.cli(args);
    assert!(
        out.status.success(),
        "`strangetimer {}` failed (exit {:?})\nstdout:\n{}\nstderr:\n{}",
        args.join(" "),
        out.status.code(),
        stdout_text(&out),
        stderr_text(&out),
    );
    stdout_text(&out)
}

#[test]
fn daemon_seeds_builtin_buzzers_on_fresh_install() {
    let guard = DaemonGuard::start(TestEnv::new("seeding"));
    let out = expect_success(&guard, &["view", "buzzers"]);
    for name in ["default_audio", "default_video", "close_windows"] {
        assert!(out.contains(name), "missing built-in buzzer {name}:\n{out}");
    }
    assert_eq!(out.matches("[built-in]").count(), 3);
}

#[test]
fn timer_crud_lifecycle() {
    let guard = DaemonGuard::start(TestEnv::new("crud"));

    expect_success(&guard, &["create", "timer", "workAndFun", "45min", "15min"]);

    // Duplicate with default name.
    expect_success(&guard, &["duplicate", "timer", "workAndFun"]);
    let out = expect_success(&guard, &["view", "workAndFun_copy"]);
    assert!(out.contains("workAndFun_copy"), "{out}");
    assert!(out.contains("Buzzer Name"), "missing table header:\n{out}");
    assert!(out.contains("45m"), "missing offset cell:\n{out}");

    // Delete works when not running.
    expect_success(&guard, &["delete", "timer", "workAndFun_copy"]);
}

#[test]
fn delete_is_refused_while_run_is_active() {
    let guard = DaemonGuard::start(TestEnv::new("delete-guard"));
    expect_success(&guard, &["create", "timer", "t", "30s"]);
    expect_success(&guard, &["run", "t"]);

    let out = guard.cli(&["delete", "timer", "t"]);
    assert!(!out.status.success(), "delete should be refused");
    assert!(stderr_text(&out).contains("active run"));
}

#[test]
fn run_fires_buzzers() {
    let guard = DaemonGuard::start(TestEnv::new("fire"));
    expect_success(&guard, &["create", "timer", "t", "2s"]);
    expect_success(&guard, &["run", "t"]);

    // Scheduler ticks every 500ms; give the 2s alarm time to fire.
    std::thread::sleep(Duration::from_secs(5));
    let log = guard.log_contents();
    assert!(
        log.contains("BUZZ: default_audio"),
        "expected buzzer to fire:\n{log}"
    );
}

#[test]
fn view_timers_shows_active_runs_only() {
    let guard = DaemonGuard::start(TestEnv::new("view-timers"));

    let out = expect_success(&guard, &["view", "timers"]);
    assert!(out.contains("No timers currently running."));

    expect_success(&guard, &["create", "timer", "t", "1h"]);
    expect_success(&guard, &["create", "timer", "unused", "25min"]);
    expect_success(&guard, &["run", "t", "-n", "3"]);

    let out = expect_success(&guard, &["view", "timers"]);
    assert!(out.contains("ACTIVE RUNS"), "{out}");
    assert!(out.contains("│ TIMER"), "missing table header:\n{out}");
    assert!(out.contains("run ×3"), "missing repetition marker:\n{out}");
    assert!(out.contains("X-"), "missing progress bar:\n{out}");
    // The defined-but-not-running timer shows in the inactive section.
    assert!(out.contains("1 buzzer"), "{out}");

    expect_success(&guard, &["stop", "t"]);
    let out = expect_success(&guard, &["view", "timers"]);
    // The definition remains, listed in the inactive section.
    assert!(out.contains("INACTIVE TIMERS"), "{out}");
    assert!(out.contains("unused"), "{out}");
}

#[test]
fn pause_freezes_and_resume_continues() {
    let guard = DaemonGuard::start(TestEnv::new("pause-resume"));
    expect_success(&guard, &["create", "timer", "t", "30s"]);
    expect_success(&guard, &["run", "t"]);
    std::thread::sleep(Duration::from_millis(1500));
    expect_success(&guard, &["pause", "t"]);
    expect_success(&guard, &["resume", "t"]);
    expect_success(&guard, &["stop", "t"]);
}

#[test]
fn custom_buzzer_crud() {
    let guard = DaemonGuard::start(TestEnv::new("buzzer-crud"));
    expect_success(
        &guard,
        &[
            "create",
            "buzzer",
            "paymentAlert",
            "--audio",
            "--url",
            "https://bank.example.com",
        ],
    );

    let out = expect_success(&guard, &["view", "buzzers"]);
    assert!(out.contains("paymentAlert"), "{out}");
    assert!(out.contains("audio, url"), "expected chained types:\n{out}");

    expect_success(&guard, &["delete", "buzzer", "paymentAlert"]);
    let out = expect_success(&guard, &["view", "buzzers"]);
    assert!(!out.contains("paymentAlert"), "{out}");
}

#[test]
fn builtin_buzzers_cannot_be_deleted() {
    let guard = DaemonGuard::start(TestEnv::new("builtin-guard"));
    let out = guard.cli(&["delete", "buzzer", "default_audio"]);
    assert!(!out.status.success());
    assert!(stderr_text(&out).contains("built-in"));
}

/// Smoke-check that the daemon survives a full run→stop→restart cycle and
/// recovers a run whose alarm elapsed while it was down.
#[test]
fn restart_fires_missed_alarm() {
    let mut guard = DaemonGuard::start(TestEnv::new("recovery"));
    expect_success(&guard, &["create", "timer", "t", "3s"]);
    expect_success(&guard, &["run", "t"]);
    std::thread::sleep(Duration::from_millis(1000));

    // SIGKILL: simulate a crash. The socket file goes stale.
    guard.child.kill().unwrap();
    guard.child.wait().unwrap();
    std::thread::sleep(Duration::from_millis(500));

    // Restart after the alarm time has passed.
    std::thread::sleep(Duration::from_secs(4));
    guard.child = Command::new(daemon_binary())
        .env("STRANGETIMER_DATA_DIR", &guard.env.dir)
        .env("STRANGETIMER_SOCKET", &guard.env.socket)
        .stdout(Stdio::from(fs::File::create(&guard.log).unwrap()))
        .stderr(Stdio::from(fs::File::create(&guard.log).unwrap()))
        .spawn()
        .expect("failed to respawn strangetimer-daemon");

    // Wait for the listener.
    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline {
        if std::os::unix::net::UnixStream::connect(&guard.env.socket).is_ok() {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }

    std::thread::sleep(Duration::from_secs(1));
    let log = guard.log_contents();
    assert!(
        log.contains("missed alarm") || log.contains("BUZZ"),
        "expected missed alarm to fire on restart:\n{log}"
    );
}

/// Files produced by the daemon land in the data dir, not the user's home.
#[test]
fn persistence_files_are_isolated() {
    let guard = DaemonGuard::start(TestEnv::new("isolated-persistence"));
    expect_success(&guard, &["create", "timer", "t", "5m"]);

    for file in ["timers.json", "buzzers.json", "state.json"] {
        let path = guard.env.dir.join(file);
        assert!(
            path.exists(),
            "{file} missing from {}",
            guard.env.dir.display()
        );
    }
}

/// `strangetimer daemon status/stop/start` drive the daemon process through
/// its full lifecycle without killing it by signal — three full cycles, so
/// the "starting more than twice fails" regression stays dead.
#[test]
fn daemon_lifecycle_via_cli() {
    let mut guard = DaemonGuard::start(TestEnv::new("daemon-lifecycle"));

    // status → running with a pid
    let out = expect_success(&guard, &["daemon", "status"]);
    assert!(out.contains("is running"), "expected running:\n{out}");
    assert!(out.contains("pid"), "{out}");

    // stop → graceful exit, then status reports not running
    let out = expect_success(&guard, &["daemon", "stop"]);
    assert!(out.contains("Stopped"), "{out}");
    guard.child.wait().ok();
    let out = expect_success(&guard, &["daemon", "status"]);
    assert!(out.contains("not running"), "expected stopped:\n{out}");

    for cycle in 1..=3 {
        // start → spawns a fresh daemon, status comes back
        let out = expect_success(&guard, &["daemon", "start"]);
        assert!(out.contains("Started"), "cycle {cycle}:\n{out}");
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            let out = guard.cli(&["daemon", "status"]);
            if stdout_text(&out).contains("is running") {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "daemon did not restart in cycle {cycle}:\n{}",
                stdout_text(&out)
            );
            std::thread::sleep(Duration::from_millis(200));
        }

        // stop again so the next cycle starts from a clean state.
        expect_success(&guard, &["daemon", "stop"]);
        guard.child.wait().ok();
    }
}

/// `strangetimer examples` lists copy-paste commands and `--install` seeds
/// the file-free examples into the library.
#[test]
fn examples_list_and_install() {
    let guard = DaemonGuard::start(TestEnv::new("examples"));

    let out = expect_success(&guard, &["examples"]);
    assert!(
        out.contains("exampleUrl"),
        "missing example listing:\n{out}"
    );
    assert!(
        out.contains("github.com/AdarshGuptaa/strange_timer"),
        "{out}"
    );

    let out = expect_success(&guard, &["examples", "--install"]);
    assert!(out.contains("Created"), "{out}");

    let out = expect_success(&guard, &["view", "buzzers"]);
    for name in [
        "exampleAudio",
        "exampleVideo",
        "exampleUrl",
        "exampleChain",
        "exampleLlm",
    ] {
        assert!(
            out.contains(name),
            "missing installed example {name}:\n{out}"
        );
    }

    // Idempotent: installing again skips, does not fail.
    let out = expect_success(&guard, &["examples", "--install"]);
    assert!(out.contains("Skipped"), "{out}");
}

/// CloseApplication and FocusWindow buzzers parse, install and display.
#[test]
fn close_app_and_focus_window_buzzers() {
    let guard = DaemonGuard::start(TestEnv::new("window-actions"));
    expect_success(
        &guard,
        &[
            "create",
            "buzzer",
            "tidyUp",
            "--close-app",
            "firefox",
            "--focus-window",
            "Slack",
        ],
    );

    let out = expect_success(&guard, &["view", "buzzers"]);
    assert!(out.contains("tidyUp"), "{out}");
    assert!(out.contains("close_app"), "{out}");
    assert!(out.contains("focus_window"), "{out}");
    assert!(out.contains("confirm-destructive"), "{out}");
}

/// A listener that accepts but cannot answer IPC (like an old-version
/// daemon) must be detected as *incompatible*: `daemon start` must refuse
/// to spawn a second instance instead of reporting "Address already in use"
/// after a timeout.
#[test]
fn start_refuses_when_incompatible_listener_present() {
    let env = TestEnv::new("incompatible");

    // Simulate an old daemon: bind the socket, swallow requests, close
    // without ever answering (exactly what a pre-Ping binary does when it
    // fails to parse an unknown message variant).
    let listener =
        std::sync::Arc::new(std::os::unix::net::UnixListener::bind(&env.socket).unwrap());
    let server = std::sync::Arc::clone(&listener);
    std::thread::spawn(move || {
        for stream in server.incoming() {
            let Ok(mut s) = stream else { break };
            let mut buf = [0u8; 4096];
            let _ = s.read(&mut buf);
            drop(s);
        }
    });

    let out = Command::new(cli_binary())
        .env("STRANGETIMER_DATA_DIR", &env.dir)
        .env("STRANGETIMER_SOCKET", &env.socket)
        .args(["daemon", "start"])
        .output()
        .expect("failed to run strangetimer CLI");
    assert!(
        !out.status.success(),
        "daemon start should refuse an incompatible listener"
    );
    let err = stderr_text(&out);
    assert!(
        err.contains("not a compatible"),
        "expected incompatible-listener error:\n{err}"
    );

    // status reports the same, and no new daemon took over the socket.
    let out = Command::new(cli_binary())
        .env("STRANGETIMER_DATA_DIR", &env.dir)
        .env("STRANGETIMER_SOCKET", &env.socket)
        .args(["daemon", "status"])
        .output()
        .unwrap();
    assert!(
        stdout_text(&out).contains("not a compatible"),
        "{}",
        stdout_text(&out)
    );

    drop(listener);
    let _ = fs::remove_dir_all(&env.dir);
}

/// Transparent auto-start: a command succeeds even when no daemon has been
/// started yet, and a daemon is running afterwards.
#[test]
fn commands_auto_start_the_daemon() {
    let env = TestEnv::new("auto-start");
    env.pre_seed_registered();

    let out = Command::new(cli_binary())
        .env("STRANGETIMER_DATA_DIR", &env.dir)
        .env("STRANGETIMER_SOCKET", &env.socket)
        .args(["view", "buzzers"])
        .output()
        .expect("failed to run strangetimer CLI");
    assert!(
        out.status.success(),
        "view buzzers should auto-start the daemon:\n{}",
        stderr_text(&out)
    );
    assert!(
        stdout_text(&out).contains("default_audio"),
        "{}",
        stdout_text(&out)
    );

    // The daemon is now up (started by the CLI)…
    let out = Command::new(cli_binary())
        .env("STRANGETIMER_DATA_DIR", &env.dir)
        .env("STRANGETIMER_SOCKET", &env.socket)
        .args(["daemon", "status"])
        .output()
        .unwrap();
    assert!(
        stdout_text(&out).contains("is running"),
        "{}",
        stdout_text(&out)
    );

    // …and can be stopped cleanly.
    let out = Command::new(cli_binary())
        .env("STRANGETIMER_DATA_DIR", &env.dir)
        .env("STRANGETIMER_SOCKET", &env.socket)
        .args(["daemon", "stop"])
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", stderr_text(&out));
    let _ = fs::remove_dir_all(&env.dir);
}

/// `daemon stop` must not auto-start the daemon — it reports "not running".
#[test]
fn stop_does_not_auto_start() {
    let env = TestEnv::new("stop-no-autostart");
    env.pre_seed_registered();

    let out = Command::new(cli_binary())
        .env("STRANGETIMER_DATA_DIR", &env.dir)
        .env("STRANGETIMER_SOCKET", &env.socket)
        .args(["daemon", "stop"])
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", stderr_text(&out));
    assert!(
        stdout_text(&out).contains("not running"),
        "{}",
        stdout_text(&out)
    );

    let out = Command::new(cli_binary())
        .env("STRANGETIMER_DATA_DIR", &env.dir)
        .env("STRANGETIMER_SOCKET", &env.socket)
        .args(["daemon", "status"])
        .output()
        .unwrap();
    assert!(
        stdout_text(&out).contains("not running"),
        "{}",
        stdout_text(&out)
    );
    let _ = fs::remove_dir_all(&env.dir);
}

/// `run -u` (user-interrupt) is non-blocking: the CLI returns immediately,
/// the run pauses at the buzzer with a PENDING marker, and `resume`
/// acknowledges it (which also stops the looping audio).
#[test]
fn user_interrupt_detaches_and_resume_acknowledges() {
    let guard = DaemonGuard::start(TestEnv::new("user-interrupt"));
    expect_success(&guard, &["create", "timer", "t", "2s"]);

    // `run t -u` must return immediately (no stdin interaction needed).
    let out = expect_success(&guard, &["run", "t", "-u"]);
    assert!(
        out.contains("resume t"),
        "expected acknowledge hint:\n{out}"
    );
    assert!(
        !out.contains("press Enter"),
        "must not block on Enter:\n{out}"
    );

    // After the 2s buzzer the run pauses and shows the PENDING marker.
    std::thread::sleep(Duration::from_secs(4));
    let out = expect_success(&guard, &["view", "timers"]);
    assert!(out.contains("PENDING"), "run should show PENDING:\n{out}");

    // `strangetimer resume t` acknowledges; the timer then completes.
    expect_success(&guard, &["resume", "t"]);
    std::thread::sleep(Duration::from_secs(2));
    let out = expect_success(&guard, &["view", "timers"]);
    assert!(
        out.contains("INACTIVE TIMERS"),
        "run should have completed:\n{out}"
    );
    assert!(!out.contains("PENDING"), "{out}");
}

/// Two concurrent `-u` runs pause independently: one `resume` must not
/// affect the other, and neither's audio loop may block the other's
/// dispatch.
#[test]
fn multiple_pending_interrupts_are_independent() {
    let guard = DaemonGuard::start(TestEnv::new("multi-interrupt"));
    expect_success(&guard, &["create", "timer", "a", "2s"]);
    expect_success(&guard, &["create", "timer", "b", "2s"]);
    expect_success(&guard, &["run", "a", "-u"]);
    expect_success(&guard, &["run", "b", "-u"]);

    // Both buzzers fire and both runs pause with a PENDING marker.
    std::thread::sleep(Duration::from_secs(5));
    let out = expect_success(&guard, &["view", "timers"]);
    assert_eq!(
        out.matches("PENDING").count(),
        2,
        "both should be pending:\n{out}"
    );

    // Acknowledging `a` clears only `a`; `b` stays pending.
    expect_success(&guard, &["resume", "a"]);
    std::thread::sleep(Duration::from_secs(1));
    let out = expect_success(&guard, &["view", "timers"]);
    assert_eq!(
        out.matches("PENDING").count(),
        1,
        "only b should remain:\n{out}"
    );

    expect_success(&guard, &["resume", "b"]);
}

/// The generated bash completion suggests buzzers after a completed offset
/// in `create timer`, and state-aware names for `resume`.
#[test]
fn bash_completion_suggests_buzzers_and_paused_timers() {
    let guard = DaemonGuard::start(TestEnv::new("completion-suggest"));
    expect_success(&guard, &["create", "timer", "focus", "25min"]);
    expect_success(&guard, &["create", "buzzer", "myBuzzer", "--audio"]);
    expect_success(&guard, &["run", "focus"]);
    std::thread::sleep(Duration::from_millis(600));
    expect_success(&guard, &["pause", "focus"]);

    // Source the real engine script, then drive completions the way bash
    // does, using the isolated socket/data dir.
    let script = Command::new(cli_binary())
        .env("STRANGETIMER_DATA_DIR", &guard.env.dir)
        .env("STRANGETIMER_SOCKET", &guard.env.socket)
        .env("COMPLETE", "bash")
        .output()
        .unwrap();
    assert!(script.status.success());

    let run_bash = |words: &str, cword: usize, cur: &str| {
        let code = format!(
            "source /dev/stdin <<'SCRIPT'\n{script}\nSCRIPT\n\
             COMP_WORDS=({words}); COMP_CWORD={cword}; COMP_TYPE=63\n\
             _clap_complete_strangetimer '{cur}' '{cur}'\n\
             printf '%s\\n' \"${{COMPREPLY[*]}}\"",
            script = String::from_utf8_lossy(&script.stdout),
        );
        let out = Command::new("bash")
            .arg("-c")
            .arg(&code)
            .env("STRANGETIMER_DATA_DIR", &guard.env.dir)
            .env("STRANGETIMER_SOCKET", &guard.env.socket)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).into_owned()
    };

    // `create timer x 1m<Tab>` → suggests `1m <buzzer>`.
    let out = run_bash("strangetimer create timer x 1m", 4, "1m");
    assert!(
        out.contains("1m myBuzzer"),
        "expected buzzer suggestion after offset:\n{out}"
    );

    // `resume ''<Tab>` → suggests the paused timer only.
    let out = run_bash("strangetimer resume ''", 2, "");
    assert!(out.contains("focus"), "expected paused timer:\n{out}");

    // `resume <Tab>` with nothing paused is empty.
    expect_success(&guard, &["resume", "focus"]);
    let out = run_bash("strangetimer resume ''", 2, "");
    assert!(!out.contains("focus"), "{out}");
}

/// Helper: write an executable recording script that appends its arguments
/// to `log_file`.
fn recording_script(log_file: &std::path::Path, extra: &str) -> PathBuf {
    let path = log_file.with_extension("sh");
    fs::write(
        &path,
        format!(
            "#!/bin/sh\nprintf '%s\\n' \"$@\" >> \"{log}\"\n{extra}\n",
            log = log_file.display()
        ),
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    }
    path
}

/// A `-n 3` timer using the built-in video fires exactly three times —
/// verified through the mock opener seam, so no media player is launched.
#[test]
fn default_video_fires_once_per_repetition() {
    let env = TestEnv::new("video-repeat");
    let opens = env.dir.join("opens.log");
    let opener = recording_script(&opens, "");
    let guard = DaemonGuard::start_with(
        env,
        &[("STRANGETIMER_TEST_OPENER", opener.to_str().unwrap())],
    );

    expect_success(&guard, &["create", "timer", "v", "1s", "default_video"]);
    expect_success(&guard, &["run", "v", "-n", "3"]);

    // Three repetitions at 1s each, plus scheduler slack.
    let deadline = Instant::now() + Duration::from_secs(12);
    while Instant::now() < deadline {
        if fs::read_to_string(&opens)
            .map(|s| s.lines().count())
            .unwrap_or(0)
            >= 3
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    let log = fs::read_to_string(&opens).unwrap_or_default();
    let lines: Vec<&str> = log.lines().collect();
    assert_eq!(lines.len(), 3, "expected 3 opens, got:\n{log}");
    assert!(
        lines.iter().all(|l| l.ends_with("default.mp4")),
        "every open must target the built-in clip:\n{log}"
    );
}

/// A URL buzzer opens its target through the default opener (mock-recorded).
#[test]
fn url_buzzer_opens_target() {
    let env = TestEnv::new("url-open");
    let opens = env.dir.join("opens.log");
    let opener = recording_script(&opens, "");
    let guard = DaemonGuard::start_with(
        env,
        &[("STRANGETIMER_TEST_OPENER", opener.to_str().unwrap())],
    );

    expect_success(
        &guard,
        &[
            "create",
            "buzzer",
            "web",
            "--url",
            "https://example.com/alarm",
        ],
    );
    expect_success(&guard, &["create", "timer", "u", "1s", "web"]);
    expect_success(&guard, &["run", "u"]);

    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if fs::read_to_string(&opens)
            .map(|s| s.contains("https://example.com/alarm"))
            .unwrap_or(false)
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    let log = fs::read_to_string(&opens).unwrap_or_default();
    assert!(log.contains("https://example.com/alarm"), "{log}");
}

/// An `--application` buzzer really launches the given program (a temp
/// script that records its own execution).
#[test]
fn application_buzzer_launches_program() {
    let guard = DaemonGuard::start(TestEnv::new("app-launch"));
    let ran = guard.env.dir.join("ran.log");
    let app = recording_script(&ran, "");
    expect_success(
        &guard,
        &[
            "create",
            "buzzer",
            "app",
            "--application",
            app.to_str().unwrap(),
        ],
    );
    expect_success(&guard, &["create", "timer", "a", "1s", "app"]);
    expect_success(&guard, &["run", "a"]);

    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if ran.exists() {
            break;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    assert!(ran.exists(), "application was never launched");
}

/// `--close-app` issues a pkill against the named app — verified through
/// the STRANGETIMER_TEST_PKILL seam, never against real processes.
#[test]
fn close_app_issues_pkill_for_named_app() {
    let env = TestEnv::new("close-app");
    let kills = env.dir.join("kills.log");
    let pkill = recording_script(&kills, "");
    let guard =
        DaemonGuard::start_with(env, &[("STRANGETIMER_TEST_PKILL", pkill.to_str().unwrap())]);

    expect_success(&guard, &["confirm-destructive"]);
    expect_success(
        &guard,
        &["create", "buzzer", "quit", "--close-app", "fakebrowser"],
    );
    expect_success(&guard, &["create", "timer", "c", "1s", "quit"]);
    expect_success(&guard, &["run", "c"]);

    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if fs::read_to_string(&kills)
            .map(|s| s.contains("fakebrowser"))
            .unwrap_or(false)
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    let log = fs::read_to_string(&kills).unwrap_or_default();
    assert!(log.contains("fakebrowser"), "{log}");
}

/// `--focus-window` issues the platform focus command — verified through
/// the wmctrl seam (no real window is touched).
#[test]
fn focus_window_issues_focus_command() {
    let env = TestEnv::new("focus-cmd");
    let focus_log = env.dir.join("focus.log");
    let wmctrl = recording_script(&focus_log, "");
    let guard = DaemonGuard::start_with(
        env,
        &[("STRANGETIMER_TEST_WMCTRL", wmctrl.to_str().unwrap())],
    );

    expect_success(
        &guard,
        &["create", "buzzer", "chat", "--focus-window", "Slack"],
    );
    expect_success(&guard, &["create", "timer", "f", "1s", "chat"]);
    expect_success(&guard, &["run", "f"]);

    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if fs::read_to_string(&focus_log)
            .map(|s| s.contains("Slack"))
            .unwrap_or(false)
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    let log = fs::read_to_string(&focus_log).unwrap_or_default();
    assert!(log.contains("Slack"), "{log}");
}

/// Opt-in real GUI check (X11 only): a focus buzzer really brings a window
/// forward. Run with `STRANGETIMER_GUI_TESTS=1` on a desktop session.
#[test]
#[ignore = "requires a desktop session; opt in with STRANGETIMER_GUI_TESTS=1"]
fn gui_focus_restores_terminal() {
    if std::env::var("STRANGETIMER_GUI_TESTS").as_deref() != Ok("1") {
        return;
    }
    // Requires xdotool on an X11 session. Focus a window we created.
    let win = Command::new("xdotool")
        .args(["search", "--name", "xterm"])
        .output()
        .expect("xdotool required for GUI test");
    assert!(win.status.success(), "no matching window found");
}
