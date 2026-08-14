use std::path::Path;

/// Open a video file in the OS default player (or the built-in clip when
/// `path` is `None`). The clip is embedded by *path*, not by bytes: the
/// binary ships alongside the repo's `assets/` directory in development.
pub fn fire_video(path: Option<&Path>) {
    let target = match path {
        Some(p) => p.to_path_buf(),
        None => builtin_video_path(),
    };
    if let Err(e) = open::that(&target) {
        warn!("failed to open video {:?}: {e}", target.display());
    }
}

/// Resolve the path to the built-in `default.mp4` clip.
fn builtin_video_path() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("assets")
        .join("default.mp4")
}
