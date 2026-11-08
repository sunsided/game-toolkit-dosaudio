//! Play the original demo phrase through game-toolkit-audio.
mod support;
use game_toolkit_dosaudio::prelude::*;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut host = game_toolkit_audio::Audio::new().map_err(|error| error.to_string())?;
    let (_ports, mut engine) = DosAudio::split(SoundConfig::adlib())?;
    let rate = engine.sample_rate();
    let _music = engine.play(support::tune(), PlayOptions::default())?;
    let _stream = host.play_stereo_stream(engine, rate)?;
    std::thread::sleep(std::time::Duration::from_millis(1_600));
    Ok(())
}
