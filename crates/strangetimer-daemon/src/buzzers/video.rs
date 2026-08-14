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
