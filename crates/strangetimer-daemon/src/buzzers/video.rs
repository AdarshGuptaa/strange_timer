use std::path::{Path, PathBuf};

/// Open a video file in the OS default player (or the built-in clip when
/// `path` is `None`). The built-in clip is resolved next to the installed
/// binary (`<exe_dir>/assets/default.mp4`), falling back to the source-tree
/// asset for development builds.
pub fn fire_video(path: Option<&Path>) {
    let target = match path {
        Some(p) => p.to_path_buf(),
        None => builtin_video_path(),
    };
    crate::platform::open_target(&target.to_string_lossy());
}

/// Resolve the path to the built-in `default.mp4` clip. Packaging installs
/// `assets/` beside the binaries, so an installed daemon finds it next to
/// itself; development builds fall back to the crate's asset directory.
pub fn builtin_video_path() -> PathBuf {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let installed = dir.join("assets").join("default.mp4");
            if installed.is_file() {
                return installed;
            }
        }
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("assets")
        .join("default.mp4")
}

/// True when the built-in clip exists and starts with a valid MP4 `ftyp`
/// header — used by tests to catch packaging regressions.
#[cfg_attr(not(test), allow(dead_code))]
pub fn builtin_video_valid() -> bool {
    let path = builtin_video_path();
    let Ok(bytes) = std::fs::read(&path) else {
        return false;
    };
    // MP4 files start with box size (4 bytes) followed by "ftyp".
    bytes.len() >= 12 && &bytes[4..8] == b"ftyp"
}

/// Best-effort duration of an MP4 file, parsed from its `moov/mvhd`
/// metadata — no player, no ffprobe, bounded work.
pub fn video_duration(path: &Path) -> Option<chrono::Duration> {
    let bytes = std::fs::read(path).ok()?;
    parse_mvhd(&bytes)
}

fn parse_mvhd(bytes: &[u8]) -> Option<chrono::Duration> {
    let mut off = 0usize;
    while off + 8 <= bytes.len() {
        let size = u32::from_be_bytes(bytes[off..off + 4].try_into().ok()?) as usize;
        let typ = &bytes[off + 4..off + 8];
        if typ == b"moov" {
            return parse_mvhd_inside(&bytes[off + 8..off + size.min(bytes.len())]);
        }
        if size < 8 || size == 0 {
            break;
        }
        off += size;
    }
    None
}

fn parse_mvhd_inside(bytes: &[u8]) -> Option<chrono::Duration> {
    let mut off = 0usize;
    while off + 8 <= bytes.len() {
        let size = u32::from_be_bytes(bytes[off..off + 4].try_into().ok()?) as usize;
        let typ = &bytes[off + 4..off + 8];
        if typ == b"mvhd" {
            let inner = &bytes[off + 8..off + size.min(bytes.len())];
            if inner.len() < 36 {
                return None;
            }
            let version = inner[0];
            if version == 0 {
                let timescale = u32::from_be_bytes(inner[12..16].try_into().ok()?) as i64;
                let duration = u32::from_be_bytes(inner[16..20].try_into().ok()?) as i64;
                if timescale == 0 {
                    return None;
                }
                return Some(chrono::Duration::milliseconds(duration * 1000 / timescale));
            } else {
                let timescale = u32::from_be_bytes(inner[24..28].try_into().ok()?) as i64;
                let duration = i64::from_be_bytes(inner[28..36].try_into().ok()?);
                if timescale == 0 {
                    return None;
                }
                return Some(chrono::Duration::milliseconds(duration * 1000 / timescale));
            }
        }
        if size < 8 || size == 0 {
            break;
        }
        off += size;
    }
    None
}

/// Format a duration as seconds with two decimals ("0.80s", "12.40s").
pub fn fmt_duration_secs(d: chrono::Duration) -> String {
    format!("{:.2}s", d.num_milliseconds() as f64 / 1000.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_clip_exists_and_is_mp4() {
        assert!(
            builtin_video_path().is_file(),
            "assets/default.mp4 missing — run from the source tree or install with assets/"
        );
        assert!(
            builtin_video_valid(),
            "assets/default.mp4 is not a valid MP4"
        );
    }
}
