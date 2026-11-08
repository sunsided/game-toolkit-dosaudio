//! Render an original eight-note AdLib phrase to WAV without an audio device.
mod support;
use game_toolkit_dosaudio::prelude::*;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "adlib.wav".into());
    let mut machine = OfflineMachine::new(SoundConfig::adlib(), 48_000)?;
    let _music = machine.play(support::tune())?;
    let wav = machine.render_to_end()?;
    std::fs::write(&path, wav.to_bytes()?)?;
    println!("Wrote {} frames to {path}", wav.frames().len());
    Ok(())
}
