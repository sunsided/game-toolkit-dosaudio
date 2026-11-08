//! Original MIDI phrase, or a caller's SMF and optional instrument bank, on OPL.
use game_toolkit_dosaudio::{MidiMessage, prelude::*};
use std::sync::Arc;

fn original() -> game_toolkit_dosaudio::Result<PreparedMusic> {
    let mut events = Vec::new();
    let mut add = |micros, bytes: &[u8]| -> game_toolkit_dosaudio::Result<()> {
        events.push(TimedEvent {
            at: Tick::from_micros(micros),
            event: MusicEvent::Midi(MidiMessage::new(bytes)?),
        });
        Ok(())
    };
    for (phrase, program) in [0, 24, 40, 64, 80, 81].into_iter().enumerate() {
        let base = phrase as u64 * 640_000;
        add(base, &[0xc0, program])?;
        add(base, &[0xb0, 7, 127])?;
        add(base, &[0xb9, 7, 127])?;
        add(base, &[0xb0, 10, if phrase % 2 == 0 { 16 } else { 112 }])?;
        for (step, note) in [60, 64, 67, 72].into_iter().enumerate() {
            let t = base + step as u64 * 160_000;
            add(t, &[0x90, note, 110])?;
            add(t, &[0x99, if step % 2 == 0 { 36 } else { 42 }, 100])?;
            add(t + 70_000, &[0x89, if step % 2 == 0 { 36 } else { 42 }, 0])?;
            add(t + 130_000, &[0x80, note, 0])?;
        }
    }
    PreparedMusic::new(MusicTarget::Midi, events, Tick::from_micros(3_900_000), [])
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let model = args.next().unwrap_or_else(|| "opl2".into());
    let path = args.next().unwrap_or_else(|| "opl_midi.wav".into());
    let input = args.next();
    let bank_path = args.next();
    if args.next().is_some() {
        return Err(
            "usage: opl_midi [opl2|dual|opl3] [out.wav] [input.mid] [bank.op2|wopl|bnk|sbi]".into(),
        );
    }
    let (chip, config) = match model.as_str() {
        "opl2" => (OplChip::Opl2, SoundConfig::adlib()),
        "dual" => (OplChip::DualOpl2, SoundConfig::dual_opl2()),
        "opl3" => (OplChip::Opl3, SoundConfig::opl3()),
        _ => return Err("chip must be opl2, dual or opl3".into()),
    };
    let bank = if let Some(bank_path) = bank_path {
        let data = std::fs::read(bank_path)?;
        if data.starts_with(b"#OPL_II#") {
            OplBank::from_op2(&data)?
        } else if data.starts_with(b"WOPL3-BANK\0") {
            OplBank::from_wopl(&data)?
        } else if data.starts_with(b"SBI\x1a") {
            OplBank::from_sbi(&data)?
        } else {
            OplBank::from_bnk(&data)?
        }
    } else {
        OplBank::starter()
    };
    let midi = if let Some(input) = input {
        Smf::load(&std::fs::read(input)?)?
    } else {
        original()?
    };
    let driver = OplMidiDriver::new(chip, Arc::new(bank));
    let mut machine = OfflineMachine::new(config, 48_000)?;
    let _music = machine.play(driver.prepare(&midi)?)?;
    std::fs::write(&path, machine.render_to_end()?.to_bytes()?)?;
    println!("Wrote {path} using {chip:?}; MIDI compiled to the installed FM core");
    Ok(())
}
