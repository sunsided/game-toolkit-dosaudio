//! Render an original three-channel arpeggio and white-noise percussion.
use game_toolkit_dosaudio::{OfflineMachine, SoundConfig, Tick};
use std::time::Duration;

fn tone(
    machine: &mut OfflineMachine,
    channel: u8,
    hz: f64,
    attenuation: u8,
) -> game_toolkit_dosaudio::Result<()> {
    let crystal = Tick::HZ_NUMERATOR as f64 / Tick::HZ_DENOMINATOR as f64;
    let divisor = ((crystal / 4.0) / (32.0 * hz)).round().clamp(1.0, 1024.0) as u16 & 0x3ff;
    // Latch the low four divisor bits, then send the remaining six bits.
    machine.outb(0xc0, 0x80 | (channel << 5) | (divisor as u8 & 15))?;
    machine.outb(0xc0, (divisor >> 4) as u8)?;
    machine.outb(0xc0, 0x90 | (channel << 5) | attenuation)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let path = args.next().unwrap_or_else(|| "psg.wav".into());
    if args.next().is_some() {
        return Err("usage: psg [out.wav]".into());
    }
    let mut machine = OfflineMachine::new(SoundConfig::none().psg(true), 48_000)?;
    for (index, root) in [
        261.63, 293.66, 329.63, 392.0, 349.23, 329.63, 293.66, 261.63,
    ]
    .into_iter()
    .enumerate()
    {
        let origin = index as u64 * 120_000;
        machine.advance_to(Tick::from_micros(origin))?;
        tone(&mut machine, 0, root, 4)?;
        machine.outb(0xc0, 0xe4)?; // White noise, fastest fixed divider; resets its seed.
        machine.outb(0xc0, 0xf8)?;
        machine.advance_to(Tick::from_micros(origin + 12_000))?;
        machine.outb(0xc0, 0xff)?; // Short percussion burst.
        machine.advance_to(Tick::from_micros(origin + 40_000))?;
        tone(&mut machine, 1, root * 1.25, 7)?;
        machine.advance_to(Tick::from_micros(origin + 80_000))?;
        tone(&mut machine, 2, root * 1.5, 8)?;
    }
    machine.advance_to(Tick::from_micros(960_000))?;
    for channel in 0..4 {
        machine.outb(0xc0, 0x9f | (channel << 5))?;
    }
    machine.render_for(Duration::from_millis(20))?;
    std::fs::write(&path, machine.finish().to_bytes()?)?;
    println!("Wrote {path}: original three-tone PSG arpeggio with noise percussion");
    Ok(())
}
