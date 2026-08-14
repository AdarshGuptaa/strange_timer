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
```
