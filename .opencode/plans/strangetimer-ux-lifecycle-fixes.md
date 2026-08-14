# StrangeTimer — UX & Lifecycle Fix Plan (Prompts 26–30)

Root causes identified by investigation on 2026-08-14:
- `daemon start` probe uses a Ping round-trip; an older daemon binary
  answers nothing, so the probe reports "not running" and spawns a second
  daemon → "Address already in use" → timeout error (seen in daemon.log).
- Autostart registration runs `systemctl --user enable --now`, racing the
  CLI-spawned daemon for the socket.
- The systemd unit pins the first binary path it saw (target/debug) and is
  never healed.
- Daemon logs (info-level IPC chatter) go straight to stderr → interleaves
  with typing when run in foreground.
- view.rs: width sampled once, Resize exits the view, hardcoded 49-col
  buzzer table, unbounded header lines wrap at narrow widths.
- `completions` prints to stdout but nothing installs it into the shell.

User decisions: transparent auto-start; alternate-screen TUI; quiet-by-
default logging; implement all five now.

---

## Prompt 26 — Daemon lifecycle correctness (probe + systemd awareness)

1. New probe semantics in `commands/daemon.rs`:
   - `probe() -> Probe` where `Probe = Running{pid,version} |
     Incompatible | NotRunning`.
   - Raw socket connect decides *listening*; Ping decides *compatible*.
   - `daemon start` on `Incompatible`: error with remedy text ("an older
     strangetimer-daemon is listening on <socket>; run `strangetimer
     daemon stop --force` or kill it").
   - `daemon stop`: try IPC Shutdown; if the endpoint survives (old
     binary), fall back to `pkill -x strangetimer-daemon` (unix) /
     `taskkill /IM strangetimer-daemon.exe` (windows) — behind the same
     command, reported to the user.
2. systemd-aware `daemon start` (Linux only, and ONLY when neither
   `STRANGETIMER_SOCKET` nor `STRANGETIMER_DATA_DIR` is set — test/dev
   isolation always direct-spawns):
   - If `~/.config/systemd/user/strangetimer.service` exists: rewrite the
     unit when its ExecStart no longer matches the located daemon binary
     (heals the stale target/debug pin), `systemctl --user daemon-reload`,
     then `systemctl --user start strangetimer`; poll `probe()` readiness.
   - If systemctl is missing or start fails → direct-spawn fallback.
   - `daemon stop`: if the unit exists and is active, `systemctl --user
     stop strangetimer` instead of IPC/pkill.
3. `platform.rs::register_autostart`: `enable` WITHOUT `--now` (kills the
   double-start race; reboot autostart unaffected). macOS: write plist
   without `launchctl load`; `daemon start` uses `launchctl kickstart -
   k gui/<uid>/com.strangetimer.daemon` when the plist exists (fallback:
   direct spawn). Windows: `schtasks /Run /TN StrangeTimerDaemon` when the
   task exists (fallback: direct spawn).
4. e2e: (a) a std::os::unix::net::UnixListener bound at the test socket
   that accepts but never replies → `daemon start` must report
   incompatible and spawn nothing; (b) start→stop→start→stop loop x3
   asserting success each time.

## Prompt 27 — Transparent auto-start (no wrapper script)

1. `commands/mod.rs`: `send_and_receive` gains auto-start behaviour — on
   connect failure, unless `STRANGETIMER_AUTO_START=0`, print one stderr
   notice ("StrangeTimer daemon not running — starting it…"), run the
   shared start routine from daemon.rs, wait for readiness (10 s), retry
   the connection once; only then error with the hint.
2. Exemptions: `daemon status`, `daemon stop` (never auto-start).
3. e2e: fresh env, `strangetimer view buzzers` with no daemon running →
   command succeeds and a daemon is up afterwards; then `daemon stop`.

## Prompt 28 — Quiet, file-based daemon logging

1. New tiny logger in the daemon (dependency-free): `log::info!/warn!/
   debug!` writing to `data_dir()/daemon.log` (append, Mutex<File>);
   level from `STRANGETIMER_LOG` (debug|info|warn, default warn).
   Terminal (stderr) output only at/above the level AND only when
   stderr is a TTY; warn+ always reaches the log file.
2. Convert daemon eprintln sites: IPC chatter + BUZZ lines → info;
   failures/warnings → warn. anyhow errors from main keep printing to
   stderr (foreground) but are also written to the log under
   `daemon start`.
3. README/DEVELOPMENT notes: where the log lives, how to get verbose
   output.

## Prompt 29 — Responsive alternate-screen view

1. `view.rs` animated path: EnterAlternateScreen + full Clear(All) per
   frame; LeaveAlternateScreen + terminal restore on exit (Drop guard).
   Poll at 100 ms; `Event::Resize` triggers immediate re-render and the
   loop CONTINUES; any key exits (unchanged).
2. Re-query `terminal::size()` every frame; responsive layout helpers:
   - Block header: truncate name with '…'; compact timestamps
     (HH:MM below ~60 cols; drop End below ~80; drop Mult below ~50).
   - Next line: truncate buzzer name; compact remaining.
   - Bar: `min(width-4, 40)` cap, floor 8; hide bar below 12 cols.
   - Buzzer table: column widths derived from available width; truncate
     name column; rule length = width.
   - Below ~30 cols: one-line-per-timer summary.
   - Height: render only as many blocks as rows-1 allow; append
     "+N more not shown".
3. Static non-TTY snapshot reuses the same layout functions (width 76
   fallback) so scripts/tests stay deterministic.
4. Unit tests: layout functions called with synthetic widths (30/50/80/
   200) assert no emitted line exceeds the width; truncation visible.
   Update existing view tests to the new signature.

## Prompt 30 — `strangetimer install-completions` + docs

1. New command: detects shell from `$SHELL` (override: positional arg).
   - bash  → ~/.local/share/bash-completion/completions/strangetimer
   - fish  → ~/.config/fish/completions/strangetimer.fish
   - zsh   → ~/.zfunc/_strangetimer + prints the fpath/compinit rc line
     if ~/.zshrc doesn't reference it
   - powershell → prints the exact $PROFILE line (never edits profile)
   Idempotent; prints what it did.
2. README: Installation gains the one-liner (`strangetimer install-
   completions`); auto-start behaviour documented (incl.
   STRANGETIMER_AUTO_START=0); view resize behaviour noted.
3. DEVELOPMENT.md / SYSTEM_DESIGN.md: probe semantics, systemd-aware
   start, no-`--now` registration, logging design (§8.4 + new logging
   section), view TUI description (§9.3).
4. Unit tests for install-completions path selection with overridden
   HOME/SHELL.

## Build order

Prompt 26 → 27 (lifecycle before auto-start) → 28 (logging) → 29 (view)
→ 30 (completions + docs). Run `cargo test --workspace`, clippy, fmt
after each; verify manually: resize during `view timers`, `daemon
start/stop` x3, tab-completion in a fresh bash.
