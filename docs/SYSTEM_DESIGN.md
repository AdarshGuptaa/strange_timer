# StrangeTimer — System Design

A detailed walkthrough of how StrangeTimer is put together: processes,
data flow, the wire protocol, the scheduler's time model, failure modes,
and the reasoning behind the important design decisions.

---

## 1. Overview

StrangeTimer is a client/server timer application. The *server* is a
background daemon that holds all live timer state, advances it on a 500 ms
loop, and fires alarms. The *client* is a stateless CLI that talks to the
daemon over local IPC, prints results, and renders live progress views.

```
                 IPC: Unix socket / named pipe
                 ┌─────────────────────────────────────────────┐
                 │   length-prefixed JSON frames               │
                 ▼                                             │
┌──────────────┐      ┌────────────────────────────────────────▼───┐
│  strangetimer │      │  strangetimer-daemon                      │
│  (CLI)        │─────►│  ┌────────────────────┐                   │
│  commands/    │      │  │ scheduler (500ms)  │                   │
│  view (TUI)   │      │  └─────────┬──────────┘                   │
└──────────────┘      │            │ buzzer names                  │
                      │            ▼ (mpsc channel)                │
                      │  ┌────────────────────┐   ┌──────────────┐  │
                      │  │ fire task          │──►│ dispatch()   │  │
                      │  └────────────────────┘   └──────┬───────┘  │
                      │        ┌───────────────────────┐│││││││      │
                      │        │ AppState (Mutex)      ││││││││      │
                      │        │  timers, buzzers,     ││││││││      │
                      │        │  runs                 ││││││││      │
                      │        └──────────┬────────────┘│││││││      │
                      │                   │ persist     │││││││      │
                      │        ┌──────────▼────────────┐ │││││││      │
                      │        │ persistence (atomic)  │ │││││││      │
                      │        └──────────┬────────────┘ │││││││      │
                      │                   ▼              ▼▼▼▼▼▼▼      │
                      │         timers.json buzzers.json state.json  │
                      │         + platform actions (audio, open, ...)│
                      └────────────────────────────────────────────────┘
```

### Why split into daemon + CLI?

Time must keep running after the command that started it exits. A CLI-only
timer stops the moment the process dies, and a foreground process can't
serve both an interactive session and an alarm. A daemon persists live
state, survives logout/reboot (via autostart registration), and lets any
number of CLI invocations observe and control the same clock.

---

## 2. Workspace Layout

```
Cargo.toml                        workspace root; shared dependencies
crates/
  strangetimer/                   CLI binary
    src/cli.rs                    clap command tree
    src/main.rs                   dispatch to command handlers
    src/commands/
      mod.rs                      IPC connection + request/response helper
      timers.rs                   create/duplicate/delete timer
      buzzers.rs                  create/delete/view buzzers
      control.rs                  run/pause/resume/stop/confirm-destructive
      daemon.rs                   daemon start/stop/status/restart + probe
      examples.rs                 `strangetimer examples [--install]`
      completions.rs              shell completion script generation
      install_completions.rs      `install-completions` (per-user install)
      view.rs                     static + animated progress rendering
    tests/e2e.rs                  end-to-end tests (real binaries + IPC)
  strangetimer-daemon/            background service binary
    src/main.rs                   bootstrap, IPC accept loop, recovery
    src/state.rs                  AppState: in-memory model + persistence hooks
    src/scheduler.rs              500ms event loop advancing all runs
    src/log.rs                    level-gated logger (daemon.log + stderr)
    src/buzzers/                  alarm dispatch (one module per action type)
    src/platform.rs               autostart registration, window focus
    assets/
      chime.wav                   built-in default audio (embedded in binary)
      default.mp4                 built-in default video (referenced by path)
  strangetimer-core/              library shared by both binaries
    src/model.rs                  all data types
    src/duration_parse.rs         "1h30m"-style offset parsing
    src/persistence.rs            data-dir resolution + atomic JSON store
    src/ipc.rs                    message types + framing helpers
```

Dependency direction is strictly one-way: `strangetimer` and
`strangetimer-daemon` depend on `strangetimer-core`; the core never depends
on either binary. All heavy dependencies (tokio, clap, crossterm, rodio,
interprocess, reqwest) live in the workspace manifest and are inherited.

---

## 3. Data Model (`strangetimer-core/src/model.rs`)

All types derive `Serialize`, `Deserialize`, `Debug`, `Clone` (+ `PartialEq`
where tests need equality) and round-trip through JSON unchanged.

### 3.1 Timer (definition — stored, not running)

```
Timer {
  name:       String,
  buzzers:    Vec<BuzzerRef>,   // ordered alarm list
  created_at: DateTime<Local>,
}
BuzzerRef {
  offset:      Duration,        // time from run start when this alarm fires
  buzzer_name: String,          // reference into the buzzer library
}
```

The offset grammar is `30s | 5m | 5min | 2h | 1D | 1W` plus concatenated
compounds (`1h30m`), parsed left-to-right and summed by
`parse_offset()` in `duration_parse.rs`.

### 3.2 Buzzer (library entry)

```
Buzzer {
  name:    String,
  actions: Vec<BuzzerAction>,   // chained: all fire in sequence
  builtin: bool,                // guards against deletion
}
BuzzerAction = DefaultAudio | DefaultVideo | CloseAllWindows
             | Audio(Option<PathBuf>) | Video(Option<PathBuf>)
             | Application(PathBuf) | Url(String) | Bash(PathBuf)
             | CloseApplication(String) | FocusWindow(String)
             | Llm { model: String, prompt: LlmPromptSource }
LlmPromptSource = Inline(String) | File(PathBuf)
```

`Option<PathBuf>` distinguishes *use the built-in asset* (`None`) from *use
this file* (`Some`). `builtin: true` entries are seeded on first run and the
daemon refuses to delete them.

### 3.3 TimerRun (live state — daemon only)

```
TimerRun {
  timer_name:           String,
  started_at:           DateTime<Local>,
  repetitions:          RepeatMode,      // Count(u32) | Infinite
  current_rep:          u32,
  schedule_time:        Option<DateTime<Local>>,
  status:               Running | Paused | Scheduled | Completed,
  paused_at:            Option<DateTime<Local>>,
  elapsed_before_pause: Duration,
  fired_indices:        Vec<usize>,      // alarms already fired this repetition
}
```

`fired_indices` is the scheduler's per-repetition bookkeeping — see §6 for
how it drives repetition and completion.

### 3.4 DaemonState (persisted)

```
DaemonState {
  runs:          Vec<TimerRun>,
  registered:    bool,            // autostart service installed?
  last_saved_at: Option<DateTime<Local>>,  // stamped automatically on save
}
```

---

## 4. Persistence (`strangetimer-core/src/persistence.rs`)

### 4.1 Location

`data_dir()` resolves the OS-appropriate data directory and creates it:

| OS | Path |
|---|---|
| Linux | `~/.local/share/strangetimer/` |
| macOS | `~/Library/Application Support/strangetimer/` |
| Windows | `%APPDATA%\strangetimer\` |

`STRANGETIMER_DATA_DIR` overrides the location. This exists for test
isolation (each test run gets a fresh temp dir) and for advanced users who
want a custom data location.

### 4.2 Files

| File | Contents |
|---|---|
| `timers.json` | `Vec<Timer>` |
| `buzzers.json` | `Vec<Buzzer>` |
| `state.json` | `DaemonState` (live runs + flags) |

Loads return an empty default when the file is absent (and write it out, so
the on-disk shape is always current). Saves write a unique `<name>.<pid>.<seq>.tmp`
then `rename(2)` into place.

### 4.3 Atomicity

The tmp-then-rename scheme guarantees a reader never observes a partially
written file — a power loss mid-write leaves the previous version intact.
The temp name is made **unique per writer** (process id + monotonic counter).
This matters more than it looks: the daemon's scheduler task and IPC handler
tasks save state concurrently, and with a fixed temp name one writer could
rename another's temp file away mid-flight. (This race was actually
discovered by the test suite and fixed in the persistence layer.)

### 4.4 Save discipline

Every `AppState` mutation holds the state mutex **and** saves before
releasing it. The scheduler's tick does the same. So the on-disk state is
never older than the in-memory state except for the few hundred
milliseconds between the last tick and a crash.

`save_state()` additionally stamps `last_saved_at = now`. The field is
written into the state file, so after a crash the daemon knows exactly when
the state was last synced.

---

## 5. IPC Protocol (`strangetimer-core/src/ipc.rs`)

### 5.1 Endpoint

`SOCKET_NAME` is `/tmp/strangetimer.sock` on Unix (filesystem Unix-domain
socket) and `strangetimer` on Windows (named pipe, `\\.\pipe\strangetimer`).
`socket_name()` reads `STRANGETIMER_SOCKET` as an override — again, for
test isolation and multi-instance setups.

### 5.2 Framing

Every message is a length-prefixed JSON frame:

```
┌───────────────┬─────────────────────────────┐
│ length: u32 BE │ JSON payload                │
└───────────────┴─────────────────────────────┘
```

- `write_message(stream, msg)` — serialise, prepend 4-byte big-endian length.
- `read_message(stream)` — read length, read that many bytes, parse JSON.
  A 64 MiB sanity cap prevents a corrupt peer from forcing a huge
  allocation.

The CLI uses the synchronous helpers; the daemon has async equivalents
(`read_message_async` / `write_message_async` in `main.rs`) with identical
framing. A connection carries exactly one request and one response, then
closes — no session state, no multiplexing. This keeps the daemon simple:
each connection is handled in a spawned task with no cross-connection state.

### 5.3 Messages

Client → daemon:

```
CreateTimer { timer }            DuplicateTimer { source, new_name }
DeleteTimer { name }             CreateBuzzer { buzzer }
DeleteBuzzer { name }            RunTimer { name, repeat, schedule_time }
Pause { name }                   PauseAll
Resume { name }                  Stop { name }
StopAll                          GetTimers
GetTimer { name }                GetBuzzers
ConfirmDestructive               ← opt-in for the close_windows buzzer
Ping                             ← liveness probe (daemon lifecycle)
Shutdown                         ← graceful daemon stop (daemon lifecycle)
```

Daemon → client:

```
Ok | Error(String)
TimerList { timers, runs }        ← runs included so `view` can render
TimerDetail { timer, runs }
BuzzerList(Vec<Buzzer>)
Status { pid, version }           ← reply to Ping
```

**Design note:** `TimerList` carries the live runs as well as the timer
definitions. Prompt 5 originally defined it as a bare `Vec<Timer>`, but the
`view` commands need started_at / elapsed / fired_indices to render progress
blocks, and all of that lives on the daemon side. Rather than N round-trips
of `GetTimer`, the list response bundles both. This is the one deliberate
protocol deviation from the plan.

---

## 6. The Scheduler's Time Model (`strangetimer-daemon/src/scheduler.rs`)

### 6.1 The ticking loop

`run_scheduler` spawns a tokio task that wakes every 500 ms, runs one
`tick(state)`, and forwards every returned buzzer name through the
`mpsc::Sender<String>` channel. `tick` is a pure-ish function (it mutates
and persists state, returns the names to fire) — extracting it from the loop
is what makes the scheduler unit-testable without wall-clock sleeps.

### 6.2 When does an alarm fire?

```
fire_time(run, offset) = run.started_at + run.elapsed_before_pause + offset
```

An unfired buzzer at index `i` fires when `now >= fire_time`. `fired_indices`
remembers what has fired *this repetition*.

### 6.3 Why `elapsed_before_pause` is added, not subtracted

The run's timeline is *pause-shifted*: pausing stops the countdown, and
resuming moves the whole schedule forward by the pause duration. Formally:

- pause at `t1` → `paused_at = t1`
- resume at `t2` → `elapsed_before_pause += (t2 - t1)`

Then the effective elapsed time at wall-clock `now` is:

```
effective_elapsed = (now - started_at) - elapsed_before_pause
```

which equals the true accumulated running time: before the pause it is
`t1 - started_at`, after the resume it continues from exactly where it left
off. The scheduler compares `now` against `started_at + elapsed_before_pause
+ offset` — the same shift applied to the fire deadline. A run paused for
two hours behaves identically to one started two hours later.

### 6.4 Repetition & completion

When `fired_indices.len() == timer.buzzers.len()` (every alarm of this
repetition has fired):

```
Count(n) with current_rep+1 < n  → current_rep += 1, clear fired_indices,
                                    started_at = now          (next rep)
Infinite                         → same                        (next rep)
otherwise                        → status = Completed
```

Note that `started_at = now` at the boundary keeps the next repetition
aligned with wall-clock time — this is also what makes restart recovery
(§8.1) work for repeated runs.

### 6.5 Scheduled runs

A run created with `-t HH:MM` is `Scheduled` and inert. Each tick checks
`status == Scheduled && now >= schedule_time` and flips it to `Running`.
`started_at` is set to the schedule time, so offsets count from the
scheduled moment — a run scheduled for 09:00 fires its 45-minute alarm at
09:45, not 45 minutes after the daemon happens to notice.

---

## 7. Buzzer Dispatch (`strangetimer-daemon/src/buzzers/`)

The scheduler only ever sends **names** over the channel. The fire task in
`main.rs` looks the name up in the buzzer library and calls
`dispatch(state, &action)` for each chained action:

| Action | Implementation |
|---|---|
| `DefaultAudio` / `Audio(p)` | `buzzers/audio.rs` — rodio decodes and plays in a background thread so the daemon never blocks. `None` plays the built-in `chime.wav` embedded via `include_bytes!`. After starting playback, calls `platform::focus_media_window()` (currently a documented stub). |
| `DefaultVideo` / `Video(p)` | `buzzers/video.rs` — `open::that()` on the file. The built-in clip (`assets/default.mp4`) is referenced by path, resolved relative to `CARGO_MANIFEST_DIR`. |
| `Application(p)` | `buzzers/application.rs` — `Command::new(p).spawn()`. |
| `Url(u)` | `buzzers/url.rs` — `open::that(u)`. |
| `Bash(p)` | `buzzers/bash.rs` — `sh -c <p>` on Unix, `cmd /C <p>` on Windows, spawned detached. |
| `CloseAllWindows` | `buzzers/close_windows.rs` — see §7.1. |
| `CloseApplication(n)` | `buzzers/close_application.rs` — `pkill -x n` → `pkill -f n` (Linux), `osascript quit` → `pkill -x` (macOS), `taskkill /IM n` → `/F` (Windows). Destructive: gated like §7.1. |
| `FocusWindow(n)` | `buzzers/focus_window.rs` — `wmctrl -a n` → `xdotool search --name n windowactivate` (Linux), `osascript tell application n to activate` (macOS), PowerShell `AppActivate` (Windows). Non-destructive. |
| `Llm { model, prompt }` | `buzzers/llm.rs` — see §7.2. |

An unknown buzzer name degrades to a logged `BUZZ: <name>` warning instead
of a silent failure.

### 7.1 The destructive buzzers

`close_windows` closes every visible window except the daemon's own
terminal; `close_app` closes one named application. Both require an
explicit, per-daemon-session opt-in:

1. `strangetimer confirm-destructive` sends `ConfirmDestructive`.
2. `dispatch` checks `AppState::is_close_windows_confirmed()`; without it
   the action prints a warning and returns — it never closes anything.
3. The flag lives in memory (not in `state.json`), so every daemon restart
   re-arms the protection. Deliberate: a destructive action should not
   silently survive a reboot.

Platform backends:
- **Linux/X11:** `wmctrl -lp` lists `(window_id, pid, title)`. Every window
  whose PID differs from the daemon's is closed with `wmctrl -ic <id>`.
  Falls back to `xdotool search --onlyvisible --name ""` +
  `getwindowpid`/`windowclose` when wmctrl is absent. (The plan suggested
  a `/proc` lookup; `wmctrl -lp` already provides PIDs directly, so the
  skip-own-window logic is simpler and equivalent.)
- **macOS:** one `osascript` call closes every window of every process whose
  unix id is not the daemon's.
- **Windows:** `taskkill /F /FI "PID ne <daemon>" /FI "STATUS eq RUNNING"`.

### 7.2 The LLM buzzer

At fire time the daemon POSTs to `http://localhost:11434/api/generate`:

```json
{ "model": "<model>", "prompt": "<resolved prompt>", "stream": false }
```

with a 10 s timeout. The prompt is inline text or read from a file at fire
time (so the prompt can be edited without recreating the buzzer). If Ollama
is unreachable, the daemon logs a warning and falls back to the built-in
chime — a silent buzzer is worse than a beep.

---

## 8. Lifecycle: Startup, Recovery, Shutdown

### 8.1 Startup sequence (`main.rs`)

1. **Load** timers, buzzers, state from disk.
2. **Seed** the built-in buzzer library if `buzzers.json` was empty
   (fresh install): `default_audio`, `default_video`, `close_windows`.
3. **Autostart registration** — if `state.registered` is false, write the
   OS service unit (`~/.config/systemd/user/strangetimer.service`,
   launchd plist, or schtasks entry), start it, and persist
   `registered = true`. Runs exactly once per install.
4. **Spawn tasks** — the scheduler and the fire task, joined by an
   `mpsc` channel.
5. **Recover** (§8.2).
6. **Bind** the IPC listener and serve.

### 8.2 Restart recovery (Prompt 20)

On startup, for every run still in `Running`:

- compute `effective_elapsed = (now - started_at) - elapsed_before_pause`.
  Because the time model is wall-clock-based, the downtime is automatically
  included — `last_saved_at` is recorded for diagnostics rather than being
  needed for arithmetic.
- every buzzer whose `offset <= effective_elapsed` and is not in
  `fired_indices` is marked fired and its name is queued to the fire
  channel **immediately** — missed alarms fire on restart.

`Scheduled` runs whose `schedule_time <= now` are flipped to `Running` at
once. The state is saved, then the normal scheduler takes over.

This is what makes StrangeTimer's persistence meaningful: kill the daemon,
reboot the machine, and when it comes back the alarms that were due during
the downtime all go off immediately.

### 8.3 Shutdown

Three ways to stop the daemon, all ending in the same save-and-exit path:

1. **Signal** — SIGINT/SIGTERM (Ctrl+C on Windows) breaks the `tokio::select!`
   in `main`; the final state is saved before exit.
2. **IPC** — `strangetimer daemon stop` sends `Shutdown`. It is intercepted
   in `handle_connection` (before the normal dispatch) so the client
   receives its `Ok` first, then a shared `tokio::sync::Notify` is fired.
   The accept loop is a `select!` between `accept()` and that notify, so
   it returns and the same teardown runs. This makes the daemon lifecycle
   scriptable without signals.
3. **Crash** — SIGKILL / power loss; handled by §8.2 on the next start.

One detail worth noting: a Unix-domain socket leaves a stale file after an
unclean death. Before binding, the daemon probes the path — if nothing is
listening, it removes the stale file and rebinds; if something is
listening, the bind fails loudly (another daemon owns the endpoint).
Without this, a crashed daemon could never restart. `daemon start` /
`daemon stop` rely on the same probe: a new daemon never steals a live
endpoint, so `stop` (graceful handover) then `start` is the only supported
way to replace a running daemon.

### 8.4 Daemon lifecycle management (`strangetimer daemon`)

The CLI (`commands/daemon.rs`) manages the process:

- **Probe** — two steps: a raw socket connect decides *listening*; a `Ping`
  round-trip decides *compatible*. `Probe` is one of
  `Running{pid,version}` / `Incompatible` / `NotRunning`. A listener that
  cannot answer `Ping` (older binary) is *incompatible*, never "not
  running" — otherwise `start` would spawn a second instance that dies
  with "Address already in use" (the original failure mode, found in
  `daemon.log`).
- `status` — prints the probe result.
- `start` — on `Incompatible` it refuses with a remedy hint. Otherwise it
  locates the daemon binary (sibling of the CLI exe → `PATH` →
  `STRANGETIMER_DAEMON`) and prefers the OS service manager when one is
  registered: systemd `systemctl --user start` (healing the unit's
  `ExecStart` first — dev builds move between `target/debug` and
  `target/release`), launchd `launchctl kickstart`, or schtasks `/Run`.
  Only when none of those apply (or they fail) does it spawn directly:
  detached (new process group on Unix, `DETACHED_PROCESS` on Windows,
  stdout/stderr → `daemon.log` in the data dir). Readiness is polled via
  the probe. The isolation env vars (`STRANGETIMER_SOCKET` /
  `STRANGETIMER_DATA_DIR`) always force the direct path — systemd would
  not inherit them, and tests stay hermetic.
- `stop` — if the OS service manager owns the daemon, it stops the service
  (systemd would otherwise restart a pkill'd process). Otherwise it sends
  `Shutdown` and polls until the socket stops accepting; if the listener
  is incompatible, or the daemon ignores `Shutdown`, it force-kills by
  process name (`pkill -x strangetimer-daemon` / `taskkill`).
- `restart` — stop followed by start.

**Auto-start**: every other CLI command transparently starts the daemon on
connect failure (`commands/mod.rs::send_and_receive`) — one stderr notice,
then the shared start routine, then one retry. `Ping` and `Shutdown` are
exempt so `daemon status`/`stop` can still report "not running".
`STRANGETIMER_AUTO_START=0` opts out. This is what makes first-run UX work
without a wrapper script: `strangetimer create timer …` just works.

**Registration** (`platform.rs`) writes the service unit but no longer
starts it (`enable` without `--now`, plist without `launchctl load`) —
starting is the CLI's job, and having the daemon start itself from inside
a just-spawned process raced it for the socket.

### 8.6 User-interrupt mode (`run -u`)

`run -u` is per-run opt-in: `RunTimer` carries `user_interrupt` and the
captured `interrupt_focus` window; `TimerRun` persists both and
`DaemonState::interrupt_pending` records the timer awaiting
acknowledgement (serde defaults keep older state files readable).

On a buzzer fire the fire task (which now receives structured
[`FireEvent`]s — timer, buzzer, buzzer index, repetition — over the
scheduler channel) pauses the run via `begin_interrupt`, dispatches the
actions — audio actions **loop** (`fire_audio_until`) until `resume`
clears the pending marker, each loop in its own spawned task so one
pending timer never blocks another — and finally focuses the captured
terminal window. The CLI is **detached**: `run -u` prints the
acknowledge hint and returns; `strangetimer resume <name>` is the
acknowledgement path and the live view shows a blinking PENDING marker.
`DaemonState::pending_interrupts: Vec<String>` supports several pending
runs at once (the legacy `interrupt_pending` marker is migrated on
load). A restart keeps runs paused-pending without looping audio during
downtime.

### 8.5a Buzzer view metadata

`GetBuzzerInfo` returns `BuzzerInfo` (per-action targets and media
durations plus timer/live reference counts) and `GetBuzzerDetail`
adds the referencing timers. Durations come from file headers — rodio
`total_duration` for audio, a bounded `moov/mvhd` parser for MP4 — cached
by path in `StateInner::media_cache`; embedded assets resolve through the
same paths the dispatcher uses. `view timers` gained an `ELAPSED` column
(counts upward for running runs, freezes while paused/pending, zero while
scheduled) and `view <timer>` shows `Elapsed:` too.

### 8.5b Confirmations and destructive flags

`create timer --replace [--yes] [--stop-running]` and
`delete buzzer --cascade [--yes]` are CLI-prompted; the daemon performs
the final atomic checks under its lock (`add_timer_options`,
`delete_buzzer_cascade`). Noninteractive stdin without `--yes` fails
closed.

### 8.6a Window closing

`--close-window <id-or-title>` closes one selected window: Linux prefers
`wmctrl -i -c <id>` (title via `wmctrl -c`), falling back to `xdotool
windowclose`; macOS uses AppleScript; Windows and Wayland report
unsupported instead of pretending. The legacy `CloseAllWindows` action
is deserialized for compatibility but refused at dispatch with a
migration hint — closing the entire desktop was too destructive.
`CloseApplication` remains for process-level closing.

### 8.6b Fire outbox (durable dispatch)

Before handing a `FireEvent` to the fire task, the scheduler persists it
in `DaemonState.pending_fires`; the fire task removes it after dispatch.
Startup replays any remaining entries, so a crash between scheduling and
dispatch never loses an alarm. `BuzzerEvent.outcome` carries block
reasons (deprecated action, missing confirmation, awaiting
acknowledgement) surfaced by `strangetimer watch`.

### 8.7 Buzzer ringing events and watch

The daemon keeps a bounded, memory-only `VecDeque<BuzzerEvent>` (id,
timer, buzzer, types, fired_at, repetition, requires_ack) and answers
`GetEvents { after_id }`. `strangetimer watch` polls it every 400ms and
prints `<timer> ringing | <types> | <time>`, plus the
`-> strangetimer resume <timer>` hint for user-interrupt runs and any
`outcome` (blocked actions). Events are
never emitted through the daemon logger, so they cannot corrupt
interactive terminals; `after_id` makes watchers at-least-once without
duplicates.

### 8.8 Terminal focus (`run -u`)

`FocusSpec` (core model) captures the X11 window id, title, DISPLAY and
XAUTHORITY at `run -u` time, JSON-encoded in `interrupt_focus`; Wayland
sessions are flagged. At fire time the daemon activates by window id
(`wmctrl -i -a`, fallback `xdotool windowactivate --sync`, then title
search) carrying the session env, with retries after async
player/browser launches; Wayland reports unsupported rather than failing
silently. The autostart unit also carries the common GUI env vars.

### 8.5 Logging

`log.rs` appends every message (debug/info/warn) to `daemon.log` in the
data dir and mirrors to stderr only messages at or above
`STRANGETIMER_LOG` (default `warn`). IPC chatter and `BUZZ:` lines are
info/debug, so a foreground daemon no longer interleaves log lines with
the user's typing; `STRANGETIMER_LOG=debug` restores full verbosity.

---

## 9. The CLI

### 9.1 Stateless request/response

Every command handler does the same three steps:

```
connect()  →  write_message(ClientMessage)  →  read_message(ServerMessage)
```

`commands/mod.rs` exposes `send_and_receive()` and `ensure_ok()` (which
turns `ServerMessage::Error` into a user-facing error with a non-zero exit
code). The CLI holds **no** timer state — all of it lives in the daemon —
so the two binaries can never disagree about what is running.

The daemon-lifecycle commands (`daemon.rs`) are the one family that also
spawns a process: `daemon start` launches the daemon binary detached and
polls the socket until the listener accepts, and `daemon stop` sends
`Shutdown` and polls until it stops accepting.

### 9.2 Parsing rules worth knowing

- **create timer:** the variadic tokens are parsed as alternating
  `(offset, optional buzzer)` pairs. A token that parses as an offset opens
  a slot; the next non-offset token is that slot's buzzer name; a slot left
  without a name gets `default_audio`. `45min 15min` yields two alarms on
  the default buzzer; `1W paymentBuzzer` yields one alarm on the named one.
- **run `-t`:** `HH:MM` is parsed into today's local `DateTime`; if that
  moment has passed, it rolls over to tomorrow.
- **create buzzer `--llm`:** the second argument is treated as a file path
  if it exists on disk, otherwise as inline text.
- `-n` and `-i` are a clap `conflicts_with` pair.

### 9.3 View rendering (`commands/view.rs`)

`view timers` fetches **one** snapshot (`TimerList` with all runs) and
renders from it. In a TTY it enters raw mode plus the **alternate screen**
and re-renders every 100 ms with the cursor character cycling
`▂ → ▄ → ▆`; the layout functions are pure and take `(width, height)`, so
terminal size is re-queried every frame and a `Resize` event merely
re-lays out instead of exiting. Any key exits and restores the shell
(a guard restores raw mode, cursor and alternate screen even on panic).
On a non-TTY stdout it prints the same layout as a static snapshot —
useful for scripts and tests.

The overview is a bordered table (muted `DarkGrey` rules, `DarkCyan`
headers, color-coded statuses):

```
ACTIVE RUNS
│ TIMER      │ STATUS    │ START → END     │ NEXT          │ PROGRESS
───────────────────────────────────────────────────────────────────────
│ workAndFun │ run ×3    │ 09:00 → 09:45   │ default_audio │ X-███▄██-X
───────────────────────────────────────────────────────────────────────
│ focusTest  │ —         │ total 25m       │ 1 buzzer      │ —
```

- **Active** rows are live runs sorted by next-buzzer time, each spanning
  two physical lines - details, then a full-width progress bar on its own
  line; **inactive** rows are defined timers without a live run.
- Columns are sized from the exact terminal width; cells are truncated and
  padded by display width (unicode-width) *before* styling, so ANSI codes
  never shift columns and no line ever wraps.
- `view timers` runs live by default: alternate screen, only q/Escape/
  Ctrl+C exit (arrows and mouse scroll are ignored), the daemon snapshot
  is refetched every ~1s, and a final snapshot is printed to the primary
  screen on exit so the display persists in scrollback. `--snapshot`
  prints a static, scrollable view.
- Rows are capped to the terminal height with a "+N more" line.

The table, buzzer library, command confirmations, prompts and help all
share the muted Cosmic-like palette from `style.rs` (NO_COLOR-aware).

Each block:

```
<name>  Start: <datetime>  End: <datetime>  Mult: <n>
Next: <next_buzzer_name>  <remaining>
X-<bar>-X
```

- `End` = `started_at + total_duration` where total is the largest offset.
- `Mult` = repetition count (`∞` for infinite).
- `Next` = the earliest unfired buzzer of the current repetition; remaining
  = `fire_time - now` (frozen for paused runs).
- The bar width is `terminal_width - 4`. Each cell is a proportional slice
  of the total; `▓` marks buzzer positions; the cursor cell marks the
  current position; everything else is `█`.

`view <name>` renders the single block plus the buzzer countdown table
(repetition 1), matching the format in `claude.md`.

---

## 10. Concurrency Model

The daemon is single-process, single-state, with one mutex:

```
AppState { inner: Mutex<StateInner> }
StateInner { timers, buzzers, state: DaemonState, close_windows_confirmed }
```

Every actor — IPC handler tasks, scheduler tick, fire task, recovery —
takes the mutex for the duration of its mutation, so there is exactly one
logical writer of the state at any time. Persistence happens while the lock
is held, which serialises disk writes and makes the `unique-tmp` requirement
(from §4.3) a belt-and-braces guarantee rather than a race-prone necessity.

The scheduler is the *only* component that mutates `TimerRun.status`,
`fired_indices` and `current_rep`; IPC handlers mutate definitions and
paused flags. This separation keeps the state transitions easy to reason
about and to test.

---

## 11. Failure Modes

| Failure | Behaviour |
|---|---|
| Daemon not running | CLI commands auto-start it (one stderr notice + retry); `daemon status` reports "not running". `STRANGETIMER_AUTO_START=0` restores the plain hint error. |
| Incompatible listener (older daemon / foreign process on the socket) | Probe reports *incompatible*; `daemon start` refuses with a remedy hint; `daemon stop` force-kills it. |
| Daemon crashes (SIGKILL) | Stale socket removed on restart; state replayed from disk; missed alarms fire immediately. |
| Power loss mid-write | Atomic rename leaves the previous state file intact. |
| Two daemons | Second bind fails loudly ("Address already in use"); the probe refuses to steal a live endpoint. Replace via `strangetimer daemon stop` → `start`. |
| Ollama down | LLM buzzer logs a warning and plays the built-in chime. |
| No audio device | rodio logs a playback error; the rest of the daemon is unaffected. |
| Unknown buzzer name | Fire task logs `BUZZ: <name> — no such buzzer` instead of dying. |
| Corrupt JSON | Persistence surfaces a descriptive parse error; daemon refuses to start (no silent data loss). |
| Deleted timer still running | `delete timer` is refused while a *live* run is active (running/paused/scheduled/pending); stop first. Completed runs are terminal — deletion succeeds and cleans up the run record. |
| `close_app` / `close_windows` not opted in | Action logs a warning and does nothing — never closes without `confirm-destructive`. |

---

## 12. Test Strategy

- **Core unit tests** (`strangetimer-core`): offset grammar, IPC framing
  round-trips, persistence atomicity/round-trips/env override.
- **Daemon unit tests** (`strangetimer-daemon`): `AppState` operations and
  guards (duplicate names, active-run delete protection, builtin buzzers,
  pause/resume elapsed accounting) and the scheduler's `tick` semantics
  (due/unfired alarms, repetition advance, completion, infinite, scheduled
  transitions). The scheduler loop is factored so tests drive a single
  tick against a state built in the past — no sleeps.
- **CLI unit tests** (`strangetimer`): offset-pair parsing, buzzer flag
  parsing, `-t` parsing, view rendering (bar markers, cursor position,
  pause freeze, countdown table).
- **E2E tests** (`strangetimer/tests/e2e.rs`): spawn the real daemon and
  drive it with the real CLI over real IPC, isolated per test via
  `STRANGETIMER_DATA_DIR` + `STRANGETIMER_SOCKET`. Covers seeding, CRUD,
  delete guards, alarm firing (asserting on daemon stderr), pause/resume,
  crash-recovery, persistence-file placement, the full daemon lifecycle
  (three start→stop cycles), auto-start, stop-without-auto-start, an
  incompatible listener (a dummy Unix socket that never answers — `daemon
  start` must refuse), `examples --install`, and the window-action buzzers.

Run everything with `cargo test --workspace` (builds all binaries first)
and lint with `cargo clippy --workspace --all-targets`.

---

## 13. Security & Safety Notes

- The IPC endpoint is a user-scoped Unix socket / named pipe with no
  authentication. Anything that can write to it can start/stop timers or
  trigger buzzers. This is acceptable for a single-user desktop tool but is
  worth knowing before enabling multi-user access.
- `bash` buzzers execute arbitrary commands and `close_windows` /
  `close_app` are destructive; the latter two are gated behind an explicit
  opt-in per daemon session (§7.1). Prompt payloads sent to Ollama may
  contain anything the user wrote; they stay on the local machine.
- Secrets in LLM prompts or bash scripts are handled exactly as the user
  configures them — nothing is logged beyond the action type.

---

## 14. Deviations & Decisions vs. the Original Plan

| # | Plan | Implementation | Why |
|---|---|---|---|
| 1 | `TimerList(Vec<Timer>)` | `TimerList { timers, runs }` | `view` needs run data; one round-trip beats N+1. |
| 2 | Recovery arithmetic via `last_saved_at` | Recovery via wall-clock elapsed; `last_saved_at` kept for diagnostics | The time model already accounts for downtime; the saved timestamp is redundant for arithmetic but useful in state files. |
| 3 | `close_windows` PID check via `/proc` | PID directly from `wmctrl -lp` / `xdotool getwindowpid` | Same result, no proc parsing. |
| 4 | Scheduler handles only `Running` runs | Scheduler also promotes `Scheduled` → `Running` when time passes | Otherwise `-t` would only work across a daemon restart. |
| 5 | Fixed `.tmp` names | Unique per-writer temp names | Fixed names race under concurrent writers (found by tests). |
| 6 | `create timer` accepts duplicate names | Refused | Names are the identity key for every other command. |
| 7 | View is animated only | Static fallback when stdout is not a TTY | Scriptable/piped output and e2e assertions. |
| 8 | `close_windows_confirmed` unpersisted | Stays in-memory | Destructive opt-in should not survive a restart. |
| 9 | Daemon takeover (Prompt 22 option) | CLI-managed lifecycle only; daemon never steals a live endpoint | Predictable: "which daemon is mine?" has one answer; graceful stop+start is the supported handover. |
| 10 | `examples --install` installs every example | Only file-free examples installable; path-based ones are docs-only | Audio/app/script paths are machine-specific; a broken default path would be worse than a copy-paste command. |
| 11 | Ping-only liveness probe (Prompt 22) | Two-step probe: connect (listening) then Ping (compatible) | An old binary can't answer Ping; treating it as "not running" spawned a doomed second daemon ("Address already in use"). |
| 12 | Registration starts the service (`enable --now` / `launchctl load`) | Registration enables/writes only; starting is the CLI's job | The daemon starting a second copy of itself from inside a service manager raced it for the IPC socket. |
| 13 | `daemon start` always spawns | Prefers systemd/launchd/schtasks when registered (unless env-isolated) | One owner per socket; the unit path is healed when the binary moves (dev vs release). |
| 14 | Foreground daemon logs to stderr | Level-gated logging to `daemon.log`; terminal shows warn+ | IPC chatter interleaved with typing in a foreground run. |
| 15 | View exits on resize | Resize re-lays out on the alternate screen | The old render sampled the width once and left wrapped stale content on shrink. |
