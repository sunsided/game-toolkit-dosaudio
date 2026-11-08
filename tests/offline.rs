//! The offline wrapper must match the live engine and produce a valid WAV container.
use game_toolkit_dosaudio::prelude::*;
use std::time::Duration;

#[test]
fn offline_speaker_matches_the_live_engine_and_wav_header() {
    let config = SoundConfig::none().speaker(true).latency(Tick::ZERO);
    let mut offline = OfflineMachine::new(config.clone(), 48_000).unwrap();
    offline.sound(440.0).unwrap();
    offline.render_for(Duration::from_millis(22)).unwrap();
    let wav = offline.finish();
    assert_eq!(wav.frames().len(), 1_056);
    let (mut audio, mut engine) = DosAudio::split(config).unwrap();
    audio.sound(440.0).unwrap();
    let mut direct = vec![[0.0; 2]; 1_056];
    engine.render(&mut direct);
    assert_eq!(wav.frames(), direct);
    let bytes = wav.to_bytes().unwrap();
    assert_eq!(&bytes[..4], b"RIFF");
    assert_eq!(&bytes[8..16], b"WAVEfmt ");
    assert_eq!(
        u32::from_le_bytes(bytes[4..8].try_into().unwrap()) as usize,
        bytes.len() - 8
    );
    assert_eq!(
        u32::from_le_bytes(bytes[40..44].try_into().unwrap()) as usize,
        direct.len() * 4
    );
    assert_eq!(bytes.len(), 44 + direct.len() * 4);
    assert_eq!(wav.sample_rate(), 48_000);
}

#[test]
fn offline_advance_sizes_do_not_change_output() {
    fn capture(chunks: &[u64]) -> Vec<u8> {
        let mut machine = OfflineMachine::new(SoundConfig::none().speaker(true), 44_100).unwrap();
        machine.sound(262.0).unwrap();
        for &at in chunks {
            machine.advance_to(Tick::from_micros(at)).unwrap();
        }
        machine.finish().to_bytes().unwrap()
    }
    assert_eq!(
        capture(&[22_000]),
        capture(&[1, 77, 1_234, 9_000, 10_000, 22_000])
    );
}

#[cfg(feature = "opl")]
#[test]
fn offline_music_renders_through_the_release_tail_deterministically() {
    #[path = "common.rs"]
    mod common;
    fn capture() -> Vec<u8> {
        let mut machine = OfflineMachine::new(SoundConfig::adlib(), 48_000).unwrap();
        let _music = machine.play(common::tune(OplChip::Opl2, 0, 0)).unwrap();
        machine.render_to_end().unwrap().to_bytes().unwrap()
    }
    let bytes = capture();
    assert_eq!(bytes, capture());
    assert!(bytes[44..].iter().any(|&value| value != 0));
    let frames = (bytes.len() - 44) / 4;
    assert!((16_798..=16_800).contains(&frames));
}

#[test]
fn invalid_configuration_and_impossible_offline_sizes_are_rejected() {
    assert!(DosAudio::split(SoundConfig::none().sample_rate(0)).is_err());
    assert!(DosAudio::split(SoundConfig::none().queue_capacity(0)).is_err());
    let mut machine = OfflineMachine::new(SoundConfig::none(), 48_000).unwrap();
    assert!(machine.advance_to(Tick::MAX).is_err());
    assert_eq!(machine.now(), Tick::ZERO);
}
