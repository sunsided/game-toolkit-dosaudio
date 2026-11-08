//! Render an original MIDI phrase with a user-supplied SF2. No assets are bundled.
use game_toolkit_dosaudio::{
    GmSynth, MidiMessage, MusicEvent, MusicTarget, OfflineMachine, PreparedMusic, SoundConfig,
    Tick, TimedEvent,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let font_path = args.next().ok_or("usage: gm <font.sf2> [out.wav]")?;
    let path = args.next().unwrap_or_else(|| "gm.wav".into());
    if args.next().is_some() {
        return Err("usage: gm <font.sf2> [out.wav]".into());
    }
    let font = GmSynth::load_soundfont(&mut std::fs::File::open(font_path)?)?;
    let mut events = Vec::new();
    let mut event = |micros: u64, bytes: &[u8]| -> game_toolkit_dosaudio::Result<()> {
        events.push(TimedEvent {
            at: Tick::from_micros(micros),
            event: MusicEvent::Midi(MidiMessage::new(bytes)?),
        });
        Ok(())
    };
    event(0, &[0xc0, 0])?;
    event(0, &[0xb0, 7, 100])?;
    event(0, &[0xb0, 10, 64])?;
    for (i, note) in [60, 64, 67, 72, 71, 67, 62, 60].into_iter().enumerate() {
        event(i as u64 * 180_000, &[0x90, note, 90])?;
        event(i as u64 * 180_000 + 150_000, &[0x80, note, 0])?;
    }
    let song = PreparedMusic::new(MusicTarget::Midi, events, Tick::from_micros(1_800_000), [])?;
    let mut machine = OfflineMachine::new(SoundConfig::gm(font), 48_000)?;
    let _handle = machine.play(song)?;
    std::fs::write(&path, machine.render_to_end()?.to_bytes()?)?;
    println!("Wrote {path}: original eight-note General MIDI phrase");
    Ok(())
}
