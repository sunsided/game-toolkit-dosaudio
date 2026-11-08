//! Original channel-two phrase or caller's MT-32 MIDI file, using caller's ROMs.
use game_toolkit_dosaudio::{MidiMessage, Mt32Roms, prelude::*};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let directory = args
        .next()
        .ok_or("usage: mt32 <rom-directory> [out.wav] [input.mid]")?;
    let path = args.next().unwrap_or_else(|| "mt32.wav".into());
    let input = args.next();
    if args.next().is_some() {
        return Err("too many arguments".into());
    }
    let roms = Mt32Roms::from_rom_dir(directory)?;
    let song = if let Some(input) = input {
        Smf::load(&std::fs::read(input)?)?
    } else {
        let mut events = vec![];
        for (i, note) in [60, 64, 67, 72, 71, 67, 62, 60].into_iter().enumerate() {
            for (time, bytes) in [
                (i as u64 * 180_000, [0x91, note, 100]),
                (i as u64 * 180_000 + 150_000, [0x81, note, 0]),
            ] {
                events.push(TimedEvent {
                    at: Tick::from_micros(time),
                    event: MusicEvent::Midi(MidiMessage::new(&bytes)?),
                });
            }
        }
        PreparedMusic::new(MusicTarget::Midi, events, Tick::from_micros(1_800_000), [])?
    };
    let mut machine = OfflineMachine::new(SoundConfig::mt32(roms), 48_000)?;
    let _music = machine.play(song)?;
    std::fs::write(&path, machine.render_to_end()?.to_bytes()?)?;
    println!("Wrote {path}: user-supplied CM-32L firmware, 32 kHz native synthesis");
    Ok(())
}
