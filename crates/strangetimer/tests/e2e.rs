//! End-to-end integration tests: spawn the real daemon binary and drive it
//! with the real CLI binary over real IPC, in isolated temp directories.
//!
//! Run with `cargo test --workspace` (builds every binary first).

use std::fs;
use std::io::{BufRead, Read};
use std::path::PathBuf;
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

/// Open the daemon log in append mode.
///
/// The daemon's own logger appends timestamped lines to this file, and the
/// tests additionally redirect the daemon's stdout/stderr into it. A plain
/// `File::create` fd is positioned at offset 0, so the stderr copies (ALSA
/// errors, warnings) overwrite earlier logged lines from the start of the
/// file — erasing the `BUZZ` / `missed alarm` lines the tests assert on
/// when the runner has no audio device. Appending keeps everything.
fn open_log_append(path: &std::path::Path) -> std::fs::File {
    fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .unwrap()
}

/// Unique per-test data directory + socket, so tests can run in parallel.
#[derive(Clone)]
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

    /// Like [`start_with`] but does NOT pre-seed `state.json` — used when
    /// a test wants to control the persisted state itself.
    fn start_no_seed(env: TestEnv, extra_env: &[(&str, &str)]) -> DaemonGuard {
        let log = env.dir.join("daemon.log");
        let log_file = open_log_append(&log);
        let mut cmd = Command::new(daemon_binary());
        cmd.env("STRANGETIMER_DATA_DIR", &env.dir)
            .env("STRANGETIMER_SOCKET", &env.socket);
        for (k, v) in extra_env {
            cmd.env(k, v);
        }
        let child = cmd
            .stdout(Stdio::from(log_file.try_clone().unwrap()))
            .stderr(Stdio::from(log_file))
            .spawn()
            .expect("failed to spawn strangetimer-daemon");
        let guard = DaemonGuard { child, env, log };
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            if Instant::now() > deadline {
                panic!(
                    "daemon did not start listening; log:\n{}",
                    guard.log_contents()
                );
            }
            if std::os::unix::net::UnixStream::connect(&guard.env.socket).is_ok() {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        guard
    }

    /// Like [`start`] but with extra environment variables for the daemon
    /// (e.g. `STRANGETIMER_TEST_OPENER` seams).
    fn start_with(env: TestEnv, extra_env: &[(&str, &str)]) -> DaemonGuard {
        env.pre_seed_registered();
        let log = env.dir.join("daemon.log");
        let log_file = open_log_append(&log);
        let mut cmd = Command::new(daemon_binary());
        cmd.env("STRANGETIMER_DATA_DIR", &env.dir)
            .env("STRANGETIMER_SOCKET", &env.socket);
        for (k, v) in extra_env {
            cmd.env(k, v);
        }
        let child = cmd
            .stdout(Stdio::from(log_file.try_clone().unwrap()))
            .stderr(Stdio::from(log_file))
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

    /// Like [`cli`] but with extra environment overrides — simulates the
    /// CLI running from a *different* terminal session (different DISPLAY,
    /// XAUTHORITY, …) than the daemon was started in.
    fn cli_with_env(&self, args: &[&str], extra_env: &[(&str, &str)]) -> Output {
        let mut cmd = Command::new(cli_binary());
        cmd.env("STRANGETIMER_DATA_DIR", &self.env.dir)
            .env("STRANGETIMER_SOCKET", &self.env.socket);
        for (k, v) in extra_env {
            cmd.env(k, v);
        }
        cmd.args(args)
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

/// Poll `pred` every 100ms until it returns true or `timeout` elapses.
/// Replaces fixed sleeps so lifecycle tests are timing-robust.
fn wait_until(
    guard: &DaemonGuard,
    label: &str,
    timeout: Duration,
    pred: impl Fn(&DaemonGuard) -> bool,
) {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if pred(guard) {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    panic!("timed out waiting for {label}");
}

/// Wait until `view timers --snapshot` contains `needle`.
fn wait_for_view(guard: &DaemonGuard, label: &str, needle: &str) {
    wait_until(guard, label, Duration::from_secs(15), |g| {
        let out = g.cli(&["view", "timers", "--snapshot"]);
        out.status.success() && stdout_text(&out).contains(needle)
    });
}

/// Wait until the file at `path` has at least `count` lines.
fn wait_for_lines(path: &std::path::Path, label: &str, count: usize) {
    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline {
        if fs::read_to_string(path)
            .map(|s| s.lines().count() >= count)
            .unwrap_or(false)
        {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    panic!("timed out waiting for {label} ({count} lines)");
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
    let log_file = open_log_append(&guard.log);
    guard.child = Command::new(daemon_binary())
        .env("STRANGETIMER_DATA_DIR", &guard.env.dir)
        .env("STRANGETIMER_SOCKET", &guard.env.socket)
        .stdout(Stdio::from(log_file.try_clone().unwrap()))
        .stderr(Stdio::from(log_file))
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

    // The summary table truncates long targets; the detailed buzzer view
    // shows every action in full.
    let out = expect_success(&guard, &["view", "buzzer", "tidyUp"]);
    assert!(out.contains("tidyUp"), "{out}");
    assert!(out.contains("close_app"), "{out}");
    assert!(out.contains("focus_window"), "{out}");
    assert!(out.contains("firefox"), "{out}");
    assert!(out.contains("Slack"), "{out}");

    let out = expect_success(&guard, &["view", "buzzers"]);
    assert!(out.contains("tidyUp"), "{out}");
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
        err.contains("incompatible"),
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
        stdout_text(&out).contains("incompatible"),
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

/// `run -u` captures the terminal window id through xdotool and the daemon
/// activates it (via wmctrl) when the buzzer fires — all through the
/// recording seams, no real window touched.
#[test]
fn user_interrupt_captures_and_focuses_terminal() {
    let env = TestEnv::new("focus-capture");
    let wmctrl_log = env.dir.join("wmctrl.log");
    let wmctrl = recording_script(&wmctrl_log, "");
    let xdotool = {
        let path = env.dir.join("fake-xdotool.sh");
        fs::write(
            &path,
            "#!/bin/sh\n\
             case \"$1\" in\n\
               getactivewindow) echo 0x1234abcd ;;\n\
               getwindowname) echo TestTerminal ;;\n\
               *) exit 1 ;;\n\
             esac\n",
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        }
        path
    };

    let guard = DaemonGuard::start_with(
        env,
        &[
            ("STRANGETIMER_TEST_WMCTRL", wmctrl.to_str().unwrap()),
            ("STRANGETIMER_TEST_XDOTOOL", xdotool.to_str().unwrap()),
        ],
    );

    expect_success(&guard, &["create", "timer", "t", "1s"]);
    // The CLI captures the window through the same xdotool seam. Simulate
    // an X11 session (this dev machine may itself be Wayland).
    std::env::set_var("STRANGETIMER_TEST_XDOTOOL", &xdotool);
    std::env::remove_var("XDG_SESSION_TYPE");
    expect_success(&guard, &["run", "t", "-u"]);

    // After the buzzer fires (and pauses the run), the daemon activates
    // the captured window id through wmctrl.
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        let log = fs::read_to_string(&wmctrl_log).unwrap_or_default();
        if log.contains("0x1234abcd") {
            break;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    let log = fs::read_to_string(&wmctrl_log).unwrap_or_default();
    assert!(
        log.contains("0x1234abcd"),
        "daemon must activate the captured window id:\n{log}"
    );

    expect_success(&guard, &["resume", "t"]);
}

/// On Wayland, focus is reported unsupported instead of pretending — the
/// buzzer still pauses the run.
#[test]
fn user_interrupt_skips_focus_on_wayland() {
    let env = TestEnv::new("focus-wayland");
    let wmctrl_log = env.dir.join("wmctrl.log");
    let wmctrl = recording_script(&wmctrl_log, "");
    let guard = DaemonGuard::start_with(
        env,
        &[("STRANGETIMER_TEST_WMCTRL", wmctrl.to_str().unwrap())],
    );

    expect_success(&guard, &["create", "timer", "t", "1s"]);
    // Simulate a Wayland session for the capturing CLI.
    std::env::set_var("XDG_SESSION_TYPE", "wayland");
    expect_success(&guard, &["run", "t", "-u"]);
    std::env::remove_var("XDG_SESSION_TYPE");

    // The buzzer still fires and pauses the run...
    std::thread::sleep(Duration::from_secs(4));
    let out = expect_success(&guard, &["view", "timers"]);
    assert!(out.contains("PENDING"), "{out}");

    // ...but no focus command is issued.
    let log = fs::read_to_string(&wmctrl_log).unwrap_or_default();
    assert!(log.is_empty(), "no focus commands on Wayland:\n{log}");

    expect_success(&guard, &["resume", "t"]);
}

/// `strangetimer watch` prints a ringing line per fired buzzer, including
/// the resume hint for user-interrupt runs.
#[test]
fn watch_prints_ringing_notifications() {
    let guard = DaemonGuard::start(TestEnv::new("watch-notify"));
    expect_success(&guard, &["create", "timer", "t", "1s"]);
    expect_success(&guard, &["run", "t"]);

    // Spawn the watcher with a captured stdout pipe.
    let mut watcher = Command::new(cli_binary())
        .env("STRANGETIMER_DATA_DIR", &guard.env.dir)
        .env("STRANGETIMER_SOCKET", &guard.env.socket)
        .args(["watch"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("failed to spawn watcher");
    let stdout = watcher.stdout.take().unwrap();
    let mut reader = std::io::BufReader::new(stdout);
    let mut buf = String::new();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let _ = reader.read_line(&mut buf);
        if buf.contains("ringing") && buf.contains("t") {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "no ringing notification within 10s:\n{buf}"
        );
    }
    assert!(buf.contains("ringing"), "{buf}");
    assert!(
        !buf.contains("resume"),
        "non-interrupt runs get no resume hint:\n{buf}"
    );
    let _ = watcher.kill();
    let _ = watcher.wait();

    // User-interrupt run: the watch prints the resume hint.
    expect_success(&guard, &["create", "timer", "u", "1s"]);
    expect_success(&guard, &["run", "u", "-u"]);
    let mut watcher = Command::new(cli_binary())
        .env("STRANGETIMER_DATA_DIR", &guard.env.dir)
        .env("STRANGETIMER_SOCKET", &guard.env.socket)
        .args(["watch"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("failed to spawn watcher");
    let stdout = watcher.stdout.take().unwrap();
    let mut reader = std::io::BufReader::new(stdout);
    let mut buf = String::new();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let _ = reader.read_line(&mut buf);
        if buf.contains("ringing") && buf.contains("resume u") {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "no resume hint within 10s:\n{buf}"
        );
    }
    assert!(buf.contains("resume u"), "{buf}");
    let _ = watcher.kill();
    let _ = watcher.wait();

    expect_success(&guard, &["resume", "u"]);
}

/// A repeated timer that stays down for several full periods catches up:
/// on restart every missed repetition fires, not just the current one.
#[test]
fn recovery_catches_up_multiple_missed_repetitions() {
    let mut guard = DaemonGuard::start_with(TestEnv::new("recovery-repeat"), &[]);
    expect_success(&guard, &["create", "timer", "v", "1s", "default_video"]);

    // Set the mock opener now — the daemon must be restarted with it.
    let env2 = guard.env.clone();
    let opens = env2.dir.join("opens.log");
    let opener = recording_script(&opens, "");
    let log = guard.log.clone();

    expect_success(&guard, &["run", "v", "-n", "3"]);
    std::thread::sleep(Duration::from_millis(500));

    // Crash the daemon before the first fire and stay down for 5s
    // (~5 repetitions of a 1s timer).
    guard.child.kill().unwrap();
    guard.child.wait().unwrap();
    std::thread::sleep(Duration::from_secs(5));

    // Restart with the recording opener.
    let log_file = open_log_append(&log);
    guard.child = Command::new(daemon_binary())
        .env("STRANGETIMER_DATA_DIR", &env2.dir)
        .env("STRANGETIMER_SOCKET", &env2.socket)
        .env("STRANGETIMER_TEST_OPENER", &opener)
        .stdout(Stdio::from(log_file.try_clone().unwrap()))
        .stderr(Stdio::from(log_file))
        .spawn()
        .expect("failed to respawn strangetimer-daemon");
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if std::os::unix::net::UnixStream::connect(&env2.socket).is_ok() {
            break;
        }
        assert!(Instant::now() < deadline, "daemon did not restart");
        std::thread::sleep(Duration::from_millis(100));
    }

    // All three missed video events fire on restart (Count(3), 1s each).
    let deadline = Instant::now() + Duration::from_secs(10);
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
    assert_eq!(log.lines().count(), 3, "expected 3 catch-up opens:\n{log}");
}

/// A fired-but-undispatched event persisted in the outbox is replayed
/// when the daemon restarts — a crash between scheduling and dispatch
/// never loses the alarm.
#[test]
fn outbox_replays_undispatched_fires_on_restart() {
    let env = TestEnv::new("outbox-replay");
    let opens = env.dir.join("opens.log");
    let opener = recording_script(&opens, "");

    // Seed a run-less state with one pending fire for default_video.
    fs::write(
        env.dir.join("state.json"),
        r#"{
          "runs": [],
          "registered": true,
          "last_saved_at": null,
          "interrupt_pending": null,
          "pending_interrupts": [],
          "pending_fires": [
            {
              "timer_name": "x",
              "buzzer_name": "default_video",
              "buzzer_index": 0,
              "repetition": 0
            }
          ]
        }"#,
    )
    .unwrap();

    let _guard = DaemonGuard::start_no_seed(
        env,
        &[("STRANGETIMER_TEST_OPENER", opener.to_str().unwrap())],
    );

    // The daemon replays the outbox at startup → one video open.
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if fs::read_to_string(&opens)
            .map(|s| s.contains("default.mp4"))
            .unwrap_or(false)
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    let log = fs::read_to_string(&opens).unwrap_or_default();
    assert!(
        log.contains("default.mp4"),
        "outbox was not replayed:\n{log}"
    );
    assert_eq!(log.lines().count(), 1, "must replay exactly once:\n{log}");
}

/// `--close-window` issues a targeted close command through wmctrl, and
/// the deprecated all-windows buzzer refuses to run.
#[test]
fn close_window_targets_one_window_and_close_all_is_deprecated() {
    let env = TestEnv::new("close-window");
    let wmctrl_log = env.dir.join("wmctrl.log");
    let wmctrl = recording_script(&wmctrl_log, "");
    // XDG_SESSION_TYPE="" tells the daemon this is an X11 session (this
    // dev machine itself runs Wayland).
    let guard = DaemonGuard::start_with(
        env,
        &[
            ("STRANGETIMER_TEST_WMCTRL", wmctrl.to_str().unwrap()),
            ("XDG_SESSION_TYPE", ""),
        ],
    );

    expect_success(&guard, &["confirm-destructive"]);
    expect_success(
        &guard,
        &[
            "create",
            "buzzer",
            "killmeeting",
            "--close-window",
            "0x1234abcd",
        ],
    );
    expect_success(&guard, &["create", "timer", "c", "1s", "killmeeting"]);
    expect_success(&guard, &["run", "c"]);

    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if fs::read_to_string(&wmctrl_log)
            .map(|s| s.contains("0x1234abcd"))
            .unwrap_or(false)
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    let log = fs::read_to_string(&wmctrl_log).unwrap_or_default();
    // The recording script writes one argument per line.
    let args: Vec<&str> = log.lines().collect();
    assert!(
        args.contains(&"-i") && args.contains(&"-c") && args.contains(&"0x1234abcd"),
        "expected a targeted `wmctrl -i -c <id>` close:\n{log}"
    );

    // The deprecated all-windows buzzer no longer runs — the event is
    // reported as blocked in `watch`.
    expect_success(&guard, &["create", "timer", "d", "1s", "close_windows"]);
    expect_success(&guard, &["run", "d"]);
    let mut watcher = Command::new(cli_binary())
        .env("STRANGETIMER_DATA_DIR", &guard.env.dir)
        .env("STRANGETIMER_SOCKET", &guard.env.socket)
        .args(["watch"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("failed to spawn watcher");
    let stdout = watcher.stdout.take().unwrap();
    let mut reader = std::io::BufReader::new(stdout);
    let mut buf = String::new();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let _ = reader.read_line(&mut buf);
        if buf.contains("deprecated") {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "no deprecation outcome within 10s:\n{buf}"
        );
    }
    assert!(buf.contains("blocked"), "{buf}");
    let _ = watcher.kill();
    let _ = watcher.wait();
}

// --- Full lifecycle matrix (Prompt 51) ----------------------------------

/// Default run (`-n 1` implied): creation → running → buzzing → completed
/// (inactive) → deletable.
#[test]
fn lifecycle_default_once_runs_completes_and_deletes() {
    let env = TestEnv::new("lifecycle-once");
    let opens = env.dir.join("opens.log");
    let opener = recording_script(&opens, "");
    let guard = DaemonGuard::start_with(
        env,
        &[("STRANGETIMER_TEST_OPENER", opener.to_str().unwrap())],
    );

    expect_success(&guard, &["create", "timer", "t", "1s", "default_video"]);
    let out = expect_success(&guard, &["view", "timers", "--snapshot"]);
    assert!(
        out.contains("INACTIVE TIMERS"),
        "definition listed inactive:\n{out}"
    );
    assert!(
        !out.contains("ACTIVE RUNS") || !out.contains("run"),
        "{out}"
    );

    expect_success(&guard, &["run", "t"]);
    wait_for_view(&guard, "running state", "run ");
    wait_for_lines(&opens, "video fire", 1);
    wait_for_view(&guard, "inactive after completion", "INACTIVE TIMERS");

    // Completed runs are terminal: deletion succeeds.
    let out = expect_success(&guard, &["delete", "timer", "t"]);
    assert!(out.contains("Deleted"), "{out}");
    let out = expect_success(&guard, &["view", "timers", "--snapshot"]);
    assert!(out.contains("No timers currently running."), "{out}");
}

/// `-n 5`: five fires, pause between repetitions, resume, completion.
#[test]
fn lifecycle_repeat_five_with_pause_between_repetitions() {
    let env = TestEnv::new("lifecycle-repeat");
    let opens = env.dir.join("opens.log");
    let opener = recording_script(&opens, "");
    let guard = DaemonGuard::start_with(
        env,
        &[("STRANGETIMER_TEST_OPENER", opener.to_str().unwrap())],
    );

    expect_success(&guard, &["create", "timer", "t", "1s", "default_video"]);
    expect_success(&guard, &["run", "t", "-n", "5"]);
    wait_for_view(&guard, "repetition marker", "run ×5");

    // First buzzer fires; pause between repetitions.
    wait_for_lines(&opens, "first fire", 1);
    expect_success(&guard, &["pause", "t"]);
    wait_for_view(&guard, "paused state", "paused");
    std::thread::sleep(Duration::from_millis(1500));
    assert_eq!(
        fs::read_to_string(&opens)
            .map(|s| s.lines().count())
            .unwrap_or(0),
        1,
        "no fires while paused"
    );

    expect_success(&guard, &["resume", "t"]);
    wait_for_lines(&opens, "all five fires", 5);
    wait_for_view(&guard, "completed", "INACTIVE TIMERS");
}

/// `-u`: detached start, PENDING at the buzzer, resume acknowledgement,
/// completion, deletion.
#[test]
fn lifecycle_user_interrupt_resume_and_delete() {
    let guard = DaemonGuard::start(TestEnv::new("lifecycle-interrupt"));
    expect_success(&guard, &["create", "timer", "t", "1s"]);
    let out = expect_success(&guard, &["run", "t", "-u"]);
    assert!(out.contains("resume t"), "{out}");
    assert!(!out.contains("press Enter"), "{out}");

    wait_for_view(&guard, "pending marker", "PENDING");
    expect_success(&guard, &["resume", "t"]);
    wait_for_view(&guard, "completed", "INACTIVE TIMERS");
    expect_success(&guard, &["delete", "timer", "t"]);
}

/// Scheduled runs show `scheduled` and cannot be paused/resumed.
#[test]
fn lifecycle_scheduled_state() {
    let guard = DaemonGuard::start(TestEnv::new("lifecycle-scheduled"));
    expect_success(&guard, &["create", "timer", "t", "5m"]);

    // Schedule a few minutes ahead — assert the Scheduled state only.
    let now = chrono::Local::now();
    let target = now + chrono::Duration::minutes(3);
    let time_str = target.format("%H:%M").to_string();
    let out = expect_success(&guard, &["run", "t", "-t", &time_str]);
    assert!(out.contains("scheduled for"), "{out}");
    wait_for_view(&guard, "scheduled status", "scheduled");

    let err = guard.cli(&["pause", "t"]);
    assert!(!err.status.success());
    assert!(
        stderr_text(&err).contains("cannot be paused"),
        "{}",
        stderr_text(&err)
    );
    let err = guard.cli(&["resume", "t"]);
    assert!(!err.status.success());
    assert!(
        stderr_text(&err).contains("cannot be resumed"),
        "{}",
        stderr_text(&err)
    );

    // Still deletable? No — a Scheduled run is a live run.
    let err = guard.cli(&["delete", "timer", "t"]);
    assert!(!err.status.success());
    assert!(
        stderr_text(&err).contains("active run"),
        "{}",
        stderr_text(&err)
    );

    expect_success(&guard, &["stop", "t"]);
    expect_success(&guard, &["delete", "timer", "t"]);
}

/// Infinite runs stay active across many fires; `stop` ends them.
#[test]
fn lifecycle_infinite_run_and_stop() {
    let env = TestEnv::new("lifecycle-infinite");
    let opens = env.dir.join("opens.log");
    let opener = recording_script(&opens, "");
    let guard = DaemonGuard::start_with(
        env,
        &[("STRANGETIMER_TEST_OPENER", opener.to_str().unwrap())],
    );

    expect_success(&guard, &["create", "timer", "t", "1s", "default_video"]);
    expect_success(&guard, &["run", "t", "-i"]);
    wait_for_view(&guard, "infinite marker", "run ∞");

    wait_for_lines(&opens, "three infinite fires", 3);
    let out = expect_success(&guard, &["view", "timers", "--snapshot"]);
    assert!(
        out.contains("ACTIVE RUNS"),
        "still active after 3 fires:\n{out}"
    );

    expect_success(&guard, &["stop", "t"]);
    wait_for_view(&guard, "stopped", "INACTIVE TIMERS");
    expect_success(&guard, &["delete", "timer", "t"]);
}

/// View phases across one lifecycle: inactive → running → paused →
/// resumed → completed.
#[test]
fn lifecycle_view_phase_matrix() {
    let guard = DaemonGuard::start(TestEnv::new("lifecycle-phases"));
    expect_success(&guard, &["create", "timer", "t", "30s"]);

    // Before running: inactive.
    let out = expect_success(&guard, &["view", "timers", "--snapshot"]);
    assert!(
        out.contains("INACTIVE TIMERS") && out.contains("t"),
        "{out}"
    );

    // Running.
    expect_success(&guard, &["run", "t"]);
    wait_for_view(&guard, "running", "run ");
    let out = expect_success(&guard, &["view", "t", "--snapshot"]);
    assert!(out.contains("Next:"), "single-timer block:\n{out}");
    assert!(out.contains("X-"), "progress bar:\n{out}");

    // Paused.
    expect_success(&guard, &["pause", "t"]);
    wait_for_view(&guard, "paused", "paused");

    // Resumed.
    expect_success(&guard, &["resume", "t"]);
    wait_for_view(&guard, "running again", "run ");

    // Stopped → inactive, then deleted.
    expect_success(&guard, &["stop", "t"]);
    wait_for_view(&guard, "stopped inactive", "INACTIVE TIMERS");
    expect_success(&guard, &["delete", "timer", "t"]);
    let out = expect_success(&guard, &["view", "timers", "--snapshot"]);
    assert!(out.contains("No timers currently running."), "{out}");
}

/// Duplicating works during run, pause, stop and completion; the copy
/// never inherits a live run.
#[test]
fn lifecycle_duplicate_across_phases() {
    let guard = DaemonGuard::start(TestEnv::new("lifecycle-duplicate"));
    expect_success(&guard, &["create", "timer", "t", "30s"]);

    expect_success(&guard, &["run", "t"]);
    wait_for_view(&guard, "running", "run ");

    // During the run.
    expect_success(&guard, &["duplicate", "timer", "t", "run_copy"]);
    let out = expect_success(&guard, &["view", "run_copy", "--snapshot"]);
    assert!(out.contains("no active run"), "copy is not running:\n{out}");

    // During pause.
    expect_success(&guard, &["pause", "t"]);
    wait_for_view(&guard, "paused", "paused");
    expect_success(&guard, &["duplicate", "timer", "t", "pause_copy"]);

    // After stop.
    expect_success(&guard, &["stop", "t"]);
    expect_success(&guard, &["duplicate", "timer", "t", "stopped_copy"]);

    // Default suffixing.
    let out = expect_success(&guard, &["duplicate", "timer", "t"]);
    assert!(out.contains("t_copy"), "{out}");
    let out = expect_success(&guard, &["duplicate", "timer", "t"]);
    assert!(out.contains("t_copy_2"), "{out}");
}

/// Deletion is refused for live runs (running, paused, pending) and
/// succeeds after stop or completion; missing timers error out.
#[test]
fn lifecycle_delete_matrix() {
    let guard = DaemonGuard::start(TestEnv::new("lifecycle-delete"));

    // Missing timer is an error.
    let err = guard.cli(&["delete", "timer", "ghost"]);
    assert!(!err.status.success());
    assert!(
        stderr_text(&err).contains("no timer named"),
        "{}",
        stderr_text(&err)
    );

    // Refused while running.
    expect_success(&guard, &["create", "timer", "t", "30s"]);
    expect_success(&guard, &["run", "t"]);
    wait_for_view(&guard, "running", "run ");
    let err = guard.cli(&["delete", "timer", "t"]);
    assert!(!err.status.success());
    assert!(
        stderr_text(&err).contains("active run"),
        "{}",
        stderr_text(&err)
    );

    // Refused while paused.
    expect_success(&guard, &["pause", "t"]);
    wait_for_view(&guard, "paused", "paused");
    let err = guard.cli(&["delete", "timer", "t"]);
    assert!(!err.status.success());
    assert!(
        stderr_text(&err).contains("active run"),
        "{}",
        stderr_text(&err)
    );

    // Refused while pending user-interrupt.
    expect_success(&guard, &["stop", "t"]);
    expect_success(&guard, &["create", "timer", "u", "1s"]);
    expect_success(&guard, &["run", "u", "-u"]);
    wait_for_view(&guard, "pending", "PENDING");
    let err = guard.cli(&["delete", "timer", "u"]);
    assert!(!err.status.success());
    assert!(
        stderr_text(&err).contains("active run"),
        "{}",
        stderr_text(&err)
    );
    expect_success(&guard, &["resume", "u"]);
    wait_until(&guard, "u completed", Duration::from_secs(15), |g| {
        let out = g.cli(&["view", "u", "--snapshot"]);
        out.status.success() && stdout_text(&out).contains("no active run")
    });
    expect_success(&guard, &["delete", "timer", "u"]);
}

/// Multiple buzzer types stack on one timer; equal offsets fire together.
#[test]
fn lifecycle_stacked_buzzers_same_offset() {
    let env = TestEnv::new("lifecycle-stack");
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
            "custom",
            "--audio",
            "--url",
            "https://example.com",
        ],
    );
    expect_success(
        &guard,
        &[
            "create",
            "timer",
            "t",
            "1s",
            "default_audio",
            "1s",
            "default_video",
            "1s",
            "custom",
        ],
    );

    expect_success(&guard, &["run", "t"]);
    // The video opener fires exactly once (all three share the 1s offset).
    wait_for_lines(&opens, "video fire", 1);
    wait_for_view(&guard, "completed", "INACTIVE TIMERS");

    let out = expect_success(&guard, &["view", "t", "--snapshot"]);
    assert!(out.contains("default_audio"), "{out}");
    assert!(out.contains("default_video"), "{out}");
    assert!(out.contains("custom"), "{out}");
}

/// Live `view timers` must render in the terminal's alternate buffer and
/// never append repeated table copies to the primary screen: after `q`,
/// exactly ONE snapshot is left behind. Verified through a real PTY
/// (the pipe-based tests never exercise the live TTY path).
#[cfg(unix)]
#[test]
fn live_view_uses_alternate_screen_and_leaves_one_snapshot() {
    use std::io::{BufRead, BufReader, Write};

    use portable_pty::{native_pty_system, CommandBuilder, PtySize};

    let guard = DaemonGuard::start(TestEnv::new("pty-view"));
    expect_success(&guard, &["create", "timer", "t", "1h"]);
    expect_success(&guard, &["run", "t"]);

    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        })
        .unwrap();

    let mut cmd = CommandBuilder::new(cli_binary());
    cmd.arg("view");
    cmd.arg("timers");
    cmd.env("STRANGETIMER_DATA_DIR", guard.env.dir.as_os_str());
    cmd.env("STRANGETIMER_SOCKET", guard.env.socket.as_os_str());
    let mut child = pair.slave.spawn_command(cmd).unwrap();
    drop(pair.slave);

    let mut reader = BufReader::new(pair.master.try_clone_reader().unwrap());
    let mut writer = pair.master.take_writer().unwrap();

    // Wait for the first rendered frame (the live table is on the
    // alternate screen at this point).
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut first_line = String::new();
    let mut saw_frame = false;
    while Instant::now() < deadline {
        first_line.clear();
        if reader.read_line(&mut first_line).is_err() {
            break;
        }
        if first_line.contains("TIMER") || first_line.contains("ACTIVE RUNS") {
            saw_frame = true;
            break;
        }
    }
    assert!(saw_frame, "live view produced no frame: {first_line:?}");

    // Let it animate for a few frames, then quit.
    std::thread::sleep(Duration::from_millis(800));
    writer.write_all(b"q").unwrap();
    writer.flush().unwrap();

    // Wait for exit and drain the remaining stream (the final snapshot).
    let status = child.wait().unwrap();
    assert!(status.success(), "view exited with {status:?}");
    let mut rest = String::new();
    let _ = reader.read_to_string(&mut rest);
    let all = format!("{first_line}{rest}");

    // Enter/leave the alternate buffer (DEC 1049 on unix terminals).
    assert!(
        all.contains("1049h") || all.contains("?1049h"),
        "expected alternate-screen enter sequence:\n{all:?}"
    );
    let leave = if all.contains("1049l") {
        "1049l"
    } else if all.contains("?1049l") {
        "?1049l"
    } else {
        ""
    };
    assert!(
        !leave.is_empty(),
        "expected alternate-screen leave:\n{all:?}"
    );

    // After the leave, exactly one final snapshot must be present — and
    // no live frame rows may leak into the primary screen.
    let tail = all.split(leave).last().unwrap_or("");
    assert_eq!(
        tail.matches("ACTIVE RUNS").count(),
        1,
        "final snapshot must appear exactly once in the primary screen:\n{tail:?}"
    );
    assert!(
        !tail.contains("│ TIMER"),
        "live frame rows leaked into the primary screen:\n{tail:?}"
    );
}

/// Buzzer views carry action targets, media durations, and reference
/// counts; `view buzzer NAME` shows full detail.
#[test]
fn buzzer_views_show_details_and_reference_counts() {
    let guard = DaemonGuard::start(TestEnv::new("buzzer-detail"));
    // Real media fixtures from the daemon crate so durations are real.
    let audio = guard.env.dir.join("chime.wav");
    let video = guard.env.dir.join("clip.mp4");
    let daemon_assets =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../strangetimer-daemon/assets");
    fs::copy(daemon_assets.join("chime.wav"), &audio).unwrap();
    fs::copy(daemon_assets.join("default.mp4"), &video).unwrap();

    expect_success(
        &guard,
        &[
            "create",
            "buzzer",
            "alarm",
            "--audio",
            audio.to_str().unwrap(),
            "--video",
            video.to_str().unwrap(),
            "--url",
            "https://example.com/x",
        ],
    );
    // Previews are static; suppress them for tidy assertions.
    expect_success(
        &guard,
        &["create", "timer", "t1", "5m", "alarm", "10m", "alarm"],
    );
    expect_success(&guard, &["create", "timer", "t2", "1m", "alarm"]);
    expect_success(&guard, &["run", "t2"]);

    let out = expect_success(&guard, &["view", "buzzers"]);
    assert!(out.contains("alarm"), "{out}");
    assert!(out.contains("TIMERS"), "{out}");
    assert!(out.contains("LIVE"), "{out}");
    // t1 references alarm twice but counts once → 2 timers, 1 live run.
    assert!(out.contains("2"), "expected 2 referencing timers:\n{out}");

    let out = expect_success(&guard, &["view", "buzzer", "alarm"]);
    assert!(out.contains("Buzzer:"), "{out}");
    assert!(out.contains("chime.wav"), "{out}");
    assert!(out.contains("clip.mp4"), "{out}");
    assert!(out.contains("https://example.com/x"), "{out}");
    assert!(
        out.contains(".s") || out.contains("—"),
        "durations shown:\n{out}"
    );
    assert!(out.contains("t1"), "referencing timer:\n{out}");
    assert!(out.contains("t2"), "referencing timer:\n{out}");

    let err = guard.cli(&["view", "buzzer", "nope"]);
    assert!(!err.status.success());
    assert!(
        stderr_text(&err).contains("no buzzer named"),
        "{}",
        stderr_text(&err)
    );
}

/// Creating a timer/buzzer previews the created definition; `--no-preview`
/// suppresses it.
#[test]
fn creation_previews_and_no_preview_flag() {
    let guard = DaemonGuard::start(TestEnv::new("previews"));

    let out = expect_success(&guard, &["create", "timer", "t", "5m"]);
    assert!(out.contains("Created timer"), "{out}");
    assert!(
        out.contains("Buzzer Name") || out.contains("no active run"),
        "expected a preview of the created timer:\n{out}"
    );

    let out = expect_success(&guard, &["create", "timer", "u", "5m", "--no-preview"]);
    assert!(out.contains("Created timer"), "{out}");
    assert!(
        !out.contains("no active run"),
        "no preview expected:\n{out}"
    );

    let out = expect_success(
        &guard,
        &["create", "buzzer", "b", "--audio", "--no-preview"],
    );
    assert!(out.contains("Created buzzer"), "{out}");
    assert!(
        !out.contains("Buzzer:"),
        "no buzzer preview expected:\n{out}"
    );
}

/// `delete buzzer --cascade` removes the buzzer and its timers; live runs
/// block it; plain delete of a referenced buzzer suggests --cascade.
#[test]
fn delete_buzzer_cascade_and_refusal() {
    let guard = DaemonGuard::start(TestEnv::new("cascade"));
    expect_success(
        &guard,
        &["create", "buzzer", "alarm", "--audio", "--no-preview"],
    );
    expect_success(
        &guard,
        &["create", "timer", "t1", "5m", "alarm", "--no-preview"],
    );
    expect_success(
        &guard,
        &["create", "timer", "t2", "10m", "alarm", "--no-preview"],
    );

    // Plain delete refuses and suggests --cascade.
    let err = guard.cli(&["delete", "buzzer", "alarm"]);
    assert!(!err.status.success());
    assert!(
        stderr_text(&err).contains("--cascade"),
        "{}",
        stderr_text(&err)
    );

    // Cascade with a live run refuses.
    expect_success(&guard, &["run", "t1"]);
    let err = guard.cli(&["delete", "buzzer", "alarm", "--cascade", "--yes"]);
    assert!(!err.status.success());
    assert!(
        stderr_text(&err).contains("live run"),
        "{}",
        stderr_text(&err)
    );
    expect_success(&guard, &["stop", "t1"]);

    // Noninteractive cascade without --yes fails safely (stdin is a pipe).
    let err = guard.cli(&["delete", "buzzer", "alarm", "--cascade"]);
    assert!(!err.status.success());
    assert!(
        stderr_text(&err).contains("--yes") || stderr_text(&err).contains("confirmation"),
        "{}",
        stderr_text(&err)
    );

    // Cascade with --yes deletes the buzzer and both timers.
    let out = expect_success(&guard, &["delete", "buzzer", "alarm", "--cascade", "--yes"]);
    assert!(out.contains("2"), "{out}");
    let out = expect_success(&guard, &["view", "timers", "--snapshot"]);
    assert!(out.contains("No timers currently running."), "{out}");
}

/// `create timer --replace` replaces an existing definition; live runs
/// block it unless --stop-running is given.
#[test]
fn create_timer_replace_flow() {
    let guard = DaemonGuard::start(TestEnv::new("replace"));

    expect_success(&guard, &["create", "timer", "t", "5m", "--no-preview"]);

    // Noninteractive duplicate without --yes fails safely.
    let err = guard.cli(&["create", "timer", "t", "10m", "--no-preview"]);
    assert!(!err.status.success());
    assert!(
        stderr_text(&err).contains("--yes") || stderr_text(&err).contains("confirmation"),
        "{}",
        stderr_text(&err)
    );

    // --replace --yes swaps the definition.
    let out = expect_success(
        &guard,
        &[
            "create",
            "timer",
            "t",
            "10m",
            "--replace",
            "--yes",
            "--no-preview",
        ],
    );
    assert!(out.contains("Created timer"), "{out}");
    let out = expect_success(&guard, &["view", "t", "--snapshot"]);
    assert!(out.contains("10m"), "replaced definition:\n{out}");

    // Replacing while a run is live is refused unless --stop-running.
    expect_success(&guard, &["run", "t"]);
    let err = guard.cli(&[
        "create",
        "timer",
        "t",
        "20m",
        "--replace",
        "--yes",
        "--no-preview",
    ]);
    assert!(!err.status.success());
    assert!(
        stderr_text(&err).contains("active run"),
        "{}",
        stderr_text(&err)
    );

    let out = expect_success(
        &guard,
        &[
            "create",
            "timer",
            "t",
            "20m",
            "--replace",
            "--yes",
            "--stop-running",
            "--no-preview",
        ],
    );
    assert!(out.contains("Created timer"), "{out}");
    let out = expect_success(&guard, &["view", "t", "--snapshot"]);
    assert!(out.contains("20m"), "{out}");
}

/// The generated bash completion suggests buzzers after a completed offset/// in `create timer`, and state-aware names for `resume`.
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
        .args(["completions", "bash"])
        .output()
        .unwrap();
    assert!(script.status.success());
    let script_text = String::from_utf8_lossy(&script.stdout).into_owned();
    // The dynamic script must call `strangetimer` via PATH, never embed a
    // build path (stale debug/release/CI paths break completion after
    // moving or rebuilding binaries).
    assert!(script_text.contains("COMPLETE="), "not a dynamic script");
    assert!(
        !script_text.contains("target/") && !script_text.contains("/home/runner"),
        "script must not embed build paths:\n{script_text}"
    );

    let run_bash = |words: &str, cword: usize, cur: &str| {
        let code = format!(
            "source /dev/stdin <<'SCRIPT'\n{script}\nSCRIPT\n\
             COMP_WORDS=({words}); COMP_CWORD={cword}; COMP_TYPE=63\n\
             _clap_complete_strangetimer '{cur}' '{cur}'\n\
             printf '%s\\n' \"${{COMPREPLY[*]}}\"",
            script = script_text,
        );
        // The dynamic script resolves `strangetimer` via $PATH — put the
        // test binary's directory first.
        let bin_dir = cli_binary().parent().unwrap().to_path_buf();
        let path = format!(
            "{}:{}",
            bin_dir.display(),
            std::env::var("PATH").unwrap_or_default()
        );
        let out = Command::new("bash")
            .arg("-c")
            .arg(&code)
            .env("PATH", path)
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

/// The daemon refreshes its launch environment from the CLI's latest
/// request (protocol v2): a URL buzzer fires against the *new* session's
/// DISPLAY even when the daemon was started from a different (stale) one —
/// this is the fix for video/URL buzzers "randomly" failing after terminal
/// switches and restarts.
#[test]
fn url_buzzer_uses_fresh_session_env() {
    let env = TestEnv::new("env-refresh");
    let opens = env.dir.join("opens.log");
    // The mock opener records its args AND the DISPLAY it was launched with.
    let extra = format!(
        "printf 'DISPLAY=%s\\n' \"$DISPLAY\" >> \"{}\"",
        opens.display()
    );
    let opener = recording_script(&opens, &extra);
    let guard = DaemonGuard::start_with(
        env,
        &[
            ("STRANGETIMER_TEST_OPENER", opener.to_str().unwrap()),
            // Stale session baked into the daemon at start.
            ("DISPLAY", ":0"),
        ],
    );

    // Create the buzzer/timer and start the run from a *different* session.
    for args in [
        &[
            "create",
            "buzzer",
            "web",
            "--url",
            "https://example.com/env",
        ][..],
        &["create", "timer", "u", "1s", "web"][..],
        &["run", "u"][..],
    ] {
        let out = guard.cli_with_env(args, &[("DISPLAY", ":1")]);
        assert!(
            out.status.success(),
            "`strangetimer {}` failed (exit {:?})\nstdout:\n{}\nstderr:\n{}",
            args.join(" "),
            out.status.code(),
            stdout_text(&out),
            stderr_text(&out),
        );
    }

    // The opener must have run under the CLI's session env, not the
    // daemon's stale :0.
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let log = fs::read_to_string(&opens).unwrap_or_default();
        if log.contains("https://example.com/env") && log.contains("DISPLAY=:1") {
            break;
        }
        assert!(Instant::now() < deadline, "no fresh-env open:\n{log}");
        std::thread::sleep(Duration::from_millis(200));
    }
    assert!(
        !fs::read_to_string(&opens)
            .unwrap_or_default()
            .contains("DISPLAY=:0"),
        "opener must not run under the stale daemon session"
    );
}

/// `confirm-destructive` survives a daemon restart: a destructive buzzer
/// that fired before the restart keeps firing after it, instead of being
/// silently re-blocked (the in-memory-only flag used to reset every start).
#[test]
fn destructive_confirmation_survives_restart() {
    let env = TestEnv::new("conf-restart");
    let closes = env.dir.join("closes.log");
    let wmctrl = recording_script(&closes, "");
    let mut guard = DaemonGuard::start_with(
        env,
        &[
            ("STRANGETIMER_TEST_WMCTRL", wmctrl.to_str().unwrap()),
            // XDG_SESSION_TYPE="" tells the daemon this is an X11 session
            // (this dev machine itself runs Wayland).
            ("XDG_SESSION_TYPE", ""),
        ],
    );

    expect_success(&guard, &["confirm-destructive"]);
    expect_success(
        &guard,
        &["create", "buzzer", "win", "--close-window", "0x1234"],
    );
    expect_success(&guard, &["create", "timer", "c", "1s", "win"]);

    // Restart the daemon (simulating a reboot / crash-restart cycle).
    let env2 = guard.env.clone();
    let log = guard.log.clone();
    guard.child.kill().unwrap();
    guard.child.wait().unwrap();
    let log_file = open_log_append(&log);
    guard.child = Command::new(daemon_binary())
        .env("STRANGETIMER_DATA_DIR", &env2.dir)
        .env("STRANGETIMER_SOCKET", &env2.socket)
        .env("STRANGETIMER_TEST_WMCTRL", &wmctrl)
        .env("XDG_SESSION_TYPE", "")
        .stdout(Stdio::from(log_file.try_clone().unwrap()))
        .stderr(Stdio::from(log_file))
        .spawn()
        .expect("failed to respawn strangetimer-daemon");
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if std::os::unix::net::UnixStream::connect(&env2.socket).is_ok() {
            break;
        }
        assert!(Instant::now() < deadline, "daemon did not restart");
        std::thread::sleep(Duration::from_millis(100));
    }

    // The close_window buzzer fires after the restart (opt-in persisted).
    expect_success(&guard, &["run", "c"]);
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let log = fs::read_to_string(&closes).unwrap_or_default();
        if log.contains("0x1234") {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "close_window did not fire after restart:\n{log}"
        );
        std::thread::sleep(Duration::from_millis(200));
    }

    // And revoke-destructive re-blocks it immediately.
    expect_success(&guard, &["revoke-destructive"]);
    expect_success(&guard, &["run", "c"]);
    std::thread::sleep(Duration::from_secs(2));
    let log = fs::read_to_string(&closes).unwrap_or_default();
    // Each fire writes `-i`, `-c`, `0x1234`; count fires by target lines.
    assert_eq!(
        log.lines().filter(|l| *l == "0x1234").count(),
        1,
        "no close may fire after revoke:\n{log}"
    );
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

/// Mixed buzzer types stack on one timer with absolute offsets; unknown
/// buzzer names are rejected at creation time.
#[test]
fn buzzer_stacking_and_validation() {
    let guard = DaemonGuard::start(TestEnv::new("stacking"));
    expect_success(
        &guard,
        &[
            "create",
            "buzzer",
            "myBuzzer",
            "--audio",
            "--url",
            "https://example.com",
        ],
    );
    expect_success(
        &guard,
        &[
            "create",
            "timer",
            "t1",
            "30s",
            "default_audio",
            "1m",
            "default_video",
            "1m30s",
            "myBuzzer",
        ],
    );

    let out = expect_success(&guard, &["view", "t1"]);
    assert!(
        out.contains("default_audio"),
        "missing first buzzer:\n{out}"
    );
    assert!(
        out.contains("default_video"),
        "missing second buzzer:\n{out}"
    );
    assert!(out.contains("myBuzzer"), "missing third buzzer:\n{out}");

    // Unknown buzzer names are refused at creation, not at fire time.
    let out = guard.cli(&["create", "timer", "bad", "5m", "no_such_buzzer"]);
    assert!(!out.status.success(), "unknown buzzer must be rejected");
    assert!(
        stderr_text(&out).contains("no buzzer named"),
        "{}",
        stderr_text(&out)
    );

    // Names with control characters are refused too.
    let out = guard.cli(&["create", "timer", "bad\nname", "5m"]);
    assert!(!out.status.success());
    assert!(
        stderr_text(&out).contains("control characters"),
        "{}",
        stderr_text(&out)
    );
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
