# StrangeTimer — Claude Code Prompt Sequence

Copy each prompt verbatim into Claude Code. Do not move to the next prompt until the
current one compiles/passes. Run `/compact` at the checkpoints marked below.

---

## Prompt 1 — Workspace Structure

```
Set up a Cargo workspace in the current directory with three crates:
- `strangetimer/`         — binary (the CLI)
- `strangetimer-daemon/`  — binary (the background service)
- `strangetimer-core/`    — library (shared types, used by both binaries)

Root `Cargo.toml` should declare the workspace and define all shared dependencies:

clap = { version = "4", features = ["derive"] }
chrono = { version = "0.4", features = ["serde"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
tokio = { version = "1", features = ["full"] }
crossterm = "0.27"
rodio = "0.17"
open = "5"
dirs = "5"
interprocess = "2"
reqwest = { version = "0.11", features = ["json"] }

Each crate's Cargo.toml should inherit dependencies from the workspace.
strangetimer/ and strangetimer-daemon/ depend on strangetimer-core.
Do not write any Rust logic yet — just the workspace skeleton, Cargo.toml files,
and empty main.rs / lib.rs stubs. Confirm with `cargo check`.
```

---

## Prompt 2 — Shared Data Model

```
In `strangetimer-core/src/model.rs`, define all structs and enums from the
Data Model section of CLAUDE.md. Requirements:
- Derive Serialize, Deserialize, Debug, Clone on every type
- Add a `builtin: bool` field to the Buzzer struct (used later to protect
  built-in buzzers from deletion)
- Add a `registered: bool` field to DaemonState (used later for autostart)
- Add `elapsed_before_pause: chrono::Duration` to TimerRun
- No functions, no logic — types only

Re-export everything from `strangetimer-core/src/lib.rs` via `pub mod model`.
Confirm with `cargo check`.
```

---

## Prompt 3 — Time Offset Parser

```
In `strangetimer-core/src/duration_parse.rs`, write:

  pub fn parse_offset(s: &str) -> Result<chrono::Duration, String>

It must parse all formats from the Time Offset Format section of CLAUDE.md:
30s, 5m, 5min, 2h, 1D, 1W

Also accept compound offsets like "1h30m" (parse left-to-right, sum the parts).
Return a descriptive error string for unrecognised formats.

Add unit tests for: "30s", "5m", "5min", "2h", "1D", "1W", "1h30m",
and an invalid string like "banana".

Expose via `pub mod duration_parse` in lib.rs.
Confirm with `cargo test -p strangetimer-core`.
```

---

## Prompt 4 — Persistence Layer

```
In `strangetimer-core/src/persistence.rs`, implement:

  pub fn data_dir() -> PathBuf
    — uses dirs::data_dir(), appends "strangetimer/", creates dir if absent.
    Paths per OS as in the Persistence Layout section of CLAUDE.md.

  pub fn load_timers()  -> anyhow::Result<Vec<Timer>>
  pub fn save_timers(v: &[Timer]) -> anyhow::Result<()>

  pub fn load_buzzers() -> anyhow::Result<Vec<Buzzer>>
  pub fn save_buzzers(v: &[Buzzer]) -> anyhow::Result<()>

  pub fn load_state()   -> anyhow::Result<DaemonState>
  pub fn save_state(s: &DaemonState) -> anyhow::Result<()>

Rules:
- All saves: write to <filename>.tmp then std::fs::rename (atomic write)
- All loads: return a default empty value if the file does not exist
- Add anyhow = "1" to workspace dependencies

Expose via `pub mod persistence` in lib.rs.
Confirm with `cargo check`.
```

---

## Prompt 5 — IPC Message Protocol

```
In `strangetimer-core/src/ipc.rs`, define:

ClientMessage enum with variants (each carries the data needed for that command):
  CreateTimer { timer: Timer }
  DuplicateTimer { source: String, new_name: Option<String> }
  DeleteTimer { name: String }
  CreateBuzzer { buzzer: Buzzer }
  DeleteBuzzer { name: String }
  RunTimer { name: String, repeat: RepeatMode, schedule_time: Option<chrono::DateTime<chrono::Local>> }
  Pause { name: String }
  PauseAll
  Resume { name: String }
  Stop { name: String }
  StopAll
  GetTimers
  GetTimer { name: String }
  GetBuzzers

ServerMessage enum:
  Ok
  Error(String)
  TimerList(Vec<Timer>)
  TimerDetail { timer: Timer, runs: Vec<TimerRun> }
  BuzzerList(Vec<Buzzer>)

SOCKET_NAME constant:
  "/tmp/strangetimer.sock"  on unix  (#[cfg(unix)])
  "strangetimer"             on windows (#[cfg(windows)])  — interprocess named pipe name

Two helper functions using length-prefixed JSON framing (4-byte big-endian length):
  pub fn write_message<T: Serialize>(stream: &mut impl Write, msg: &T) -> anyhow::Result<()>
  pub fn read_message<T: DeserializeOwned>(stream: &mut impl Read) -> anyhow::Result<T>

Expose via `pub mod ipc` in lib.rs.
Confirm with `cargo check`.
```

---

## Prompt 6 — CLI Argument Parser

```
In `strangetimer/src/cli.rs`, define the full clap command tree matching every
command in the CLI API section of CLAUDE.md. Use #[derive(Parser)] throughout.

Commands to define:
  create timer <name> [offset [buzzer_name] ...]   (variadic args)
  duplicate timer <source> [new_name]
  delete timer <name>
  create buzzer <name> [--audio [path]] [--video [path]]
                        [--application path] [--url url]
                        [--bash path] [--llm model prompt_or_file]
  delete buzzer <name>
  view buzzers
  run <name> [-n count | -i] [-t HH:MM]            (-n and -i in a clap group)
  pause <name>
  pauseall
  resume <name>
  stop <name>
  stopall
  view timers
  view <name>                                       (single timer)

In `strangetimer/src/main.rs`, parse the CLI args and print
"not yet implemented: <command>" for every branch.

Confirm `cargo check` and that `strangetimer --help` shows all subcommands.
```

---

## Prompt 7 — Daemon Skeleton

```
In `strangetimer-daemon/src/main.rs`, write a tokio async main that:
1. Loads timers, buzzers, and state from persistence (or initialises fresh)
2. Binds an interprocess listener on SOCKET_NAME from ipc.rs
3. Accepts connections in a loop; each connection is handled in a spawned task
4. Each handler reads one ClientMessage and responds with ServerMessage::Ok
   for every variant (stub — no real logic yet)
5. Logs each received ClientMessage variant name to stderr

Also handle SIGINT/SIGTERM (unix) or Ctrl+C (windows) for clean shutdown:
save state to disk before exiting.

Confirm the daemon starts, you can connect with `nc -U /tmp/strangetimer.sock`
(on unix), and it shuts down cleanly with Ctrl+C.
```

---

> **`/compact` checkpoint** — run `/compact` before Prompt 8.
> The workspace, types, IPC protocol, CLI shell, and daemon skeleton are stable.

---

## Prompt 8 — Daemon In-Memory State

```
In `strangetimer-daemon/src/state.rs`, implement AppState as a struct
wrapping tokio::sync::Mutex. It holds:
  timers:  Vec<Timer>
  buzzers: Vec<Buzzer>
  runs:    Vec<TimerRun>

Implement these methods (all async, lock the mutex internally):
  add_timer, remove_timer, get_timer(name) -> Option<Timer>
  add_buzzer, remove_buzzer, get_buzzer(name) -> Option<Buzzer>
  add_run, remove_run(timer_name), get_run(timer_name) -> Option<TimerRun>
  pause_run(name) -> Result<()>   — sets status to Paused, records paused_at
  resume_run(name) -> Result<()>  — sets status to Running, adds paused duration
                                    to elapsed_before_pause, clears paused_at

Every mutating method must call the matching persistence::save_* function
after modifying state (save_timers, save_buzzers, or save_state).

Thread the AppState through the daemon as Arc<AppState>.
Update the connection handler from Prompt 7 to accept Arc<AppState> and pass it
into each spawned task. Confirm with `cargo check`.
```

---

## Prompt 9 — Daemon Timer Event Loop

```
In `strangetimer-daemon/src/scheduler.rs`, implement:

  pub async fn run_scheduler(
      state: Arc<AppState>,
      buzzer_tx: tokio::sync::mpsc::Sender<String>,
  )

Behaviour:
- Runs in its own spawned tokio task
- Every 500ms, iterates all TimerRun entries with status Running
- For each run, computes which BuzzerRefs have a fire_time that has passed
  (fire_time = started_at + elapsed_before_pause + offset) and haven't
  been fired yet in this repetition
- Marks each such buzzer as fired (track with a fired_indices: Vec<usize>
  field on TimerRun)
- Sends the buzzer NAME (not the action) over buzzer_tx
- When all buzzers in a repetition have fired:
    - If RepeatMode::Count and current_rep < count: increment current_rep,
      reset fired_indices, update started_at to now
    - If RepeatMode::Infinite: same as above
    - Otherwise: set status to Completed

Spawn a second task in main that receives from the buzzer channel and
calls a stub: async fn fire_buzzer(name: &str) { eprintln!("BUZZ: {name}"); }

Confirm with `cargo check`. Manually test: create a 5-second timer and
verify "BUZZ: <name>" appears after 5 seconds.
```

---

## Prompt 10 — Buzzer Dispatch: Audio

```
Create `strangetimer-daemon/src/buzzers/mod.rs` and
`strangetimer-daemon/src/buzzers/audio.rs`.

Implement:
  pub fn fire_audio(path: Option<&std::path::Path>)

- If path is None: embed a short built-in chime WAV using include_bytes!
  (place the file at strangetimer-daemon/assets/chime.wav — generate or
  download a simple public-domain beep WAV and commit it to the project).
  Play it via rodio using a background thread so the function returns quickly.
- If path is Some: play that file via rodio the same way.
- After starting playback, attempt to bring the media player to focus.
  Wrap this in a platform.rs stub for now:
    pub fn focus_media_window() { /* TODO */ }
  Call it but don't implement it yet.

In `strangetimer-daemon/src/buzzers/mod.rs`, write:
  pub async fn dispatch(buzzer: &Buzzer, action: &BuzzerAction)
  and handle BuzzerAction::DefaultAudio and BuzzerAction::Audio(_).

Replace the fire_buzzer stub in main with a real call:
  look up the Buzzer by name in AppState, iterate its actions, call dispatch.

Confirm with `cargo check` and a manual end-to-end test (chime should play).
```

---

## Prompt 11 — Buzzer Dispatch: Video, Application, URL, Bash

```
Add to `strangetimer-daemon/src/buzzers/`:

video.rs — fire_video(path: Option<&Path>)
  - None: open a built-in short MP4 (embed path, not bytes — place a small
    public-domain clip at strangetimer-daemon/assets/default.mp4)
  - Some(p): open the file
  - Both cases use the `open` crate

application.rs — fire_application(path: &Path)
  - std::process::Command::new(path).spawn()

url.rs — fire_url(url: &str)
  - open::that(url)

bash.rs — fire_bash(path: &Path)
  - On unix (#[cfg(unix)]): Command::new("sh").arg("-c").arg(path)
  - On windows (#[cfg(windows)]): Command::new("cmd").args(["/C", path])
  - Spawn (non-blocking)

Add all four BuzzerAction variants to the dispatch() match in mod.rs:
  BuzzerAction::DefaultVideo, BuzzerAction::Video(_),
  BuzzerAction::Application(_), BuzzerAction::Url(_), BuzzerAction::Bash(_)

Confirm with `cargo check`.
```

---

## Prompt 12 — Buzzer Dispatch: Close Windows

```
Add `strangetimer-daemon/src/buzzers/close_windows.rs`.

Implement: pub fn fire_close_windows(daemon_pid: u32)

Use #[cfg(target_os)] blocks:

#[cfg(target_os = "linux")]
  - Run `wmctrl -l` to list window IDs, then `wmctrl -ic <id>` to close each.
  - Skip any window whose PID matches daemon_pid (check via /proc/<pid>/stat).
  - If wmctrl is not found, fall back to xdotool: `xdotool search --onlyvisible
    --name ""` to get IDs, then `xdotool windowclose <id>` for each.

#[cfg(target_os = "macos")]
  - osascript via Command:
    tell application "System Events" to close every window of every process
    whose unix id is not <daemon_pid>

#[cfg(target_os = "windows")]
  - EnumWindows approach via taskkill:
    taskkill /F /FI "PID ne <daemon_pid>" /FI "STATUS eq RUNNING"
    (use Command to shell out)

In AppState, add: close_windows_confirmed: bool
In dispatch(), before calling fire_close_windows:
  if !state.close_windows_confirmed {
      eprintln!("WARNING: close_windows buzzer will close ALL open windows.
Run `strangetimer confirm-destructive` to enable it.");
      return;
  }

Add a `confirm-destructive` CLI command that sends a new ClientMessage::ConfirmDestructive
and sets close_windows_confirmed = true in state.

Wire into dispatch(). Confirm with `cargo check`.
```

---

> **`/compact` checkpoint** — run `/compact` before Prompt 13.

---

## Prompt 13 — Buzzer Dispatch: LLM (Ollama)

```
Add `strangetimer-daemon/src/buzzers/llm.rs`.

Implement: pub async fn fire_llm(model: &str, prompt: &LlmPromptSource)

- Resolve the prompt text:
    LlmPromptSource::Inline(s) => use s directly
    LlmPromptSource::File(p)   => tokio::fs::read_to_string(p).await
- POST to http://localhost:11434/api/generate with body:
    { "model": model, "prompt": prompt_text, "stream": false }
  using reqwest.
- If the connection is refused or times out (set a 10s timeout):
    eprintln!("Ollama unavailable — falling back to default audio");
    fire_audio(None);
    return;
- On success, print the response .response field to stdout.

Add BuzzerAction::Llm { .. } to dispatch() in mod.rs.
Confirm with `cargo check`. (Does not require Ollama to be running to compile.)
```

---

## Prompt 14 — CLI Handlers: Timer CRUD

```
Create `strangetimer/src/commands/timers.rs`.

Implement handlers for three CLI commands. Each opens a connection to the
daemon over the IPC socket, sends a ClientMessage, reads the ServerMessage,
and prints a user-friendly result.

create timer:
  - Parse the variadic args as alternating offsets and optional buzzer names.
    Use parse_offset() from strangetimer-core.
    If a token fails parse_offset and the previous slot has no buzzer name,
    treat it as a buzzer name. If no buzzer name follows an offset, default
    to "default_audio".
  - Build a Timer and send ClientMessage::CreateTimer.
  - Daemon handler: call state.add_timer(), respond Ok or Error.

duplicate timer:
  - Send ClientMessage::DuplicateTimer.
  - Daemon handler: load the source timer, clone it with the new name
    (default "<source>_copy", increment suffix if taken), call add_timer.

delete timer:
  - Send ClientMessage::DeleteTimer.
  - Daemon handler: call state.remove_timer(). Refuse if a run is active.

Wire these three from the clap match in main.rs.
Confirm end-to-end: create a timer, list with `strangetimer view timers`
(stub print ok), duplicate it, delete it.
```

---

## Prompt 15 — CLI Handlers: Buzzer CRUD

```
Create `strangetimer/src/commands/buzzers.rs`.

Implement handlers for:

create buzzer:
  - Parse all --audio, --video, --application, --url, --bash, --llm flags.
  - Each flag appends a BuzzerAction to a Vec<BuzzerAction>.
  - --audio with no argument => BuzzerAction::DefaultAudio
  - --audio <path>           => BuzzerAction::Audio(Some(path))
  - Same pattern for --video.
  - Build Buzzer { name, actions, builtin: false } and send CreateBuzzer.
  - Daemon handler: refuse if name already exists (builtin or custom).
    Call state.add_buzzer().

delete buzzer:
  - Send DeleteBuzzer.
  - Daemon handler: refuse if buzzer.builtin == true.
    Refuse if any active timer run references this buzzer.
    Call state.remove_buzzer().

view buzzers:
  - Send GetBuzzers, receive BuzzerList.
  - Print a table: Name | Type(s) | Built-in
  - Built-in buzzers shown with a [built-in] tag.

Wire from main.rs. Confirm end-to-end.
```

---

## Prompt 16 — CLI Handlers: Run, Pause, Resume, Stop

```
Create `strangetimer/src/commands/control.rs`.

Implement handlers for all runtime control commands:

run:
  - Parse -n / -i (mutually exclusive clap group).
  - Parse -t as HH:MM into today's DateTime<Local>;
    if that time has already passed today, use tomorrow.
  - Send ClientMessage::RunTimer.
  - Daemon handler: look up the timer by name (error if not found).
    Build a TimerRun with status Scheduled (if -t given) or Running (immediate).
    For Running: set started_at = now, elapsed_before_pause = Duration::zero().
    Call state.add_run(). The scheduler (Prompt 9) picks it up automatically.

pause <name>:
  - Send Pause. Daemon: state.pause_run(name).

pauseall:
  - Send PauseAll. Daemon: iterate all Running runs, pause each.

resume <name>:
  - Send Resume. Daemon: state.resume_run(name).

stop <name>:
  - Send Stop. Daemon: state.remove_run(name) (does not delete the timer).

stopall:
  - Send StopAll. Daemon: remove all runs.

Wire all from main.rs. Confirm end-to-end: run a 10s timer, pause it,
resume it, stop it.
```

---

> **`/compact` checkpoint** — run `/compact` before Prompt 17.

---

## Prompt 17 — View: Static Snapshot

```
Create `strangetimer/src/commands/view.rs`.

Implement static (non-animated) rendering of both view commands.

view timers:
  - Send GetTimers, receive TimerList.
  - For each timer that has an active TimerRun, print the block from the
    `view timers` Output Format section of CLAUDE.md:

      <name>  Start: <datetime>  End: <datetime>  Mult: <n>
      Next: <next_buzzer_name>  <HH:MM:SS remaining>
      X-████▄████▓████▓███▓███▓█████-X

  - Progress bar width = terminal width minus 4 (for "X-" and "-X").
  - Scale █ units proportionally across total timer duration.
  - Place ▓ at the correct fractional position for each buzzer.
  - Use ▄ as a static placeholder for the current-time cursor.
  - "Next buzzer" = the earliest unfired buzzer in the active run.
  - "End" = started_at + total timer duration (last offset).
  - If no runs are active, print "No timers currently running."

view <timer_name>:
  - Send GetTimer, receive TimerDetail.
  - Print the same single-timer block.
  - Then print the buzzer countdown table from the
    `view <timer_name>` Output Format section of CLAUDE.md.

Wire from main.rs. Confirm output matches the format in CLAUDE.md exactly.
```

---

## Prompt 18 — View: Animated Progress Bar

```
Upgrade the view commands from Prompt 17 to live-animated using crossterm.

Changes to view timers and view <timer_name>:

1. Capture the TimerList/TimerDetail snapshot once at the start.
2. Enter crossterm raw mode, hide the cursor, save cursor position.
3. Every 300ms:
   a. Clear from saved position to end of screen.
   b. Re-render all timer blocks using the snapshot, but compute current
      elapsed time locally: elapsed = original_elapsed + (now - snapshot_time).
   c. Cycle the current-position cursor through ['▂','▄','▆'] using
      frame_count % 3 as the index.
   d. Check for any keypress using crossterm::event::poll(Duration::ZERO).
      On any key: break the loop.
4. On exit: show cursor, leave raw mode, print a newline.

Do NOT request a new snapshot from the daemon on each frame — all motion
is computed locally from the one initial snapshot.

Confirm: animation runs, cursor cycles, Ctrl+C and any key exit cleanly,
terminal is left in a clean state.
```

---

## Prompt 19 — First-Run Buzzer Seeding

```
In `strangetimer-daemon/src/main.rs`, after loading buzzers from persistence:

If the loaded buzzer list is empty, seed it with the three built-in buzzers
from the Built-in Buzzer Library section of CLAUDE.md:

  Buzzer { name: "default_audio",  actions: [DefaultAudio],       builtin: true }
  Buzzer { name: "default_video",  actions: [DefaultVideo],        builtin: true }
  Buzzer { name: "close_windows",  actions: [CloseAllWindows],     builtin: true }

Save with persistence::save_buzzers() immediately after seeding.

Add a `strangetimer view buzzers` end-to-end test: start the daemon fresh
(delete buzzers.json first), run the CLI, confirm all three built-in
buzzers appear in the table with [built-in] tags.
```

---

## Prompt 20 — Daemon Restart Recovery

```
In `strangetimer-daemon/src/main.rs`, after loading state, add recovery logic
for runs that were active when the daemon last stopped:

For each TimerRun with status Running:
  1. Compute total_elapsed = elapsed_before_pause + (now - started_at).
     Note: started_at here is the original start, not last-save time.
     Adjust: recalculate what now - started_at would have been.
     Actually: store last_saved_at: DateTime<Local> in DaemonState.
     Downtime = now - last_saved_at.
     Add downtime to the run's internal position.
  2. Identify all BuzzerRefs whose offset <= total_elapsed and are not in
     fired_indices.
  3. For each such buzzer, call dispatch() immediately (missed alarms fire
     on restart).
  4. Update fired_indices and resume the run in the scheduler normally.

For each TimerRun with status Scheduled where schedule_time <= now:
  Transition to Running immediately and start the scheduler entry.

Update last_saved_at in DaemonState on every save_state call (set to now).

Confirm by: starting a 30s timer, killing the daemon after 10s, waiting 25s,
restarting — the buzzer should fire immediately on restart.
```

---

## Prompt 21 — Startup Service Registration

```
Create `strangetimer-daemon/src/platform.rs`.

Implement: pub fn register_autostart() -> anyhow::Result<()>

#[cfg(target_os = "linux")]
  - Resolve the daemon binary path: std::env::current_exe()
  - Write a systemd user unit file to:
      ~/.config/systemd/user/strangetimer.service
    Content:
      [Unit]
      Description=StrangeTimer Daemon
      After=network.target

      [Service]
      ExecStart=<daemon_binary_path>
      Restart=on-failure

      [Install]
      WantedBy=default.target
  - Run: systemctl --user daemon-reload
  - Run: systemctl --user enable --now strangetimer

#[cfg(target_os = "macos")]
  - Write a launchd plist to:
      ~/Library/LaunchAgents/com.strangetimer.daemon.plist
    With keys: Label, ProgramArguments, RunAtLoad=true, KeepAlive=true
  - Run: launchctl load ~/Library/LaunchAgents/com.strangetimer.daemon.plist

#[cfg(target_os = "windows")]
  - Run via Command:
      schtasks /Create /TN "StrangeTimerDaemon" /SC ONLOGON
               /TR "<daemon_binary_path>" /RL HIGHEST /F

In main.rs:
  - After loading state, if !state.registered:
      match register_autostart() {
          Ok(_)  => { state.registered = true; save_state(&state)?;
                      eprintln!("StrangeTimer registered for autostart."); }
          Err(e) => { eprintln!("Autostart registration failed: {e}"); }
      }

Confirm on your development OS: kill and restart your machine (or simulate
with a manual service start) and verify the daemon is running.
```

---

## Prompt 22 — Daemon Lifecycle Management (start / stop / status / restart)

```
Problem: running a second strangetimer-daemon while one is alive fails with
"failed to bind IPC listener: Address already in use". The only way to stop
the daemon today is SIGTERM/SIGINT. Users must manage background processes
manually (`... &`, pkill).

Design decision: the daemon binary keeps refusing to bind when another
instance owns the socket (predictable, no surprise takeovers). Daemon
lifecycle is managed explicitly through the CLI.

1. IPC protocol changes (strangetimer-core/src/ipc.rs):
   - Add ClientMessage::Ping
   - Add ServerMessage::Status { pid: u32, version: String }
   - Add ClientMessage::Shutdown
2. Daemon side (strangetimer-daemon/src/main.rs):
   - handle_message answers Ping with Status { pid: std::process::id(),
     version: env!("CARGO_PKG_VERSION") }
   - handle_message handles Shutdown by:
       a. saving state (same path as the Ctrl+C handler),
       b. dropping/closing the IPC listener,
       c. exiting with code 0.
     Implement as: set an AtomicBool "shutdown requested" checked by the main
     select loop, so the exit goes through the same graceful teardown as
     shutdown_signal().
3. CLI side (strangetimer crate):
   - New subcommand group: `strangetimer daemon <start|stop|status|restart>`
   - daemon status:
       - probe socket (UnixStream::connect equivalent via interprocess)
       - if alive: send Ping, print "running (pid N, version X)"
       - if not: print "not running"
   - daemon start:
       - probe first; if already running, say so and exit 0
       - locate the daemon binary: same directory as the current CLI exe
         (std::env::current_exe() sibling "strangetimer-daemon"), falling
         back to $PATH, falling back to $STRANGETIMER_DAEMON env override
       - spawn detached (stdout/stderr to a log file under the data dir,
         e.g. daemon.log; setsid on Unix; DETACHED_PROCESS on Windows)
       - poll the socket for up to ~5 s until it accepts; report success
   - daemon stop:
       - send Shutdown, then poll up to ~5 s for the socket to stop
         accepting; report success or timeout
   - daemon restart: stop followed by start
4. Update commands/mod.rs daemon_hint() to suggest
   `strangetimer daemon start` instead of telling users to run the binary
   manually.
5. Update README quick start to:
       strangetimer daemon start
   (no more `./target/release/strangetimer-daemon &`).

Tests: e2e test that starts a daemon, asserts status reports its pid,
issues stop, asserts the process exits and status reports "not running".
```

---

## Prompt 23 — Buzzer Examples (`strangetimer examples`) + Docs

```
Goal: ready-to-use examples covering every buzzer action type.

1. New CLI subcommand `strangetimer examples`:
   - `strangetimer examples` — prints a table of example buzzers with
     descriptions and the exact creation command for each
   - `strangetimer examples --install` — creates the example buzzers
     through the existing CreateBuzzer IPC path (skips any that already
     exist, non-builtin so users can delete them)
2. The example set (one per action type, plus chaining):
   - exampleAudio       — `create buzzer exampleAudio --audio`
       (built-in chime)
   - exampleAudioFile   — `create buzzer exampleAudioFile --audio ~/Music/alert.wav`
   - exampleVideo       — `create buzzer exampleVideo --video`
       (built-in default video)
   - exampleUrl         — `create buzzer exampleUrl
       --url https://github.com/AdarshGuptaa/strange_timer`
       (opens the project's GitHub page in the default browser)
   - exampleApplication — `create buzzer exampleApplication
       --application /usr/bin/gnome-calculator`
   - exampleBash        — `create buzzer exampleBash --bash ~/notify.sh`
   - exampleChain       — `create buzzer exampleChain --audio
       --url https://github.com/AdarshGuptaa/strange_timer`
       (demonstrates chained actions)
   - exampleLlm         — `create buzzer exampleLlm --llm llama3
       "Announce that the timer finished and suggest a 5 minute break."`
3. docs/BUZZER_EXAMPLES.md: a page listing all of the above with
   explanations of when each action type is useful, plus a worked example
   timer (`create timer exampleBreak 25min exampleChain 5min exampleAudio`).
4. Link the new page from README.

Tests: unit test that --install produces the expected BuzzerAction lists;
e2e test that `examples --install` then `view buzzers` shows them.
```

---

## Prompt 24 — New Buzzer Actions: CloseApplication & FocusWindow

```
Goal: allow a buzzer to close a specific application or focus (switch to)
a specific window. Opening applications is already supported
(BuzzerAction::Application). Tab switching is explicitly OUT of scope.

Note: these are powerful desktop-automation actions. Like CloseAllWindows,
single targeted actions (CloseApplication) should require the existing
`confirm-destructive` opt-in before the daemon will execute them.
FocusWindow is non-destructive and needs no opt-in.

1. Data model (strangetimer-core/src/model.rs):
   - Add BuzzerAction::CloseApplication(String)  // app name or path
   - Add BuzzerAction::FocusWindow(String)       // window title substring
     or application name
2. CLI (strangetimer/src/cli.rs + commands/buzzers.rs):
   - `--close-app <NAME>` and `--focus-window <NAME>` flags on
     `create buzzer`, wired into build_actions()
3. Daemon dispatch (strangetimer-daemon/src/buzzers/):
   - close_application.rs:
     - Linux:   `pkill -x <NAME>` (exact process name match); fallback
                `pkill -f <NAME>`
     - macOS:   `osascript -e 'quit app "<NAME>"'` (graceful), then after
                a short timeout `pkill -x <NAME>`
     - Windows: `taskkill /IM <NAME>.exe` (add /F fallback)
   - focus_window.rs:
     - Linux:   `wmctrl -a <NAME>`; fallback
                `xdotool search --name <NAME> windowactivate`
     - macOS:   `osascript -e 'tell application "<NAME>" to activate'`
     - Windows: PowerShell
                `(New-Object -ComObject WScript.Shell).AppActivate('<NAME>')`
   - Both follow the existing pattern in close_windows.rs: cfg-gated helper
     per platform, anyhow errors logged to stderr, buzzer dispatch
     continues to the next action.
   - Gate CloseApplication on state.is_close_windows_confirmed() the same
     way CloseAllWindows is gated today (buzzers/mod.rs).
4. Implement wire_media_focus() / focus_media_window() note: the audio
   dispatcher already calls platform::focus_media_window() (currently a
   no-op stub); leave it stubbed or implement via focus_window helper.

Tests: unit tests for CLI flag → action mapping; manual verification on
the development OS per platform (document in docs/DEVELOPMENT.md which
platforms were exercised).
```

---

## Prompt 25 — Release Packaging & Out-of-the-Box Experience

```
Goal: a GitHub release whose binaries install smoothly and work
immediately: no manual background process management, no missing
dependencies, discoverable commands.

1. Crate metadata (all Cargo.toml files):
   - repository = "https://github.com/AdarshGuptaa/strange_timer"
   - description, license = "MIT", readme, keywords, categories
2. Shell completions + man pages:
   - Add clap_complete and clap_mangen dependencies to the CLI crate
   - `strangetimer completions <bash|zsh|fish|powershell>` subcommand
     printing the completion script to stdout
   - Release step generates man pages for both binaries
3. GitHub Actions release workflow (.github/workflows/release.yml):
   - Trigger on tags matching v*
   - Test job: cargo test on the matrix
   - Build matrix:
     - linux x86_64 (consider musl static target so glibc version does
       not matter), macOS aarch64 + x86_64, windows x86_64
   - Package per platform: archive containing strangetimer,
     strangetimer-daemon, completions/, man/
   - Create the GitHub release and upload artifacts
     (softprops/action-gh-release or `gh release upload`)
4. README overhaul (comprehensive buzzer documentation):
   - New "Buzzer actions" section with a subsection per action type
     (audio, video, URL, application, bash, close-all-windows,
     close-application, focus-window, LLM), each with a copy-paste
     example command and what it looks like when it fires
   - A "Chaining actions" subsection (ordering semantics)
   - Installation section: download release archive → put both binaries
     on PATH → `strangetimer daemon start` → first-run autostart
     registration takes over from there
   - `cargo install --git` alternative for Rust users
5. Out-of-box UX checks:
   - `strangetimer daemon start` works with binaries in the same
     directory as well as via PATH
   - CLI error messages never tell users to run a binary manually
   - First run on a clean data dir seeds built-in buzzers and registers
     autostart (already implemented — verify end to end)

Verification: cut a pre-release tag, install the archive artifacts into a
clean VM/container, run the README quick start verbatim.
```

---

## Prompt 31 — State-aware shell completions

1. Workspace: `clap_complete = { version = "4", features =
   ["unstable-dynamic"] }`.
2. `main.rs`: wire `CompleteEnv::with_factory(|| Cli::command())` at the
   top of `main()` so `COMPLETE=<shell> <bin> …` invocations answer
   completion queries and exit (verify exact engine API at impl time;
   engine scripts call back into the binary).
3. `completions` / `install-completions` switch to **engine** scripts
   (spawn self with `COMPLETE=$shell` to capture the script) — aot
   scripts cannot do dynamic candidates.
4. Candidates (`ArgValueCandidates::new(closure)`, closures must NOT
   auto-start the daemon — query via a no-auto-start IPC helper,
   silently returning empty on failure):
   - timer names: `run`, `pause`, `resume`, `stop`, `view <name>`,
     `delete timer`, `duplicate timer --source`
   - buzzer library names: `delete buzzer`; also the variadic
     `create timer … OFFSET [BUZZER]` positional — suggest buzzer names
     only when the current word does not start with a digit
   - static candidates mixed in: `view` suggests `timers` + `buzzers`
     + timer names; `completions` suggests the shell enum (automatic)
   - file paths: `--audio/--video/--bash/--application` via
     `PathCompleter`/`ValueHint::AnyPath`
   - each `CompletionCandidate` carries `.help(...)` text: timers show
     their next buzzer + total; buzzers show their action types.
5. Tests: e2e — start daemon, create timer+buzzer, then invoke
   `COMPLETE=bash <cli> -- run ""` / `-- delete buzzer ""` and assert
   the candidate lines contain the created names; with the daemon
   stopped, completion returns empty rather than erroring or spawning.

## Prompt 32 — Help overhaul with line-separated usage examples

1. Every subcommand gets `after_help` (and `after_long_help`) containing
   an `Examples:` block — one usage line per line, each a copy-pasteable
   command. Cover at minimum: create timer (3 offset/buzzer combos),
   create buzzer (one per action + one chained), run (plain, -n, -i,
   -t, -u), pause/resume/stop, view timers/buzzers/<name>, daemon
   start/stop/status/restart, examples, install-completions,
   confirm-destructive.
2. Root command gets an after_help "Getting started" workflow block.
3. Fix the confusing `[REST]...` positional in `create timer --help`:
   rename the arg's `value_name` to `OFFSET [BUZZER]...` (field can stay
   `rest`), and put the pair grammar into the examples instead of prose.
4. `view timers` distinction: help text explicitly says `timers` shows
   the live overview of RUNNING runs plus an "Inactive" section of
   defined-but-not-running timers (implemented in Prompt 34).
5. Tests: snapshot assertions — `Cli::command().render_help()` for each
   subcommand contains its `Examples:` block; the create-timer help no
   longer contains `REST`.

## Prompt 34 — `view timers` as an organized, colored table

1. Reformat both the TUI and the static snapshot as a bordered table
   (Prompt 33 colors, `DarkGrey` borders):
   `│ TIMER │ STATUS │ START → END │ NEXT BUZZER │ PROGRESS │`
   - STATUS cell: `● running` (green) / `❙❙ paused` (yellow) /
     `◔ scheduled` (cyan) — plain fallback `running`/`paused`/…
   - sections: **Active** rows first (sorted by next-buzzer time), then
     an `Inactive` divider row and one row per defined timer without a
     live run (name, definition total, buzzer count, `—` for progress).
2. Columns sized from the live terminal width (re-queried per frame;
     resize re-lays out); per-column truncation with `…`; PROGRESS keeps
     the capped bar (≤ 40 cells) or a percentage when the column is too
     narrow; height cap with `+N more`; below ~40 cols fall back to the
     minimal one-line-per-timer list (colored status).
3. `view <name>` restyled to match (block + buzzer table borders/colors).
4. Update existing view unit tests to the table layout; add tests:
   active/inactive split, column fitting at 40/80/200 cols, colored
   status only when color enabled, plain-text alignment preserved.

## Prompt 35 — `run -u / --userinterrupt`

1. Model (`strangetimer-core`): `TimerRun` gains
   `#[serde(default)] user_interrupt: bool` and
   `#[serde(default)] interrupt_focus: Option<String>` (recorded
   terminal window id/name). `ClientMessage::RunTimer` gains
   `user_interrupt: bool`. Old state.json files deserialize unchanged.
2. CLI: `RunArgs` gets `-u, --userinterrupt`. Before sending RunTimer,
   capture the active window: Linux `xdotool getactivewindow
   getwindowname` (fallback `wmctrl -k` / None), macOS AppleScript
   frontmost window name, Windows None for now. Sent alongside the run
   (extend RunTimer or a follow-up message — keep it one field set at
   start_run).
3. Daemon fire path (per buzzer fire on a `user_interrupt` run):
   a. dispatch actions; **audio** actions loop (rodio Sink per replay,
      ~0.5 s gap) until the run's interrupt is acknowledged;
   b. after non-audio actions, focus the recorded terminal window via
      the existing focus_window helper;
   c. pause the run through the existing pause machinery (schedule
      shifts forward — same semantics as manual pause);
   d. set `interrupt_pending = Some(timer_name)` (in-memory +
      persisted on the run so a restart keeps it paused).
4. Attached CLI: after `RunTimer` succeeds, `run -u` polls `GetTimer`
   every 500 ms. When it sees Paused + pending it prints
   `⏸ <name> paused — press Enter to resume` (accent color), reads a
   line from stdin, sends `Resume` (which clears pending and stops the
   audio loop). Ctrl+C detaches the CLI only — the run stays
   paused-pending; `strangetimer resume <name>` remains the fallback
   ack. The CLI exits when the run completes.
5. Recovery: a restart leaves the run Paused with pending set; no audio
   loop during downtime; `resume` still clears it. Documented behavior.
6. Tests:
   - unit: `-u` parse; RunTimer roundtrip with the new fields; the pure
     "should interrupt?" predicate (user_interrupt && buzzer fired);
     audio-loop controller start/stop semantics.
   - e2e: create timer 2s → `run -u` with piped stdin; assert the run
     flips to Paused, the daemon log shows the interrupt, the CLI
     prints the prompt; write `\n` to stdin → run Running again; stop.
   - e2e tolerance: focus capture returns None without an X server —
     the run still works, only the focus step is skipped.

## Build order

33 (styling) → 32 (help) → 31 (completions) → 34 (view table) →
35 (userinterrupt). `cargo test --workspace`, clippy, fmt after each.
Docs to update at the end: README (completions/getting started/userinterrupt),
DEVELOPMENT.md (unstable-dynamic note, testing completions), SYSTEM_DESIGN.md
(§9 CLI, new §7.x interrupt flow, protocol addition), and merge this file
into plan.md as Prompts 31–35 + Build Order Summary update.


---

---

## Prompt 36 — View persistence and scrolling

```
Problem: the live view vanished when the user scrolled; arrow keys exited.

- Keep live mode as the default (`strangetimer view timers`), but only
  q / Escape / Ctrl+C exit; arrow keys and mouse scrolling are ignored.
- The daemon snapshot is refetched every ~1s instead of animating one
  stale snapshot forever.
- On exit a final snapshot is printed to the primary screen so the
  display persists in scrollback instead of disappearing.
- New `--snapshot` flag prints a static, persistent view (no alternate
  screen) — ideal for scripts and scrollback.
- `view <timer> --snapshot` works the same way.
- Help text updated with examples for both modes.
```

## Prompt 37 — Exact-width table renderer

```
Problem: rows were ~10 columns too wide and wrapped; ANSI codes shifted
columns; the progress bar shared the details line; rules wrapped.

- New display-width-aware layout: cells are truncated and padded on raw
  text with `unicode-width`, styled only afterwards — ANSI never shifts
  columns and no physical line exceeds the terminal width.
- Column widths derived from the exact terminal width (overhead: borders
  + " │ " separators); narrow terminals shrink columns proportionally.
- Each active timer spans two rows: a details row (TIMER | STATUS |
  START → END | NEXT) and a full-width progress-bar row on its own line.
- Rules are a single full-width line; ACTIVE RUNS / INACTIVE TIMERS
  section headers with a divider; PENDING (blinking in live mode) marks
  user-interrupt runs; height-capped with "+N more".
- Tests: rows never exceed 40/80/120/200 columns; pending marker;
  inactive section; unicode truncation.
```

## Prompt 38 — Context-aware completions

```
Problem: `create timer t 1m<Tab>` only offered --help; resume didn't
suggest paused timers.

- Custom `ValueCompleter` with `complete_at` for the `create timer`
  variadic: even tokens are offset slots (suggests 30s/1m/5m/…), odd
  tokens are buzzer slots; a completed offset (`1m<Tab>`) expands to
  `1m <buzzer>` instead of being replaced.
- State-aware name sets: pause → running runs; resume → paused/pending
  runs; stop → live runs; run/duplicate/delete/view → all definitions;
  delete buzzer → non-builtin buzzers only.
- Daemon-down fallback reads the persisted state files (no auto-start).
- e2e drives the real bash script and asserts `1m myBuzzer` and paused
  timer suggestions.
```

## Prompt 39 — Non-blocking user-interrupt mode

```
Problem: `run -u` held the terminal waiting for Enter.

- `run -u` is fully detached: prints the acknowledge hint
  (`strangetimer resume <name>`) and returns immediately.
- `strangetimer resume <name>` is the only acknowledgement path; the live
  view shows a blinking PENDING marker, snapshots a plain one.
- Pending state generalized from a single `Option<String>` to
  `pending_interrupts: Vec<String>` (legacy field migrated on load) so
  several runs can pause independently.
- Audio loops run in their own spawned tasks so one pending timer never
  blocks another timer's dispatch (found and fixed via a live test).
- Tests: detached CLI returns at once; PENDING marker; resume clears;
  two concurrent `-u` runs pause and resume independently.
```

## Prompt 40 — Repeated video events and remote video

```
Problem: repeated `default_video` timers appeared to fire once.

- Scheduler channel now carries structured `FireEvent`
  (timer, buzzer, buzzer index, repetition); Count(3) fires exactly
  three events (unit test asserts reps 0,1,2).
- `elapsed_before_pause` is reset when a repetition advances, so pause
  offsets never leak into later repetitions.
- Opener seam: `STRANGETIMER_TEST_OPENER` records opens instead of
  launching a GUI; e2e asserts three opens of `default.mp4`.
- `default.mp4` resolves next to the installed binary
  (`<exe_dir>/assets/default.mp4`) with a source-tree fallback; the
  release workflow packages `assets/`; a test asserts a valid MP4 ftyp.
- New `exampleVideoUrl` example streams a stable HTTPS MP4 via --url;
  real network/GUI checks stay opt-in (`STRANGETIMER_GUI_TESTS=1`).
```

## Prompt 41 — Safe desktop-action tests

```
- External-tool seams: STRANGETIMER_TEST_PKILL / _WMCTRL / _XDOTOOL /
  _OSASCRIPT / _TASKKILL replace the real binaries with recording
  scripts, so close-app and focus-window command construction is tested
  without touching the desktop.
- e2e: application buzzer launches a temp script (no GUI needed); URL
  buzzer opens its target through the mock opener; close-app issues
  pkill for the named app; focus-window issues the platform command.
- Real-GUI tests are `#[ignore]`d and opt in via STRANGETIMER_GUI_TESTS=1
  (X11 focus); CI never runs them.
```

---

---

## Prompt 42 — Completion system (priority)

```
- Scripts invoke `strangetimer` via $PATH (argv[0] fix) so stale
  target/debug, target/release or CI paths never break completion;
  release archives and install-completions re-generate from the live
  binary. Reinstall after upgrades.
- `strangetimer completions --doctor` diagnoses missing suggestions:
  binary/version, shell, installed script locations, dynamic vs static
  script, stale embedded paths, and the fix commands.
- `create timer` variadic slots use a content heuristic (digit token =
  offset slot; name/empty = buzzer slot), so consecutive bare offsets
  like `30s 30s 30s` complete correctly; a complete offset expands to
  `1m <buzzer>` instead of being replaced.
- State-aware sets: pause (running), resume (paused), stop (live runs),
  delete buzzer (non-builtin); one consistent daemon snapshot per
  request; built-in buzzers synthesized locally before first daemon run.
- Tests: unit + real bash-script e2e (PATH-resolved), no build paths
  embedded, doctor output.
```

## Prompt 43 — Terminal-only animated view

```
- Remove the alternate screen; the live view animates on the primary
  terminal (saved cursor position + Clear(FromCursorDown) per frame), so
  output stays in normal scrollback while animating.
- Only q/Escape/Ctrl+C exit; arrow keys and mouse scrolling are ignored
  (the wheel scrolls the primary buffer natively).
- Daemon snapshots refetched every ~1s for both the overview and the
  single-timer view.
- A final snapshot is printed on exit; --snapshot remains the static,
  scrollable printout.
```

## Prompt 44 — Exact-width table fixes

```
- Row overhead corrected to exactly 13 columns; headers padded to the
  same raw widths as data cells.
- Progress rows padded by display width (unicode-width), not byte length.
- Buzzer table handles very narrow terminals (name-only rows below 12
  columns; Time Remaining column dropped correctly).
- Minimal list filters completed runs and styles after truncation.
- Timer/buzzer names with control characters are rejected at creation.
- Tests sweep widths 40-240 (plus narrow buzzer-table cases) with ANSI
  enabled via a visible-width measurement.
```

## Prompt 45 — Reliable terminal focus

```
- FocusSpec (JSON in interrupt_focus) captures the X11 window id, title,
  DISPLAY and XAUTHORITY at run -u time; Wayland sessions are flagged.
- At fire time: activate by window id (wmctrl -i -a, fallback xdotool
  windowactivate --sync, then title search) with the session env, plus
  retries after async player/browser launches.
- Wayland reports unsupported instead of pretending success; the
  autostart unit carries the common GUI env vars.
- Tests: capture/activation via STRANGETIMER_TEST_XDOTOOL/WMCTRL seams;
  Wayland skip; missing tools degrade gracefully.
```

## Prompt 46 — Buzzer stacking, validation, and notifications

```
- Offsets are absolute from run start; equal offsets fire together.
  Help/examples document `30s a 1m b 1m30s c` chaining.
- Unknown buzzer names and control-character names are rejected at
  creation.
- Daemon keeps a bounded, memory-only BuzzerEvent queue (id, timer,
  buzzer, types, fired_at, repetition, requires_ack) and answers
  GetEvents { after_id }.
- New `strangetimer watch` prints per event:
      <timer> ringing | <types> | <time>
      -> strangetimer resume <timer> to resume!   (run -u only)
  Events never come from the daemon logger, so interactive terminals
  stay clean; after_id gives at-least-once without duplicates.
- Tests: watch e2e (ringing line, resume hint), stacking parse/view e2e,
  unknown-buzzer rejection.
```

## Prompt 47 — Video and recovery robustness

```
- Recovery catches up multiple fully-elapsed repetitions: a Count(3)
  timer down for three periods fires all three alarms on restart.
- e2e: kill daemon before the first fire, stay down ~5 periods, restart
  with the recording opener, assert three opens.
- Existing mock-opener repeat tests, remote exampleVideoUrl, opt-in GUI
  tests (STRANGETIMER_GUI_TESTS=1) unchanged.
```

---

---

## Prompt 48 — Targeted window closing

```
- New BuzzerAction::CloseWindow(String) + `--close-window <id-or-title>`
  closes ONE selected window; Linux prefers wmctrl -i -c <id> (title via
  wmctrl -c), falls back to xdotool windowclose; macOS via AppleScript;
  Windows/Wayland report unsupported instead of pretending.
- The legacy CloseAllWindows action is deserialized for compatibility but
  refused at dispatch with a migration hint (closing the entire desktop
  was too destructive); close_windows.rs module removed.
- Confirmation (confirm-destructive) still required; the origin terminal
  is never targeted.
- Tests: wmctrl seam records -i -c <id>; watch reports the deprecated
  action as blocked; Wayland skip covered.
```

## Prompt 49 — Completed-run deletion

```
- remove_timer guard now ignores Completed runs: `status != Completed`.
- Missing timers now error ("no timer named") instead of succeeding.
- Deleting a timer also removes its orphaned run records and pending
  marker.
- Tests: refused for running/paused/scheduled, allowed after completion,
  missing-timer error, orphan cleanup.
```

## Prompt 50 — Durable fire outbox + event outcomes

```
- DaemonState.pending_fires persists each FireEvent before the fire task
  receives it; the fire task removes it after dispatch; startup replays
  the outbox, so a crash between scheduling and dispatch never loses an
  alarm.
- BuzzerEvent.outcome reports blocked actions (deprecated close_windows,
  missing confirm-destructive, awaiting acknowledgement); watch prints
  it.
- Tests: seeded state.json pending_fires is replayed exactly once on
  daemon start.
```

## Prompt 51 — Full lifecycle e2e matrix

```
- Polling helpers (wait_until / wait_for_view / wait_for_lines) replace
  fixed sleeps.
- Tests: default once (create → running → buzz → completed → delete);
  -n 5 with pause between repetitions; -u PENDING + resume + delete;
  scheduled state (pause/resume errors, delete refused, stop, delete);
  infinite (-i) with 3+ fires then stop; view phase matrix (inactive →
  running → paused → resumed → completed → deleted); duplicate during
  run/pause/stop + default suffixing (t_copy, t_copy_2); delete matrix
  (missing, running, paused, pending, after completion); stacked buzzers
  with equal offsets fire together.
- Fixed the duplicate-name display bug: the daemon now replies with the
  actually-generated name (DuplicateTimerOk), so t_copy_2 is reported
  correctly.
```

## Prompt 52 — Documentation and diagnostics

```
- Help/README/BUZZER_EXAMPLES document --close-window, close_windows
  deprecation, absolute offsets, equal-offset stacking, and completed
  timers being deletable.
- DEVELOPMENT/SYSTEM_DESIGN cover the close_window backends, the fire
  outbox, and event outcomes.
- Diagnostics: `strangetimer completions --doctor`, `command -v wmctrl`,
  `echo $XDG_SESSION_TYPE`.
```

---

## Build Order Summary

```
Prompt  1  — Workspace + Cargo.toml
Prompt  2  — Data model (all types)
Prompt  3  — Time offset parser
Prompt  4  — Persistence layer
Prompt  5  — IPC message protocol
Prompt  6  — CLI argument parser (stubs)
Prompt  7  — Daemon skeleton (listens, echoes Ok)
── /compact ──
Prompt  8  — Daemon in-memory AppState
Prompt  9  — Daemon scheduler event loop
Prompt 10  — Buzzer: audio
Prompt 11  — Buzzer: video, application, URL, bash
Prompt 12  — Buzzer: close windows (platform-specific)
── /compact ──
Prompt 13  — Buzzer: LLM / Ollama
Prompt 14  — CLI: create/duplicate/delete timer
Prompt 15  — CLI: create/delete/view buzzers
Prompt 16  — CLI: run, pause, resume, stop
── /compact ──
Prompt 17  — View: static snapshot
Prompt 18  — View: animated progress bar
Prompt 19  — First-run buzzer seeding
Prompt 20  — Daemon restart recovery
Prompt 21  — Startup service registration
── /compact ──
Prompt 22  — Daemon lifecycle management (start/stop/status/restart)
Prompt 23  — Buzzer examples (`strangetimer examples`) + docs
Prompt 24  — Buzzer actions: close-application, focus-window
Prompt 25  — Release packaging, completions, README overhaul
── /compact ──
Prompt 31  — State-aware shell completions (unstable-dynamic engine)
Prompt 32  — Help overhaul with line-separated examples
Prompt 33  — Muted Cosmic styling module + colored CLI/help
Prompt 34  — `view timers` colored table (active/inactive)
Prompt 35  — `run -u / --userinterrupt`
── /compact ──
Prompt 36  — View persistence & scrolling (`--snapshot`, q/Ctrl+C exit)
Prompt 37  — Exact-width table renderer (progress on its own line)
Prompt 38  — Context-aware completions (create-timer slots, state-aware)
Prompt 39  — Non-blocking user-interrupt + multi-pending support
Prompt 40  — FireEvent, repetition reset, mock opener, remote video
Prompt 41  — Safe desktop-action test seams (PKILL/WMCTRL/... overrides)
── /compact ──
Prompt 42  — Completion system (PATH-resolved scripts, --doctor)
Prompt 43  — Terminal-only animated view (no alternate screen)
Prompt 44  — Exact-width table fixes (overhead 13, padded headers)
Prompt 45  — Reliable terminal focus (window id, env, Wayland detect)
Prompt 46  — Buzzer stacking validation + `strangetimer watch`
Prompt 47  — Recovery catch-up of missed repetitions
── /compact ──
Prompt 48  — Targeted window closing (--close-window, deprecate all-windows)
Prompt 49  — Completed-run deletion + missing-timer errors
Prompt 50  — Durable fire outbox + event outcomes
Prompt 51  — Full lifecycle e2e matrix (run/view/duplicate/delete phases)
Prompt 52  — Documentation and diagnostics
```

(Implemented in one pass; prompts 48-52 shipped together.)
