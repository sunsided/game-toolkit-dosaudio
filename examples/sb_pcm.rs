//! Render an original 8-bit tone through the Sound Blaster 2.0 DSP and host DMA.
use game_toolkit_dosaudio::prelude::*;
use std::{f64::consts::TAU, time::Duration};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "sb_pcm.wav".into());
    let mut machine = OfflineMachine::new(SoundConfig::sb2(), 48_000)?;
    machine.outb(0x226, 1)?;
    machine.render_for(Duration::from_micros(3))?;
    machine.outb(0x226, 0)?;
    assert_eq!(machine.inb(0x22a), 0xaa);
    // 1,000,000 / (256 - 131) = 8,000 unsigned samples per second.
    for value in [0xd1, 0x40, 131, 0x14, 0x3f, 0x06] {
        machine.outb(0x22c, value)?;
    }
    let channel = DmaChannel::new(1).unwrap();
    for index in 0..1_600 {
        let at = machine.next_dma_tick().expect("DSP requests another byte");
        machine.advance_to(at)?;
        let envelope = (std::f64::consts::PI * f64::from(index) / 1_599.0)
            .sin()
            .powi(2);
        let sample = (128.0 + 100.0 * envelope * (TAU * 440.0 * f64::from(index) / 8_000.0).sin())
            .round() as u8;
        machine.dma_write(channel, sample)?;
    }
    assert!(machine.irq_request().unwrap().asserted);
    machine.inb(0x22e); // Acknowledge the completed DSP block.
    machine.outb(0x22c, 0xd3)?;
    machine.render_for(Duration::from_millis(10))?;
    std::fs::write(&path, machine.finish().to_bytes()?)?;
    println!("Wrote {path}");
    Ok(())
}
