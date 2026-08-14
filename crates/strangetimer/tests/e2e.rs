//! End-to-end integration tests: spawn the real daemon binary and drive it
//! with the real CLI binary over real IPC, in isolated temp directories.
//!
//! Run with `cargo test --workspace` (builds every binary first).

use std::fs;
use std::io::{BufRead, Read, Write};
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
        env.pre_seed_registered();
        let log = env.dir.join("daemon.log");
        let child = Command::new(daemon_binary())
            .env("STRANGETIMER_DATA_DIR", &env.dir)
            .env("STRANGETIMER_SOCKET", &env.socket)
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
    assert!(out.contains("No timers currently running."));
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

/// `run -u` (user-interrupt): the run pauses at the buzzer, the attached
/// CLI prompts, and Enter resumes it (which also stops any looping audio).
#[test]
fn user_interrupt_pauses_and_resumes_on_enter() {
    let guard = DaemonGuard::start(TestEnv::new("user-interrupt"));
    expect_success(&guard, &["create", "timer", "t", "2s"]);

    // Spawn `run t -u` with a piped stdin so the test can "press Enter".
    let mut child = Command::new(cli_binary())
        .env("STRANGETIMER_DATA_DIR", &guard.env.dir)
        .env("STRANGETIMER_SOCKET", &guard.env.socket)
        .args(["run", "t", "-u"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn CLI");

    // Wait for the interrupt prompt (the 2s buzzer fires quickly).
    let stderr = child.stderr.take().unwrap();
    let mut reader = std::io::BufReader::new(stderr);
    let mut line = String::new();
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        line.clear();
        let read = reader.read_line(&mut line).expect("read CLI stderr");
        assert!(read > 0, "CLI exited before prompting");
        if line.contains("press Enter to resume") {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "no interrupt prompt within 15s; last line: {line}"
        );
    }
    assert!(line.contains("press Enter"), "{line}");

    // The run is paused while awaiting acknowledgement (STATUS column).
    let out = expect_success(&guard, &["view", "timers"]);
    assert!(out.contains("paused"), "run should be paused:\n{out}");

    // "Press Enter" → resume; the run then completes and the CLI exits.
    child.stdin.as_mut().unwrap().write_all(b"\n").unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    let mut status = None;
    while Instant::now() < deadline {
        if let Some(s) = child.try_wait().unwrap() {
            status = Some(s);
            break;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    let status = match status {
        Some(s) => s,
        None => {
            let _ = child.kill();
            let _ = child.wait();
            panic!("CLI did not exit after Enter within 15s");
        }
    };
    assert!(status.success(), "CLI exited with {status:?}");
}
