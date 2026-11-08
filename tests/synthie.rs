//! One original song through native synthie, FM and speaker using shared handles.
#![cfg(feature = "synthie")]
use game_toolkit_dosaudio::prelude::*;
use std::sync::Arc;
use synthie::{
    params::{ChannelNo, SynthParams},
    pattern::{Song, Tempo},
};
fn song() -> Song {
    Song::new(Tempo::bpm(137.0, 2))
        .patch(SynthParams::default())
        .lane(ChannelNo(0), "C4@0 . E4 G4 A4 . G4 -")
        .unwrap()
        .lane(ChannelNo(1), "C3 . . E3 . . G3 -")
        .unwrap()
        .lane(ChannelNo(3), "k h s h k h s h")
        .unwrap()
        .one_shot()
}
fn config() -> SoundConfig {
    SoundConfig::adlib()
        .speaker(true)
        .native(true)
        .latency(Tick::ZERO)
}
#[test]
fn original_song_has_matched_duration_and_audible_deterministic_arrangements() {
    let soundtrack = Arc::new(Soundtrack::synthie(&song(), OplChip::Opl2, 48_000).unwrap());
    let duration = soundtrack
        .arrangement(MusicDevice::Native)
        .unwrap()
        .duration();
    for device in [MusicDevice::Native, MusicDevice::Opl2, MusicDevice::Speaker] {
        assert_eq!(soundtrack.arrangement(device).unwrap().duration(), duration);
        let run = |chunk: usize| {
            let (_, mut e) = DosAudio::split(config()).unwrap();
            let _h = e
                .play_soundtrack(soundtrack.clone(), device, PlayOptions::default())
                .unwrap();
            let mut frames = vec![[0.0; 2]; 48_000];
            for part in frames.chunks_mut(chunk) {
                e.render(part);
            }
            frames
        };
        let frames = run(48000);
        assert_eq!(frames, run(113));
        assert!(frames.iter().flatten().all(|v| v.is_finite()));
        assert!(
            frames.iter().flatten().any(|v| v.abs() > 0.005),
            "{device:?}"
        );
    }
}
#[test]
fn switching_native_fm_and_speaker_keeps_time_and_never_reclaims_on_callback() {
    let soundtrack = Arc::new(Soundtrack::synthie(&song(), OplChip::Opl2, 48_000).unwrap());
    let (_, mut e) = DosAudio::split(config()).unwrap();
    let handle = e
        .play_soundtrack(soundtrack, MusicDevice::Native, PlayOptions::looped())
        .unwrap();
    let mut frames = [[0.0; 2]; 4800];
    for device in [
        MusicDevice::Native,
        MusicDevice::Opl2,
        MusicDevice::Speaker,
        MusicDevice::Native,
    ] {
        let before = handle.position();
        handle.switch_device(device).unwrap();
        let usage = allocation_counter::measure(|| e.render(&mut frames));
        assert_eq!(usage.count_total, 0);
        assert_eq!(usage.count_current, 0);
        assert_eq!(handle.device(), Some(device));
        assert!(handle.position() > before);
    }
    handle.pause();
    e.render(&mut frames);
    let paused = handle.position();
    handle.switch_device(MusicDevice::Opl2).unwrap();
    e.render(&mut frames);
    assert_eq!(handle.position(), paused);
    handle.resume();
    e.render(&mut frames);
    assert!(handle.position() > paused);
    handle.stop();
    let usage = allocation_counter::measure(|| e.render(&mut frames));
    assert_eq!(usage.count_total, 0);
    assert_eq!(usage.count_current, 0);
    assert!(handle.is_finished());
}
#[test]
fn native_pcm_has_exact_same_rate_samples_and_requires_explicit_installation() {
    let frames: Vec<_> = (0..512)
        .map(|i| [i as f32 / 1024.0, -(i as f32) / 1024.0])
        .collect();
    let music = PreparedMusic::from_pcm(frames.clone(), 48_000).unwrap();
    let (_, mut absent) = DosAudio::split(SoundConfig::none()).unwrap();
    assert!(absent.play(music.clone(), PlayOptions::default()).is_err());
    let (_, mut e) = DosAudio::split(SoundConfig::none().native(true)).unwrap();
    let _handle = e.play(music, PlayOptions::default()).unwrap();
    let mut output = vec![[0.0; 2]; 512];
    e.render(&mut output);
    assert_eq!(frames, output);
    assert!(PreparedMusic::from_pcm(vec![[f32::NAN, 0.0]], 48_000).is_err());
    assert!(PreparedMusic::from_pcm(vec![], 0).is_err());
}
#[test]
fn unsupported_channels_patch_indices_arpeggios_and_empty_songs_fail() {
    let prepare = |s: &Song| Soundtrack::synthie(s, OplChip::Opl2, 48_000);
    assert!(prepare(&Song::new(Tempo::bpm(120.0, 2))).is_err());
    assert!(
        prepare(
            &Song::new(Tempo::bpm(120.0, 2))
                .lane(ChannelNo(4), "C4")
                .unwrap()
        )
        .is_err()
    );
    assert!(
        prepare(
            &Song::new(Tempo::bpm(120.0, 2))
                .lane(ChannelNo(0), "C4@9")
                .unwrap()
        )
        .is_err()
    );
    assert!(
        prepare(
            &Song::new(Tempo::bpm(120.0, 2))
                .lane(ChannelNo(0), "C4^047")
                .unwrap()
        )
        .is_err()
    );
}
#[test]
fn chord_mapping_uses_highest_speaker_note_and_stop_drop_are_safe() {
    let song = Song::new(Tempo::bpm(120.0, 2))
        .lane(ChannelNo(0), "C4+E4+G4 . -")
        .unwrap();
    let s = Soundtrack::synthie(&song, OplChip::Opl2, 48_000).unwrap();
    let speaker = s.arrangement(MusicDevice::Speaker).unwrap();
    let game_toolkit_dosaudio::MusicEvent::Speaker { divisor, enabled } = speaker.events()[0].event
    else {
        panic!("speaker event")
    };
    assert!(enabled);
    let hz = 315_000_000.0 / (22.0 * 12.0 * f64::from(divisor));
    assert!((hz - Note::from_midi(67).unwrap().frequency()).abs() < 1.0);
    let (_, mut e) = DosAudio::split(config()).unwrap();
    let h = e
        .play_soundtrack(Arc::new(s), MusicDevice::Native, PlayOptions::default())
        .unwrap();
    let clone = h.clone();
    drop(h);
    e.render(&mut [[0.0; 2]; 64]);
    assert!(!clone.is_finished());
    drop(clone);
    let usage = allocation_counter::measure(|| e.render(&mut [[0.0; 2]; 64]));
    assert_eq!(usage.count_total, 0);
    assert_eq!(usage.count_current, 0);
}
