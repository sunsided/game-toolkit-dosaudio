//! Usage: cargo run --example render_file -- input.dro output.wav [IMF tick rate]
use game_toolkit_dosaudio::prelude::*;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut arguments = std::env::args().skip(1);
    let input = arguments.next().ok_or("expected input .dro or .imf")?;
    let output = arguments.next().ok_or("expected output .wav")?;
    let bytes = std::fs::read(&input)?;
    let sequence = if bytes.starts_with(b"DBRAWOPL") {
        Dro::load(&bytes)?
    } else {
        Imf::load(
            &bytes,
            arguments
                .next()
                .ok_or("IMF requires an explicit tick rate, e.g. 560")?
                .parse()?,
        )?
    };
    let config = match sequence.chip() {
        OplChip::Opl2 => SoundConfig::adlib(),
        OplChip::Opl3 => SoundConfig::opl3(),
        OplChip::DualOpl2 => SoundConfig::dual_opl2(),
    };
    let mut machine = OfflineMachine::new(config, 48_000)?;
    let _music = machine.play(sequence)?;
    std::fs::write(output, machine.render_to_end()?.to_bytes()?)?;
    Ok(())
}
