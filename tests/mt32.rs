//! No-ROM validation; the explicit asset test uses only the caller's ROM directory.
#![cfg(feature = "mt32")]
use game_toolkit_dosaudio::{MidiSynth, Mt32Roms, Mt32Synth};
#[test]
fn missing_wrong_length_and_unrecognized_roms_fail_before_construction() {
    assert!(Mt32Roms::from_bytes(&[], &[]).is_err());
    assert!(Mt32Roms::from_bytes(&vec![0; 65536], &vec![0; 1048576]).is_err());
    assert!(Mt32Roms::from_rom_dir("/definitely/absent/dosaudio-roms").is_err());
}
#[test]
fn rom_dir_finds_mame_versioned_names_before_validating_contents() {
    let dir = std::env::temp_dir().join(format!("dosaudio-mt32-names-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("cm32l_ctrl_1_02.rom"), [0; 16]).unwrap();
    std::fs::write(dir.join("cm32l_pcm.rom"), [0; 16]).unwrap();
    let result = Mt32Roms::from_rom_dir(&dir);
    std::fs::remove_dir_all(&dir).unwrap();
    // Found by name, then rejected for its contents rather than as missing.
    assert!(matches!(
        result,
        Err(game_toolkit_dosaudio::Error::Format(_))
    ));
}
#[test]
#[ignore = "requires caller-owned CM-32L ROMs and DOSAUDIO_MT32_ROM_DIR"]
fn caller_roms_render_audible_channel_two_without_callback_allocation() {
    let path = std::env::var("DOSAUDIO_MT32_ROM_DIR").expect("set DOSAUDIO_MT32_ROM_DIR");
    let roms = Mt32Roms::from_rom_dir(path).unwrap();
    // Original MIDI phrase plus Roland DT1 reverb writes. Construct messages
    // on the control thread; firmware remains caller-owned.
    let mut events = vec![(0, vec![0x91, 60, 110])];
    for mode in 0..4 {
        let mut bytes = vec![0xf0, 0x41, 0x10, 0x16, 0x12, 0x10, 0, 1, mode, 5, 3];
        let sum = bytes[5..]
            .iter()
            .fold(0u8, |sum, byte| sum.wrapping_add(*byte));
        bytes.extend([(128 - (sum & 127)) & 127, 0xf7]);
        events.push((4000 * (usize::from(mode) + 1), bytes));
    }
    events.extend([
        (20_000, vec![0x81, 60, 0]),
        (22_000, vec![0xff]),
        (24_000, vec![0x91, 64, 110]),
        (30_000, vec![0x81, 64, 0]),
    ]);
    let render = |chunk| {
        let mut synth = Mt32Synth::new(roms.clone()).unwrap();
        let mut frames = vec![[0.0; 2]; 32_000];
        let usage = allocation_counter::measure(|| {
            for (index, (at, message)) in events.iter().enumerate() {
                synth.message(message);
                let end = events.get(index + 1).map_or(frames.len(), |event| event.0);
                for block in frames[*at..end].chunks_mut(chunk) {
                    synth.render(block);
                }
            }
        });
        assert_eq!(usage.count_total, 0);
        assert_eq!(usage.count_current, 0);
        assert_eq!(synth.dropped_messages(), 0);
        assert!(frames.iter().flatten().all(|sample| sample.is_finite()));
        for segment in [&frames[..20_000], &frames[24_000..30_000]] {
            assert!(segment.iter().flatten().any(|sample| sample.abs() > 0.001));
        }
        frames
    };
    assert_eq!(render(32_000), render(113));
}
