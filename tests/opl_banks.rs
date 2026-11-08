//! Original byte fixtures exercise bank layouts without redistributing game banks.
#![cfg(feature = "opl")]
use game_toolkit_dosaudio::{OplBank, OplBankEntry, OplBankKey, OplInstrument, OplPatch};

fn key(program: u8, percussion: bool) -> OplBankKey {
    OplBankKey {
        msb: 0,
        lsb: 0,
        program,
        percussion,
    }
}
fn op2(names: bool) -> Vec<u8> {
    let mut b = vec![0; 8 + 175 * 36 + if names { 175 * 32 } else { 0 }];
    b[..8].copy_from_slice(b"#OPL_II#");
    for i in 0..175 {
        let d = &mut b[8 + i * 36..8 + (i + 1) * 36];
        d[2] = 128;
        d[3] = 60;
        d[4..20].copy_from_slice(&[
            0x21, 0xf2, 0x56, 1, 0x80, 7, 0x0e, 1, 0xe3, 0x67, 2, 0x40, 8, 0, 12, 0,
        ]);
        d[20..36].copy_from_slice(&[
            2, 0xe4, 0x78, 0, 0, 9, 2, 1, 0xf5, 0x89, 0, 0, 10, 0, 0xf4, 0xff,
        ]);
    }
    if names {
        b[8 + 175 * 36..8 + 175 * 36 + 5].copy_from_slice(b"First");
    }
    b
}
fn wopl(version: u16) -> Vec<u8> {
    let metadata = if version >= 2 { 68 } else { 0 };
    let size = if version == 3 { 66 } else { 62 };
    let mut b = vec![0; 19 + metadata + 256 * size];
    b[..11].copy_from_slice(b"WOPL3-BANK\0");
    b[11..13].copy_from_slice(&version.to_le_bytes());
    b[13..15].copy_from_slice(&1u16.to_be_bytes());
    b[15..17].copy_from_slice(&1u16.to_be_bytes());
    for i in 0..256 {
        b[19 + metadata + i * size + 39] = 4;
    }
    for (index, program) in [(0, 7), (1, 38)] {
        let off = 19 + metadata + (index * 128 + program) * size;
        let d = &mut b[off..off + size];
        d[..4].copy_from_slice(b"Tone");
        d[32..34].copy_from_slice(&(-12_i16).to_be_bytes());
        d[34..36].copy_from_slice(&12_i16.to_be_bytes());
        d[36] = (-7_i8) as u8;
        d[37] = 64;
        d[38] = 48;
        d[39] = 2;
        d[40] = 0x0e;
        d[41] = 2;
        d[42..62].copy_from_slice(&[
            1, 4, 0xf3, 0x65, 2, 2, 7, 0xe4, 0x76, 1, 3, 8, 0xd5, 0x87, 0, 4, 9, 0xc6, 0x98, 3,
        ]);
    }
    b
}
fn bnk() -> Vec<u8> {
    let mut b = vec![0; 28 + 24 + 60];
    b[..8].copy_from_slice(b"\x01\0ADLIB-");
    b[8..12].copy_from_slice(&[2, 0, 2, 0]);
    b[12..16].copy_from_slice(&28u32.to_le_bytes());
    b[16..20].copy_from_slice(&52u32.to_le_bytes());
    for (n, index) in [(0, 1u16), (1, 0)] {
        let p = 28 + n * 12;
        b[p..p + 2].copy_from_slice(&index.to_le_bytes());
        b[p + 2] = 1;
        b[p + 3..p + 7].copy_from_slice(if n == 0 { b"Wire" } else { b"Bell" });
    }
    for i in 0..2 {
        let d = &mut b[52 + i * 30..52 + (i + 1) * 30];
        d[2..15].copy_from_slice(&[2, 3, 4, 15, 6, 1, 2, 5, 7, 1, 0, 1, 1]);
        d[15..28].copy_from_slice(&[1, 1, 0, 14, 7, 0, 3, 6, 8, 0, 1, 0, 1]);
        d[28] = i as u8;
        d[29] = 2;
    }
    b
}

#[test]
fn empty_starter_insert_and_exact_lookup_have_explicit_fallback_provenance() {
    assert!(OplBank::new().entries().is_empty());
    let bank = OplBank::starter();
    assert_eq!(bank.entries().len(), 11);
    assert_eq!(bank.get(key(24, false)).unwrap().name, "Plucked wire");
    assert_eq!(
        bank.get(key(36, true)).unwrap().instrument.fixed_note,
        Some(36)
    );
    assert!(bank.get(key(127, false)).is_none());
    let mut bank = OplBank::new();
    let entry = OplBankEntry {
        key: key(0, false),
        name: "Test".into(),
        instrument: OplPatch::default().into(),
    };
    bank.insert(entry.clone()).unwrap();
    bank.insert(entry.clone()).unwrap();
    assert_eq!(bank.entries().len(), 1);
    let mut bad = entry.clone();
    bad.instrument.fixed_note = Some(128);
    assert!(bank.insert(bad).is_err());
    let mut bad = entry.clone();
    bad.instrument.note_offsets[0] = 128;
    assert!(bank.insert(bad).is_err());
    let mut bad = entry.clone();
    bad.key.msb = 128;
    assert!(bank.insert(bad).is_err());
    assert_eq!(bank.entries(), [entry]);
}

#[test]
fn op2_maps_split_levels_offsets_layers_fixed_notes_and_gm_drum_keys() {
    let mut b = op2(true);
    b[8..10].copy_from_slice(&4u16.to_le_bytes());
    b[10] = 192;
    let bank = OplBank::from_op2(&b).unwrap();
    assert_eq!(bank.entries().len(), 175);
    let entry = bank.get(key(0, false)).unwrap();
    assert_eq!(entry.name, "First");
    let i = entry.instrument;
    assert_eq!(i.patch.modulator.level, 0x87);
    assert_eq!(i.patch.carrier.level, 0x48);
    assert_eq!(i.patch.modulator.waveform, 1);
    assert_eq!(i.patch.feedback, 0x0e);
    assert_eq!(i.note_offsets, [12, -12]);
    assert_eq!(i.detune, 64);
    assert!(i.second.is_some());
    assert_eq!(i.second.unwrap().carrier.attack_decay, 0xf5);
    let d = 8 + 128 * 36;
    b[d..d + 2].copy_from_slice(&1u16.to_le_bytes());
    b[d + 3] = 36;
    let bank = OplBank::from_op2(&b).unwrap();
    assert_eq!(
        bank.get(key(35, true)).unwrap().instrument.fixed_note,
        Some(36)
    );
    assert_eq!(
        bank.get(key(35, true)).unwrap().instrument.note_offsets,
        [0, 0]
    );
    assert!(bank.get(key(81, true)).is_some());
    assert!(OplBank::from_op2(&op2(false)).is_ok());
}

#[test]
fn op2_rejects_partial_records_names_unsupported_flags_and_pitch() {
    let b = op2(true);
    for end in (0..b.len()).filter(|&end| end != 8 + 175 * 36) {
        assert!(OplBank::from_op2(&b[..end]).is_err(), "prefix {end}");
    }
    for flags in [2u16, 8, 0xffff] {
        let mut b = b.clone();
        b[8..10].copy_from_slice(&flags.to_le_bytes());
        assert!(OplBank::from_op2(&b).is_err());
    }
    let mut b = b;
    b[8] = 1;
    b[11] = 128;
    assert!(OplBank::from_op2(&b).is_err());
}

#[test]
fn wopl_versions_map_endianness_operator_roles_banks_and_layer_detune() {
    for version in 1..=3 {
        let mut b = wopl(version);
        if version >= 2 {
            b[19 + 32] = 2;
            b[19 + 33] = 3;
        }
        let bank = OplBank::from_wopl(&b).unwrap();
        assert_eq!(bank.entries().len(), 2);
        let k = OplBankKey {
            msb: if version >= 2 { 3 } else { 0 },
            lsb: if version >= 2 { 2 } else { 0 },
            program: 7,
            percussion: false,
        };
        let i = bank.get(k).unwrap().instrument;
        assert_eq!(i.patch.modulator.characteristics, 2);
        assert_eq!(i.patch.carrier.characteristics, 1);
        assert_eq!(i.patch.carrier.waveform, 2);
        assert_eq!(i.note_offsets, [-12, 12]);
        assert_eq!(i.detune, 64);
        assert_eq!(i.velocity_offset, -7);
        assert_eq!(i.second.unwrap().modulator.characteristics, 4);
        assert_eq!(
            bank.get(key(38, true)).unwrap().instrument.fixed_note,
            Some(48)
        );
    }
}

#[test]
fn wopl_rejects_lengths_versions_counts_global_models_and_unsupported_modes() {
    let b = wopl(3);
    for end in 0..b.len() {
        assert!(OplBank::from_wopl(&b[..end]).is_err());
    }
    for (offset, value) in [
        (11, 0),
        (11, 4),
        (13, 0xff),
        (17, 4),
        (18, 1),
        (19 + 32, 128),
    ] {
        let mut b = b.clone();
        b[offset] = value;
        assert!(OplBank::from_wopl(&b).is_err(), "field {offset}");
    }
    for flag in [1, 3, 8, 0x80] {
        let mut b = b.clone();
        b[19 + 68 + 7 * 66 + 39] = flag;
        assert!(OplBank::from_wopl(&b).is_err(), "flags {flag}");
    }
    let mut b = b;
    b.push(0);
    assert!(OplBank::from_wopl(&b).is_err());
}

#[test]
fn bnk_maps_named_indices_and_packed_operator_fields_without_assuming_gm() {
    let bank = OplBank::from_bnk(&bnk()).unwrap();
    assert_eq!(bank.entries().len(), 2);
    let e = bank.get(key(0, false)).unwrap();
    assert_eq!(e.name, "Wire");
    assert_eq!(e.instrument.patch.modulator.characteristics, 0xb3);
    assert_eq!(e.instrument.patch.modulator.level, 0x87);
    assert_eq!(e.instrument.patch.modulator.attack_decay, 0xf2);
    assert_eq!(e.instrument.patch.modulator.sustain_release, 0x65);
    assert_eq!(e.instrument.patch.feedback, 8);
    assert_eq!(e.instrument.patch.modulator.waveform, 1);
    assert_eq!(
        bank.get(key(1, false))
            .unwrap()
            .instrument
            .patch
            .modulator
            .waveform,
        0
    );
}

#[test]
fn bnk_rejects_out_of_range_references_overlap_counts_rhythm_and_bad_fields() {
    let b = bnk();
    for end in 0..b.len() {
        assert!(OplBank::from_bnk(&b[..end]).is_err(), "prefix {end}");
    }
    for (offset, value) in [
        (0, 2),
        (8, 3),
        (12, 1),
        (16, 28),
        (28, 2),
        (30, 2),
        (40, 1),
        (52, 1),
        (54, 4),
    ] {
        let mut b = b.clone();
        b[offset] = value;
        assert!(OplBank::from_bnk(&b).is_err(), "field {offset}");
    }
}

#[test]
fn sbi_bank_names_and_all_short_random_inputs_are_safe() {
    let mut sbi = [0; 52];
    sbi[..4].copy_from_slice(b"SBI\x1a");
    sbi[4..8].copy_from_slice(b"Name");
    assert_eq!(
        OplBank::from_sbi(&sbi)
            .unwrap()
            .get(key(0, false))
            .unwrap()
            .name,
        "Name"
    );
    let mut seed = 17u32;
    for len in 0..512 {
        let data: Vec<_> = (0..len)
            .map(|_| {
                seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                (seed >> 24) as u8
            })
            .collect();
        for loader in [
            OplBank::from_sbi,
            OplBank::from_bnk,
            OplBank::from_op2,
            OplBank::from_wopl,
        ] {
            assert!(loader(&data).is_err());
        }
    }
    let _: OplInstrument = OplPatch::default().into();
}

#[test]
fn wopl_duplicate_bank_addresses_and_depth_flags_are_explicit() {
    let mut duplicate = wopl(3);
    duplicate[13..17].copy_from_slice(&[0, 2, 0, 0]);
    assert!(OplBank::from_wopl(&duplicate).is_err());
    let mut b = wopl(3);
    b[17] = 3;
    let bank = OplBank::from_wopl(&b).unwrap();
    let mut driver = game_toolkit_dosaudio::OplMidiDriver::new(
        game_toolkit_dosaudio::OplChip::Opl2,
        std::sync::Arc::new(bank),
    );
    let mut depth = 0;
    driver.message(
        game_toolkit_dosaudio::MidiMessage::new(&[0xc0, 7]).unwrap(),
        |r, v| {
            if r == 0xbd {
                depth = v;
            }
        },
    );
    assert_eq!(depth, 0xc0);
}
