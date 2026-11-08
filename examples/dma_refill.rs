//! Mix four voices into a memory-backed, auto-init Sound Blaster DMA ring.
use game_toolkit_dosaudio::{
    ConventionalMemory, DmaChannel, OfflineMachine, SfxMixer, SoundConfig, Tick,
};
use std::{f64::consts::TAU, sync::Arc, time::Duration};

fn chirp(frequency: f64) -> Arc<[u8]> {
    (0..6_400)
        .map(|index| {
            let time = f64::from(index) / 8_000.0;
            let envelope = (f64::from(index.min(6_399 - index)) / 160.0).min(1.0);
            let phase = TAU * (frequency * time + 220.0 * time * time);
            (128.0 + 96.0 * envelope * phase.sin()).round() as u8
        })
        .collect()
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let path = args.next().unwrap_or_else(|| "dma_refill.wav".into());
    if args.next().is_some() {
        return Err("usage: dma_refill [out.wav]".into());
    }
    let memory = Arc::new(ConventionalMemory::new());
    let mut machine = OfflineMachine::new(SoundConfig::sb2(), 48_000)?;
    machine.attach_dma(Arc::clone(&memory))?;
    let mut mixer = SfxMixer::new(4, 8_000)?;
    let waves = [chirp(220.0), chirp(330.0), chirp(440.0), chirp(550.0)];
    let mut handles = Vec::new();
    for wave in &waves {
        let handle = mixer.trigger(Arc::clone(wave), 8_000)?;
        handle.set_volume(0.25);
        handles.push(handle);
    }
    let mut initial = [128; 512];
    mixer.render(&mut initial);
    memory.write(0x2000, &initial)?;

    // Byte DMA1 reads a 512-byte ring at 0000:2000. The controller reloads
    // after 512 units; the DSP interrupts after each 256-byte half instead.
    for (port, value) in [
        (0x0a, 5),
        (0x0c, 0),
        (2, 0),
        (2, 0x20),
        (3, 0xff),
        (3, 1),
        (0x83, 0),
        (0x0b, 0x59),
        (0x0a, 1),
    ] {
        machine.outb(port, value)?;
    }
    for byte in [0xd1, 0x40, 131, 0x48, 0xff, 0, 0x1c] {
        machine.outb(0x22c, byte)?;
    }

    let mut refill = [128; 256];
    for block in 1..=40_u64 {
        let boundary = machine
            .next_dma_block_tick()
            .expect("active auto-init block");
        machine.advance_to(boundary)?; // Attached controller supplies every due byte.
        let completion = machine
            .poll_dma_completion()
            .expect("one completed DSP half");
        assert_eq!(completion.at, boundary);
        assert_eq!(completion.at, Tick::from_micros(block * 32_000));
        assert_eq!(completion.block, block);
        assert_eq!(completion.channel, DmaChannel::new(1).unwrap());
        assert_eq!(completion.irq.line.number(), 5);
        assert!(completion.irq.asserted);
        assert!(machine.poll_dma_completion().is_none());
        machine.inb(0x22e); // Polling the completion does not acknowledge the IRQ.
        assert!(!machine.irq_request().expect("Sound Blaster IRQ").asserted);

        if block % 4 == 0 {
            let wave = &waves[(block as usize / 4) % waves.len()];
            let handle = mixer.trigger(Arc::clone(wave), 8_000)?;
            handle.set_volume(0.25);
            handles.push(handle); // Retain handles; full capacity steals by age.
        }
        mixer.render(&mut refill);
        let consumed_half = ((block - 1) % 2) as u32 * 256;
        memory.write(0x2000 + consumed_half, &refill)?;
    }
    machine.outb(0x22c, 0xd0)?; // Stop DRQ before advancing through the final tail.
    machine.outb(0x22c, 0xd3)?;
    machine.render_for(Duration::from_millis(20))?;
    std::fs::write(&path, machine.finish().to_bytes()?)?;
    println!(
        "Wrote {path}: 40 IRQ refills, {} triggered voices, four mixed at once",
        handles.len()
    );
    Ok(())
}
