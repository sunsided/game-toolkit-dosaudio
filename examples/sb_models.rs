//! Render an original tone through one selected Sound Blaster DSP generation.
//! Usage: sb_models <sb1|sb2|pro|pro2|sb16> [out.wav]
use game_toolkit_dosaudio::{SbModel, prelude::*};
use std::{f64::consts::TAU, time::Duration};

fn dsp(machine: &mut OfflineMachine, bytes: &[u8]) -> Result<(), Box<dyn std::error::Error>> {
    for &byte in bytes {
        machine.outb(0x22c, byte)?;
    }
    Ok(())
}

fn mixer(
    machine: &mut OfflineMachine,
    register: u8,
    value: u8,
) -> Result<(), Box<dyn std::error::Error>> {
    machine.outb(0x224, register)?;
    machine.outb(0x225, value)?;
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let Some(name) = args.next() else {
        return Err("usage: sb_models <sb1|sb2|pro|pro2|sb16> [out.wav]".into());
    };
    if name == "--help" || name == "-h" {
        println!("Usage: sb_models <sb1|sb2|pro|pro2|sb16> [out.wav]");
        println!("Writes about 620 ms; stereo models use 440 Hz left and 660 Hz right.");
        return Ok(());
    }
    let model = match name.as_str() {
        "sb1" => SbModel::Sb1,
        "sb2" => SbModel::Sb2,
        "pro" => SbModel::SbPro,
        "pro2" => SbModel::SbPro2,
        "sb16" => SbModel::Sb16,
        _ => return Err("model must be sb1, sb2, pro, pro2 or sb16".into()),
    };
    let path = args.next().unwrap_or_else(|| format!("sb_{name}.wav"));
    if args.next().is_some() {
        return Err("too many arguments".into());
    }
    let pro = matches!(model, SbModel::SbPro | SbModel::SbPro2);
    let word = model == SbModel::Sb16;
    let channels = if pro || word { 2 } else { 1 };
    // TC165 gives 10989.011 Hz mono. Pro TC211 doubles the aggregate rate:
    // one byte every 45 us, alternating channels => 11111.111 Hz per channel.
    let time_constant = if pro { 211 } else { 165 };
    let source_rate = if word {
        22_050.0
    } else {
        1_000_000.0 / (256.0 - f64::from(time_constant)) / channels as f64
    };
    let frames = (source_rate * 0.6_f64).round() as usize;
    let units = frames * channels;
    let count = u16::try_from(units - 1)?; // Scalar bytes or words, not stereo frames.
    let [count_low, count_high] = count.to_le_bytes();

    let mut machine = OfflineMachine::new(SoundConfig::none().sb_model(model), 48_000)?;
    machine.outb(0x226, 1)?;
    machine.render_for(Duration::from_micros(3))?;
    machine.outb(0x226, 0)?;
    assert_eq!(machine.inb(0x22a), 0xaa, "DSP reset handshake");
    dsp(&mut machine, &[0xe1])?;
    assert_eq!(
        (machine.inb(0x22a), machine.inb(0x22a)),
        model.dsp_version()
    );
    dsp(&mut machine, &[0xd1])?;

    if pro {
        mixer(&mut machine, 0x22, 0xff)?; // Master and voice at maximum.
        mixer(&mut machine, 0x04, 0xff)?;
        mixer(&mut machine, 0x0e, 0x22)?; // Stereo enabled, output filter bypassed.
        // Creative's stereo setup primes the DAC with one silent DMA byte.
        dsp(&mut machine, &[0x14, 0, 0])?;
        machine.advance_to(machine.next_dma_tick().expect("stereo priming request"))?;
        machine.dma_write(DmaChannel::new(1).unwrap(), 128)?;
        assert!(machine.irq_request().expect("Sound Blaster IRQ").asserted);
        machine.inb(0x22e);
        dsp(
            &mut machine,
            &[0x40, time_constant, 0x48, count_low, count_high, 0x91],
        )?;
    } else if word {
        for register in [0x30, 0x31, 0x32, 0x33] {
            mixer(&mut machine, register, 0xf8)?;
        }
        // 41h takes Hz per channel, big-endian. B0h transfers signed 16-bit
        // stereo with mode30h; the following little-endian count counts words.
        let [rate_high, rate_low] = 22_050_u16.to_be_bytes();
        dsp(
            &mut machine,
            &[0x41, rate_high, rate_low, 0xb0, 0x30, count_low, count_high],
        )?;
    } else {
        dsp(
            &mut machine,
            &[0x40, time_constant, 0x14, count_low, count_high],
        )?;
    }

    let channel = DmaChannel::new(if word { 5 } else { 1 }).unwrap();
    for unit in 0..units {
        let frame = unit / channels;
        let frequency = if unit % channels == 0 { 440.0 } else { 660.0 };
        let fade = (frame.min(frames - 1 - frame) as f64 / (source_rate * 0.02)).min(1.0);
        let amplitude = fade * (TAU * frequency * frame as f64 / source_rate).sin();
        let at = machine
            .next_dma_tick()
            .expect("DSP requested the programmed block");
        machine.advance_to(at)?;
        if word {
            let sample = (22_000.0 * amplitude).round() as i16;
            machine.dma_write16(channel, sample as u16)?;
        } else {
            machine.dma_write(channel, (128.0 + 96.0 * amplitude).round() as u8)?;
        }
    }
    assert!(
        machine.next_dma_tick().is_none(),
        "single-cycle block completed"
    );
    assert!(machine.irq_request().expect("Sound Blaster IRQ").asserted);
    machine.inb(if word { 0x22f } else { 0x22e });
    assert!(!machine.irq_request().expect("acknowledged IRQ").asserted);
    dsp(&mut machine, &[0xd3])?;
    if pro {
        mixer(&mut machine, 0x0e, 0)?;
    }
    machine.render_for(Duration::from_millis(20))?;
    std::fs::write(&path, machine.finish().to_bytes()?)?;
    println!(
        "Wrote {path}: {model:?}, {}-bit, {channels} channel(s), {source_rate:.3} Hz",
        if word { 16 } else { 8 }
    );
    Ok(())
}
