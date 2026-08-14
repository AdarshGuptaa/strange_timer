# Buzzer Examples

Every StrangeTimer buzzer action type, with ready-to-paste commands. A
buzzer is a reusable alarm that **chains actions** — they fire in the order
given, in sequence. All examples below are for the `strangetimer` CLI; the
daemon must be running (`strangetimer daemon start`).

> Tip: `strangetimer examples` prints this list in your terminal and
> `strangetimer examples --install` creates the file-free examples for you.

## The examples

| Buzzer | Demonstrates | Command |
|---|---|---|
| `exampleAudio` | Built-in chime sound | `strangetimer create buzzer exampleAudio --audio` |
| `exampleAudioFile` | Custom audio file | `strangetimer create buzzer exampleAudioFile --audio ~/Music/alert.wav` |
| `exampleVideo` | Built-in default video | `strangetimer create buzzer exampleVideo --video` |
| `exampleVideoUrl` | Video streamed from the internet (default player/browser) | `strangetimer create buzzer exampleVideoUrl --url https://storage.googleapis.com/gtv-videos-bucket/sample/ForBiggerBlazes.mp4` |
| `exampleUrl` | Open a URL in the browser | `strangetimer create buzzer exampleUrl --url https://github.com/AdarshGuptaa/strange_timer` |
| `exampleApplication` | Launch an application | `strangetimer create buzzer exampleApplication --application /usr/bin/gnome-calculator` |
| `exampleBash` | Run a shell script | `strangetimer create buzzer exampleBash --bash ~/notify.sh` |
| `exampleChain` | Chained actions (audio + URL) | `strangetimer create buzzer exampleChain --audio --url https://github.com/AdarshGuptaa/strange_timer` |
| `exampleLlm` | Local LLM announcement (Ollama) | `strangetimer create buzzer exampleLlm --llm llama3 "Announce that the timer finished and suggest a 5 minute break."` |
| (see below) | Close one application | `strangetimer create buzzer quitBrowser --close-app firefox` |
| (see below) | Focus a window | `strangetimer create buzzer focusChat --focus-window Slack` |

## When to use each action type

- **`--audio`** — the classic alarm. Omit the path for the built-in chime,
  or pass a `.wav`/`.mp3` file. Plays on a background thread so the daemon
  never blocks.
- **`--video`** — opens the video in your default player. Good for longer
  attention-grabbing alerts, e.g. a "break is over" clip. The built-in
  `default_video` clip ships offline with the install (`assets/default.mp4`,
  resolved next to the daemon binary); `exampleVideoUrl` shows the remote
  alternative via `--url`.
- **`--url`** — opens a page in the default browser. Useful for meeting
  links, dashboards, or a pomodoro wrap-up page.
- **`--application`** — launches a program. Ideal for "time to run my
  daily report" style buzzers. Use an absolute path.
- **`--bash`** — runs a shell script. Anything you can script, a buzzer can
  do: desktop notifications (`notify-send`), SSH commands, writing a log
  line. The script path is executed with `sh -c` (Unix) / `cmd /C`
  (Windows).
- **`--close-app`** — closes a *specific* application by process name
  (e.g. `firefox`, `chrome`). **Destructive**: it requires the
  `strangetimer confirm-destructive` opt-in first, once per daemon session.
- **`--focus-window`** — brings a window to the foreground by title
  substring or application name (Linux: `wmctrl`/`xdotool`, macOS:
  AppleScript, Windows: PowerShell `AppActivate`). Non-destructive.
- **`--llm`** — asks a local [Ollama](https://ollama.com) model to produce
  the alarm (speech, a summary, a pep talk). If Ollama is unreachable, the
  buzzer falls back to the built-in chime rather than staying silent.

## Chaining

Pass several flags to one buzzer; the actions run in command-line order:

```sh
strangetimer create buzzer dayEnd \
  --audio \
  --url https://github.com/AdarshGuptaa/strange_timer \
  --focus-window "Notes"
```

When `dayEnd` fires, the chime plays, the project page opens, and Notes is
brought to the front — in that order.

## A worked timer

A 25-minute focus block with a chime at the end and a video at 30 minutes:

```sh
strangetimer create buzzer focusDone --audio
strangetimer create buzzer breakOver --video

strangetimer create timer focusBlock 25min focusDone 30min breakOver
strangetimer run focusBlock -n 4     # four rounds
strangetimer view timers             # watch the progress bars
```

The offset grammar (`25min`, `30min`, `45min`, `1h`, `1D`, `1W`, …) is
documented in the README.

## Destroying examples

Examples are ordinary (non-built-in) buzzers — delete them like any other:

```sh
strangetimer delete buzzer exampleUrl
```

Built-in buzzers (`default_audio`, `default_video`, `close_windows`) are
seeded on first run and cannot be deleted.
