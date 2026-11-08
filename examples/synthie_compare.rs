//! Original song as native synthie, OPL, speaker, or a seamless timeline switch.
use game_toolkit_dosaudio::prelude::*;
use std::sync::Arc;
use synthie::{
    params::{ChannelNo, SynthParams},
    pattern::{Song, Tempo},
};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let mode = args.next().unwrap_or_else(|| "switch".into());
    let path = args.next().unwrap_or_else(|| format!("synthie_{mode}.wav"));
    if args.next().is_some() {
        return Err("usage: synthie_compare [native|opl|speaker|switch] [out.wav]".into());
    }
    let song = Song::new(Tempo::bpm(140.0, 2))
        .patch(SynthParams::default())
        .lane(
            ChannelNo(0),
            "C4@0 E4 G4 C5 B4 G4 D4 . | A4 G4 E4 D4 C4 . . -",
        )?
        .lane(ChannelNo(1), "C3 . G3 . C3 . G3 . | A2 . E3 . C3 . . -")?
        .lane(ChannelNo(3), "k h s h k h s h | k h s h k h s h")?
        .one_shot();
    let soundtrack = Arc::new(Soundtrack::synthie(&song, OplChip::Opl2, 48_000)?);
    let duration = soundtrack
        .arrangement(MusicDevice::Native)
        .unwrap()
        .duration();
    let device = match mode.as_str() {
        "native" | "switch" => MusicDevice::Native,
        "opl" => MusicDevice::Opl2,
        "speaker" => MusicDevice::Speaker,
        _ => return Err("mode must be native, opl, speaker or switch".into()),
    };
    let mut machine = OfflineMachine::new(SoundConfig::adlib().speaker(true).native(true), 48_000)?;
    let music = machine.play_soundtrack(soundtrack, device, PlayOptions::default().volume(0.8))?;
    if mode == "switch" {
        machine.advance_to(Tick(duration.0 / 3))?;
        music.switch_device(MusicDevice::Opl2)?;
        machine.advance_to(Tick(duration.0 * 2 / 3))?;
        music.switch_device(MusicDevice::Speaker)?;
    }
    std::fs::write(&path, machine.render_to_end()?.to_bytes()?)?;
    println!("Wrote {path}: original synthie song, {mode}; shared duration and controls");
    Ok(())
}
