//! Sequencer timing, native chip output, controls and callback allocation checks.
#![cfg(feature = "opl")]
mod common;
use game_toolkit_dosaudio::prelude::*;

fn render(config: SoundConfig, sequence: Sequence, chunks: &[usize]) -> Vec<Frame> {
    let (_, mut engine) = DosAudio::split(config.latency(Tick::ZERO)).unwrap();
    let _handle = engine.play(sequence, PlayOptions::default()).unwrap();
    let mut result = vec![[0.0; 2]; 12_000];
    let mut offset = 0;
    for &chunk in chunks.iter().cycle() {
        let end = (offset + chunk).min(result.len());
        engine.render(&mut result[offset..end]);
        offset = end;
        if offset == result.len() {
            break;
        }
    }
    result
}

#[test]
fn fm_music_is_nonzero_finite_and_independent_of_callback_size() {
    let sequence = common::tune(OplChip::Opl2, 0, 0);
    let whole = render(SoundConfig::adlib(), sequence.clone(), &[12_000]);
    assert!(whole.iter().any(|frame| frame[0].abs() > 0.001));
    assert!(
        whole
            .iter()
            .flatten()
            .all(|value| value.is_finite() && value.abs() <= 1.0)
    );
    assert_eq!(
        whole,
        render(SoundConfig::adlib(), sequence, &[1, 3, 73, 512])
    );
}

#[test]
fn opl3_panning_and_dual_opl2_use_the_correct_speakers() {
    let left = render(
        SoundConfig::opl3(),
        common::tune(OplChip::Opl3, 0x100, 0x10),
        &[512],
    );
    assert!(left.iter().any(|frame| frame[0].abs() > 0.001));
    assert!(left.iter().all(|frame| frame[1] == 0.0));
    let right = render(
        SoundConfig::dual_opl2(),
        common::tune(OplChip::DualOpl2, 0x100, 0),
        &[512],
    );
    assert!(right.iter().all(|frame| frame[0] == 0.0));
    assert!(right.iter().any(|frame| frame[1].abs() > 0.001));
}

#[test]
fn paused_music_keeps_its_position_and_resumes() {
    let (_, mut engine) = DosAudio::split(SoundConfig::adlib()).unwrap();
    let handle = engine
        .play(common::tune(OplChip::Opl2, 0, 0), PlayOptions::looped())
        .unwrap();
    let mut buffer = [[0.0; 2]; 512];
    engine.render(&mut buffer);
    handle.pause();
    engine.render(&mut buffer);
    let position = handle.position();
    engine.render(&mut buffer);
    assert_eq!(handle.position(), position);
    assert!(buffer.iter().all(|&frame| frame == [0.0; 2]));
    handle.resume();
    engine.render(&mut buffer);
    assert!(handle.position() > position);
    assert!(buffer.iter().any(|frame| frame[0].abs() > 0.001));
}

#[test]
fn finite_and_looped_playback_finish_and_wrap_correctly() {
    let (_, mut engine) = DosAudio::split(SoundConfig::adlib()).unwrap();
    let sequence = common::tune(OplChip::Opl2, 0, 0);
    let handle = engine
        .play(sequence.clone(), PlayOptions::default())
        .unwrap();
    engine.render(&mut [[0.0; 2]; 5_000]);
    assert!(handle.is_finished());
    let looped = engine.play(sequence, PlayOptions::looped()).unwrap();
    engine.render(&mut [[0.0; 2]; 12_000]);
    assert!(!looped.is_finished());
    assert!(looped.position() < Tick::from_micros(100_000));
    looped.stop();
    engine.render(&mut [[0.0; 2]; 1]);
    assert!(looped.is_finished());
}

#[test]
fn rendering_including_first_use_loop_end_and_stop_allocates_nothing() {
    let (mut audio, mut engine) = DosAudio::split(SoundConfig::adlib().speaker(true)).unwrap();
    audio.sound(440.0).unwrap();
    let handle = engine
        .play(common::tune(OplChip::Opl2, 0, 0), PlayOptions::looped())
        .unwrap();
    let clone = handle.clone();
    let mut buffer = [[0.0; 2]; 10_000];
    let allocations = allocation_counter::measure(|| engine.render(&mut buffer));
    assert_eq!(allocations.count_total, 0);
    assert_eq!(allocations.count_current, 0);
    drop(handle); // The clone keeps playback alive.
    engine.render(&mut buffer[..1]);
    assert!(!clone.is_finished());
    drop(clone); // Last drop requests a stop; the engine retains sequence storage.
    let allocations = allocation_counter::measure(|| engine.render(&mut buffer));
    assert_eq!(allocations.count_total, 0);
    assert_eq!(allocations.count_current, 0);
}

#[test]
fn invalid_arrangements_and_zero_duration_loops_fail_before_playback() {
    let (_, mut engine) = DosAudio::split(SoundConfig::adlib()).unwrap();
    assert!(
        engine
            .play(
                common::tune(OplChip::Opl3, 0x100, 0x10),
                PlayOptions::default()
            )
            .is_err()
    );
    let empty = Sequence::new(OplChip::Opl2, Vec::new(), Tick::ZERO).unwrap();
    assert!(engine.play(empty, PlayOptions::looped()).is_err());
}

#[test]
fn custom_cores_observe_writes_before_the_correct_native_boundary() {
    use game_toolkit_dosaudio::OplCore;
    struct Probe(f32);
    impl OplCore for Probe {
        fn write(&mut self, register: u16, value: u8) {
            if register == 0xa0 {
                self.0 = f32::from(value);
            }
        }
        fn generate(&mut self) -> Frame {
            [self.0; 2]
        }
    }
    let backend = OplBackend::Custom(std::sync::Arc::new(|| Box::new(Probe(0.0))));
    let (mut ports, mut engine) = DosAudio::split(
        SoundConfig::adlib()
            .opl_backend(backend)
            .latency(Tick::ZERO),
    )
    .unwrap();
    ports.try_outb_at(Tick(300), 0x388, 0xa0).unwrap();
    ports.try_outb_at(Tick(300), 0x389, 1).unwrap();
    let mut frames = [[0.0; 2]; 3];
    engine.render(&mut frames);
    let width = 315_000_000.0 / (48_000.0 * 22.0);
    assert_eq!(frames[0], [0.0; 2]);
    assert!((f64::from(frames[1][0]) - (2.0 * width - 576.0) / width).abs() < 1e-6);
    assert_eq!(frames[2], [1.0; 2]);
}

#[cfg(feature = "nuked")]
#[test]
fn nuked_uses_the_same_timeline_and_produces_audio() {
    let config = SoundConfig::adlib().opl_backend(OplBackend::Nuked);
    let sequence = common::tune(OplChip::Opl2, 0, 0);
    let whole = render(config.clone(), sequence.clone(), &[12_000]);
    assert!(whole.iter().any(|frame| frame[0].abs() > 0.001));
    assert_eq!(whole, render(config, sequence, &[1, 37, 511]));
}
