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
pub fn data_dir() -> PathBuf {
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
    write_json_atomic(&data_dir().join(STATE_FILE), s)
}

/// Read JSON from `path`. If the file is absent, return `default()`.
/// Any other I/O or parse error is bubbled up with context.
fn read_json_or_default<T: serde::de::DeserializeOwned + serde::Serialize + ?Sized>(
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
        Err(e) => Err(anyhow::Error::new(e))
            .with_context(|| format!("failed to read {}", path.display())),
    }
}

/// Write `value` as pretty JSON to `path` atomically: serialise, write to
/// `<path>.tmp` in the same directory, then `rename(2)` into place.
fn write_json_atomic<T: serde::Serialize + ?Sized>(path: &Path, value: &T) -> Result<()> {
    let dir = path
        .parent()
        .with_context(|| format!("{} has no parent directory", path.display()))?;
    fs::create_dir_all(dir)
        .with_context(|| format!("failed to create directory {}", dir.display()))?;

    let tmp = path.with_extension(
        path.extension()
            .map(|e| format!("{}.tmp", e.to_string_lossy()))
            .unwrap_or_else(|| "tmp".to_string()),
    );

    let bytes = serde_json::to_vec_pretty(value)
        .context("failed to serialise value")?;
    fs::write(&tmp, &bytes)
        .with_context(|| format!("failed to write {}", tmp.display()))?;
    fs::rename(&tmp, path).with_context(|| {
        format!(
            "failed to rename {} -> {}",
            tmp.display(),
            path.display()
        )
    })?;
    Ok(())
}
