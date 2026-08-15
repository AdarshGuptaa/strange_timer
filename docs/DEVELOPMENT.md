# StrangeTimer — Developer Guide

How to build, test, and extend StrangeTimer.

## Prerequisites

- Rust 1.75+ (stable toolchain via [rustup](https://rustup.rs))
- For the full experience on Linux: `wmctrl` or `xdotool` (close_windows,
  focus_window), `pkill` (close_app), and [Ollama](https://ollama.com) for
  `--llm` buzzers.
- A C/C++ system compiler is required to build the native crates
  (rodio/alsa on Linux).

## Building

```sh
cargo build --workspace            # debug binaries in target/debug/
cargo build --release --workspace  # release binaries in target/release/
```

Binaries:

```
target/debug/strangetimer          CLI
target/debug/strangetimer-daemon   background service
```

## Running

```sh
# Terminal 1: start the daemon (managed, detached; logs to the data dir)
target/debug/strangetimer daemon start

# Terminal 2: the CLI
target/debug/strangetimer create timer workAndFun 45min 15min
target/debug/strangetimer run workAndFun -n 3
target/debug/strangetimer view timers     # full-screen; press any key to exit
```

Every CLI command (except `daemon status`/`daemon stop`) **auto-starts**
the daemon when it isn't running; `STRANGETIMER_AUTO_START=0` disables
that. On first run the daemon registers itself for autostart
(systemd/launchd/Task Scheduler). Manage it with:

```sh
target/debug/strangetimer daemon status    # running? pid + version
target/debug/strangetimer daemon stop      # graceful: saves state, exits
target/debug/strangetimer daemon restart
```

`daemon stop` sends an IPC `Shutdown` message — the same graceful teardown
as SIGINT/SIGTERM. A second daemon process still refuses to bind while one
is running ("Address already in use"); the CLI commands above are the
supported way to hand the socket over. To run the daemon in the foreground
instead (e.g. while hacking on it): `target/debug/strangetimer-daemon`.

### Daemon lifecycle details

- **Probe semantics**: `daemon status/start/stop` first probe the socket
  with a raw connect; if something accepts but cannot answer `Ping` (e.g.
  an older binary), it is reported as *incompatible* and `daemon start`
  refuses to spawn a second instance.
- **Service-manager awareness (Linux)**: once the systemd user unit
  exists, `daemon start` heals its `ExecStart` to the current daemon
  binary and starts via `systemctl --user start`; `daemon stop` stops the
  service. The isolation env vars (`STRANGETIMER_SOCKET` /
  `STRANGETIMER_DATA_DIR`) always force a direct spawn, which keeps tests
  hermetic. Registration itself only *enables* the unit (`--now` was
  dropped) so it never races a just-spawned daemon for the socket.
- `daemon start` locates the daemon binary next to the CLI binary, then on
  `PATH`, and honours the `STRANGETIMER_DAEMON` env var as an override.
  Its stdout/stderr go to `daemon.log` in the data dir.

### Logging

The daemon logs every message to `daemon.log` in the data dir; the
terminal only shows messages at or above `STRANGETIMER_LOG` (default:
`warn`). The chatter ("received ClientMessage::…", "BUZZ: …") is info- or
debug-level, so a foreground daemon no longer interleaves with your
typing. Raise it with `STRANGETIMER_LOG=debug` (or `info`).

### Shell completions

`strangetimer install-completions` writes the completion script to the
per-user location for your shell (bash-completion dir, fish completions,
`~/.zfunc` for zsh, or prints the PowerShell profile line). `strangetimer
completions <shell>` prints the raw script, and `strangetimer completions
--doctor` diagnoses missing suggestions (stale scripts, wrong $PATH
binary).

Generated scripts invoke `strangetimer` via `$PATH` (argv[0] trick), never
a build path - so moving/rebuilding binaries never breaks completion.
Re-run `install-completions` after upgrades. In tests, put the test binary
directory first on $PATH so the script resolves it.

Completions use the clap_complete **engine** (`unstable-dynamic` feature):
the script calls back into the CLI binary, which answers with live
candidates (`src/commands/candidates.rs`) — timer names, buzzer names,
`view` targets — fetched from the daemon without auto-starting it (a down
daemon means no candidates, never an error). To exercise it manually:

```sh
source <(strangetimer completions bash)
COMP_WORDS=(strangetimer run ''); COMP_CWORD=2; _clap_complete_strangetimer ''
echo "${COMPREPLY[*]}"
```

### Table rendering and view modes

`commands/view.rs` builds the overview as an exact-width table: cells are
truncated and padded by *display* width (`unicode-width`) on raw text and
only then styled, so ANSI codes never shift columns and no line wraps.
Each active timer spans two physical rows - a details row and a progress
row with the bar on its own line.

`view timers` animates on the terminal **alternate buffer**: each frame is
drawn from an explicit `(0, 0)` origin after `Clear(All)`, so updates
happen in place and the primary screen's scrollback is never polluted
(repeated `view timers` runs no longer stack copies of the table). Only
`q`/`Escape`/`Ctrl+C` exit; arrow keys and mouse scrolling are ignored.
The daemon snapshot is refetched every ~1s (single-timer view too), and
on exit exactly one final snapshot is printed to the primary screen.
`view timers --snapshot` prints a static, persistent view.

The live path is covered by a real PTY e2e test (`live_view_uses_alternate_
screen_and_leaves_one_snapshot`, via `portable-pty`): it asserts the
alternate-screen enter/leave sequences and that the primary buffer ends
up with exactly one snapshot — the pipe-based tests never exercise the
live TTY path.

### Buzzer view metadata

`view buzzers` shows per-buzzer action targets, media durations and
reference counts; `view buzzer NAME` lists every chained action with its
target/duration and the referencing timers. Durations are computed by the
daemon from file headers (rodio `total_duration` for audio, a bounded
`moov/mvhd` parser for MP4) and cached per path; built-in chime/video
durations are derived from the packaged assets. Remote URLs show `—`.
Reference counts count each timer definition once and live (non-completed)
runs separately.

### Interactive confirmations and destructive flags

- `create timer --replace [--yes] [--stop-running]` replaces an existing
  definition after a y/N prompt; noninteractive use requires `--yes`.
- `delete buzzer --cascade [--yes]` deletes a buzzer and all timers using
  it (refused while any of them has a live run); plain delete of a
  referenced buzzer suggests `--cascade`.
- Confirmations live in the CLI (`commands::confirm`): the daemon is
  noninteractive and rechecks everything atomically under its state lock.

### Window closing and the fire outbox

`--close-window <id-or-title>` closes a selected window (`wmctrl -i -c`
on X11, `xdotool windowclose` fallback; AppleScript on macOS; reported
unsupported on Windows/Wayland rather than failing silently). The old
`close_windows` all-windows action is deprecated and refuses to run with
a migration hint; `--close-app` handles process-level closing.

The scheduler persists every fire in `DaemonState.pending_fires` (the
outbox) before handing it to the fire task, which removes it after
dispatch. A daemon crash between scheduling and dispatch therefore never
loses the alarm: startup replays the outbox. `BuzzerEvent.outcome`
reports why an action was blocked (deprecated action, missing
confirmation, awaiting acknowledgement) and `strangetimer watch` prints
it.

### Buzzer ringing events

The daemon keeps a bounded, memory-only queue of `BuzzerEvent`s and
`strangetimer watch` polls them, printing:

```text
<timer> ringing | <types> | <time>
-> strangetimer resume <timer> to resume!   (only for run -u)
```

Events are never printed from the daemon logger (that would corrupt
interactive terminals); watchers poll `GetEvents { after_id }` so they
never miss or duplicate events.

### Test seams

The daemon honours environment overrides that let tests exercise desktop
side effects without touching the desktop:

- `STRANGETIMER_TEST_OPENER` — binary run with the target as argv instead
  of the system opener (URL/video tests).
- `STRANGETIMER_TEST_PKILL`, `STRANGETIMER_TEST_WMCTRL`,
  `STRANGETIMER_TEST_XDOTOOL`, `STRANGETIMER_TEST_OSASCRIPT`,
  `STRANGETIMER_TEST_TASKKILL` — replace the external tool (recording
  scripts assert command construction).
- `STRANGETIMER_GUI_TESTS=1` opts into the `#[ignore]`d real-desktop
  tests (focus, playback) — never run in CI.

### Terminal focus (`run -u`)

At `run -u` time the CLI captures a `FocusSpec` (X11 window id + title +
DISPLAY/XAUTHORITY; Wayland is flagged) and stores it JSON-encoded in
`TimerRun::interrupt_focus`. When a buzzer fires the daemon activates the
window id via `wmctrl -i -a` (fallback `xdotool windowactivate --sync`,
then title search), carrying the session env, with retries after async
player/browser launches. Wayland is reported unsupported instead of
pretending. The systemd unit also carries the common GUI env vars.

### Colored output

`src/style.rs` centralises the muted Cosmic-like theme. Color is off when
stdout is not a TTY, `NO_COLOR` is set, or `STRANGETIMER_COLOR=0`; set
`STRANGETIMER_COLOR=always` to force it on (tests use this). `--help`
uses the same palette via clap `Styles` in `cli.rs`.

### User-interrupt mode (`run -u`)

The run captures the active terminal window (`xdotool`/`osascript`,
best-effort), `RunTimer` carries `user_interrupt` + `interrupt_focus`, and
`TimerRun`/`DaemonState` persist them (serde defaults keep old state
files working; `pending_interrupts: Vec<String>` replaces the legacy
single `interrupt_pending` marker, folded in on load). On a buzzer fire
the daemon's fire task:

1. `begin_interrupt` — pauses the run and pushes it onto the pending list
   (`state.rs`), synchronously mirrored for background threads.
2. dispatches actions: audio **loops** (`buzzers/audio.rs::fire_audio_until`)
   until `resume` clears the marker; each loop runs in its own spawned
   task so one pending timer never blocks another's dispatch.
3. focuses the captured terminal window afterwards.

The CLI is fully **detached**: `run -u` prints the acknowledge hint and
returns immediately. `strangetimer resume <name>` is the only
acknowledgement path; the live view shows a blinking PENDING marker. A
daemon restart keeps runs paused-pending (no audio loops during
downtime).

### Isolated instances (for testing / demos)

Both binaries honour two environment variables:

```sh
export STRANGETIMER_DATA_DIR=/tmp/mydemo      # where timers.json etc. live
export STRANGETIMER_SOCKET=/tmp/mydemo.sock   # where the daemon listens
```

Useful to run a daemon without touching `~/.local/share/strangetimer` and
to run multiple instances side by side.

## Testing

```sh
cargo test --workspace        # unit + e2e tests (builds every binary first)
cargo clippy --workspace --all-targets
```

The e2e suite (`crates/strangetimer/tests/e2e.rs`) spawns the real daemon
binary and drives it with the real CLI over real IPC. Every test gets an
isolated temp data dir and socket, and kills its daemon on teardown. It
asserts on the daemon's stderr log (e.g. `BUZZ:` lines) and the CLI's
stdout. `cargo test -p strangetimer --test e2e` runs just the e2e suite.

Notes:

- Tests must not run against a live daemon on the default socket — they
  never use it (every test overrides the socket path).
- The e2e tests take ~7s (they exercise real 500 ms scheduler ticks and
  actual alarm firing).
- If `strangetimer-daemon` is missing from `target/debug`, build the
  workspace first: `cargo build --workspace`.

## Project Layout

```
crates/
  strangetimer/          CLI: clap tree in cli.rs, handlers in commands/
  strangetimer-daemon/   state.rs (AppState), scheduler.rs (tick loop),
                         buzzers/ (dispatch), platform.rs (OS integration),
                         assets/ (built-in chime + video)
  strangetimer-core/     model.rs, duration_parse.rs, persistence.rs, ipc.rs
```

## Extending

### Adding a CLI command

1. Add the variant to `Command` in `strangetimer/src/cli.rs` (clap derive).
2. Add a handler in the relevant file under `strangetimer/src/commands/`.
3. If the daemon must do something new, add a `ClientMessage` variant in
   `strangetimer-core/src/ipc.rs` and a match arm in the daemon's
   `handle_message` (`strangetimer-daemon/src/main.rs`). If it touches
   state, put the logic on `AppState` and persist before returning.

### Adding a daemon-management command

`strangetimer daemon <start|stop|status|restart>` lives in
`strangetimer/src/commands/daemon.rs` and uses three IPC messages:
`Ping` (→ `ServerMessage::Status { pid, version }`), `Shutdown` (graceful
exit) and the connect probe. The daemon answers `Shutdown` in
`handle_connection` (before dispatch) by notifying a
`tokio::sync::Notify` that tears down the accept loop, so the CLI always
gets its `Ok` before the listener closes.

### Adding a buzzer action type

1. Add the variant to `BuzzerAction` in `strangetimer-core/src/model.rs`.
2. Add a dispatch arm in `strangetimer-daemon/src/buzzers/mod.rs` and a
   module (or inline logic) implementing the side effect. Destructive
   actions (anything that closes things) must be gated behind
   `AppState::is_close_windows_confirmed()` like `CloseAllWindows` /
   `CloseApplication`.
3. If it takes CLI flags, extend `CreateBuzzerArgs` and
   `build_actions` in `strangetimer/src/commands/buzzers.rs`.
4. Add a label in `action_label` (same file) for the `view buzzers` table.

### Examples (`strangetimer examples`)

`strangetimer/src/commands/examples.rs` owns the example set. Entries with
`installable_actions: Some(...)` (no user-specific files) can be installed
with `examples --install`; the rest are docs-only and listed in
`docs/BUZZER_EXAMPLES.md`. Keep one example per action type when adding a
new one.

### Changing persistence or the IPC protocol

Both are in `strangetimer-core`; any shape change to the serialized types
must be accompanied by a migration story (currently: none — wipe the data
dir or bump the schema). Keep `write_json_atomic`'s unique-temp invariant
(§4.3 of `docs/SYSTEM_DESIGN.md`) intact.

### Testing checklist for new code

- Unit tests alongside the implementation (scheduler `tick` is factored to
  be testable without sleeps; the view's layout functions are tested with
  synthetic widths — see `crates/strangetimer/src/commands/view.rs`).
- For anything observable across processes, add an e2e case to
  `crates/strangetimer/tests/e2e.rs`.
- Run `cargo test --workspace` and `cargo clippy --workspace --all-targets`.

## Platform Notes

| Area | Linux | macOS | Windows |
|---|---|---|---|
| IPC | `/tmp/strangetimer.sock` | `/tmp/strangetimer.sock` | named pipe `\\.\pipe\strangetimer` |
| Data dir | `~/.local/share/strangetimer/` | `~/Library/Application Support/strangetimer/` | `%APPDATA%\strangetimer\` |
| Autostart | systemd user unit + `systemctl --user enable --now` | launchd plist + `launchctl load` | `schtasks /Create /SC ONLOGON` |
| Close windows | `wmctrl` → fallback `xdotool` | `osascript` | `taskkill` |
| Close app | `pkill -x` → fallback `pkill -f` | `osascript quit` → `pkill -x` | `taskkill /IM` → `/F` |
| Focus window | `wmctrl -a` → fallback `xdotool search --name` | `osascript activate` | PowerShell `AppActivate` |
| Media focus | stub (TODO) | stub (TODO) | stub (TODO) |

Only Linux has been exercised in CI; macOS/Windows paths are implemented
but untested — report issues if you hit them.

## Installing for users

The release pipeline ships a one-command installer for every platform:

- Linux/macOS: `scripts/install.sh` (user-local `~/.local`, checksum
  verified, atomic versioned install with a `current` symlink, PATH and
  completions setup, autostart registration, daemon start, health check,
  `--uninstall` / `--purge-data`).
- Windows: `scripts/install.ps1` (same flow in `%LOCALAPPDATA%`).

Both are uploaded as release assets named `install.sh` / `install.ps1`
so the documented `curl ... | sh` and `irm ... | iex` one-liners work.

Autostart registration now uses a stable service path (the installer's
`current` symlink) and refuses to register from a `target/debug` or
`target/release` checkout. The daemon reports an IPC protocol version
(`IPC_PROTOCOL_VERSION`); the CLI refuses to talk to a mismatched daemon
and points the user at `strangetimer daemon restart`.

## Releasing

Cutting a release is tag-driven: `.github/workflows/release.yml` runs the
test suite, then builds and packages release archives for linux-x86_64,
macos-x86_64, macos-aarch64 and windows-x86_64 and uploads them to the
GitHub release for the tag.

```sh
git tag v0.1.0 && git push origin v0.1.0
```

Each archive contains both binaries, shell completions
(`completions/`), man pages (`man/`, Linux archive only — generated with
`help2man`), `LICENSE` and `README.md`. Completions are generated by the
built binary itself (`strangetimer completions <shell>`), so they always
match the shipped CLI.
