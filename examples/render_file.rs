//! Usage: render_file <input.dro|imf|vgm|mid> <output.wav> [IMF Hz|SF2 path]
use game_toolkit_dosaudio::prelude::*;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut arguments = std::env::args().skip(1);
    let input = arguments
        .next()
        .ok_or("expected input .dro, .imf, .vgm or .mid")?;
    let output = arguments.next().ok_or("expected output .wav")?;
    let parameter = arguments.next();
    if arguments.next().is_some() {
        return Err("usage: render_file <input> <output.wav> [IMF Hz|SF2 path]".into());
    }
    let bytes = std::fs::read(&input).map_err(|e| format!("{input}: {e}"))?;
    let sequence: PreparedMusic = if bytes.starts_with(b"DBRAWOPL") {
        Dro::load(&bytes)?.into()
    } else if bytes.starts_with(b"Vgm ") || bytes.starts_with(&[0x1f, 0x8b]) {
        Vgm::load(&bytes)?
    } else if bytes.starts_with(b"MThd") {
        Smf::load(&bytes)?
    } else {
        Imf::load(
            &bytes,
            parameter
                .as_deref()
                .ok_or("IMF requires an explicit tick rate, e.g. 560")?
                .parse()?,
        )?
        .into()
    };
    let config = match sequence.target() {
        MusicTarget::Opl(chip) | MusicTarget::OplPsg(chip) => match chip {
            OplChip::Opl2 => SoundConfig::adlib(),
            OplChip::Opl3 => SoundConfig::opl3(),
            OplChip::DualOpl2 => SoundConfig::dual_opl2(),
        },
        MusicTarget::Psg => SoundConfig::none(),
        MusicTarget::Midi => midi_config(parameter.as_deref())?,
        _ => return Err("unsupported file music target".into()),
    };
    let config = match sequence.target() {
        MusicTarget::Psg | MusicTarget::OplPsg(_) => config.psg(true),
        _ => config,
    };
    let mut machine = OfflineMachine::new(config, 48_000)?;
    let _music = machine.play(sequence)?;
    std::fs::write(output, machine.render_to_end()?.to_bytes()?)?;
    Ok(())
}

#[cfg(feature = "gm")]
fn midi_config(path: Option<&str>) -> Result<SoundConfig, Box<dyn std::error::Error>> {
    let path = path.ok_or("MIDI requires a user-supplied SF2 path as the third argument")?;
    let font = game_toolkit_dosaudio::GmSynth::load_soundfont(&mut std::fs::File::open(path)?)?;
    Ok(SoundConfig::gm(font))
}

#[cfg(not(feature = "gm"))]
fn midi_config(_path: Option<&str>) -> Result<SoundConfig, Box<dyn std::error::Error>> {
    Err("MIDI rendering requires --features gm and a user-supplied SF2 path".into())
}
