//! Public music controls produce sample-accurate gain ramps without callback allocation.
#![cfg(feature = "opl")]

use game_toolkit_dosaudio::{OplCore, prelude::*};
use std::{sync::Arc, time::Duration};

const FADE: Duration = Duration::from_millis(10);
const FADE_FRAMES: usize = 480;

struct ConstantCore;

impl OplCore for ConstantCore {
    fn write(&mut self, _: u16, _: u8) {}

    fn generate(&mut self) -> Frame {
        [1.0; 2]
    }
}

fn config() -> SoundConfig {
    SoundConfig::adlib()
        .sample_rate(48_000)
        .latency(Tick::ZERO)
        .opl_backend(OplBackend::Custom(Arc::new(|| Box::new(ConstantCore))))
}

fn machine(volume: f32) -> (DosAudio, AudioEngine, MusicHandle) {
    let (ports, mut engine) = DosAudio::split(config()).unwrap();
    let sequence = Sequence::new(OplChip::Opl2, Vec::new(), Tick::from_secs_f64(1.0)).unwrap();
    let handle = engine
        .play(sequence, PlayOptions::default().volume(volume))
        .unwrap();
    (ports, engine, handle)
}

fn assert_gain(frame: Frame, expected: f32) {
    for actual in frame {
        assert!((actual - expected).abs() < 1e-6, "{actual} != {expected}");
    }
}

#[test]
fn fade_uses_old_gain_first_and_reaches_target_after_480_frames() {
    let (_, mut engine, handle) = machine(1.0);
    handle.fade_volume(0.0, FADE).unwrap();
    let mut frames = [[0.0; 2]; FADE_FRAMES + 1];
    engine.render(&mut frames);
    for (index, frame) in frames.into_iter().enumerate() {
        assert_gain(frame, 1.0 - index as f32 / FADE_FRAMES as f32);
    }
    assert_eq!(frames[0], [1.0; 2]);
    assert_eq!(frames[FADE_FRAMES], [0.0; 2]);
}

#[test]
fn fade_output_is_identical_for_whole_and_irregular_callback_buffers() {
    fn render(chunks: &[usize]) -> Vec<Frame> {
        let (_, mut engine, handle) = machine(1.0);
        handle.fade_volume(0.0, FADE).unwrap();
        let mut frames = vec![[0.0; 2]; FADE_FRAMES + 19];
        let mut offset = 0;
        for &chunk in chunks.iter().cycle() {
            let end = (offset + chunk).min(frames.len());
            engine.render(&mut frames[offset..end]);
            offset = end;
            if offset == frames.len() {
                break;
            }
        }
        frames
    }
    assert_eq!(render(&[499]), render(&[1, 7, 43, 3, 128]));
}

#[test]
fn interrupted_fade_starts_at_the_current_ramp_gain() {
    let (_, mut engine, handle) = machine(1.0);
    handle.fade_volume(0.0, FADE).unwrap();
    engine.render(&mut [[0.0; 2]; 120]);
    handle.fade_volume(0.25, FADE).unwrap();
    let mut frames = [[0.0; 2]; FADE_FRAMES + 1];
    engine.render(&mut frames);
    assert_gain(frames[0], 0.75);
    assert_gain(frames[240], 0.5);
    assert_gain(frames[480], 0.25);
}

#[test]
fn zero_duration_is_immediate_and_finite_targets_are_clamped() {
    let (_, mut engine, handle) = machine(0.5);
    let mut frame = [[0.0; 2]; 1];
    handle.fade_volume(2.0, Duration::ZERO).unwrap();
    engine.render(&mut frame);
    assert_gain(frame[0], 1.0);
    handle.fade_volume(-1.0, Duration::ZERO).unwrap();
    engine.render(&mut frame);
    assert_gain(frame[0], 0.0);
}

#[test]
fn pause_freezes_the_fade_and_resume_continues_it() {
    let (_, mut engine, handle) = machine(1.0);
    handle.fade_volume(0.0, FADE).unwrap();
    engine.render(&mut [[0.0; 2]; 120]);
    handle.pause();
    let mut paused = [[1.0; 2]; 513];
    engine.render(&mut paused);
    assert!(paused.iter().all(|frame| *frame == [0.0; 2]));
    handle.resume();
    let mut resumed = [[0.0; 2]; 121];
    engine.render(&mut resumed);
    assert_gain(resumed[0], 0.75);
    assert_gain(resumed[120], 0.5);
}

#[test]
fn latest_volume_or_fade_request_supersedes_the_previous_request() {
    let (_, mut engine, handle) = machine(1.0);
    handle.fade_volume(0.0, FADE).unwrap();
    engine.render(&mut [[0.0; 2]; 120]);
    handle.set_volume(0.2);
    let mut frame = [[0.0; 2]; 1];
    engine.render(&mut frame);
    assert_gain(frame[0], 0.2);

    handle.fade_volume(0.8, FADE).unwrap();
    handle.set_volume(0.4);
    engine.render(&mut frame);
    assert_gain(frame[0], 0.4);

    handle.set_volume(0.9);
    handle.fade_volume(0.3, FADE).unwrap();
    let mut frames = [[0.0; 2]; FADE_FRAMES + 1];
    engine.render(&mut frames);
    assert_gain(frames[0], 0.4);
    assert_gain(frames[FADE_FRAMES], 0.3);
}

#[test]
fn invalid_fades_preserve_pending_volume_and_an_existing_ramp() {
    let (_, mut engine, handle) = machine(1.0);
    handle.set_volume(0.625);
    for target in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        assert!(handle.fade_volume(target, FADE).is_err());
    }
    assert!(handle.fade_volume(0.0, Duration::MAX).is_err());
    let mut frame = [[0.0; 2]; 1];
    engine.render(&mut frame);
    assert_gain(frame[0], 0.625);

    handle.set_volume(1.0);
    engine.render(&mut frame);
    handle.fade_volume(0.0, FADE).unwrap();
    engine.render(&mut [[0.0; 2]; 120]);
    assert!(handle.fade_volume(f32::NAN, FADE).is_err());
    assert!(handle.fade_volume(1.0, Duration::MAX).is_err());
    engine.render(&mut frame);
    assert_gain(frame[0], 0.75);
}

#[test]
fn first_fade_interruption_and_pause_render_without_allocation_or_deallocation() {
    let (_, mut engine, handle) = machine(1.0);
    let mut frames = [[0.0; 2]; FADE_FRAMES + 1];
    let allocations = allocation_counter::measure(|| {
        handle.fade_volume(0.0, FADE).unwrap();
        engine.render(&mut frames[..120]);
        handle.fade_volume(0.5, FADE).unwrap();
        engine.render(&mut frames[..120]);
        handle.pause();
        engine.render(&mut frames);
        handle.resume();
        engine.render(&mut frames);
        handle.set_volume(0.2);
        engine.render(&mut frames[..1]);
    });
    assert_eq!(allocations.count_total, 0);
    assert_eq!(allocations.count_current, 0);
}

#[cfg(feature = "sb")]
#[test]
fn fading_fm_leaves_the_shared_cards_pcm_output_unchanged() {
    let sb_config = SoundConfig::sb2().sample_rate(48_000).latency(Tick::ZERO);
    let (mut plain_ports, mut plain) = DosAudio::split(sb_config.clone()).unwrap();
    let (mut ports, mut engine) = DosAudio::split(
        sb_config.opl_backend(OplBackend::Custom(Arc::new(|| Box::new(ConstantCore)))),
    )
    .unwrap();
    let sequence = Sequence::new(OplChip::Opl2, Vec::new(), Tick::from_secs_f64(1.0)).unwrap();
    let handle = engine
        .play(sequence, PlayOptions::default().volume(0.2))
        .unwrap();
    let samples = vec![160; 128];
    let _plain_sound = plain_ports.play_pcm(samples.clone(), 8_000).unwrap();
    let _sound = ports.play_pcm(samples, 8_000).unwrap();
    plain_ports.advance_to(Tick::from_micros(12_000));
    ports.advance_to(Tick::from_micros(12_000));
    handle.fade_volume(0.0, FADE).unwrap();
    let mut expected_pcm = [[0.0; 2]; 576];
    let mut actual = expected_pcm;
    plain.render(&mut expected_pcm);
    engine.render(&mut actual);
    for (index, (with_fm, pcm)) in actual.iter().zip(expected_pcm).enumerate() {
        let gain = 0.2 * (1.0 - index.min(FADE_FRAMES) as f32 / FADE_FRAMES as f32);
        assert_gain([with_fm[0] - pcm[0], with_fm[1] - pcm[1]], gain);
    }
    assert!(expected_pcm[480..].iter().any(|frame| frame[0] > 0.0));
    assert_eq!(actual[480..], expected_pcm[480..]);
}
