//! DSP control and host-serviced DMA share the port and audio timelines.
#![cfg(feature = "sb")]
use game_toolkit_dosaudio::{DmaDirection, Error, IrqLine, prelude::*};

const BASE: u16 = 0x220;
const DMA: DmaChannel = DmaChannel::new(1).unwrap();

fn machine() -> (DosAudio, AudioEngine) {
    DosAudio::split(SoundConfig::sb2().latency(Tick::ZERO)).unwrap()
}

fn command(audio: &mut DosAudio, bytes: &[u8]) {
    for &byte in bytes {
        audio.outb(BASE + 0x0c, byte);
    }
}

fn start(audio: &mut DosAudio, count: u16) {
    let [lo, hi] = (count - 1).to_le_bytes();
    command(audio, &[0xd1, 0x40, 131, 0x14, lo, hi]); // 8,000 Hz.
}

fn deliver(audio: &mut DosAudio, value: u8) -> Tick {
    let at = audio.next_dma_tick().unwrap();
    audio.try_dma_write_at(at, DMA, value).unwrap();
    at
}

#[test]
fn reset_version_echo_and_speaker_status_work_without_the_renderer() {
    let (mut audio, _) = machine();
    audio.outb(BASE + 6, 1);
    assert_eq!(audio.inb(BASE + 0x0c) & 0x80, 0x80);
    command(&mut audio, &[0xe1]); // Ignored while reset is held.
    audio.wait_us(3);
    audio.outb(BASE + 6, 0);
    assert_eq!(audio.inb(BASE + 0x0e) & 0x80, 0x80);
    assert_eq!(audio.inb(BASE + 0x0a), 0xaa);
    assert_eq!(audio.inb(BASE + 0x0e) & 0x80, 0);
    assert_eq!(audio.inb(BASE + 0x0a), 0xff);
    assert_eq!(audio.inb(BASE + 0x0c) & 0x80, 0);
    command(&mut audio, &[0xe1, 0xe0, 0x55, 0xd8, 0xd1, 0xd8]);
    for expected in [2, 1, 0xaa, 0, 0xff] {
        assert_eq!(audio.inb(BASE + 0x0a), expected);
    }
    // Too short a reset pulse produces no acknowledgement.
    audio.outb(BASE + 6, 1);
    audio.outb(BASE + 6, 0);
    assert_eq!(audio.inb(BASE + 0x0e) & 0x80, 0);
}

#[test]
fn single_cycle_dma_requests_exactly_count_plus_one_bytes_and_latches_irq() {
    let (mut audio, _) = machine();
    start(&mut audio, 2);
    let first = audio.next_dma_tick().unwrap();
    assert_eq!(first, Tick::from_micros(125));
    assert!(audio.dma_request(Tick::ZERO).is_none());
    assert!(audio.try_dma_write_at(Tick::ZERO, DMA, 0).is_err());
    let request = audio.dma_request(first).unwrap();
    assert_eq!(request.channel, DMA);
    assert_eq!(request.direction, DmaDirection::MemoryToDevice);
    assert!(
        audio
            .try_dma_write_at(first, DmaChannel::new(3).unwrap(), 0)
            .is_err()
    );
    deliver(&mut audio, 0);
    assert!(!audio.irq_pending(audio.now()));
    assert_eq!(deliver(&mut audio, 255), Tick::from_micros(250));
    assert!(audio.next_dma_tick().is_none());
    assert!(audio.dma_request(audio.now()).is_none());
    let irq = audio.irq_request(audio.now()).unwrap();
    assert_eq!(irq.line, IrqLine::new(5).unwrap());
    assert!(irq.asserted);
    assert!(audio.irq_pending(audio.now())); // Observing does not acknowledge.
    audio.inb(BASE + 0x0a);
    assert!(audio.irq_pending(audio.now()));
    audio.inb(BASE + 0x0e);
    assert!(!audio.irq_request(audio.now()).unwrap().asserted);
    assert!(audio.try_dma_write_at(audio.now(), DMA, 128).is_err());
}

#[test]
fn auto_init_reloads_block_count_and_exit_finishes_the_current_block() {
    let (mut audio, _) = machine();
    command(&mut audio, &[0x40, 131, 0x48, 1, 0, 0x1c]);
    for _ in 0..3 {
        deliver(&mut audio, 0);
        assert!(!audio.irq_pending(audio.now()));
        deliver(&mut audio, 255);
        assert!(audio.irq_pending(audio.now()));
        audio.inb(BASE + 0x0e);
    }
    deliver(&mut audio, 128);
    command(&mut audio, &[0xda]);
    deliver(&mut audio, 128);
    assert!(audio.irq_pending(audio.now()));
    assert!(audio.next_dma_tick().is_none());
}

#[test]
fn pause_freezes_the_remaining_period_and_speaker_mute_does_not_stop_dma() {
    let (mut audio, _) = machine();
    start(&mut audio, 2);
    audio.advance_to(Tick(500));
    command(&mut audio, &[0xd0]);
    assert!(audio.next_dma_tick().is_none());
    audio.advance_to(Tick(5_000));
    assert!(audio.dma_request(audio.now()).is_none());
    command(&mut audio, &[0xd4, 0xd3]);
    assert_eq!(
        audio.next_dma_tick(),
        Some(Tick::from_micros(125) + Tick(4_500))
    );
    deliver(&mut audio, 0);
    command(&mut audio, &[0xd8]);
    assert_eq!(audio.inb(BASE + 0x0a), 0);
    deliver(&mut audio, 255);
    assert!(audio.irq_pending(audio.now()));
}

#[test]
fn rational_sample_clock_has_no_cumulative_tick_rounding() {
    let (mut audio, mut engine) = machine();
    start(&mut audio, 10_000);
    for index in 1..=10_000 {
        assert_eq!(deliver(&mut audio, 128), Tick::from_micros(index * 125));
        if index % 100 == 0 {
            engine.render(&mut [[0.0; 2]; 600]);
        }
    }
    assert!(audio.irq_pending(audio.now()));
}

#[test]
fn late_dma_stalls_and_never_backdates_a_sample() {
    let (mut audio, _) = machine();
    start(&mut audio, 2);
    audio.advance_to(Tick::from_micros(1_000));
    assert!(audio.dma_request(audio.now()).is_some());
    audio.try_dma_write_at(audio.now(), DMA, 255).unwrap();
    assert_eq!(audio.next_dma_tick(), Some(Tick::from_micros(1_125)));
    assert_eq!(
        audio.try_dma_write_at(Tick::ZERO, DMA, 0),
        Err(Error::BackwardsTime)
    );
}

#[test]
fn dsp_reset_cancels_dma_irq_replies_and_partial_commands() {
    let (mut audio, mut engine) = machine();
    start(&mut audio, 10);
    command(&mut audio, &[0xf2, 0xe1, 0x14, 9]);
    audio.outb(BASE + 6, 1);
    assert!(!audio.irq_pending(audio.now()));
    assert!(audio.next_dma_tick().is_none());
    audio.wait_us(3);
    audio.outb(BASE + 6, 0);
    assert_eq!(audio.inb(BASE + 0x0a), 0xaa);
    command(&mut audio, &[0xe1]);
    assert_eq!(audio.inb(BASE + 0x0a), 2);
    assert_eq!(audio.inb(BASE + 0x0a), 1);
    let mut frames = [[1.0; 2]; 32];
    engine.render(&mut frames);
    assert_eq!(frames, [[0.0; 2]; 32]);
}

#[test]
fn queue_overflow_is_transactional_for_dac_parameters_and_dma_completion() {
    let (mut audio, mut engine) =
        DosAudio::split(SoundConfig::sb2().latency(Tick::ZERO).queue_capacity(1)).unwrap();
    command(&mut audio, &[0xd1, 0x10]);
    assert_eq!(
        audio.try_outb_at(Tick(1), BASE + 0x0c, 255),
        Err(Error::QueueFull)
    );
    assert_eq!(audio.now(), Tick::ZERO);
    engine.render(&mut [[0.0; 2]]);
    audio.try_outb_at(Tick(1), BASE + 0x0c, 255).unwrap();
    command(&mut audio, &[0x40, 131, 0x14, 0, 0]);
    let at = audio.next_dma_tick().unwrap();
    assert_eq!(audio.try_dma_write_at(at, DMA, 0), Err(Error::QueueFull));
    assert_eq!(audio.now(), Tick(1));
    assert_eq!(audio.next_dma_tick(), Some(at));
    assert!(!audio.irq_pending(at));
    engine.render(&mut [[0.0; 2]]);
    audio.try_dma_write_at(at, DMA, 0).unwrap();
    assert!(audio.irq_pending(at));
    assert!(audio.next_dma_tick().is_none());
}

#[test]
fn direct_dac_is_integrated_inside_samples_and_speaker_switches_gate_output() {
    let (mut audio, mut engine) = machine();
    command(&mut audio, &[0xd1, 0x10]);
    audio.try_outb_at(Tick(100), BASE + 0x0c, 255).unwrap();
    audio.try_outb_at(Tick(200), BASE + 0x0c, 0xd3).unwrap();
    let mut frames = [[0.0; 2]; 2];
    engine.render(&mut frames);
    let expected = (127.0 / 128.0) * 100.0 / (315_000_000.0 / (48_000.0 * 22.0));
    assert!((f64::from(frames[0][0]) - expected).abs() < 1e-7);
    assert_eq!(frames[0][0], frames[0][1]);
    assert_eq!(frames[1], [0.0; 2]);
}

fn scheduled_pcm(latency: Tick) -> (DosAudio, AudioEngine) {
    let (mut audio, engine) = DosAudio::split(SoundConfig::sb2().latency(latency)).unwrap();
    start(&mut audio, 64);
    for index in 0..64 {
        deliver(&mut audio, if index % 2 == 0 { 32 } else { 224 });
    }
    command(&mut audio, &[0xd3]);
    (audio, engine)
}

#[test]
fn dma_pcm_is_callback_independent_and_rendering_allocates_nothing() {
    let (_, mut whole) = scheduled_pcm(Tick::ZERO);
    let (_, mut chunked) = scheduled_pcm(Tick::ZERO);
    let mut reference = [[0.0; 2]; 512];
    let mut actual = reference;
    let measured = allocation_counter::measure(|| {
        whole.render(&mut reference);
        for frame in &mut actual {
            chunked.render(std::slice::from_mut(frame));
        }
    });
    assert_eq!(measured.count_total, 0);
    assert_eq!(reference, actual);
    assert!(actual.iter().any(|frame| frame[0].abs() > 0.5));
    assert_eq!(whole.stats().late_commands, 0);
}

#[test]
fn dma_pcm_uses_the_same_fixed_latency_as_port_commands() {
    let (_, mut engine) = scheduled_pcm(Tick::from_micros(10_000));
    let mut early = [[1.0; 2]; 480];
    engine.render(&mut early);
    assert_eq!(early, [[0.0; 2]; 480]);
    let mut later = [[0.0; 2]; 20];
    engine.render(&mut later);
    assert!(later.iter().any(|frame| frame[0].abs() > 0.5));
}

#[test]
fn offline_and_live_dma_output_match_at_partial_sample_boundaries() {
    let (_, mut engine) = scheduled_pcm(Tick::ZERO);
    let mut expected = [[0.0; 2]; 512];
    engine.render(&mut expected);
    let mut machine = OfflineMachine::new(SoundConfig::sb2(), 48_000).unwrap();
    for byte in [0xd1, 0x40, 131, 0x14, 63, 0] {
        machine.outb(BASE + 0x0c, byte).unwrap();
    }
    for index in 0..64 {
        machine
            .advance_to(machine.next_dma_tick().unwrap())
            .unwrap();
        machine
            .dma_write(DMA, if index % 2 == 0 { 32 } else { 224 })
            .unwrap();
    }
    assert!(machine.irq_request().unwrap().asserted);
    machine.outb(BASE + 0x0c, 0xd3).unwrap();
    let until = Tick((512u128 * u128::from(Tick::HZ_NUMERATOR)).div_ceil(48_000 * 22) as u64);
    machine.advance_to(until).unwrap();
    assert_eq!(machine.finish().frames(), expected);
}

#[test]
fn relocated_resources_expose_ports_irq_and_dma_through_the_bus() {
    let (mut audio, _) = DosAudio::split(SoundConfig::none().sound_blaster(SoundBlasterConfig {
        base: 0x240,
        irq: 7,
        dma: 3,
    }))
    .unwrap();
    assert!(audio.ports().iter().any(|range| range.contains(0x24c)));
    assert!(!audio.ports().iter().any(|range| range.contains(0x22c)));
    assert_eq!(audio.inb(0x22c), 0xff);
    audio.outb(0x24c, 0xf2);
    let mut device: Box<dyn IoDevice> = Box::new(audio);
    assert_eq!(device.irq_request(Tick::ZERO).unwrap().line.number(), 7);
    assert!(device.irq_pending(Tick::ZERO));
    for byte in [0x14, 0, 0] {
        device.outb(Tick::ZERO, 0x24c, byte);
    }
    assert_eq!(
        device
            .dma_request(Tick::from_micros(100))
            .unwrap()
            .channel
            .number(),
        3
    );
}

#[test]
fn invalid_sound_blaster_resources_are_rejected_before_construction() {
    for resources in [
        SoundBlasterConfig {
            base: 0x221,
            ..Default::default()
        },
        SoundBlasterConfig {
            base: 0xfff0,
            ..Default::default()
        },
        SoundBlasterConfig {
            irq: 16,
            ..Default::default()
        },
        SoundBlasterConfig {
            dma: 5,
            ..Default::default()
        },
        SoundBlasterConfig {
            dma: 2,
            ..Default::default()
        },
    ] {
        assert!(DosAudio::split(SoundConfig::none().sound_blaster(resources)).is_err());
    }
}

#[cfg(feature = "formats")]
#[test]
fn card_aliases_share_opl_latches_and_are_preserved_in_dro_capture() {
    let (mut audio, _) = machine();
    audio.outb(BASE + 8, 0x20);
    audio.start_capture().unwrap();
    audio.outb(0x389, 1);
    audio.outb(0x388, 0xb0);
    audio.outb(BASE + 9, 0x31);
    let capture = audio.finish_capture().unwrap();
    assert_eq!(capture.writes()[2].port, BASE + 9);
    let sequence = Dro::load(&capture.to_dro().unwrap()).unwrap();
    assert_eq!(sequence.events().len(), 2);
    assert_eq!(
        sequence.events()[0].event,
        MusicEvent::Opl {
            register: 0x20,
            value: 1
        }
    );
    assert_eq!(
        sequence.events()[1].event,
        MusicEvent::Opl {
            register: 0xb0,
            value: 0x31
        }
    );
    for byte in [2, 0xff] {
        audio.outb(BASE + 8 + u16::from(byte == 0xff), byte);
    }
    audio.outb(BASE + 8, 4);
    audio.outb(BASE + 9, 0x21);
    audio.wait_us(80);
    assert_eq!(audio.inb(0x388) & 0xe0, 0xc0);
    assert_eq!(audio.inb(BASE + 8) & 0xe0, 0xc0);
}
