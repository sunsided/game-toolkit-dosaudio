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
    let mut synth = Mt32Synth::new(roms).unwrap();
    let mut frames = vec![[0.0; 2]; 16_000];
    let usage = allocation_counter::measure(|| {
        synth.message(&[0x91, 60, 110]);
        synth.render(&mut frames);
        synth.message(&[0x81, 60, 0]);
    });
    assert_eq!(usage.count_total, 0);
    assert_eq!(usage.count_current, 0);
    assert!(frames.iter().flatten().any(|v| v.abs() > 0.001));
}
