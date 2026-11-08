//! Replace and control original FM music after moving the engine into a stream.
mod support;

use game_toolkit_dosaudio::prelude::*;
use std::time::{Duration, Instant};

fn wait_until(started: Instant, at: Duration) {
    if let Some(remaining) = at.checked_sub(started.elapsed()) {
        std::thread::sleep(remaining);
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::args_os().len() != 1 {
        return Err("usage: live_music (no arguments)".into());
    }
    let mut host = game_toolkit_audio::Audio::new().map_err(|error| error.to_string())?;
    let (_ports, mut engine) = DosAudio::split(SoundConfig::adlib())?;
    let rate = engine.sample_rate();
    let mut music = engine.music_controller(16)?;
    let _stream = host.play_stereo_stream(engine, rate)?;
    // Engine ownership now belongs to the callback. All subsequent playback
    // changes go through the bounded controller and atomic MusicHandle controls.
    let started = Instant::now();
    let first = music.play(support::tune(), PlayOptions::looped().volume(0.75))?;
    println!("Playing the original phrase; fading down, up, then replacing it.");

    wait_until(started, Duration::from_millis(350));
    first.fade_volume(0.12, Duration::from_millis(250))?;
    wait_until(started, Duration::from_millis(850));
    first.fade_volume(0.75, Duration::from_millis(250))?;

    wait_until(started, Duration::from_millis(1_500));
    let second = music.play(support::tune(), PlayOptions::default().volume(0.75))?;
    // Also submit a live register event. Waveform selection stays on sine for
    // this patch, so enabling alternate waveforms does not interrupt the phrase.
    music.send_event(MusicEvent::Opl {
        register: 0x01,
        value: 0x20,
    })?;
    wait_until(started, Duration::from_millis(3_000));
    music.reclaim(); // Release any retired sequence on this control thread.
    // Retain both handles through the demonstration: dropping the last clone
    // requests a stop. The first handle is already finished after replacement.
    drop((first, second));
    Ok(())
}
