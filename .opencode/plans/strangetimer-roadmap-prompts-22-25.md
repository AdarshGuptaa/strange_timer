# StrangeTimer — Planned Prompt Sections for plan.md (Prompts 22–25)

These sections are ready to be appended to plan.md (inserted before
"## Build Order Summary", which is also updated below). Blocked from direct
edit by plan-mode permissions.

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

## Build Order Summary (updated)

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
```
