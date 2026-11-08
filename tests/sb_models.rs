//! Sound Blaster model detection, formats and mixer routing at the public bus.
#![cfg(feature = "sb")]
use game_toolkit_dosaudio::prelude::*;

#[test]
fn each_model_reports_its_dsp_and_fm_hardware() {
    for (config, version, status, banked) in [
        (SoundConfig::sb1(), (1, 5), 6, false),
        (SoundConfig::sb2(), (2, 1), 6, false),
        (SoundConfig::sb_pro(), (3, 0), 6, true),
        (SoundConfig::sb_pro2(), (3, 2), 0, true),
        (SoundConfig::sb16(), (4, 5), 0, true),
    ] {
        let (mut ports, _) = DosAudio::split(config).unwrap();
        ports.outb(0x22c, 0xe1);
        assert_eq!((ports.inb(0x22a), ports.inb(0x22a)), version);
        assert_eq!(ports.inb(0x388), status);
        assert_eq!(
            ports.ports().iter().any(|range| range.contains(0x38b)),
            banked
        );
    }
}

#[test]
fn blaster_parsing_validates_types_resources_and_non_ascii_without_panicking() {
    let (mut ports, _) =
        DosAudio::split(SoundConfig::from_blaster("a240 i10 d3 h7 t6").unwrap()).unwrap();
    ports.outb(0x24c, 0xe1);
    assert_eq!(ports.inb(0x24a), 4);
    assert_eq!(ports.irq_request(Tick::ZERO).unwrap().line.number(), 10);
    for value in [
        "",
        "A220 I5 D1",
        "A220 I5 D1 T5",
        "A220 I5 D1 T6 H4",
        "A220 I5 D1 T3 H5",
        "A220 I5 I7 D1 T3",
        "A221 I5 D1 T3",
        "A220 I10 D1 T3",
        "A220 I5 D1 T6 M330",
        "é220 I5 D1 T6",
        "A220 I5 D999 T6",
    ] {
        assert!(SoundConfig::from_blaster(value).is_err(), "{value}");
    }
}

fn dsp(ports: &mut DosAudio, bytes: &[u8]) {
    for &byte in bytes {
        ports.outb(0x22c, byte);
    }
}
fn mixer(ports: &mut DosAudio, register: u8, value: u8) {
    ports.outb(0x224, register);
    ports.outb(0x225, value);
}
fn mixer_read(ports: &mut DosAudio, register: u8) -> u8 {
    ports.outb(0x224, register);
    ports.inb(0x225)
}
fn maximum_volume(ports: &mut DosAudio) {
    for register in 0x30..=0x35 {
        mixer(ports, register, 0xf8);
    }
}

#[test]
fn sb16_words_are_signed_stereo_samples_and_acknowledge_only_the_word_irq() {
    let (mut ports, mut engine) = DosAudio::split(SoundConfig::sb16().latency(Tick::ZERO)).unwrap();
    maximum_volume(&mut ports);
    dsp(&mut ports, &[0x41, 0x13, 0x88, 0xb0, 0x30, 1, 0]); // 5 kHz, signed stereo, two scalar words.
    let channel = DmaChannel::new(5).unwrap();
    let first = ports.next_dma_tick().unwrap();
    assert_eq!(first, Tick::from_micros(100));
    assert_eq!(ports.dma_request(first).unwrap().channel, channel);
    assert!(ports.try_dma_write_at(first, channel, 0).is_err());
    ports.try_dma_write16_at(first, channel, 0xc000).unwrap();
    assert!(!ports.irq_pending(first));
    let second = ports.next_dma_tick().unwrap();
    assert_eq!(second, Tick::from_micros(200));
    ports.try_dma_write16_at(second, channel, 0x4000).unwrap();
    assert_eq!(mixer_read(&mut ports, 0x82), 2);
    ports.inb(0x22e);
    assert!(ports.irq_pending(second));
    ports.inb(0x22f);
    assert_eq!(mixer_read(&mut ports, 0x82), 0);
    let mut frames = [[0.0; 2]; 128];
    engine.render(&mut frames);
    assert!((frames[127][0] + 0.5).abs() < 1e-6);
    assert!((frames[127][1] - 0.5).abs() < 1e-6);
}

#[test]
fn sb16_byte_modes_and_auto_init_reload_their_own_block_length() {
    let (mut ports, mut engine) = DosAudio::split(SoundConfig::sb16().latency(Tick::ZERO)).unwrap();
    maximum_volume(&mut ports);
    dsp(&mut ports, &[0x41, 0x13, 0x88, 0xc4, 0x10, 0, 0]); // signed mono byte, one-byte auto-init.
    for _ in 0..3 {
        let at = ports.next_dma_tick().unwrap();
        ports
            .try_dma_write_at(at, DmaChannel::new(1).unwrap(), 0x80)
            .unwrap();
        assert_eq!(mixer_read(&mut ports, 0x82), 1);
        ports.inb(0x22f); // Wrong acknowledgement must not clear byte IRQ.
        assert!(ports.irq_pending(at));
        ports.inb(0x22e);
    }
    dsp(&mut ports, &[0xda]);
    let at = ports.next_dma_tick().unwrap();
    ports
        .try_dma_write_at(at, DmaChannel::new(1).unwrap(), 0x80)
        .unwrap();
    assert!(ports.next_dma_tick().is_none());
    let mut frames = [[0.0; 2]; 128];
    engine.render(&mut frames);
    assert!((frames[127][0] + 1.0).abs() < 1e-6);
    assert_eq!(frames[127][0], frames[127][1]);
}

#[test]
fn sb16_word_pause_and_exit_auto_init_keep_the_remaining_clock_and_count() {
    let (mut ports, _) = DosAudio::split(SoundConfig::sb16().latency(Tick::ZERO)).unwrap();
    dsp(&mut ports, &[0x41, 0x13, 0x88, 0xb4, 0, 0, 0]);
    ports.advance_to(Tick(100));
    dsp(&mut ports, &[0xd5]);
    assert!(ports.next_dma_tick().is_none());
    ports.advance_to(Tick(1_100));
    dsp(&mut ports, &[0xd6, 0xd9]);
    let at = ports.next_dma_tick().unwrap();
    assert_eq!(at, Tick::from_micros(200) + Tick(1_000));
    ports
        .try_dma_write16_at(at, DmaChannel::new(5).unwrap(), 0x8000)
        .unwrap();
    assert!(ports.next_dma_tick().is_none());
    assert_eq!(mixer_read(&mut ports, 0x82), 2);
}

#[test]
fn pro_stereo_uses_alternating_bytes_at_the_time_constant_clock() {
    let (mut ports, mut engine) =
        DosAudio::split(SoundConfig::sb_pro().latency(Tick::ZERO)).unwrap();
    mixer(&mut ports, 0x22, 0xee);
    mixer(&mut ports, 4, 0xee);
    mixer(&mut ports, 0x0e, 0x22); // Stereo; bypass the analog low-pass approximation.
    dsp(&mut ports, &[0xd1, 0x40, 211, 0x14, 1, 0]);
    for (micros, value) in [(45, 0), (90, 255)] {
        let at = ports.next_dma_tick().unwrap();
        assert_eq!(at, Tick::from_micros(micros));
        ports
            .try_dma_write_at(at, DmaChannel::new(1).unwrap(), value)
            .unwrap();
    }
    assert_eq!(ports.now(), Tick::from_micros(90));
    let mut frames = [[0.0; 2]; 32];
    engine.render(&mut frames);
    assert_eq!(frames[31], [-1.0, 127.0 / 128.0]);
}

#[test]
fn mixer_reset_and_legacy_aliases_preserve_resource_selection_and_dsp_reset() {
    let (mut ports, _) = DosAudio::split(SoundConfig::sb16()).unwrap();
    assert_eq!(mixer_read(&mut ports, 0x22), 0xcc);
    assert_eq!(mixer_read(&mut ports, 0x30), 0xc0);
    mixer(&mut ports, 0x22, 0xff);
    assert_eq!(mixer_read(&mut ports, 0x30), 0xf8);
    assert_eq!(mixer_read(&mut ports, 0x31), 0xf8);
    mixer(&mut ports, 0x80, 8);
    mixer(&mut ports, 0x81, 0x88);
    assert_eq!(ports.irq_request(ports.now()).unwrap().line.number(), 10);
    ports.outb(0x226, 1);
    ports.wait_us(3);
    ports.outb(0x226, 0);
    assert_eq!(mixer_read(&mut ports, 0x22), 0xff);
    mixer(&mut ports, 0, 0);
    assert_eq!(mixer_read(&mut ports, 0x22), 0xcc);
    assert_eq!(mixer_read(&mut ports, 0x80), 8);
    assert_eq!(mixer_read(&mut ports, 0x81), 0x88);
}

#[cfg(feature = "formats")]
#[test]
fn pro_common_opl_ports_broadcast_and_capture_independent_latches() {
    let (mut ports, _) = DosAudio::split(SoundConfig::sb_pro()).unwrap();
    ports.outb(0x220, 0x20);
    ports.outb(0x222, 0x23);
    ports.start_capture().unwrap();
    ports.outb(0x389, 0x31); // Both chips retain their independently selected addresses.
    let trace = Dro::load(&ports.finish_capture().unwrap().to_dro().unwrap()).unwrap();
    assert_eq!(trace.events().len(), 2);
    assert_eq!(
        trace.events()[0].event,
        MusicEvent::Opl {
            register: 0x20,
            value: 0x31
        }
    );
    assert_eq!(
        trace.events()[1].event,
        MusicEvent::Opl {
            register: 0x123,
            value: 0x31
        }
    );
}

#[test]
fn mixer_dma_disable_is_transactional_and_finishes_buffer_playback() {
    use game_toolkit_dosaudio::Error;

    let (mut ports, mut engine) =
        DosAudio::split(SoundConfig::sb16().latency(Tick::ZERO).queue_capacity(1)).unwrap();
    let handle = ports.play_pcm([128u8, 255].as_slice(), 8_000).unwrap();
    ports.outb(0x224, 0x81);
    assert_eq!(ports.try_outb_at(Tick(10), 0x225, 0), Err(Error::QueueFull));
    assert_eq!(ports.now(), Tick::ZERO);
    assert!(!handle.is_finished());
    assert_eq!(handle.samples_played(), 0);
    // The resource register and ownership must both survive the failed write.
    assert_eq!(ports.inb(0x225), 0x22);
    engine.render(&mut [[0.0; 2]; 1]);
    ports.try_outb_at(Tick(10), 0x225, 0).unwrap();
    assert!(handle.is_finished());
    assert_eq!(handle.samples_played(), 0);
    assert_eq!(ports.inb(0x225), 0);
    engine.render(&mut [[0.0; 2]; 1]);
    ports.try_advance_to(Tick::from_micros(1_000)).unwrap();
    assert!(ports.next_dma_tick().is_none());
    assert!(ports.dma_request(ports.now()).is_none());
}

#[test]
fn sb16_words_can_use_matching_blaster_low_dma_as_little_endian_bytes() {
    let config = SoundConfig::from_blaster("A220 I5 D1 H1 T6").unwrap();
    let (mut ports, mut engine) = DosAudio::split(config.latency(Tick::ZERO)).unwrap();
    maximum_volume(&mut ports);
    assert_eq!(mixer_read(&mut ports, 0x81), 2);
    dsp(&mut ports, &[0x41, 0x13, 0x88, 0xb0, 0x30, 1, 0]);
    let channel = DmaChannel::new(1).unwrap();
    for (micros, sample) in [(100, 0xc000u16), (200, 0x4000u16)] {
        let at = ports.next_dma_tick().unwrap();
        assert_eq!(at, Tick::from_micros(micros));
        assert_eq!(ports.dma_request(at).unwrap().channel, channel);
        assert!(ports.try_dma_write16_at(at, channel, sample).is_err());
        let [low, high] = sample.to_le_bytes();
        ports.try_dma_write_at(at, channel, low).unwrap();
        assert_eq!(ports.next_dma_tick(), Some(at));
        assert_eq!(mixer_read(&mut ports, 0x82), 0);
        ports.try_dma_write_at(at, channel, high).unwrap();
    }
    assert!(ports.next_dma_tick().is_none());
    assert_eq!(mixer_read(&mut ports, 0x82), 2);
    let mut frames = [[0.0; 2]; 128];
    engine.render(&mut frames);
    assert!((frames[127][0] + 0.5).abs() < 1e-6);
    assert!((frames[127][1] - 0.5).abs() < 1e-6);
}

#[test]
fn legacy_rate_updates_preserve_pending_and_paused_requests() {
    let (mut ports, _) = DosAudio::split(SoundConfig::sb2()).unwrap();
    dsp(&mut ports, &[0x40, 156, 0x48, 0, 0, 0x1c]); // 100 us, one-byte blocks.
    let pending = ports.next_dma_tick().unwrap();
    ports.advance_to(Tick::from_micros(40));
    dsp(&mut ports, &[0x40, 206]); // 50 us for subsequent requests.
    assert_eq!(ports.next_dma_tick(), Some(pending));
    let channel = DmaChannel::new(1).unwrap();
    ports.try_dma_write_at(pending, channel, 128).unwrap();
    let next = ports.next_dma_tick().unwrap();
    assert_eq!(next, Tick::from_micros(150));
    ports.advance_to(Tick::from_micros(120));
    dsp(&mut ports, &[0xd0, 0x40, 131]); // Paused remainder stays fixed; future period 125 us.
    assert!(ports.next_dma_tick().is_none());
    ports.advance_to(ports.now() + Tick(1_000));
    dsp(&mut ports, &[0xd4]);
    let resumed = ports.next_dma_tick().unwrap();
    assert_eq!(resumed, next + Tick(1_000));
    ports.try_dma_write_at(resumed, channel, 128).unwrap();
    assert!(
        ports
            .next_dma_tick()
            .unwrap()
            .0
            .abs_diff((resumed + Tick::from_micros(125)).0)
            <= 1
    );
}

#[test]
fn sb16_rate_updates_rescale_pending_and_paused_fractional_clock() {
    let (mut ports, _) = DosAudio::split(SoundConfig::sb16()).unwrap();
    dsp(&mut ports, &[0x41, 0x27, 0x10, 0xc4, 0, 0, 0]); // 10 kHz, byte auto-init.
    let pending = ports.next_dma_tick().unwrap();
    ports.advance_to(Tick::from_micros(40));
    dsp(&mut ports, &[0x41, 0x13, 0x88]); // 5 kHz, pending request remains at 100 us.
    let changed = ports.next_dma_tick().unwrap();
    assert!(changed.0.abs_diff(pending.0) <= 1);
    let channel = DmaChannel::new(1).unwrap();
    ports.try_dma_write_at(changed, channel, 128).unwrap();
    let next = ports.next_dma_tick().unwrap();
    assert!(next.0.abs_diff(Tick::from_micros(300).0) <= 1);
    ports.advance_to(Tick::from_micros(150));
    dsp(&mut ports, &[0xd0, 0x41, 0x4e, 0x20]); // Change to 20 kHz while paused.
    assert!(ports.next_dma_tick().is_none());
    ports.advance_to(ports.now() + Tick(1_000));
    dsp(&mut ports, &[0xd4]);
    let resumed = ports.next_dma_tick().unwrap();
    assert!(resumed.0.abs_diff((next + Tick(1_000)).0) <= 1);
    ports.try_dma_write_at(resumed, channel, 128).unwrap();
    assert!(
        ports
            .next_dma_tick()
            .unwrap()
            .0
            .abs_diff((resumed + Tick::from_micros(50)).0)
            <= 1
    );
}

#[test]
fn legacy_clock_limits_depend_on_transfer_mode_and_sb1_ignores_newer_modes() {
    for (config, stereo, command, time_constant, micros) in [
        (SoundConfig::sb1(), false, 0x14, 0, 250),
        (SoundConfig::sb1(), false, 0x14, 255, 44),
        (SoundConfig::sb2(), false, 0x14, 0, 250),
        (SoundConfig::sb2(), false, 0x14, 255, 44),
        (SoundConfig::sb2(), false, 0x91, 0, 43),
        (SoundConfig::sb2(), false, 0x91, 255, 23),
        (SoundConfig::sb_pro(), true, 0x91, 0, 45),
        (SoundConfig::sb_pro2(), true, 0x91, 255, 23),
    ] {
        let (mut ports, _) = DosAudio::split(config).unwrap();
        if stereo {
            mixer(&mut ports, 0x0e, 0x22);
        }
        dsp(&mut ports, &[0x40, time_constant, 0x48, 0, 0]);
        if command == 0x14 {
            dsp(&mut ports, &[command, 0, 0]);
        } else {
            dsp(&mut ports, &[command]);
        }
        assert_eq!(ports.next_dma_tick(), Some(Tick::from_micros(micros)));
    }
    for command in [0x1c, 0x90, 0x91] {
        let (mut ports, _) = DosAudio::split(SoundConfig::sb1()).unwrap();
        dsp(&mut ports, &[command]);
        assert!(ports.next_dma_tick().is_none());
    }
}
