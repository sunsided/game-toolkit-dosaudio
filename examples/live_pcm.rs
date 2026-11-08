//! Play an original PCM chirp while the game thread advances the hardware clock.
mod support;
use game_toolkit_dosaudio::prelude::*;
use std::time::{Duration, Instant};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut host = game_toolkit_audio::Audio::new().map_err(|error| error.to_string())?;
    let (mut ports, engine) = DosAudio::split(SoundConfig::sb2())?;
    let sound = ports.play_pcm(support::pcm(), 8_000)?;
    let rate = engine.sample_rate();
    let _stream = host.play_stereo_stream(engine, rate)?;
    let started = Instant::now();
    while !sound.is_finished() {
        ports.try_advance_to(
            Tick::checked_from_duration(started.elapsed()).ok_or("demo clock overflow")?,
        )?;
        std::thread::sleep(Duration::from_millis(2));
    }
    // Let the callback consume the final sample and its fixed command latency.
    std::thread::sleep(Duration::from_millis(50));
    Ok(())
}
