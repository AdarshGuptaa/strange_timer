use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::model::{Buzzer, DaemonState, Timer};

const TIMERS_FILE: &str = "timers.json";
const BUZZERS_FILE: &str = "buzzers.json";
const STATE_FILE: &str = "state.json";

/// Returns the OS-appropriate StrangeTimer data directory, creating it on
/// first call. Paths match the Persistence Layout table in CLAUDE.md:
///
/// | OS      | Path                                               |
/// |---------|----------------------------------------------------|
/// | Linux   | `~/.local/share/strangetimer/`                     |
/// | macOS   | `~/Library/Application Support/strangetimer/`     |
/// | Windows | `%APPDATA%\strangetimer\`                          |
///
/// The `STRANGETIMER_DATA_DIR` environment variable overrides the location
/// (used by the test-suite to isolate runs into a temp directory).
pub fn data_dir() -> PathBuf {
    if let Some(override_dir) = std::env::var_os("STRANGETIMER_DATA_DIR") {
        let dir = PathBuf::from(override_dir);
        let _ = fs::create_dir_all(&dir);
        return dir;
    }
    let dir = dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("strangetimer");
    // Idempotent: succeeds if the directory already exists.
    let _ = fs::create_dir_all(&dir);
    dir
}

pub fn load_timers() -> Result<Vec<Timer>> {
    read_json_or_default(&data_dir().join(TIMERS_FILE), Vec::new())
}

pub fn save_timers(v: &[Timer]) -> Result<()> {
    write_json_atomic(&data_dir().join(TIMERS_FILE), v)
}

pub fn load_buzzers() -> Result<Vec<Buzzer>> {
    read_json_or_default(&data_dir().join(BUZZERS_FILE), Vec::new())
}

pub fn save_buzzers(v: &[Buzzer]) -> Result<()> {
    write_json_atomic(&data_dir().join(BUZZERS_FILE), v)
}

pub fn load_state() -> Result<DaemonState> {
    read_json_or_default(&data_dir().join(STATE_FILE), DaemonState::default())
}

pub fn save_state(s: &DaemonState) -> Result<()> {
    let mut stamped = s.clone();
    // Keep a record of when the state was last written so restart recovery
    // can reason about daemon downtime.
    stamped.last_saved_at = Some(chrono::Local::now());
    write_json_atomic(&data_dir().join(STATE_FILE), &stamped)
}

/// Read JSON from `path`. If the file is absent, return `default()`.
/// Any other I/O or parse error is bubbled up with context.
fn read_json_or_default<T: serde::de::DeserializeOwned + serde::Serialize>(
    path: &Path,
    default: T,
) -> Result<T> {
    match fs::read(path) {
        Ok(bytes) => serde_json::from_slice::<T>(&bytes)
            .with_context(|| format!("failed to parse {}", path.display())),
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            // Write the default out so the on-disk shape matches what callers
            // will subsequently see — keeps the data dir tidy and makes the
            // first save a no-op-rename rather than a brand-new file.
            write_json_atomic(path, &default)?;
            Ok(default)
        }
        Err(e) => {
            Err(anyhow::Error::new(e)).with_context(|| format!("failed to read {}", path.display()))
        }
    }
}

/// Monotonic counter used to give every atomic-write temp file a unique name
/// within the process. Concurrent writers (daemon tasks, parallel tests) must
/// never share a temp path, or one writer can rename another's temp file away
/// mid-flight.
static TMP_SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Write `value` as pretty JSON to `path` atomically: serialise, write to a
/// uniquely-named `<path>.<unique>.tmp` in the same directory, then
/// `rename(2)` into place.
fn write_json_atomic<T: serde::Serialize + ?Sized>(path: &Path, value: &T) -> Result<()> {
    let dir = path
        .parent()
        .with_context(|| format!("{} has no parent directory", path.display()))?;
    fs::create_dir_all(dir)
        .with_context(|| format!("failed to create directory {}", dir.display()))?;

    let seq = TMP_SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let file_name = path
        .file_name()
        .map(|f| f.to_string_lossy().into_owned())
        .unwrap_or_else(|| "data.json".to_string());
    let tmp = dir.join(format!("{file_name}.{}.{}.tmp", std::process::id(), seq));

    let bytes = serde_json::to_vec_pretty(value).context("failed to serialise value")?;
    fs::write(&tmp, &bytes).with_context(|| format!("failed to write {}", tmp.display()))?;
    fs::rename(&tmp, path)
        .with_context(|| format!("failed to rename {} -> {}", tmp.display(), path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Buzzer, BuzzerAction, Timer, TimerRun, TimerStatus};
    use std::sync::Once;

    /// Point `data_dir()` at a fresh, process-unique temp directory exactly
    /// once so parallel tests share a consistent location.
    fn init_test_env() {
        static ONCE: Once = Once::new();
        ONCE.call_once(|| {
            let dir =
                std::env::temp_dir().join(format!("strangetimer-core-test-{}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            std::env::set_var("STRANGETIMER_DATA_DIR", &dir);
        });
    }

    fn sample_timer() -> Timer {
        Timer {
            name: "workAndFun".to_string(),
            buzzers: vec![
                crate::model::BuzzerRef {
                    offset: chrono::Duration::minutes(45),
                    buzzer_name: "default_audio".to_string(),
                },
                crate::model::BuzzerRef {
                    offset: chrono::Duration::minutes(15),
                    buzzer_name: "default_audio".to_string(),
                },
            ],
            created_at: chrono::Local::now(),
        }
    }

    #[test]
    fn data_dir_honours_env_override() {
        init_test_env();
        let dir = data_dir();
        assert!(dir.starts_with(std::env::temp_dir()));
        assert!(dir.exists());
    }

    #[test]
    fn timers_roundtrip() {
        init_test_env();
        let timers = vec![sample_timer()];
        save_timers(&timers).unwrap();
        assert_eq!(load_timers().unwrap(), timers);
    }

    #[test]
    fn missing_timers_file_defaults_to_empty() {
        init_test_env();
        let path = data_dir().join("timers.json");
        let _ = fs::remove_file(&path);
        assert!(load_timers().unwrap().is_empty());
    }

    #[test]
    fn buzzers_roundtrip() {
        init_test_env();
        let buzzers = vec![
            Buzzer {
                name: "default_audio".to_string(),
                actions: vec![BuzzerAction::DefaultAudio],
                builtin: true,
            },
            Buzzer {
                name: "paymentAlert".to_string(),
                actions: vec![BuzzerAction::Url("https://bank.example.com".to_string())],
                builtin: false,
            },
        ];
        save_buzzers(&buzzers).unwrap();
        assert_eq!(load_buzzers().unwrap(), buzzers);
    }

    #[test]
    fn state_roundtrip_stamps_last_saved_at() {
        init_test_env();
        let state = DaemonState {
            runs: vec![TimerRun {
                timer_name: "t".to_string(),
                started_at: chrono::Local::now(),
                repetitions: crate::model::RepeatMode::Count(3),
                current_rep: 0,
                schedule_time: None,
                status: TimerStatus::Running,
                paused_at: None,
                elapsed_before_pause: chrono::Duration::zero(),
                fired_indices: vec![],
                user_interrupt: false,
                interrupt_focus: None,
            }],
            registered: true,
            last_saved_at: None,
            interrupt_pending: None,
            pending_interrupts: vec![],
            pending_fires: vec![],
            session_env: Default::default(),
            close_windows_confirmed: false,
        };
        save_state(&state).unwrap();
        let loaded = load_state().unwrap();
        assert_eq!(loaded.runs.len(), 1);
        assert_eq!(loaded.runs[0].timer_name, "t");
        assert!(loaded.registered);
        // save_state must stamp the save time automatically (Prompt 20).
        assert!(loaded.last_saved_at.is_some());
    }

    #[test]
    fn writes_are_atomic_no_tmp_leftovers() {
        init_test_env();
        save_timers(&[sample_timer()]).unwrap();
        let dir = data_dir();
        // Sibling tests may be mid-write (parallel); poll until the dir
        // settles, then assert no temp files linger.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        let leftovers = loop {
            let left: Vec<_> = fs::read_dir(&dir)
                .unwrap()
                .filter_map(|e| e.ok())
                .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
                .collect();
            if left.is_empty() || std::time::Instant::now() > deadline {
                break left;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        };
        assert!(
            leftovers.is_empty(),
            "temporary files must be renamed away: {leftovers:?}"
        );
        // And the saved file parses.
        assert_eq!(load_timers().unwrap().len(), 1);
    }
}
