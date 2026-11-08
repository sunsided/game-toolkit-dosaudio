//! Exercise MIDI-to-register behavior and audio through public interfaces.
#![cfg(feature = "opl")]
use game_toolkit_dosaudio::{
    MidiMessage, OplBank, OplBankEntry, OplBankKey, OplMidiDriver, prelude::*,
};
use std::sync::Arc;

fn message(bytes: &[u8]) -> MidiMessage {
    MidiMessage::new(bytes).unwrap()
}
fn send(driver: &mut OplMidiDriver, registers: &mut [u8; 512], bytes: &[u8]) -> Vec<(u16, u8)> {
    let mut writes = Vec::new();
    driver.message(message(bytes), |r, v| {
        registers[usize::from(r)] = v;
        writes.push((r, v));
    });
    writes
}
fn setup(chip: OplChip) -> (OplMidiDriver, [u8; 512]) {
    (OplMidiDriver::with_starter_bank(chip), [0; 512])
}
fn active(regs: &[u8; 512]) -> usize {
    (0..2)
        .flat_map(|b| (0..9).map(move |i| b * 256 + 0xb0 + i))
        .filter(|&i| regs[i] & 32 != 0)
        .count()
}
fn frequency(regs: &[u8; 512], channel: usize) -> f64 {
    let reg = (channel / 9) * 256 + channel % 9;
    let hi = regs[0xb0 + reg];
    let number = u16::from(regs[0xa0 + reg]) | (u16::from(hi & 3) << 8);
    let block = (hi >> 2) & 7;
    f64::from(number) * (315_000_000.0 / (22.0 * 288.0)) * 2f64.powi(i32::from(block)) / 1_048_576.0
}
fn bank() -> OplBank {
    let mut bank = OplBank::starter();
    for (msb, lsb, program, level) in [(0, 0, 0, 3), (0, 0, 1, 11), (2, 3, 1, 19)] {
        let mut instrument = bank
            .get(OplBankKey {
                msb: 0,
                lsb: 0,
                program: 0,
                percussion: false,
            })
            .unwrap()
            .instrument;
        instrument.patch.modulator.level = level;
        bank.insert(OplBankEntry {
            key: OplBankKey {
                msb,
                lsb,
                program,
                percussion: false,
            },
            name: format!("Original {level}"),
            instrument,
        })
        .unwrap();
    }
    bank
}

#[test]
fn modes_enable_both_banks_and_exhaustion_steals_the_oldest_note() {
    for chip in [OplChip::Opl2, OplChip::DualOpl2, OplChip::Opl3] {
        let (mut driver, mut regs) = setup(chip);
        let count = if chip == OplChip::Opl2 { 9 } else { 18 };
        for key in 60..60 + count {
            send(&mut driver, &mut regs, &[0x90, key as u8, 100]);
        }
        assert_eq!(active(&regs), count);
        let writes = send(&mut driver, &mut regs, &[0x90, 90, 100]);
        assert_eq!(active(&regs), count);
        assert_eq!(writes[0].0, 0xb0);
        assert_eq!(writes[0].1 & 32, 0);
        assert!((frequency(&regs, 0) - Note::from_midi(90).unwrap().frequency()).abs() < 3.0);
        assert!(send(&mut driver, &mut regs, &[0x80, 60, 0]).is_empty());
        send(&mut driver, &mut regs, &[0x80, 90, 0]);
        assert_eq!(active(&regs), count - 1);
        assert_eq!(regs[0x105], u8::from(chip == OplChip::Opl3));
        assert_eq!(regs[0x104], 0);
        assert_eq!(regs[0xbd] & 32, 0);
        if chip != OplChip::Opl2 {
            assert_eq!(regs[0x101] & 32, 32);
        }
    }
}

#[test]
fn repeated_keys_release_oldest_held_instance_and_velocity_zero_is_note_off() {
    let (mut driver, mut regs) = setup(OplChip::Opl2);
    send(&mut driver, &mut regs, &[0x90, 60, 127]);
    send(&mut driver, &mut regs, &[0x90, 60, 100]);
    assert_eq!(active(&regs), 2);
    send(&mut driver, &mut regs, &[0x80, 60, 0]);
    assert_eq!(regs[0xb0] & 32, 0);
    assert_ne!(regs[0xb1] & 32, 0);
    send(&mut driver, &mut regs, &[0x90, 60, 0]);
    assert_eq!(active(&regs), 0);
}

#[test]
fn sustain_all_notes_off_all_sound_off_and_reset_controllers_differ() {
    let (mut driver, mut regs) = setup(OplChip::Opl2);
    send(&mut driver, &mut regs, &[0x90, 60, 100]);
    send(&mut driver, &mut regs, &[0x91, 64, 100]);
    send(&mut driver, &mut regs, &[0xb0, 64, 127]);
    send(&mut driver, &mut regs, &[0x80, 60, 0]);
    assert_eq!(active(&regs), 2);
    send(&mut driver, &mut regs, &[0xb0, 64, 0]);
    assert_eq!(active(&regs), 1);
    send(&mut driver, &mut regs, &[0x90, 60, 100]);
    send(&mut driver, &mut regs, &[0xb0, 64, 127]);
    send(&mut driver, &mut regs, &[0xb0, 123, 0]);
    assert_eq!(active(&regs), 2);
    send(&mut driver, &mut regs, &[0xb0, 121, 0]);
    assert_eq!(active(&regs), 1);
    send(&mut driver, &mut regs, &[0x90, 60, 100]);
    send(&mut driver, &mut regs, &[0xb0, 64, 127]);
    send(&mut driver, &mut regs, &[0xb0, 120, 0]);
    assert_eq!(active(&regs), 1);
    assert_eq!(regs[0x43] & 63, 63);
    send(&mut driver, &mut regs, &[0xb1, 123, 0]);
    assert_eq!(active(&regs), 0);
}

#[test]
fn programs_and_banks_apply_only_to_new_notes_with_explicit_fallback() {
    let mut driver = OplMidiDriver::new(OplChip::Opl3, Arc::new(bank()));
    let mut regs = [0; 512];
    send(&mut driver, &mut regs, &[0x90, 60, 100]);
    assert_eq!(regs[0x40], 3);
    send(&mut driver, &mut regs, &[0xc0, 1]);
    assert_eq!(regs[0x40], 3);
    send(&mut driver, &mut regs, &[0x90, 62, 100]);
    assert_eq!(regs[0x41], 11);
    send(&mut driver, &mut regs, &[0xb0, 0, 2]);
    send(&mut driver, &mut regs, &[0xb0, 32, 3]);
    send(&mut driver, &mut regs, &[0x90, 64, 100]);
    assert_eq!(regs[0x42], 19);
    send(&mut driver, &mut regs, &[0xb0, 0, 99]);
    send(&mut driver, &mut regs, &[0x90, 65, 100]);
    assert_eq!(regs[0x48], 11);
    send(&mut driver, &mut regs, &[0xc0, 127]);
    send(&mut driver, &mut regs, &[0x90, 67, 100]);
    assert_eq!(regs[0x49], 3);
    let mut empty = OplMidiDriver::new(OplChip::Opl2, Arc::new(OplBank::new()));
    send(&mut empty, &mut regs, &[0x90, 60, 100]);
    assert_eq!(active(&regs), 0);
}

#[test]
fn pitch_bend_rpn_and_controller_resets_retune_active_notes() {
    let (mut driver, mut regs) = setup(OplChip::Opl2);
    send(&mut driver, &mut regs, &[0x90, 69, 127]);
    assert!((frequency(&regs, 0) - 440.0).abs() < 0.5);
    send(&mut driver, &mut regs, &[0xe0, 127, 127]);
    assert!((frequency(&regs, 0) - 493.88).abs() < 1.0);
    send(&mut driver, &mut regs, &[0xb0, 101, 0]);
    send(&mut driver, &mut regs, &[0xb0, 100, 0]);
    send(&mut driver, &mut regs, &[0xb0, 6, 12]);
    assert!((frequency(&regs, 0) - 880.0).abs() < 1.0);
    send(&mut driver, &mut regs, &[0xb0, 121, 0]);
    assert!((frequency(&regs, 0) - 440.0).abs() < 0.5);
    send(&mut driver, &mut regs, &[0xe0, 0, 0]);
    assert!((frequency(&regs, 0) - 391.995).abs() < 1.0);
}

#[test]
fn gain_changes_do_not_accumulate_attenuation_and_pan_matches_hardware() {
    let (mut driver, mut regs) = setup(OplChip::Opl3);
    send(&mut driver, &mut regs, &[0xb0, 7, 127]);
    send(&mut driver, &mut regs, &[0x90, 69, 127]);
    let original = regs[0x43];
    send(&mut driver, &mut regs, &[0xb0, 11, 0]);
    assert_eq!(regs[0x43] & 63, 63);
    send(&mut driver, &mut regs, &[0xb0, 11, 127]);
    assert_eq!(regs[0x43], original);
    for (pan, routing) in [(0, 0x10), (32, 0x30), (95, 0x30), (127, 0x20)] {
        send(&mut driver, &mut regs, &[0xb0, 10, pan]);
        assert_eq!(regs[0xc0] & 0x30, routing);
    }
    let (mut driver, mut regs) = setup(OplChip::DualOpl2);
    send(&mut driver, &mut regs, &[0xb0, 10, 127]);
    send(&mut driver, &mut regs, &[0x90, 60, 100]);
    assert_eq!(regs[0xb0] & 32, 0);
    assert_ne!(regs[0x1b0] & 32, 0);
    send(&mut driver, &mut regs, &[0xb0, 10, 0]);
    assert_ne!(regs[0x1b0] & 32, 0);
    send(&mut driver, &mut regs, &[0x90, 64, 100]);
    assert_ne!(regs[0xb0] & 32, 0);
}

#[test]
fn layered_notes_are_stolen_and_released_as_a_unit_and_drum_pitch_is_fixed() {
    let mut bank = bank();
    let mut instrument = bank
        .get(OplBankKey {
            msb: 0,
            lsb: 0,
            program: 0,
            percussion: false,
        })
        .unwrap()
        .instrument;
    instrument.second = Some(instrument.patch);
    instrument.note_offsets = [0, 12];
    instrument.detune = 64;
    bank.insert(OplBankEntry {
        key: OplBankKey {
            msb: 0,
            lsb: 0,
            program: 0,
            percussion: false,
        },
        name: "Layer".into(),
        instrument,
    })
    .unwrap();
    let mut driver = OplMidiDriver::new(OplChip::Opl2, Arc::new(bank));
    let mut regs = [0; 512];
    send(&mut driver, &mut regs, &[0x90, 60, 127]);
    assert_eq!(active(&regs), 2);
    assert!(frequency(&regs, 1) > frequency(&regs, 0) * 2.05);
    for key in [62, 64, 65, 67] {
        send(&mut driver, &mut regs, &[0x90, key, 100]);
    }
    assert_eq!(active(&regs), 8);
    send(&mut driver, &mut regs, &[0x80, 67, 0]);
    assert_eq!(active(&regs), 6);
    let (mut driver, mut regs) = setup(OplChip::Opl2);
    send(&mut driver, &mut regs, &[0x99, 36, 127]);
    assert!((frequency(&regs, 0) - Note::from_midi(36).unwrap().frequency()).abs() < 0.2);
    send(&mut driver, &mut regs, &[0x99, 127, 127]);
    assert!((frequency(&regs, 1) - frequency(&regs, 0)).abs() < 0.1);
    assert_eq!(regs[0xbd] & 32, 0);
}

#[test]
fn reset_and_gm_sysex_mute_keys_and_restore_default_program_state() {
    let (mut driver, mut regs) = setup(OplChip::Opl3);
    send(&mut driver, &mut regs, &[0x90, 60, 100]);
    driver.sysex(&[0xf0, 0x41, 0xf7], |r, v| regs[usize::from(r)] = v);
    assert_eq!(active(&regs), 1);
    driver.sysex(&[0xf0, 0x7e, 0x7f, 9, 1, 0xf7], |r, v| {
        regs[usize::from(r)] = v
    });
    assert_eq!(active(&regs), 0);
    assert_eq!(regs[0x43] & 63, 63);
    send(&mut driver, &mut regs, &[0x90, 64, 100]);
    send(&mut driver, &mut regs, &[0xff]);
    assert_eq!(active(&regs), 0);
}

#[test]
fn direct_messages_first_use_layers_stealing_and_resets_allocate_and_free_nothing() {
    let (mut driver, _) = setup(OplChip::Opl3);
    let mut checksum = 0u64;
    let allocations = allocation_counter::measure(|| {
        let mut emit = |r, v| checksum = checksum.wrapping_add(u64::from(r) + u64::from(v));
        for key in 0..40 {
            driver.message(message(&[0x90, key, 100]), &mut emit);
        }
        for data in [
            [0xb0, 64, 127],
            [0x80, 39, 0],
            [0xb0, 64, 0],
            [0xe0, 127, 127],
            [0xb0, 7, 64],
            [0xb0, 120, 0],
        ] {
            driver.message(message(&data), &mut emit);
        }
        driver.sysex(&[0xf0, 0x7e, 127, 9, 1, 0xf7], &mut emit);
        driver.message(message(&[0xff]), emit);
    });
    assert_ne!(checksum, 0);
    assert_eq!(allocations.count_total, 0);
    assert_eq!(allocations.count_current, 0);
}

#[test]
fn port_batches_roll_back_driver_state_and_capture_when_full_and_match_hardware() {
    for (chip, config) in [
        (OplChip::Opl2, SoundConfig::adlib()),
        (OplChip::DualOpl2, SoundConfig::dual_opl2()),
        (OplChip::Opl3, SoundConfig::opl3()),
    ] {
        let (mut ports, mut engine) =
            DosAudio::split(config.latency(Tick::ZERO).queue_capacity(128)).unwrap();
        let mut driver = OplMidiDriver::with_starter_bank(chip);
        for _ in 0..128 {
            ports.outb(0x388, 1);
            ports.outb(0x389, 0x20);
        }
        ports.start_capture().unwrap();
        assert_eq!(
            driver.write_message(&mut ports, message(&[0x90, 69, 127])),
            Err(game_toolkit_dosaudio::Error::QueueFull)
        );
        assert!(ports.finish_capture().unwrap().writes().is_empty());
        engine.render(&mut [[0.0; 2]; 1]);
        ports.start_capture().unwrap();
        driver
            .write_message(&mut ports, message(&[0x90, 69, 127]))
            .unwrap();
        let capture = ports.finish_capture().unwrap();
        let actual: Vec<_> = capture
            .writes()
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| {
                (
                    u16::from(pair[0].value) | if pair[0].port >= 0x38a { 0x100 } else { 0 },
                    pair[1].value,
                )
            })
            .collect();
        let mut fresh = OplMidiDriver::with_starter_bank(chip);
        let mut expected = Vec::new();
        fresh.message(message(&[0x90, 69, 127]), |r, v| expected.push((r, v)));
        assert_eq!(
            actual, expected,
            "failed request must not initialize or consume a voice"
        );
        engine.render(&mut [[0.0; 2]; 1]);
        let allocation = allocation_counter::measure(|| {
            driver
                .write_message(&mut ports, message(&[0x80, 69, 0]))
                .unwrap()
        });
        assert_eq!(allocation.count_total, 0);
        assert_eq!(allocation.count_current, 0);
        engine.render(&mut [[0.0; 2]; 64]);
    }
    let (mut ports, _) = DosAudio::split(SoundConfig::adlib()).unwrap();
    let mut driver = OplMidiDriver::with_starter_bank(OplChip::Opl3);
    assert!(
        driver
            .write_message(&mut ports, message(&[0x90, 60, 100]))
            .is_err()
    );
}

fn song() -> PreparedMusic {
    let events = [
        (0, [0xc0, 0, 0], 2),
        (0, [0x90, 69, 100], 3),
        (90_000, [0xb0, 64, 127], 3),
        (100_000, [0x80, 69, 0], 3),
        (150_000, [0xe0, 0, 80], 3),
        (200_000, [0xb0, 64, 0], 3),
        (220_000, [0xc0, 24, 0], 2),
        (250_000, [0x90, 72, 110], 3),
        (350_000, [0x80, 72, 0], 3),
    ];
    PreparedMusic::new(
        MusicTarget::Midi,
        events
            .into_iter()
            .map(|(at, bytes, len)| TimedEvent {
                at: Tick::from_micros(at),
                event: MusicEvent::Midi(message(&bytes[..len])),
            })
            .collect(),
        Tick::from_micros(450_000),
        [],
    )
    .unwrap()
}

#[test]
fn preparation_preserves_duration_event_order_and_audible_chunk_independent_output() {
    for (chip, config) in [
        (OplChip::Opl2, SoundConfig::adlib()),
        (OplChip::DualOpl2, SoundConfig::dual_opl2()),
        (OplChip::Opl3, SoundConfig::opl3()),
    ] {
        let source = song();
        let driver = OplMidiDriver::with_starter_bank(chip);
        let music = driver.prepare(&source).unwrap();
        assert_eq!(music.duration(), source.duration());
        assert_eq!(music.target(), MusicTarget::Opl(chip));
        let (_, mut a) = DosAudio::split(config.clone().latency(Tick::ZERO)).unwrap();
        let (_, mut b) = DosAudio::split(config.latency(Tick::ZERO)).unwrap();
        let _ha = a.play(music.clone(), PlayOptions::default()).unwrap();
        let _hb = b.play(music, PlayOptions::default()).unwrap();
        let mut whole = vec![[0.0; 2]; 24_000];
        let mut chunks = whole.clone();
        let allocations = allocation_counter::measure(|| a.render(&mut whole));
        assert_eq!(allocations.count_total, 0);
        assert_eq!(allocations.count_current, 0);
        for part in chunks.chunks_mut(137) {
            b.render(part);
        }
        assert_eq!(whole, chunks);
        assert!(whole.iter().flatten().all(|v| v.is_finite()));
        assert!(whole.iter().flatten().any(|v| v.abs() > 0.005));
    }
    assert!(
        OplMidiDriver::with_starter_bank(OplChip::Opl2)
            .prepare(&PreparedMusic::new(MusicTarget::Speaker, vec![], Tick(1), []).unwrap())
            .is_err()
    );
}

#[test]
fn loop_compilation_restores_the_boundary_register_state_and_keeps_intro_once() {
    let source = song().with_loop_start(Tick::from_micros(220_000)).unwrap();
    let driver = OplMidiDriver::with_starter_bank(OplChip::Opl2);
    let music = driver.prepare(&source).unwrap();
    assert_eq!(music.loop_start(), source.loop_start());
    let (_, mut engine) = DosAudio::split(SoundConfig::adlib().latency(Tick::ZERO)).unwrap();
    let mut capture = engine.start_capture(4096).unwrap();
    let handle = engine.play(music, PlayOptions::looped()).unwrap();
    engine.render(&mut [[0.0; 2]; 48_000]);
    capture.stop();
    engine.render(&mut [[0.0; 2]; 1]);
    let capture = capture.try_finish().unwrap().unwrap();
    let mut regs = [0; 512];
    let mut later_keys = 0;
    for pair in capture.writes().as_chunks::<2>().0 {
        let register = usize::from(pair[0].value) + if pair[0].port >= 0x38a { 256 } else { 0 };
        regs[register] = pair[1].value;
        if register == 0xb0 && pair[1].value & 32 != 0 && pair[1].at > source.duration() {
            assert!(
                (frequency(&regs, 0)
                    - Note::from_midi(72).unwrap().frequency() * 2f64.powf(0.5 / 12.0))
                .abs()
                    < 1.0
            );
            later_keys += 1;
        }
    }
    assert!(later_keys >= 2);
    assert!(!handle.is_finished());
    assert!(handle.position() >= source.loop_start());
}

#[test]
fn extreme_pitches_and_opl2_waveforms_clamp_without_corrupting_neighbor_voices() {
    let mut bank = bank();
    let key = OplBankKey {
        msb: 0,
        lsb: 0,
        program: 0,
        percussion: false,
    };
    let mut entry = bank.get(key).unwrap().clone();
    entry.instrument.note_offsets[0] = -127;
    entry.instrument.patch.carrier.waveform = 7;
    bank.insert(entry).unwrap();
    let mut driver = OplMidiDriver::new(OplChip::Opl2, Arc::new(bank));
    let mut regs = [0; 512];
    send(&mut driver, &mut regs, &[0x90, 0, 127]);
    assert_eq!(regs[0xa0], 1);
    assert_eq!(regs[0xb0] & 31, 0);
    assert_eq!(regs[0xe3], 3);
    send(&mut driver, &mut regs, &[0xc0, 1]);
    send(&mut driver, &mut regs, &[0x90, 127, 127]);
    assert_eq!(regs[0xa1], 255);
    assert_eq!(regs[0xb1] & 31, 31);
    assert_eq!(regs[0xa0], 1);
}

#[cfg(feature = "sb")]
#[test]
fn sb_pro_driver_reaches_the_selected_chip_without_common_port_broadcast() {
    let (mut ports, mut engine) =
        DosAudio::split(SoundConfig::sb_pro().latency(Tick::ZERO)).unwrap();
    let mut driver = OplMidiDriver::with_starter_bank(OplChip::DualOpl2);
    driver
        .write_message(&mut ports, message(&[0xb0, 10, 127]))
        .unwrap();
    ports.start_capture().unwrap();
    driver
        .write_message(&mut ports, message(&[0x90, 69, 127]))
        .unwrap();
    let capture = ports.finish_capture().unwrap();
    assert!(!capture.writes().is_empty());
    assert!(
        capture
            .writes()
            .iter()
            .all(|w| (0x222..=0x223).contains(&w.port))
    );
    let mut frames = [[0.0; 2]; 4800];
    engine.render(&mut frames);
    assert!(frames.iter().all(|f| f[0] == 0.0));
    assert!(frames.iter().any(|f| f[1].abs() > 0.001));
}
