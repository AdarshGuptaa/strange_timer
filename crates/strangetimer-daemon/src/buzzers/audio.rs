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
    let bytes = match load_audio(path) {
        Some(b) => b,
        None => return,
    };

    thread::spawn(move || {
        if let Err(e) = play_wav_bytes(bytes) {
            warn!("audio playback failed: {e:#}");
        }
    });

    focus_media_window();
}

/// Replay the audio until `should_continue` turns false (user-interrupt
/// mode): the sound loops with a short gap between replays, stopping as
/// soon as the run's pending marker is cleared. Each replay happens on its
/// own thread so a slow audio device never blocks the daemon.
pub async fn fire_audio_until(path: Option<&Path>, should_continue: impl Fn() -> bool) {
    let bytes = match load_audio(path) {
        Some(b) => b,
        None => return,
    };

    loop {
        let bytes = bytes.clone();
        thread::spawn(move || {
            if let Err(e) = play_wav_bytes(bytes) {
                warn!("audio playback failed: {e:#}");
            }
        });
        // Give the (potentially long) playback room to finish before the
        // next repetition, but keep polling so the loop stops promptly on
        // acknowledgement.
        for _ in 0..20 {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            if !should_continue() {
                return;
            }
        }
    }
}

fn load_audio(path: Option<&Path>) -> Option<Vec<u8>> {
    match path {
        Some(p) => match std::fs::read(p) {
            Ok(bytes) => Some(bytes),
            Err(e) => {
                warn!("cannot read audio file {}: {e}", p.display());
                None
            }
        },
        None => Some(BUILTIN_CHIME.to_vec()),
    }
}

/// Best-effort duration of an audio source, decoded from its header
/// without opening an audio device. `None` for built-in defaults means
/// the embedded chime; the returned value is its decoded duration.
pub fn audio_duration(path: Option<&Path>) -> Option<chrono::Duration> {
    use rodio::Source;
    let bytes = load_audio(path)?;
    let decoder = rodio::Decoder::new(Cursor::new(bytes)).ok()?;
    let std_dur = decoder.total_duration()?;
    chrono::Duration::from_std(std_dur).ok()
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
