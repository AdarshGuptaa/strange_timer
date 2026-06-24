# StrangeTimer — Rust CLI Timer Application
## Project Plan & Feasibility Review

---

## Feasibility Assessment

**Overall: Yes — fully buildable in Rust.** The project is well-scoped for a CLI tool. The primary complexity sits in three areas:

1. **Daemon/CLI split** — background timer tracking requires a persistent process, separate from the CLI front-end
2. **Platform-specific features** — window management and startup registration differ per OS
3. **Terminal animation** — the `view` command progress bar needs a TUI rendering loop

All features below are achievable with existing Rust crates and standard OS APIs.

---

## Architecture Overview

StrangeTimer is split into two binaries that ship together:

```
strangetimer          ← the CLI you type commands into
strangetimer-daemon   ← the background service that tracks time and fires buzzers
```

The CLI sends commands to the daemon over a **Unix socket** (Linux/macOS) or **named pipe** (Windows). The daemon holds all live timer state in memory and writes it to disk on every state change. If the OS restarts, the daemon is re-launched by the OS service manager and resumes from the saved state.

---

## Recommended Crates

| Crate | Purpose |
|-------|---------|
| `clap` v4 | CLI argument parsing and help generation |
| `chrono` | Date, time, duration, and timezone handling |
| `serde` + `serde_json` | Serialization for all persistent data |
| `tokio` | Async runtime for the daemon's timer event loop |
| `crossterm` | Terminal rendering for the animated `view` commands |
| `rodio` | Audio playback for audio buzzers |
| `open` | Cross-platform URL and application launching |
| `dirs` | Resolves OS-appropriate config/data directories |
| `interprocess` | Unix socket / named pipe IPC between CLI and daemon |
| `notify` (optional) | OS desktop notification as a lightweight buzzer option |

---

## Data Model

### Timer Definition (stored, not running)

```
name:        String
buzzers:     Vec<BuzzerRef>     // ordered list of (offset_from_start, buzzer_name)
created_at:  DateTime<Local>
```

### BuzzerRef

```
offset:       Duration     // time from timer start when this buzzer fires
buzzer_name:  String       // references a Buzzer by name in the buzzer library
```

### Buzzer (library entry)

```
name:    String
actions: Vec<BuzzerAction>   // a buzzer can chain multiple actions
```

### BuzzerAction (enum)

```
DefaultAudio
DefaultVideo
CloseAllWindows
Audio(Option<PathBuf>)          // None = built-in default sound
Video(Option<PathBuf>)          // None = built-in default video
Application(PathBuf)
Url(String)
Bash(PathBuf)
Llm { model: String, prompt: LlmPromptSource }
```

### LlmPromptSource (enum)

```
Inline(String)
File(PathBuf)
```

### TimerRun (live daemon state)

```
timer_name:     String
started_at:     DateTime<Local>
repetitions:    RepeatMode         // Count(u32) or Infinite
current_rep:    u32
schedule_time:  Option<DateTime<Local>>
status:         Running | Paused | Scheduled | Completed
paused_at:      Option<DateTime<Local>>
elapsed_before_pause: Duration
```

---

## CLI API

### Timer Lifecycle

```sh
# Create a timer (timestamps are offsets from start)
strangetimer create timer <name> <offset> [<buzzer_name>] [<offset> [<buzzer_name>]] ...

# Default buzzer is used when no buzzer name follows an offset
strangetimer create timer workAndFun 45min 15min
strangetimer create timer weeklyPayment 1W paymentBuzzer

# Duplicate a timer
strangetimer duplicate timer <source_name> [<new_name>]
# Default new_name: <source_name>_copy (or _copy_2 etc. if taken)

# Delete a timer definition (does not stop a running instance)
strangetimer delete timer <name>
```

### Running Timers

```sh
# Run a timer
strangetimer run <timer_name> [-n <count> | -i] [-t <HH:MM>]
#   -n  number of repetitions (default: 1)
#   -i  repeat infinitely (mutually exclusive with -n)
#   -t  24h clock time at which the first run begins (e.g. -t 09:00)

# Pause / resume / stop
strangetimer pause <timer_name>
strangetimer pauseall
strangetimer resume <timer_name>
strangetimer stop <timer_name>      # cancels current run, keeps definition
strangetimer stopall
```

> **Note:** `resume` and `stop` were missing from the original plan. `pause` without `resume` is a dead end.

### Buzzer Management

```sh
# View the built-in buzzer library + all custom buzzers
strangetimer view buzzers

# Create a custom buzzer (supports chaining multiple actions)
strangetimer create buzzer <name> \
  [--audio [<filepath>]] \
  [--video [<filepath>]] \
  [--application <filepath>] \
  [--url <url>] \
  [--bash <filepath>] \
  [--llm <model> <"inline prompt" | filepath>]

# Delete a buzzer
strangetimer delete buzzer <name>
```

Multiple `--` flags on one `create buzzer` command chain the actions. All actions fire in sequence when the buzzer is triggered. For example, a buzzer that both plays a sound and opens a URL:

```sh
strangetimer create buzzer paymentAlert --audio --url https://bank.example.com
```

### View Commands

```sh
strangetimer view timers         # all running/scheduled/paused timers
strangetimer view <timer_name>   # single timer + full buzzer table
```

---

## Time Offset Format

Offsets are parsed from a compact string. Supported units:

| Format | Meaning |
|--------|---------|
| `30s` | 30 seconds |
| `5m` or `5min` | 5 minutes |
| `2h` | 2 hours |
| `1D` | 1 day |
| `1W` | 1 week |

Compound offsets like `1h30m` are a useful extension to consider adding.

---

## Built-in Buzzer Library

The following buzzers are pre-installed and cannot be deleted:

| Name | Type | Description |
|------|------|-------------|
| `default_audio` | DefaultAudio | Plays a built-in chime and opens the audio |
| `default_video` | DefaultVideo | Plays a built-in short video clip |
| `close_windows` | CloseAllWindows | ⚠ Closes all windows except the timer terminal |

> **`close_windows` Warning:** This buzzer is destructive. It will terminate visible application windows. The daemon should require explicit opt-in confirmation when this buzzer is first assigned to a timer and should print a visible warning on every `view` output that contains it.

---

## `view timers` Output Format

```
workAndFun  Start: 2025-06-04 09:00:00  End: 2025-06-04 10:00:00  Mult: 3
Next: default_audio  00:14:32
X-████████▄████▓████▓████▓███████-X

weeklyPayment  Start: 2025-06-04 09:00:00  End: 2025-06-11 09:00:00  Mult: 1
Next: paymentBuzzer  6D 22:41:07
X-█▄▓████████████████████████████-X
```

**Progress bar legend:**

| Character | Meaning |
|-----------|---------|
| `█` | Completed or future time unit (scaled to bar width) |
| `▓` | Buzzer marker position |
| `▂` `▄` `▆` | Animated current-time cursor (cycles every ~300ms) |

> **Correction from original:** The animation interval was listed as `0.3ms`. This should be **~300ms (0.3 seconds)**. At 0.3ms (3,333 FPS) the terminal would be unrenderable and would saturate the CPU.

The bar width scales to terminal width. Each `█` unit represents a proportional slice of total timer duration. Buzzer markers `▓` are positioned at their fractional offset within that total.

---

## `view <timer_name>` Output Format

Shows the single-timer progress block, then a full buzzer table for **repetition 1 only**:

```
weeklyPayment  Start: 2025-06-04 09:00:00  End: 2025-06-11 09:00:00  Mult: 1
Next: paymentBuzzer  6D 22:41:07
X-█▄▓████████████████████████████-X

 Buzzer Name       Offset     Time Remaining
 ─────────────────────────────────────────────
 paymentBuzzer     1W         6D 22:41:07
```

---

## Persistence Layout

All data is stored under the OS-appropriate user data directory:

| OS | Path |
|----|------|
| Linux | `~/.local/share/strangetimer/` |
| macOS | `~/Library/Application Support/strangetimer/` |
| Windows | `%APPDATA%\strangetimer\` |

Files within:

```
timers.json        — timer definitions (name, buzzer refs)
buzzers.json       — buzzer library (built-in metadata + custom entries)
state.json         — live daemon state (active runs, pause info, elapsed time)
assets/            — user-supplied audio/video files referenced by buzzers
```

`state.json` is written atomically (write to `.tmp`, then rename) on every state change to prevent corruption on power loss.

---

## System Startup Persistence

On first `strangetimer run`, the daemon registers itself as a system service so it restarts automatically after OS reboot:

| OS | Method |
|----|--------|
| Linux | Writes `~/.config/systemd/user/strangetimer.service` and calls `systemctl --user enable --now strangetimer` |
| macOS | Writes `~/Library/LaunchAgents/com.strangetimer.daemon.plist` and calls `launchctl load` |
| Windows | Registers via Windows Task Scheduler (`schtasks`) with trigger `AtLogon` |

On daemon startup, it reads `state.json`, reconstructs all active runs, recalculates how much time elapsed while the OS was down, and fires any buzzers that were missed during downtime.

---

## LLM Buzzer (Ollama Integration)

The `--llm` buzzer action requires Ollama to be running locally (`http://localhost:11434`). Behaviour:

- On `create buzzer`, StrangeTimer records the model and prompt — it does **not** require Ollama at creation time
- At buzzer fire time, the daemon checks Ollama availability. If unavailable, it logs a warning and falls back to `default_audio`
- `--llm <model> "<inline prompt>"` — prompt is stored as a string
- `--llm <model> <filepath.txt>` — prompt is read from file at fire time (allows prompt editing without recreating the buzzer)

---

## Platform Notes on "Switches to Window"

The `DefaultAudio` and `DefaultVideo` buzzers "switch to the playing window" — meaning they bring the media player window into focus after launching it. This is platform-specific:

| OS | Method |
|----|--------|
| Linux (X11) | `xdotool` via shell command |
| Linux (Wayland) | Limited support; window focus is compositor-specific |
| macOS | `osascript` (AppleScript via `std::process::Command`) |
| Windows | `SetForegroundWindow` via `winapi` crate |

For the initial version, using `open` crate to launch the file and letting the OS handle focus is a pragmatic starting point. Native window focus can be added as a follow-up.

---

## Summary of Corrections & Additions to Original Plan

| # | Issue | Resolution |
|---|-------|------------|
| 1 | Animation interval `0.3ms` | Corrected to `300ms` (0.3 seconds) |
| 2 | `duplicate timer` syntax undefined | Defined: `strangetimer duplicate timer <src> [<new_name>]`, default name `<src>_copy` |
| 3 | `resume` and `stop` commands missing | Added `resume <timer_name>`, `stop <timer_name>`, `stopall` |
| 4 | `-n` and `-i` conflict not addressed | Enforced as mutually exclusive flags in `clap` |
| 5 | `close_windows` buzzer scope unclear | Scoped with explicit destructive warning and opt-in confirmation |
| 6 | Multi-action buzzers not addressed | Clarified that `create buzzer` can chain multiple `--` flags |
| 7 | LLM buzzer availability | Daemon checks Ollama at fire time; graceful fallback to `default_audio` |
| 8 | `view` shows single timer format missing table | Added full buzzer countdown table for `view <timer_name>` |
| 9 | Persistence path unspecified | Defined per-OS paths with atomic write strategy |
| 10 | Daemon IPC mechanism unspecified | Defined as Unix socket / named pipe via `interprocess` crate |
