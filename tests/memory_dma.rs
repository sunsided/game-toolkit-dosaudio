//! Attached conventional-memory DMA preserves the public DSP protocol and transactions.
#![cfg(feature = "sb")]

use game_toolkit_dosaudio::{
    AudioEngine, ConventionalMemory, DmaChannel, DosAudio, Error, IoDevice, OfflineMachine,
    SoundConfig, Tick,
};
use std::sync::Arc;

const BYTE: DmaChannel = DmaChannel::new(1).unwrap();
const WORD: DmaChannel = DmaChannel::new(5).unwrap();

fn program_byte(mut write: impl FnMut(u16, u8), page: u8, address: u16, units: u16, auto: bool) {
    write(0x0a, 5);
    write(0x0c, 0);
    for value in address.to_le_bytes() {
        write(2, value);
    }
    for value in (units - 1).to_le_bytes() {
        write(3, value);
    }
    write(0x83, page);
    write(0x0b, if auto { 0x59 } else { 0x49 });
    write(0x0a, 1);
}

fn start_byte(mut write: impl FnMut(u16, u8), units: u16, auto: bool) {
    for byte in [0xd1, 0x40, 131] {
        write(0x22c, byte);
    }
    let [low, high] = (units - 1).to_le_bytes();
    for byte in [if auto { 0x48 } else { 0x14 }, low, high] {
        write(0x22c, byte);
    }
    if auto {
        write(0x22c, 0x1c);
    }
}

fn current(audio: &mut DosAudio) -> (u16, u16) {
    audio.outb(0x0c, 0);
    let address = u16::from_le_bytes([audio.inb(2), audio.inb(2)]);
    let count = u16::from_le_bytes([audio.inb(3), audio.inb(3)]);
    (address, count)
}

fn machine(capacity: usize) -> (DosAudio, AudioEngine, Arc<ConventionalMemory>) {
    let (mut audio, engine) = DosAudio::split(
        SoundConfig::sb2()
            .latency(Tick::ZERO)
            .queue_capacity(capacity),
    )
    .unwrap();
    let memory = Arc::new(ConventionalMemory::new());
    audio.attach_dma(Arc::clone(&memory)).unwrap();
    (audio, engine, memory)
}

#[test]
fn attached_memory_page_wrap_matches_manually_supplied_offline_dma() {
    let memory = Arc::new(ConventionalMemory::new());
    memory.write_byte(0x2ffff, 0).unwrap();
    memory.write(0x20000, &[255, 192, 128]).unwrap();
    let mut attached = OfflineMachine::new(SoundConfig::sb2(), 48_000).unwrap();
    let mut manual = OfflineMachine::new(SoundConfig::sb2(), 48_000).unwrap();
    attached.attach_dma(memory).unwrap();
    program_byte(
        |port, value| attached.outb(port, value).unwrap(),
        2,
        0xffff,
        4,
        false,
    );
    start_byte(|port, value| attached.outb(port, value).unwrap(), 4, false);
    start_byte(|port, value| manual.outb(port, value).unwrap(), 4, false);
    let boundary = attached.next_dma_block_tick().unwrap();
    assert_eq!(boundary, Tick::from_micros(500));
    attached.advance_to(boundary).unwrap();
    for value in [0, 255, 192, 128] {
        manual.advance_to(manual.next_dma_tick().unwrap()).unwrap();
        manual.dma_write(BYTE, value).unwrap();
    }
    let event = attached.poll_dma_completion().unwrap();
    assert_eq!(event.at, boundary);
    assert_eq!(event.channel, BYTE);
    assert_eq!(event.block, 1);
    assert!(event.irq.asserted);
    assert!(attached.irq_request().unwrap().asserted);
    assert!(attached.poll_dma_completion().is_none());
    assert_eq!(attached.inb(8) & 2, 2);
    assert_eq!(attached.inb(8) & 2, 0);
    for machine in [&mut attached, &mut manual] {
        machine.inb(0x22e);
        machine.outb(0x22c, 0xd3).unwrap();
        machine.advance_to(Tick::from_micros(1_000)).unwrap();
    }
    assert_eq!(attached.finish().frames(), manual.finish().frames());
}

#[test]
fn sb16_attached_word_dma_matches_manual_signed_stereo_units() {
    let memory = Arc::new(ConventionalMemory::new());
    let samples = [i16::MIN, i16::MAX, 8192, -8192];
    let bytes: Vec<_> = samples.into_iter().flat_map(i16::to_le_bytes).collect();
    memory.write(0x2000, &bytes).unwrap();
    let mut attached = OfflineMachine::new(SoundConfig::sb16(), 48_000).unwrap();
    let mut manual = OfflineMachine::new(SoundConfig::sb16(), 48_000).unwrap();
    attached.attach_dma(memory).unwrap();
    for (port, value) in [
        (0xd4, 5),
        (0xd8, 0),
        (0xc4, 0),
        (0xc4, 0x10),
        (0xc6, 3),
        (0xc6, 0),
        (0x8b, 0),
        (0xd6, 0x49),
        (0xd4, 1),
    ] {
        attached.outb(port, value).unwrap();
    }
    for machine in [&mut attached, &mut manual] {
        for register in 0x30..=0x33 {
            machine.outb(0x224, register).unwrap();
            machine.outb(0x225, 0xf8).unwrap();
        }
        for byte in [0x41, 0x1f, 0x40, 0xb0, 0x30, 3, 0] {
            machine.outb(0x22c, byte).unwrap();
        }
    }
    let boundary = attached.next_dma_block_tick().unwrap();
    attached.advance_to(boundary).unwrap();
    for value in samples {
        manual.advance_to(manual.next_dma_tick().unwrap()).unwrap();
        manual.dma_write16(WORD, value as u16).unwrap();
    }
    let event = attached.poll_dma_completion().unwrap();
    assert_eq!(event.at, boundary);
    assert_eq!(event.channel, WORD);
    assert!(event.irq.asserted);
    attached.inb(0x22e);
    assert!(attached.irq_request().unwrap().asserted); // Byte ack cannot clear word IRQ.
    for machine in [&mut attached, &mut manual] {
        machine.inb(0x22f);
        machine.outb(0x22c, 0xd3).unwrap();
        machine.advance_to(Tick::from_micros(1_000)).unwrap();
    }
    assert_eq!(attached.finish().frames(), manual.finish().frames());
}

#[test]
fn auto_init_completions_allow_refills_at_each_block_boundary() {
    let (mut attached, mut engine, memory) = machine(64);
    let (mut manual, mut reference) =
        DosAudio::split(SoundConfig::sb2().latency(Tick::ZERO)).unwrap();
    memory.write(0, &[0, 255, 128, 128]).unwrap();
    program_byte(|port, value| attached.outb(port, value), 0, 0, 4, true);
    start_byte(|port, value| attached.outb(port, value), 2, true);
    start_byte(|port, value| manual.outb(port, value), 2, true);
    for (index, pair) in [[0, 255], [192, 64], [32, 224]].into_iter().enumerate() {
        if index == 1 {
            memory.write(2, &pair).unwrap();
        }
        if index == 2 {
            memory.write(0, &pair).unwrap();
        }
        let boundary = attached.next_dma_block_tick().unwrap();
        attached.try_advance_to(boundary).unwrap();
        for value in pair {
            let at = manual.next_dma_tick().unwrap();
            manual.try_dma_write_at(at, BYTE, value).unwrap();
        }
        let completion = attached.poll_dma_completion().unwrap();
        assert_eq!(completion.block, index as u64 + 1);
        assert_eq!(completion.at, Tick::from_micros((index as u64 + 1) * 250));
        assert!(completion.irq.asserted);
        assert!(attached.irq_pending(attached.now())); // Polling leaves the level latched.
        attached.inb(0x22e);
        manual.inb(0x22e);
    }
    assert_eq!(current(&mut attached), (2, 1));
    let mut actual = [[0.0; 2]; 36];
    let mut expected = actual;
    engine.render(&mut actual);
    reference.render(&mut expected);
    assert_eq!(actual, expected);
}

#[test]
fn masking_and_pause_stall_without_catchup_bursts_on_resume() {
    let (mut audio, _engine, memory) = machine(64);
    memory.write(0, &[0, 255, 64, 192]).unwrap();
    program_byte(|port, value| audio.outb(port, value), 0, 0, 4, false);
    start_byte(|port, value| audio.outb(port, value), 4, false);
    audio.try_advance_to(Tick::from_micros(125)).unwrap();
    audio.outb(0x0a, 5);
    audio.try_advance_to(Tick::from_micros(1_000)).unwrap();
    assert_eq!(current(&mut audio), (1, 2));
    assert!(audio.poll_dma_completion().is_none());
    audio.outb(0x0a, 1);
    audio.try_advance_to(audio.now()).unwrap();
    assert_eq!(current(&mut audio), (2, 1));
    assert_eq!(audio.next_dma_block_tick(), Some(Tick::from_micros(1_250)));
    audio.outb(0x22c, 0xd0);
    audio.try_advance_to(Tick::from_micros(2_000)).unwrap();
    assert_eq!(current(&mut audio), (2, 1));
    assert!(audio.next_dma_block_tick().is_none());
    audio.outb(0x22c, 0xd4);
    assert_eq!(audio.next_dma_block_tick(), Some(Tick::from_micros(2_250)));
    audio.try_advance_to(Tick::from_micros(2_250)).unwrap();
    assert_eq!(
        audio.poll_dma_completion().unwrap().at,
        Tick::from_micros(2_250)
    );
}

#[test]
fn command_queue_overflow_preserves_clock_dma_count_irq_and_completion() {
    let (mut audio, mut engine, memory) = machine(2);
    memory.write(0, &[0, 255, 0, 255]).unwrap();
    program_byte(|port, value| audio.outb(port, value), 0, 0, 4, false);
    start_byte(|port, value| audio.outb(port, value), 4, false);
    let end = Tick::from_micros(375);
    assert_eq!(audio.try_advance_to(end), Err(Error::QueueFull));
    assert_eq!(audio.try_outb_at(end, 0x22c, 0xd3), Err(Error::QueueFull));
    assert_eq!(audio.try_inb_at(end, 0x22e), Err(Error::QueueFull));
    assert_eq!(audio.now(), Tick::ZERO);
    assert_eq!(current(&mut audio), (0, 3));
    assert!(!audio.irq_pending(audio.now()));
    assert!(audio.poll_dma_completion().is_none());
    engine.render(&mut [[0.0; 2]]);
    audio.try_advance_to(Tick::from_micros(250)).unwrap();
    assert_eq!(current(&mut audio), (2, 1));
    engine.render(&mut [[0.0; 2]; 32]);
    audio.try_advance_to(Tick::from_micros(500)).unwrap();
    let event = audio.poll_dma_completion().unwrap();
    assert_eq!(event.block, 1);
    assert_eq!(event.at, Tick::from_micros(500));
}

#[test]
fn completion_backpressure_rolls_back_until_events_are_polled() {
    let (mut audio, mut engine, memory) = machine(1);
    memory.write_byte(0, 192).unwrap();
    program_byte(|port, value| audio.outb(port, value), 0, 0, 1, true);
    start_byte(|port, value| audio.outb(port, value), 1, true);
    engine.render(&mut [[0.0; 2]]); // Drain the initial speaker-enable command.
    audio.try_advance_to(Tick::from_micros(125)).unwrap();
    engine.render(&mut [[0.0; 2]; 16]);
    assert_eq!(
        audio.try_advance_to(Tick::from_micros(250)),
        Err(Error::IrqQueueFull)
    );
    assert_eq!(audio.now(), Tick::from_micros(125));
    assert_eq!(current(&mut audio), (0, 0));
    let first = audio.poll_dma_completion().unwrap();
    assert_eq!(first.block, 1);
    assert!(audio.irq_pending(audio.now()));
    audio.try_advance_to(Tick::from_micros(250)).unwrap();
    let second = audio.poll_dma_completion().unwrap();
    assert_eq!(second.block, 2);
    assert_eq!(second.at, Tick::from_micros(250));
}

#[test]
fn invalid_memory_page_rolls_back_and_can_be_fixed_before_retry() {
    let (mut audio, mut engine, memory) = machine(8);
    memory.write(0, &[0, 255]).unwrap();
    program_byte(|port, value| audio.outb(port, value), 0x10, 0, 2, false);
    start_byte(|port, value| audio.outb(port, value), 2, false);
    engine.render(&mut [[0.0; 2]]);
    assert!(matches!(
        audio.try_advance_to(Tick::from_micros(250)),
        Err(Error::Config(_))
    ));
    assert_eq!(audio.now(), Tick::ZERO);
    assert_eq!(current(&mut audio), (0, 1));
    assert!(!audio.irq_pending(audio.now()));
    assert!(audio.poll_dma_completion().is_none());
    audio.outb(0x83, 0);
    audio.try_advance_to(Tick::from_micros(250)).unwrap();
    assert_eq!(audio.poll_dma_completion().unwrap().block, 1);
}

#[test]
fn finite_sample_buffers_bypass_the_attached_masked_controller() {
    let (mut audio, _engine, memory) = machine(16);
    memory.write_byte(0, 255).unwrap();
    // Controller remains masked and unprogrammed throughout buffer playback.
    let handle = audio.play_pcm([0_u8, 255].as_slice(), 8_000).unwrap();
    audio.try_advance_to(Tick::from_micros(375)).unwrap();
    assert!(handle.is_finished());
    assert_eq!(handle.samples_played(), 2);
    assert!(audio.poll_dma_completion().is_none());
    assert_eq!(current(&mut audio), (0, 0));
}

#[test]
fn attaching_memory_requires_a_card_and_exposes_controller_ports_once() {
    let memory = Arc::new(ConventionalMemory::new());
    let (mut absent, _) = DosAudio::split(SoundConfig::none()).unwrap();
    assert!(absent.attach_dma(Arc::clone(&memory)).is_err());
    let (mut audio, _) = DosAudio::split(SoundConfig::sb2()).unwrap();
    assert!(!audio.ports().iter().any(|range| range.contains(0x83)));
    audio.attach_dma(Arc::clone(&memory)).unwrap();
    assert!(audio.ports().iter().any(|range| range.contains(0x83)));
    assert!(audio.attach_dma(memory).is_err());
}

#[test]
fn sb16_words_over_byte_dma_service_both_halves_at_the_same_tick() {
    let memory = Arc::new(ConventionalMemory::new());
    memory.write(0, &[0, 0x80, 0xff, 0x7f]).unwrap();
    let mut attached = OfflineMachine::new(SoundConfig::sb16(), 48_000).unwrap();
    let mut manual = OfflineMachine::new(SoundConfig::sb16(), 48_000).unwrap();
    attached.attach_dma(memory).unwrap();
    program_byte(
        |port, value| attached.outb(port, value).unwrap(),
        0,
        0,
        4,
        false,
    );
    for machine in [&mut attached, &mut manual] {
        // Disable high DMA selection: the DSP assembles each word from DMA1 bytes.
        machine.outb(0x224, 0x81).unwrap();
        machine.outb(0x225, 2).unwrap();
        for byte in [0x41, 0x1f, 0x40, 0xb0, 0x10, 1, 0] {
            machine.outb(0x22c, byte).unwrap();
        }
    }
    attached.advance_to(Tick::from_micros(250)).unwrap();
    for value in [0, 0x80, 0xff, 0x7f] {
        manual.advance_to(manual.next_dma_tick().unwrap()).unwrap();
        manual.dma_write(BYTE, value).unwrap();
    }
    let event = attached.poll_dma_completion().unwrap();
    assert_eq!(event.at, Tick::from_micros(250));
    assert_eq!(event.channel, BYTE);
    attached.inb(0x22e);
    assert!(attached.irq_request().unwrap().asserted);
    for machine in [&mut attached, &mut manual] {
        machine.inb(0x22f);
        machine.outb(0x22c, 0xd3).unwrap();
        machine.advance_to(Tick::from_micros(500)).unwrap();
    }
    assert_eq!(attached.finish().frames(), manual.finish().frames());
}

#[test]
fn memory_dma_commit_and_completion_poll_allocate_and_free_nothing() {
    let (mut audio, mut engine, memory) = machine(8);
    memory.write(0, &[0, 255]).unwrap();
    program_byte(|port, value| audio.outb(port, value), 0, 0, 2, false);
    start_byte(|port, value| audio.outb(port, value), 2, false);
    engine.render(&mut [[0.0; 2]]);
    let measured = allocation_counter::measure(|| {
        audio.try_advance_to(Tick::from_micros(250)).unwrap();
        assert!(audio.poll_dma_completion().is_some());
    });
    assert_eq!(measured.count_total, 0);
    assert_eq!(measured.count_current, 0);
}

#[test]
fn offline_masked_dma_advances_and_bad_memory_reports_an_error() {
    let memory = Arc::new(ConventionalMemory::new());
    let mut machine = OfflineMachine::new(SoundConfig::sb2(), 48_000).unwrap();
    machine.attach_dma(memory).unwrap();
    program_byte(
        |port, value| machine.outb(port, value).unwrap(),
        0x10,
        0,
        2,
        false,
    );
    machine.outb(0x0a, 5).unwrap();
    start_byte(|port, value| machine.outb(port, value).unwrap(), 2, false);
    machine.advance_to(Tick::from_micros(500)).unwrap();
    assert_eq!(machine.now(), Tick::from_micros(500));
    assert!(machine.poll_dma_completion().is_none());
    machine.outb(0x0a, 1).unwrap();
    assert!(matches!(
        machine.advance_to(Tick::from_micros(1_000)),
        Err(Error::Config(_))
    ));
    assert_eq!(machine.now(), Tick::from_micros(500));
}
