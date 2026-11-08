//! Original VGM headers and register traces, generated without external music.
#![cfg(feature = "formats")]
use game_toolkit_dosaudio::{Error, MusicEvent, MusicTarget, OplChip, Tick, formats::Vgm};

fn set(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}
fn file(version: u32, header: usize, commands: &[u8]) -> Vec<u8> {
    assert!(header >= 0x40);
    let mut bytes = vec![0; header];
    bytes[..4].copy_from_slice(b"Vgm ");
    set(&mut bytes, 8, version);
    set(&mut bytes, 0x0c, 3_579_545);
    bytes[0x28..0x2a].copy_from_slice(&0x11u16.to_le_bytes());
    bytes[0x2a] = 15;
    bytes[0x2b] = 5;
    if version >= 0x150 {
        set(&mut bytes, 0x34, (header - 0x34) as u32);
    }
    bytes.extend(commands);
    let length = bytes.len();
    set(&mut bytes, 4, (length - 4) as u32);
    bytes
}
fn opl(version: u32, commands: &[u8], clock: usize, value: u32) -> Vec<u8> {
    let mut bytes = file(version, 0x100, commands);
    set(&mut bytes, 0x0c, 0);
    set(&mut bytes, clock, value);
    bytes
}
fn samples(count: u64) -> Tick {
    Tick(
        (u128::from(count) * u128::from(Tick::HZ_NUMERATOR)
            / (44_100 * u128::from(Tick::HZ_DENOMINATOR))) as u64,
    )
}

#[test]
fn waits_use_sample_units_keep_final_delay_and_ignore_bogus_sample_metadata() {
    let mut bytes = file(
        0x171,
        0x100,
        &[
            0x50, 0x90, 0x61, 0, 0, 0x70, 0x50, 0x91, 0x7f, 0x50, 0x92, 0x62, 0x50, 0x93, 0x63,
            0x61, 0xff, 0xff, 0x66,
        ],
    );
    set(&mut bytes, 0x18, u32::MAX);
    set(&mut bytes, 0x20, u32::MAX);
    let music = Vgm::load(&bytes).unwrap();
    assert_eq!(music.target(), MusicTarget::Psg);
    assert_eq!(
        music
            .events()
            .iter()
            .map(|e| (e.at, e.event))
            .collect::<Vec<_>>(),
        [
            (samples(0), MusicEvent::Psg(0x90)),
            (samples(1), MusicEvent::Psg(0x91)),
            (samples(17), MusicEvent::Psg(0x92)),
            (samples(752), MusicEvent::Psg(0x93)),
        ]
    );
    assert_eq!(music.duration(), samples(1 + 16 + 735 + 882 + 65_535));
    assert_eq!(music.loop_start(), Tick::ZERO);
    assert!(music.sysex().is_empty());
}

#[test]
fn single_sample_waits_retain_phase_through_exactly_one_second() {
    let mut commands = Vec::new();
    for _ in 0..44_100 {
        commands.extend([0x70, 0x50, 0x90]);
    }
    commands.push(0x66);
    let music = Vgm::load(&file(0x151, 0x100, &commands)).unwrap();
    for (index, event) in music.events().iter().enumerate() {
        assert_eq!(event.at, samples(index as u64 + 1));
    }
    assert_eq!(music.duration(), Tick::from_micros(1_000_000));
    assert_eq!(music.events().last().unwrap().at, music.duration());
}

#[test]
fn supported_opl_families_map_register_banks_and_preserve_fifo() {
    for (opcode, field) in [(0x5a, 0x50), (0x5b, 0x54)] {
        let music = Vgm::load(&opl(
            0x151,
            &[opcode, 0x20, 1, opcode, 0xa0, 0x98, 0x62, 0x66],
            field,
            3_579_545,
        ))
        .unwrap();
        assert_eq!(music.target(), MusicTarget::Opl(OplChip::Opl2));
        assert_eq!(
            music.events()[0].event,
            MusicEvent::Opl {
                register: 0x20,
                value: 1
            }
        );
        assert_eq!(
            music.events()[1].event,
            MusicEvent::Opl {
                register: 0xa0,
                value: 0x98
            }
        );
    }
    let music = Vgm::load(&opl(
        0x171,
        &[0x5e, 0x20, 1, 0x5f, 5, 1, 0x5f, 0xa0, 0x98, 0x63, 0x66],
        0x5c,
        14_318_180,
    ))
    .unwrap();
    assert_eq!(music.target(), MusicTarget::Opl(OplChip::Opl3));
    assert_eq!(
        music.events()[1].event,
        MusicEvent::Opl {
            register: 0x105,
            value: 1
        }
    );
    assert_eq!(
        music.events()[2].event,
        MusicEvent::Opl {
            register: 0x1a0,
            value: 0x98
        }
    );
}

#[test]
fn hard_panned_dual_ym3812_maps_second_chip_and_rejects_unrepresentable_routing() {
    let music = Vgm::load(&opl(
        0x171,
        &[0x5a, 0xa0, 0x44, 0xaa, 0xa0, 0x22, 0x62, 0x66],
        0x50,
        0xc000_0000 | 3_579_545,
    ))
    .unwrap();
    assert_eq!(music.target(), MusicTarget::Opl(OplChip::DualOpl2));
    assert_eq!(
        music.events()[1].event,
        MusicEvent::Opl {
            register: 0x1a0,
            value: 0x22
        }
    );
    for (field, raw) in [
        (0x50, 0x4000_0000 | 3_579_545),
        (0x50, 0x8000_0000 | 3_579_545),
        (0x54, 0x4000_0000 | 3_579_545),
        (0x54, 0xc000_0000 | 3_579_545),
        (0x5c, 0x4000_0000 | 14_318_180),
        (0x5c, 0x8000_0000 | 14_318_180),
    ] {
        assert!(Vgm::load(&opl(0x171, &[0x62, 0x66], field, raw)).is_err());
    }
    assert!(Vgm::load(&opl(0x171, &[0xaa, 0xa0, 1, 0x66], 0x50, 3_579_545)).is_err());
}

#[test]
fn mixed_and_observed_targets_are_determined_without_reading_commands_as_header_clocks() {
    let mut bytes = file(0x171, 0x100, &[0x50, 0x90, 0x5a, 0x20, 1, 0x62, 0x66]);
    set(&mut bytes, 0x50, 3_579_545);
    let music = Vgm::load(&bytes).unwrap();
    assert_eq!(music.target(), MusicTarget::OplPsg(OplChip::Opl2));
    assert_eq!(music.events()[0].event, MusicEvent::Psg(0x90));
    set(&mut bytes, 0x0c, 0);
    set(&mut bytes, 0x50, 0);
    assert_eq!(
        Vgm::load(&bytes).unwrap().target(),
        MusicTarget::OplPsg(OplChip::Opl2)
    );
    let mut commands = vec![0x70; 64];
    commands.extend([0x50, 0x90, 0x66]);
    let mut small = file(0x171, 0x40, &commands);
    set(&mut small, 0x34, 0); // v1.50+ zero offset defaults to 40h.
    assert_eq!(Vgm::load(&small).unwrap().target(), MusicTarget::Psg);
    let mut inferred = opl(0x171, &[0x5f, 5, 1, 0x66], 0x5c, 0);
    assert_eq!(
        Vgm::load(&inferred).unwrap().target(),
        MusicTarget::Opl(OplChip::Opl3)
    );
    set(&mut inferred, 0x50, 3_579_545); // Explicit incompatible hardware may not be upgraded silently.
    assert!(Vgm::load(&inferred).is_err());
}

#[test]
fn psg_requires_exact_tandy_noise_variant_and_supported_fixed_clock() {
    let valid = file(0x171, 0x100, &[0x50, 0xe4, 0x62, 0x66]);
    assert!(Vgm::load(&valid).is_ok());
    for (offset, value) in [
        (0x28, 9),
        (0x28, 3),
        (0x2a, 16),
        (0x2a, 0),
        (0x2b, 0),
        (0x2b, 2),
        (0x2b, 9),
        (0x2b, 17),
    ] {
        let mut bad = valid.clone();
        bad[offset] = value;
        assert!(Vgm::load(&bad).is_err());
    }
    for raw in [
        1,
        4_000_000,
        0x8000_0000,
        0x4000_0000 | 3_579_545,
        0xc000_0000 | 3_579_545,
        u32::MAX,
    ] {
        let mut bad = valid.clone();
        set(&mut bad, 0x0c, raw);
        assert!(Vgm::load(&bad).is_err());
    }
    let mut rounded = valid;
    set(&mut rounded, 0x0c, 3_579_546);
    assert!(Vgm::load(&rounded).is_ok());
    let old = file(0x110, 0x40, &[0x50, 0x90, 0x62, 0x66]);
    assert!(Vgm::load(&old).is_ok()); // Explicit Tandy noise fields predate flags.
    let oldest = file(0x101, 0x40, &[0x50, 0x90, 0x62, 0x66]);
    assert!(Vgm::load(&oldest).is_err()); // Earlier versions imply Sega 16-bit noise.
}

#[test]
fn header_declared_unsupported_chips_and_nonrepresentable_controls_fail() {
    let valid = file(0x171, 0x100, &[0x50, 0x90, 0x62, 0x66]);
    for field in [
        0x10, 0x2c, 0x30, 0x38, 0x40, 0x58, 0x60, 0x74, 0x80, 0xb8, 0xc0, 0xe0, 0xe4, 0xfc,
    ] {
        let mut bad = valid.clone();
        set(&mut bad, field, 3_579_545);
        assert!(Vgm::load(&bad).is_err(), "field {field:#x}");
    }
    for field in [0xbc, 0x7c] {
        let mut bad = valid.clone();
        set(&mut bad, field, 1);
        assert!(Vgm::load(&bad).is_err());
    }
    let mut conflicting = opl(0x171, &[0x62, 0x66], 0x50, 3_579_545);
    set(&mut conflicting, 0x5c, 14_318_180);
    assert!(Vgm::load(&conflicting).is_err());
}

#[test]
fn loop_intro_uses_actual_elapsed_samples_and_operand_or_empty_loops_are_errors() {
    let mut bytes = file(
        0x171,
        0x100,
        &[0x50, 0x90, 0x61, 0x44, 0xac, 0x50, 0x80, 0x62, 0x66],
    );
    set(&mut bytes, 0x1c, 0x105 - 0x1c);
    set(&mut bytes, 0x18, 1);
    set(&mut bytes, 0x20, 1);
    let music = Vgm::load(&bytes).unwrap();
    assert_eq!(music.loop_start(), samples(44_100));
    assert_eq!(music.duration(), samples(44_835));
    for at in [0x0cu32, 0x101, 0x103, 0x104, 0x108, 0x109, u32::MAX] {
        let mut bad = bytes.clone();
        set(&mut bad, 0x1c, at.wrapping_sub(0x1c));
        assert!(Vgm::load(&bad).is_err(), "loop at {at:#x}");
    }
    let mut zero = file(0x171, 0x100, &[0x50, 0x90, 0x66]);
    set(&mut zero, 0x1c, 0x100 - 0x1c);
    assert!(Vgm::load(&zero).is_err());
}

#[test]
fn offsets_and_all_truncated_prefixes_are_checked_before_indexing() {
    let valid = file(0x171, 0x100, &[0x50, 0x90, 0x61, 1, 0, 0x66]);
    for length in 0..valid.len() {
        assert!(Vgm::load(&valid[..length]).is_err(), "prefix {length}");
    }
    for (field, value) in [
        (4, 0),
        (4, u32::MAX),
        (8, 0),
        (8, 0x172),
        (0x34, 1),
        (0x34, u32::MAX),
        (0x34, 0x101 - 0x34),
        (0x14, 1),
        (0x14, u32::MAX),
    ] {
        let mut bad = valid.clone();
        set(&mut bad, field, value);
        assert!(
            Vgm::load(&bad).is_err(),
            "field {field:#x}, value {value:#x}"
        );
    }
    let mut small = file(0x171, 0x40, &[0x50, 0x90, 0x70, 0x66]);
    set(&mut small, 0x34, 0x42 - 0x34);
    assert!(Vgm::load(&small).is_err());
    let mut trailing = valid;
    trailing.push(0);
    assert!(Vgm::load(&trailing).is_err());
}

#[test]
fn gd3_is_validated_as_metadata_outside_the_command_stream() {
    let mut valid = file(0x171, 0x100, &[0x50, 0x90, 0x62, 0x66]);
    let gd3 = valid.len();
    set(&mut valid, 0x14, (gd3 - 0x14) as u32);
    valid.extend(b"Gd3 ");
    valid.extend(0x100u32.to_le_bytes());
    valid.extend(22u32.to_le_bytes());
    valid.extend([0; 22]);
    let length = valid.len();
    set(&mut valid, 4, (length - 4) as u32);
    assert_eq!(Vgm::load(&valid).unwrap().duration(), samples(735));
    for (at, value) in [
        (gd3, 0),
        (gd3 + 4, 0x200),
        (gd3 + 8, 23),
        (gd3 + 8, u32::MAX),
    ] {
        let mut bad = valid.clone();
        set(&mut bad, at, value);
        assert!(Vgm::load(&bad).is_err());
    }
    let mut overlap = valid;
    set(&mut overlap, 0x14, 0x100 - 0x14);
    assert!(Vgm::load(&overlap).is_err());
}

#[test]
fn unsupported_commands_are_errors_instead_of_silently_skipped_data() {
    for opcode in [
        0x30, 0x3f, 0x4f, 0x51, 0x52, 0x5c, 0x64, 0x67, 0x68, 0x80, 0x90, 0x95, 0xac, 0xae, 0xaf,
        0xe0, 0xff,
    ] {
        assert!(
            Vgm::load(&file(0x171, 0x100, &[opcode, 0, 0, 0, 0x66])).is_err(),
            "opcode {opcode:#x}"
        );
    }
    for commands in [
        vec![],
        vec![0x50],
        vec![0x61, 0],
        vec![0x5a, 0],
        vec![0x50, 0x90],
        vec![0x66, 0x50, 0x90],
    ] {
        assert!(
            Vgm::load(&file(0x171, 0x100, &commands)).is_err(),
            "{commands:?}"
        );
    }
    for version in [0x170, 0x171] {
        let error =
            Vgm::load(&file(version, 0x100, &[0x64, 0x62, 0xdf, 2, 0x62, 0x66])).unwrap_err();
        assert!(matches!(error, Error::Format(message) if message.contains("override")));
    }
}

#[test]
fn gzip_unknown_input_and_resource_limits_fail_without_panics() {
    assert!(
        matches!(Vgm::load(&[0x1f, 0x8b, 8]), Err(Error::Format(message)) if message.contains("compressed"))
    );
    let mut commands = vec![0; 1_000_001];
    commands.push(0x66);
    assert!(matches!(
        Vgm::load(&file(0x171, 0x100, &commands)),
        Err(Error::TooLarge)
    ));
    for length in 0..256 {
        let bytes: Vec<_> = (0..length)
            .map(|index| (index * 97 + length * 23) as u8)
            .collect();
        assert!(Vgm::load(&bytes).is_err());
    }
}

#[cfg(feature = "opl")]
#[test]
fn same_tick_intro_writes_are_not_repeated_after_the_exact_loop_boundary() {
    use game_toolkit_dosaudio::{DosAudio, PlayOptions, SoundConfig};
    let mut bytes = opl(
        0x171,
        &[0x5a, 0xa0, 1, 0x5a, 0xa0, 2, 0x70, 0x66],
        0x50,
        3_579_545,
    );
    set(&mut bytes, 0x1c, 0x103 - 0x1c); // Intro and loop onset share Tick zero.
    let music = Vgm::load(&bytes).unwrap();
    assert_eq!(music.loop_start(), Tick::ZERO);
    let (_ports, mut engine) = DosAudio::split(SoundConfig::adlib()).unwrap();
    let mut reader = engine.start_capture(64).unwrap();
    let _handle = engine.play(music, PlayOptions::looped()).unwrap();
    engine.render(&mut [[0.0; 2]; 8]);
    reader.stop();
    engine.render(&mut [[0.0; 2]; 1]);
    let trace = reader.try_finish().unwrap().unwrap();
    let values: Vec<_> = trace
        .writes()
        .as_chunks::<2>()
        .0
        .iter()
        .filter(|pair| pair[0].port == 0x388 && pair[0].value == 0xa0)
        .map(|pair| pair[1].value)
        .collect();
    assert!(values.len() >= 3);
    assert_eq!(values[0], 1);
    assert!(
        values[1..].iter().all(|&value| value == 2),
        "intro replayed: {values:?}"
    );
}
