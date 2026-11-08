//! Original patches through the public OPL helpers, then all five rhythm drums.
use game_toolkit_dosaudio::prelude::*;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let path = args.next().unwrap_or_else(|| "opl_patches.wav".into());
    if args.next().is_some() {
        return Err("usage: opl_patches [out.wav]".into());
    }
    // Capture a bounded helper trace on the control side, then use the shared
    // offline renderer. All phrase boundaries are whole milliseconds for DRO.
    let (mut ports, _engine) = DosAudio::split(SoundConfig::adlib().latency(Tick::ZERO))?;
    ports.start_capture()?;
    let bank = OplBank::starter();
    let mut now = 0;
    for program in [0, 24, 40, 64, 80, 81] {
        let patch = bank
            .get(OplBankKey {
                msb: 0,
                lsb: 0,
                program,
                percussion: false,
            })
            .unwrap()
            .instrument
            .patch;
        ports.opl()?.set_patch(0, &patch)?;
        for key in [60, 64, 67] {
            ports.opl()?.note_on(0, Note::from_midi(key)?, 112)?;
            now += 120_000;
            ports.try_advance_to(Tick::from_micros(now))?;
            ports.opl()?.note_off(0)?;
            now += 40_000;
            ports.try_advance_to(Tick::from_micros(now))?;
        }
    }
    for (channel, program, key) in [(6, 36, 36), (7, 38, 60), (8, 49, 72)] {
        let patch = bank
            .get(OplBankKey {
                msb: 0,
                lsb: 0,
                program,
                percussion: true,
            })
            .unwrap()
            .instrument
            .patch;
        ports.opl()?.set_patch(channel, &patch)?;
        ports.opl()?.note_on(channel, Note::from_midi(key)?, 127)?;
    }
    ports.opl()?.rhythm(true)?;
    for drum in [
        Drum::BassDrum,
        Drum::Snare,
        Drum::Tom,
        Drum::Cymbal,
        Drum::HiHat,
    ] {
        ports.opl()?.hit(drum)?;
        now += 100_000;
        ports.try_advance_to(Tick::from_micros(now))?;
        ports.opl()?.release(drum)?;
        now += 80_000;
        ports.try_advance_to(Tick::from_micros(now))?;
    }
    for channel in 6..=8 {
        ports.opl()?.note_off(channel)?;
    }
    ports.opl()?.rhythm(false)?;
    now += 250_000;
    ports.try_advance_to(Tick::from_micros(now))?;
    let sequence = Dro::load(&ports.finish_capture().unwrap().to_dro()?)?;
    let mut machine = OfflineMachine::new(SoundConfig::adlib(), 48_000)?;
    let _handle = machine.play(sequence)?;
    std::fs::write(&path, machine.render_to_end()?.to_bytes()?)?;
    println!("Wrote {path}: six original patches and five hardware rhythm drums");
    Ok(())
}
