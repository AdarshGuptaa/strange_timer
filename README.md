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
  audio, video, application, URL, shell script, close a specific app,
  focus a window, close-all-windows, or an LLM call through local Ollama.
- **Repetition & scheduling** — run a timer `-n` times, infinitely (`-i`),
  or at a specific clock time (`-t HH:MM`).
- **Managed daemon lifecycle** — `strangetimer daemon start/stop/status/
  restart` starts, stops and checks the background service for you; no
  manual `&` or `kill` needed.
- **Live progress view** — `strangetimer view timers` renders animated
  progress bars with buzzer markers; press any key to exit.
- **Crash recovery** — the daemon persists every state change atomically and
  fires any alarms that were missed while it was down or the machine was off.
- **Autostart** — on first run the daemon registers itself as a system
  service (systemd / launchd / Task Scheduler) and resumes persisted runs
  after reboot.
- **Shell completions** — `strangetimer completions bash|zsh|fish|
  powershell` prints a completion script for your shell.

## Installation

### From a GitHub release (recommended)

1. Download the archive for your platform from the
   [releases page](https://github.com/AdarshGuptaa/strange_timer/releases).
2. Extract it and put both binaries somewhere on your `PATH`:

   ```sh
   tar -xzf strangetimer-linux-x86_64.tar.gz
   cp strangetimer strangetimer-daemon ~/.local/bin/
   ```

3. Start the daemon — on first run it registers itself for autostart, so
   this is the only manual start you will ever need:

   ```sh
   strangetimer daemon start
   ```

4. Optionally install completions and man pages from the archive:

   ```sh
   source <(strangetimer completions bash)   # add to ~/.bashrc
   sudo cp man/*.1 /usr/local/share/man/man1/
   ```

### With Cargo

```sh
cargo install --git https://github.com/AdarshGuptaa/strange_timer \
  strangetimer-daemon --root ~/.local
strangetimer daemon start
```

### From source

```sh
cargo build --release
target/release/strangetimer daemon start
```

## Quick Start

```sh
strangetimer daemon start

# Create a timer: 45 minutes of work, 15 minutes of fun
strangetimer create timer workAndFun 45min 15min

# Run it three times, and watch the live view
strangetimer run workAndFun -n 3
strangetimer view timers
```

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
strangetimer confirm-destructive     # once per daemon session
strangetimer create buzzer quitBrowser --close-app firefox
```

When it fires: the named application quits. The opt-in resets on every
daemon restart, so a destructive buzzer can never silently survive a
reboot.

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
| `strangetimer daemon start` | Start the daemon in the background (no-op if running). |
| `strangetimer daemon stop` | Gracefully stop the daemon — it saves state and exits. |
| `strangetimer daemon status` | Is it running? Reports PID and version. |
| `strangetimer daemon restart` | Stop, then start. |

### Timer lifecycle

| Command | Description |
|---|---|
| `strangetimer create timer <name> <offset> [<buzzer> ...]` | Create a timer. Offsets support `30s`, `5m`/`5min`, `2h`, `1D`, `1W` and compounds like `1h30m`. A bare offset uses the built-in `default_audio` buzzer. |
| `strangetimer duplicate timer <source> [<new_name>]` | Clone a timer; defaults to `<source>_copy`. |
| `strangetimer delete timer <name>` | Delete a definition (refused while a run is active). |
| `strangetimer run <name> [-n count \| -i] [-t HH:MM]` | Start a run, optionally repeated or scheduled. |
| `strangetimer pause <name>` / `pauseall` | Pause the countdown. |
| `strangetimer resume <name>` | Resume a paused countdown. |
| `strangetimer stop <name>` / `stopall` | Cancel a run (keeps the definition). |

### Buzzer library

| Command | Description |
|---|---|
| `strangetimer create buzzer <name> [--audio [path]] [--video [path]] [--application path] [--url url] [--bash path] [--close-app name] [--focus-window name] [--llm model prompt_or_file]` | Create a buzzer. Multiple flags chain actions fired in sequence. |
| `strangetimer delete buzzer <name>` | Delete a custom buzzer. |
| `strangetimer view buzzers` | Table of the buzzer library. |
| `strangetimer confirm-destructive` | Opt in to `close_windows` and `close_app` buzzers. |
| `strangetimer examples [--install]` | List example buzzers for every action type, or install the file-free ones. |

The built-in buzzers `default_audio`, `default_video` and `close_windows`
are seeded on first run and cannot be deleted.

### Viewing & extras

| Command | Description |
|---|---|
| `strangetimer view timers` | Live-animated overview of every active run. |
| `strangetimer view <timer_name>` | Single-timer progress block + buzzer countdown table. |
| `strangetimer completions <bash\|zsh\|fish\|powershell>` | Print a shell completion script. |

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
every change. The daemon and CLI honour `STRANGETIMER_DATA_DIR` and
`STRANGETIMER_SOCKET` environment overrides (used by the test suite), and
`STRANGETIMER_DAEMON` points `daemon start` at a specific daemon binary.

## Architecture in Brief

- The **daemon** owns all live state. It runs a 500 ms scheduler tick that
  advances every running run, fires due buzzers, and handles repetition and
  completion. Exactly one daemon may run at a time — a second copy refuses
  to bind the IPC socket, and `strangetimer daemon stop` is the graceful
  way to hand the socket over.
- The **CLI** is stateless: it sends a length-prefixed JSON message over a
  Unix socket / named pipe and prints the reply.
- On startup the daemon **recovers**: runs that were active during downtime
  fire their missed alarms immediately, and scheduled runs whose time passed
  start at once.
- See [`docs/SYSTEM_DESIGN.md`](docs/SYSTEM_DESIGN.md) for the full design,
  [`docs/DEVELOPMENT.md`](docs/DEVELOPMENT.md) for build/test guidance, and
  [`docs/BUZZER_EXAMPLES.md`](docs/BUZZER_EXAMPLES.md) for buzzer examples.

## License

MIT
