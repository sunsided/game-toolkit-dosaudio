//! Render a PCM buffer through the Sound Blaster DSP without manually feeding DMA.
mod support;
use game_toolkit_dosaudio::prelude::*;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "pcm_buffer.wav".into());
    let mut machine = OfflineMachine::new(SoundConfig::sb2(), 48_000)?;
    let sound = machine.play_pcm(support::pcm(), 8_000)?;
    let wav = machine.render_to_end()?;
    assert!(sound.is_finished());
    std::fs::write(&path, wav.to_bytes()?)?;
    println!(
        "Wrote {} PCM bytes at {} Hz to {path}",
        sound.samples_played(),
        sound.sample_rate()
    );
    Ok(())
}
