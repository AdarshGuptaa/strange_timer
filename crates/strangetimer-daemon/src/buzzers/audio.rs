use std::io::Cursor;
use std::path::Path;
use std::thread;

use anyhow::Context;

use crate::platform::focus_media_window;

/// The built-in chime embedded into the daemon binary.
const BUILTIN_CHIME: &[u8] = include_bytes!("../../assets/chime.wav");

/// Play an audio file (or the built-in chime when `path` is `None`) in a
/// background thread so the caller returns immediately. After starting
/// playback, attempt to focus the media player window (platform-specific).
pub fn fire_audio(path: Option<&Path>) {
    let bytes = match path {
        Some(p) => match std::fs::read(p) {
            Ok(bytes) => bytes,
            Err(e) => {
                eprintln!("strangetimer-daemon: cannot read audio file {}: {e}", p.display());
                return;
            }
        },
        None => BUILTIN_CHIME.to_vec(),
    };

    thread::spawn(move || {
        if let Err(e) = play_wav_bytes(bytes) {
            eprintln!("strangetimer-daemon: audio playback failed: {e:#}");
        }
    });

    focus_media_window();
}

/// Play WAV bytes to the default output device, blocking until finished.
fn play_wav_bytes(bytes: Vec<u8>) -> anyhow::Result<()> {
    use rodio::{Decoder, OutputStream, Sink};

    let (_stream, handle) =
        OutputStream::try_default().context("no audio output device available")?;
    let source = Decoder::new(Cursor::new(bytes)).context("failed to decode WAV")?;
    let sink = Sink::try_new(&handle).context("failed to create audio sink")?;
    sink.append(source);
    sink.sleep_until_end();
    Ok(())
}
