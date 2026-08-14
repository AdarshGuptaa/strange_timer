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
  audio, video, application, URL, shell script, close-all-windows, or an
  LLM call through local Ollama.
- **Repetition & scheduling** — run a timer `-n` times, infinitely (`-i`),
  or at a specific clock time (`-t HH:MM`).
- **Live progress view** — `strangetimer view timers` renders animated
  progress bars with buzzer markers; press any key to exit.
- **Crash recovery** — the daemon persists every state change atomically and
  fires any alarms that were missed while it was down or the machine was off.
- **Autostart** — on first run the daemon registers itself as a system
  service (systemd / launchd / Task Scheduler) and resumes persisted runs
  after reboot.

## Quick Start

```sh
cargo build --release

# Start the daemon (it registers itself for autostart on first run)
./target/release/strangetimer-daemon &

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
X-█▄▓████████████████████████████-X
```

## Command Reference

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
| `strangetimer create buzzer <name> [--audio [path]] [--video [path]] [--application path] [--url url] [--bash path] [--llm model prompt_or_file]` | Create a buzzer. Multiple flags chain actions fired in sequence. |
| `strangetimer delete buzzer <name>` | Delete a custom buzzer. |
| `strangetimer view buzzers` | Table of the buzzer library. |
| `strangetimer confirm-destructive` | Opt in to the `close_windows` buzzer (closes ALL other windows). |

The built-in buzzers `default_audio`, `default_video` and `close_windows`
are seeded on first run and cannot be deleted.

### Viewing

| Command | Description |
|---|---|
| `strangetimer view timers` | Live-animated overview of every active run. |
| `strangetimer view <timer_name>` | Single-timer progress block + buzzer countdown table. |

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
`STRANGETIMER_SOCKET` environment overrides (used by the test suite).

## Architecture in Brief

- The **daemon** owns all live state. It runs a 500 ms scheduler tick that
  advances every running run, fires due buzzers, and handles repetition and
  completion.
- The **CLI** is stateless: it sends a length-prefixed JSON message over a
  Unix socket / named pipe and prints the reply.
- On startup the daemon **recovers**: runs that were active during downtime
  fire their missed alarms immediately, and scheduled runs whose time passed
  start at once.
- See [`docs/SYSTEM_DESIGN.md`](docs/SYSTEM_DESIGN.md) for the full design,
  and [`docs/DEVELOPMENT.md`](docs/DEVELOPMENT.md) for build/test guidance.

## License

MIT
