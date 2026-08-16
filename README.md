# StrangeTimer

A Rust CLI timer application with a background daemon, persistent buzzer
library, and a live-animated terminal progress view.

```
strangetimer          ← the CLI you type commands into
strangetimer-daemon   ← the background service that tracks time and fires buzzers
strangetimer-core     ← shared library (data model, persistence, IPC protocol)
```

## Features

- **Timers with multiple alarms** — a timer is a named list of
  `(offset, buzzer)` pairs, e.g. `45min` + `15min`, fired in sequence from
  the moment the timer starts.
- **Buzzer library** — reusable alarms that chain multiple actions:
  audio, video, application, URL, shell script, close a selected window
  or app, focus a window, or an LLM call through local Ollama.
- **Repetition & scheduling** — run a timer `-n` times, infinitely (`-i`),
  or at a specific clock time (`-t HH:MM`).
- **Managed daemon lifecycle** — `strangetimer daemon start/stop/status/
  restart` starts, stops and checks the background service for you; no
  manual `&` or `kill` needed. And because every other command
  **auto-starts** the daemon on first use, you can go straight from
  install to `strangetimer create timer …`. (`STRANGETIMER_AUTO_START=0`
  opts out.)
- **Live progress view** — `strangetimer view timers` renders animated
  progress bars with buzzer markers on the terminal's **alternate
  buffer**: every frame updates in place and nothing is appended to your
  scrollback. `q`/`Ctrl+C` exits; `--snapshot` prints a persistent,
  scrollable table instead.
- **Crash recovery** — the daemon persists every state change atomically and
  fires any alarms that were missed while it was down or the machine was off.
- **Autostart** — on first run the daemon registers itself as a system
  service (systemd / launchd / Task Scheduler) and resumes persisted runs
  after reboot.
- **Shell completions** — `strangetimer install-completions` wires <Tab>
  completion into your shell, with live suggestions for your timer and
  buzzer names as you type.
- **User-interrupt mode** — `run -u` pauses the timer and loops audio at
  every buzzer until you acknowledge it with `strangetimer resume <name>`;
  the CLI returns immediately, and the live view shows a blinking PENDING
  marker for runs awaiting acknowledgement.

## Installation

### One-command install (recommended)

**Linux / macOS** (installs into `~/.local`, no sudo, sets up PATH,
completions, autostart, and starts the daemon):

```sh
curl --proto '=https' --tlsv1.2 -fsSL \
  https://github.com/AdarshGuptaa/strange_timer/releases/latest/download/install.sh | sh
```

Open a new terminal, then:

```sh
strangetimer create timer demo 1m
strangetimer run demo
strangetimer view timers
```

**Windows** (PowerShell):

```powershell
irm https://github.com/AdarshGuptaa/strange_timer/releases/latest/download/install.ps1 | iex
```

The installer verifies checksums, installs atomically, updates the
versioned `current` symlink, and keeps your timer data across updates.

Uninstall: `install.sh --uninstall` (keeps data) / `--purge-data` (wipes
it); Windows: `install.ps1 -Uninstall`.

### From a GitHub release (manual)

1. Download the archive for your platform from the
   [releases page](https://github.com/AdarshGuptaa/strange_timer/releases).
2. Extract it and put both binaries AND the `assets/` folder somewhere on
   your `PATH`:

   ```sh
   tar -xzf strangetimer-v1.0.1-beta.1-linux-x86_64.tar.gz
   cp strangetimer strangetimer-daemon ~/.local/bin/
   cp -r assets ~/.local/share/strangetimer-assets/   # or beside the daemon
   ```

3. Install shell completions:

   ```sh
   strangetimer install-completions
   ```

4. Start the daemon (it registers autostart on first run):

   ```sh
   strangetimer daemon start
   ```

### With Cargo

```sh
cargo install --locked --git https://github.com/AdarshGuptaa/strange_timer \
  strangetimer strangetimer-daemon --root ~/.local
strangetimer install-completions
strangetimer daemon start
```

### From source

```sh
cargo build --release
target/release/strangetimer daemon start
```

## Quick Start

```sh
# No need to start the daemon first — this does it for you:
strangetimer create timer workAndFun 45min 15min

# Run it three times, and watch the live view (press any key to exit)
strangetimer run workAndFun -n 3
strangetimer view timers          # live in-place table; q/Ctrl+C to exit
strangetimer view timers --snapshot   # one static table, stays in scrollback

# User-interrupt: pauses at each buzzer until you run `resume`
strangetimer run workAndFun -u
```

The first command that talks to the daemon starts it in the background
(`strangetimer daemon start` does the same thing explicitly; set
`STRANGETIMER_AUTO_START=0` to disable). It also registers itself for
autostart on first run, so after a reboot your timers resume by
themselves. On Linux the daemon then runs as a systemd user service —
`strangetimer daemon stop` stops it cleanly, and `daemon status` shows
the PID, version and IPC protocol. `strangetimer daemon enable/disable/
uninstall` manage the autostart service explicitly, and `strangetimer
doctor` reports installation health and optional capabilities.

### Example session

```sh
$ strangetimer create timer weeklyPayment 1W paymentBuzzer
Created timer "weeklyPayment".

$ strangetimer run weeklyPayment
Timer "weeklyPayment" started.

$ strangetimer view timers
weeklyPayment  Start: 2026-08-14 09:00:00  End: 2026-08-21 09:00:00  Mult: 1
Next: paymentBuzzer  6D 22:41:07
X-█▄▓████████████████████████████████-X
```

## Buzzer Actions

A buzzer is a named alarm you attach to timers. When its offset elapses,
every action it chains fires in sequence. This is the full menu of actions
— see [docs/BUZZER_EXAMPLES.md](docs/BUZZER_EXAMPLES.md) for a guide with a
worked timer, or run `strangetimer examples` to get them interactively.

### Audio — `--audio [path]`

Plays a sound on a background thread (the daemon never blocks). Omit the
path for the built-in chime.

```sh
strangetimer create buzzer hourDone --audio
strangetimer create buzzer alarm --audio ~/Music/alert.wav
```

When it fires: you hear a chime (or your file).

### Video — `--video [path]`

Opens a video in your default player. Omit the path for the built-in
default clip.

```sh
strangetimer create buzzer breakOver --video
```

When it fires: your media player opens with the clip.

### URL — `--url <url>`

Opens a URL in your default browser — a meeting link, a dashboard, or this
project's page.

```sh
strangetimer create buzzer meeting --url https://meet.example.com/standup
strangetimer create buzzer github --url https://github.com/AdarshGuptaa/strange_timer
```

When it fires: your browser opens the page.

### Application — `--application <path>`

Launches a program from an absolute path.

```sh
strangetimer create buzzer report --application /usr/bin/gnome-calculator
```

When it fires: the application starts.

### Shell script — `--bash <path>`

Runs a script (`sh -c <path>` on Unix, `cmd /C <path>` on Windows). The
catch-all action: desktop notifications, logging, SSH, anything scriptable.

```sh
strangetimer create buzzer notify --bash ~/notify.sh
```

When it fires: the script runs in a detached process.

### Close one application — `--close-app <name>` *(destructive)*

Closes a single application by process name (`firefox`, `chrome`, …).
Because it is destructive, it is gated behind an explicit opt-in:

```sh
strangetimer confirm-destructive
strangetimer create buzzer quitBrowser --close-app firefox
```

When it fires: the named application quits. The opt-in is persisted, so it
survives daemon restarts and reboots; revoke it at any time with
`strangetimer revoke-destructive`.

### Focus a window — `--focus-window <name>`

Brings a window to the foreground by title substring or application name.
Non-destructive.

```sh
strangetimer create buzzer focusChat --focus-window Slack
```

When it fires: the matching window comes to the front (Linux: wmctrl /
xdotool; macOS: AppleScript; Windows: PowerShell AppActivate).

### Close all windows — `--close-app`'s big brother *(very destructive)*

The built-in `close_windows` buzzer closes **every** other window on the
desktop. It also requires `strangetimer confirm-destructive`. Use at your
own risk.

```sh
strangetimer confirm-destructive
strangetimer create timer shutdownTime 30min close_windows
```

### LLM — `--llm <model> <prompt_or_file>`

Asks a local [Ollama](https://ollama.com) model to announce the alarm. The
second argument is inline text, or a file path if it exists on disk (the
file is read at fire time). If Ollama is unreachable, the buzzer plays the
built-in chime instead of staying silent.

```sh
strangetimer create buzzer pepTalk --llm llama3 "Give a one-line pep talk about finishing the task."
```

When it fires: the model's reply is the alarm.

### Chaining actions

Pass several flags to one buzzer — they fire in command-line order:

```sh
strangetimer create buzzer dayEnd \
  --audio \
  --url https://github.com/AdarshGuptaa/strange_timer \
  --focus-window "Notes"
```

## Command Reference

### Daemon lifecycle

| Command | Description |
|---|---|
| `strangetimer daemon start` | Start the daemon in the background (no-op if running; uses the systemd/launchd/schtasks service when one is registered). |
| `strangetimer daemon stop` | Gracefully stop the daemon — it saves state and exits. |
| `strangetimer daemon status` | Is it running? Reports PID and version. |
| `strangetimer daemon restart` | Stop, then start. |
| `strangetimer daemon enable` / `disable` / `uninstall` | Register, disable, or remove the OS autostart service. |
| `strangetimer doctor` | Report installation health, versions, protocol, and optional capabilities. |

Any other command auto-starts the daemon if it isn't running
(`STRANGETIMER_AUTO_START=0` disables this).

### Timer lifecycle

| Command | Description |
|---|---|
| `strangetimer create timer <name> <offset> [<buzzer> ...] [--replace] [--yes] [--stop-running] [--no-preview]` | Create a timer (previews the definition). `--replace` swaps an existing definition after confirmation; `--yes` skips prompts; `--stop-running` also stops a live run. |
| `strangetimer duplicate timer <source> [<new_name>]` | Clone a timer; defaults to `<source>_copy`. |
| `strangetimer delete timer <name>` | Delete a definition (refused while a run is active). |
| `strangetimer run <name> [-n count \| -i] [-t HH:MM] [-u]` | Start a run, optionally repeated or scheduled. `-u` pauses at every buzzer until acknowledged with `strangetimer resume <name>`. |
| `strangetimer pause <name>` / `pauseall` | Pause the countdown. |
| `strangetimer resume <name>` | Resume a paused countdown. |
| `strangetimer stop <name>` / `stopall` | Cancel a run (keeps the definition). |

### Buzzer library

| Command | Description |
|---|---|
| `strangetimer create buzzer <name> [--audio [path]] [--video [path]] [--application path] [--url url] [--bash path] [--close-app name] [--close-window id-or-title] [--focus-window name] [--llm model prompt_or_file]` | Create a buzzer. Multiple flags chain actions fired in sequence. |
| `strangetimer delete buzzer <name>` | Delete a custom buzzer (refused while referenced). |
| `strangetimer delete buzzer <name> --cascade [--yes]` | Also delete every timer using the buzzer, after confirmation. |
| `strangetimer view buzzers` | Buzzer library table with action targets, media durations, and timer/live-run reference counts. |
| `strangetimer view buzzer <name>` | Detailed view of one buzzer (every action's target + duration, referencing timers). |
| `strangetimer confirm-destructive` | Opt in to `close_windows` and `close_app` buzzers (persisted; `revoke-destructive` undoes it). |
| `strangetimer examples [--install]` | List example buzzers for every action type, or install the file-free ones. |

The built-in buzzers `default_audio`, `default_video` and `close_windows`
are seeded on first run and cannot be deleted. `close_windows` (closing
the entire desktop) is **deprecated** and refuses to run — create a
`--close-window` or `--close-app` buzzer instead. Completed timers count
as inactive: `delete timer` works after a timer finishes.

### Viewing & extras

| Command | Description |
|---|---|
| `strangetimer view timers` | Live table of active runs (TIMER \| STATUS \| ELAPSED \| START→END \| NEXT) plus an inactive section, animated in-place on the alternate buffer; q/Ctrl+C exits. Add `--snapshot` for a persistent, scrollable printout. |
| `strangetimer view <timer_name>` | Single-timer progress block + buzzer countdown table (includes elapsed). Add `--snapshot` for static output. |
| `strangetimer completions <bash\|zsh\|fish\|powershell>` | Print a shell completion script (dynamic: suggests your timer and buzzer names). |
| `strangetimer completions --doctor` | Diagnose missing tab suggestions. |
| `strangetimer install-completions [--shell <shell>]` | Install completions into your shell (detects `$SHELL`); <Tab> then suggests subcommands, flags, and your timer/buzzer names. Re-run after upgrades - scripts call `strangetimer` via `$PATH`. |
| `strangetimer watch` | Print a `ringing` line for every fired buzzer (with the resume command for user-interrupt runs). |

When stdout is not a terminal, views render as a static snapshot instead of
animating.

## Time Offset Format

| Format | Meaning |
|---|---|
| `30s` | 30 seconds |
| `5m` / `5min` | 5 minutes |
| `2h` | 2 hours |
| `1D` | 1 day |
| `1W` | 1 week |
| `1h30m` | 1 hour 30 minutes (compound) |

## Data & Persistence

All state lives under the OS-appropriate user data directory:

| OS | Path |
|---|---|
| Linux | `~/.local/share/strangetimer/` |
| macOS | `~/Library/Application Support/strangetimer/` |
| Windows | `%APPDATA%\strangetimer\` |

`timers.json`, `buzzers.json` and `state.json` are written atomically on
every change. The daemon appends a timestamped log to `daemon.log` in the
same directory (warnings also reach the terminal; set `STRANGETIMER_LOG=
debug|info|warn` to tune that). The daemon and CLI honour
`STRANGETIMER_DATA_DIR` and `STRANGETIMER_SOCKET` environment overrides
(used by the test suite), `STRANGETIMER_DAEMON` points `daemon start` at a
specific daemon binary, and `STRANGETIMER_AUTO_START=0` disables
transparent daemon auto-starting.

## Architecture in Brief

- The **daemon** owns all live state. It runs a 500 ms scheduler tick that
  advances every running run, fires due buzzers, and handles repetition and
  completion. Exactly one daemon may run at a time — a second copy refuses
  to bind the IPC socket, and `strangetimer daemon stop` is the graceful
  way to hand the socket over.
- The **CLI** is stateless: it sends a length-prefixed JSON message over a
  Unix socket / named pipe and prints the reply. Every request also carries
  the CLI's current session environment (`DISPLAY`, `XAUTHORITY`,
  `DBUS_SESSION_BUS_ADDRESS`, …), so the daemon's GUI-side buzzer actions
  (video, URL, applications) always run against the session you are
  actually in — terminal switches, new logins and reboots no longer leave
  them pointing at a stale display. Failed launches are reported through
  `strangetimer watch` instead of vanishing into the log.
- On startup the daemon **recovers**: runs that were active during downtime
  fire their missed alarms immediately (persisted to the fire outbox first,
  so a crash during recovery can't lose an alarm), and scheduled runs whose
  time passed start at once.
- See [`docs/SYSTEM_DESIGN.md`](docs/SYSTEM_DESIGN.md) for the full design,
  [`docs/DEVELOPMENT.md`](docs/DEVELOPMENT.md) for build/test guidance, and
  [`docs/BUZZER_EXAMPLES.md`](docs/BUZZER_EXAMPLES.md) for buzzer examples.

## License

MIT
