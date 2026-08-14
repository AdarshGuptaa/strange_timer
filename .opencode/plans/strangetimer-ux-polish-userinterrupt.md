# StrangeTimer — UX Polish & User-Interrupt Plan (Prompts 31–35)

Decisions confirmed with the user:
- Interrupt ack: **attached CLI prompt** (`run -u` stays attached, Enter
  resumes; `strangetimer resume` remains a fallback).
- Focus target: **capture the terminal window at `run -u` time**.
- Tab candidates: subcommands/flags + timer names, buzzer library names,
  file paths — with help text attached, colored output.
- Colors: **colored tables, help, views, prompts** in a minimalistic,
  muted "Cosmic/Pop!_OS-like" theme (soft teal accent, nothing bright).
- `-u/--userinterrupt` is a per-run runtime flag (NOT a cargo feature
  flag — stated assumption).

---

## Prompt 33 (first) — Central styling module + muted Cosmic theme

1. New `crates/strangetimer/src/style.rs`:
   - `color_enabled()` → true unless `NO_COLOR` is set (any value) or
     stdout is not a TTY; `STRANGETIMER_COLOR=always` forces on (tests).
   - Palette (crossterm, muted): accent/headers `DarkCyan` + bold;
     running `Green`; paused `Yellow`; scheduled `Cyan`; completed/
     dim `DarkGrey`; table borders `DarkGrey`; prompt accent `Green`;
     warnings `Yellow`; errors `Red`; built-in tag `Magenta` (dim).
   - Small helpers: `header(s)`, `status(s, TimerStatus)`, `dim(s)`,
     `accent(s)`, `warn(s)`, `err(s)` — each a no-op passthrough when
     color is disabled.
2. Apply to all CLI outputs: command confirmations (colored names),
   `view buzzers` table, `examples` listing, `daemon status/start/stop`,
   the auto-start notice, and error/hint messages.
3. Colored `--help`: clap `Styles::styled()` with the same muted palette
   (headers DarkCyan bold, literals Cyan, placeholders DarkGrey, valid
   Green, invalid Yellow, error Red) on the root `Cli::command()`.
4. Tests: with `STRANGETIMER_COLOR=always` outputs contain the expected
   ANSI codes; with `NO_COLOR=1` no ANSI codes escape into stdout.

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
