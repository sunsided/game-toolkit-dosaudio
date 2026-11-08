//! Music parser and trace-export fixtures, constructed without third-party music.
#![cfg(feature = "formats")]
use game_toolkit_dosaudio::{
    MusicEvent, OplChip, Tick,
    formats::{Dro, Imf},
};

fn v2(chip: u8, map: &[u8], data: &[u8]) -> Vec<u8> {
    let mut bytes = b"DBRAWOPL".to_vec();
    bytes.extend(2u16.to_le_bytes());
    bytes.extend(0u16.to_le_bytes());
    bytes.extend(((data.len() / 2) as u32).to_le_bytes());
    bytes.extend(0u32.to_le_bytes()); // Deliberately bogus metadata duration.
    bytes.extend([chip, 0, 0, 0xfe, 0xff, map.len() as u8]);
    bytes.extend(map);
    bytes.extend(data);
    bytes
}

#[test]
fn imf_both_layouts_preserve_zero_delays_and_cumulative_clock_phase() {
    let records = [0x20, 1, 0, 0, 0x23, 2, 1, 0, 0xb0, 0x31, 0x2f, 0x02]; // Total 560 ticks.
    let sequence = Imf::load_type0(&records, 560).unwrap();
    assert_eq!(sequence.events().len(), 3);
    assert_eq!(sequence.events()[1].at, Tick::ZERO);
    assert_eq!(sequence.duration(), Tick::from_secs_f64(1.0));
    let mut type1 = (records.len() as u16).to_le_bytes().to_vec();
    type1.extend(records);
    assert_eq!(Imf::load(&type1, 560).unwrap().events(), sequence.events());
    type1.extend(b"metadata");
    assert_eq!(
        Imf::load_type1(&type1, 560).unwrap().events(),
        sequence.events()
    );
    assert!(Imf::load_type0(&records[..11], 560).is_err());
    assert!(Imf::load_type0(&records, 0).is_err());
    assert!(Imf::load_type1(&type1[..4], 560).is_err());
}

#[test]
fn dro_v2_maps_registers_banks_and_both_delay_codes() {
    let bytes = v2(2, &[0x20, 0xb0], &[0, 1, 0xfe, 2, 0x81, 0x31, 0xff, 1]);
    let sequence = Dro::load(&bytes).unwrap();
    assert_eq!(sequence.chip(), OplChip::Opl3);
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
            register: 0x1b0,
            value: 0x31
        }
    );
    assert_eq!(sequence.events()[1].at, Tick::from_micros(3_000));
    assert_eq!(sequence.duration(), Tick::from_micros(515_000));
    let mut opl2 = bytes;
    opl2[20] = 0;
    assert_eq!(Dro::load(&opl2).unwrap().events().len(), 1); // Ignore high-bank snapshots on OPL2.
}

#[test]
fn dro_v1_supports_both_hardware_field_sizes_and_escaped_registers() {
    let data = [4, 2, 0xff, 0, 0, 3, 0xb0, 0x31, 1, 0xff, 0];
    for padded in [false, true] {
        let mut bytes = b"DBRAWOPL".to_vec();
        bytes.extend(0u16.to_le_bytes());
        bytes.extend(1u16.to_le_bytes());
        bytes.extend(999u32.to_le_bytes());
        bytes.extend((data.len() as u32).to_le_bytes());
        bytes.push(1);
        if padded {
            bytes.extend([0; 3]);
        }
        bytes.extend(data);
        let sequence = Dro::load(&bytes).unwrap();
        assert_eq!(sequence.chip(), OplChip::Opl3);
        assert_eq!(
            sequence.events()[0].event,
            MusicEvent::Opl {
                register: 2,
                value: 0xff
            }
        );
        assert_eq!(
            sequence.events()[1].event,
            MusicEvent::Opl {
                register: 0x1b0,
                value: 0x31
            }
        );
        assert_eq!(sequence.duration(), Tick::from_micros(257_000));
    }
}

#[test]
fn dro_rejects_truncation_bad_lengths_and_unsupported_encoding() {
    let valid = v2(0, &[0x20], &[0, 1, 0xfe, 1]);
    for end in 0..valid.len() {
        assert!(Dro::load(&valid[..end]).is_err(), "prefix {end}");
    }
    let mut corrupt = valid.clone();
    corrupt[22] = 1;
    assert!(Dro::load(&corrupt).is_err());
    corrupt = valid.clone();
    corrupt[21] = 1;
    assert!(Dro::load(&corrupt).is_err());
    corrupt = valid.clone();
    corrupt[26 + 1] = 127;
    assert!(Dro::load(&corrupt).is_err());
    corrupt = valid.clone();
    corrupt[12..16].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(Dro::load(&corrupt).is_err());
    corrupt = valid;
    corrupt.push(0);
    assert!(Dro::load(&corrupt).is_err());
}

#[cfg(feature = "opl")]
#[test]
fn capture_replays_banked_writes_final_delay_and_latched_addresses() {
    use game_toolkit_dosaudio::{DosAudio, SoundConfig};
    let (mut audio, _) = DosAudio::split(SoundConfig::opl3()).unwrap();
    audio.outb(0x388, 0x20); // Address latch predates capture.
    audio.start_capture().unwrap();
    assert!(audio.start_capture().is_err());
    audio.outb(0x389, 1);
    audio.wait_us(10_000);
    audio.outb(0x38a, 0xb0);
    audio.outb(0x38b, 0x31);
    audio.wait_us(10_000);
    let capture = audio.finish_capture().unwrap();
    assert_eq!(capture.writes().len(), 3);
    assert!(audio.finish_capture().is_none());
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
            register: 0x1b0,
            value: 0x31
        }
    );
    assert_eq!(sequence.events()[1].at, Tick::from_micros(10_000));
    assert_eq!(sequence.duration(), Tick::from_micros(20_000));
}

#[test]
fn malformed_short_inputs_never_panic() {
    // Deterministic small-input sweep also covers empty and partial headers.
    for length in 0..128 {
        let bytes: Vec<u8> = (0..length).map(|n| (n * 137 + length * 41) as u8).collect();
        let _ = Dro::load(&bytes);
        let _ = Imf::load(&bytes, 700);
    }
}
