//! Public fixed-voice mixing, control lifetime, and rational resampling.
#![cfg(feature = "sb")]

use game_toolkit_dosaudio::SfxMixer;
use std::sync::Arc;

#[test]
fn four_voices_mix_and_the_fifth_steals_the_oldest() {
    let mut mixer = SfxMixer::new(4, 8_000).unwrap();
    assert_eq!(mixer.sample_rate(), 8_000);
    let handles: Vec<_> = [129, 130, 132, 136]
        .into_iter()
        .map(|value| mixer.trigger(vec![value; 4], 8_000).unwrap())
        .collect();
    let mut out = [0];
    mixer.render(&mut out);
    assert_eq!(out, [143]);
    assert_eq!(mixer.active_voices(), 4);
    let fifth = mixer.trigger(vec![144; 4], 8_000).unwrap();
    assert!(handles[0].is_finished());
    assert!(handles[1..].iter().all(|handle| !handle.is_finished()));
    mixer.render(&mut out);
    assert_eq!(out, [158]);
    assert!(!fifth.is_finished());
    assert_eq!(mixer.active_voices(), 4);
}

#[test]
fn inactive_slots_are_reused_before_active_voices_are_stolen() {
    let mut mixer = SfxMixer::new(2, 8_000).unwrap();
    let first = mixer.trigger(vec![129; 8], 8_000).unwrap();
    let stopped = mixer.trigger(vec![130; 8], 8_000).unwrap();
    stopped.stop();
    let replacement = mixer.trigger(vec![132; 8], 8_000).unwrap();
    assert!(!first.is_finished());
    assert!(stopped.is_finished());
    let newest = mixer.trigger(vec![136; 8], 8_000).unwrap();
    assert!(first.is_finished());
    assert!(!replacement.is_finished());
    assert!(!newest.is_finished());
    let mut out = [0];
    mixer.render(&mut out);
    assert_eq!(out, [140]);
}

#[test]
fn last_handle_drop_and_explicit_stop_are_observed_without_advancing_sources() {
    let mut mixer = SfxMixer::new(2, 8_000).unwrap();
    let handle = mixer.trigger(vec![160; 4], 8_000).unwrap();
    let clone = handle.clone();
    drop(handle);
    let mut out = [0];
    mixer.render(&mut out);
    assert_eq!(out, [160]);
    assert_eq!(clone.samples_played(), 1);
    assert!(!clone.is_finished());
    clone.stop();
    mixer.render(&mut out);
    assert_eq!(out, [128]);
    assert_eq!(clone.samples_played(), 1);
    assert!(clone.is_finished());
    let dropped = mixer.trigger(vec![255; 4], 8_000).unwrap();
    drop(dropped);
    mixer.render(&mut out);
    assert_eq!(out, [128]);
    assert_eq!(mixer.active_voices(), 0);
}

#[test]
fn volume_is_shared_between_clones_and_finite_values_are_clamped() {
    let mut mixer = SfxMixer::new(1, 8_000).unwrap();
    let handle = mixer.trigger(vec![192; 12], 8_000).unwrap();
    let clone = handle.clone();
    let mut out = [0];
    for (volume, expected) in [
        (0.5, 160),
        (-1.0, 128),
        (2.0, 192),
        (f32::NAN, 128),
        (f32::INFINITY, 128),
    ] {
        clone.set_volume(volume);
        mixer.render(&mut out);
        assert_eq!(out, [expected]);
    }
    assert_eq!(handle.samples_played(), 5);
    assert!(!handle.is_finished());
}

#[test]
fn mixing_clips_after_signed_summing_and_empty_mixer_is_silent() {
    let mut mixer = SfxMixer::new(4, 8_000).unwrap();
    let mut out = [0; 3];
    mixer.render(&mut out);
    assert_eq!(out, [128; 3]);
    let first = mixer.trigger([255_u8, 0, 160].as_slice(), 8_000).unwrap();
    let second = mixer.trigger([255_u8, 0, 96].as_slice(), 8_000).unwrap();
    mixer.render(&mut out);
    assert_eq!(out, [255, 0, 128]);
    assert!(first.is_finished());
    assert!(second.is_finished());
    mixer.render(&mut out);
    assert_eq!(out, [128; 3]);
}

#[test]
fn zero_order_hold_uses_exact_rational_phase_and_counts_elapsed_source_samples() {
    let mut mixer = SfxMixer::new(1, 7).unwrap();
    let upsampled = mixer.trigger([129_u8, 130, 131].as_slice(), 3).unwrap();
    let mut out = [0; 7];
    mixer.render(&mut out[..2]);
    assert_eq!(upsampled.samples_played(), 0);
    mixer.render(&mut out[2..]);
    assert_eq!(out, [129, 129, 129, 130, 130, 131, 131]);
    assert_eq!(upsampled.samples_played(), 3);
    assert!(upsampled.is_finished());

    let mut mixer = SfxMixer::new(1, 2).unwrap();
    let downsampled = mixer
        .trigger([129_u8, 130, 131, 132, 133, 134].as_slice(), 4)
        .unwrap();
    let mut out = [0; 4];
    mixer.render(&mut out);
    assert_eq!(out, [129, 131, 133, 128]);
    assert_eq!(downsampled.samples_played(), 6);
    assert!(downsampled.is_finished());
}

#[test]
fn fractional_rate_does_not_drift_over_one_second() {
    let mut mixer = SfxMixer::new(1, 48_000).unwrap();
    let handle = mixer.trigger(vec![137; 11_025], 11_025).unwrap();
    let mut out = [0; 48_000];
    mixer.render(&mut out[..47_999]);
    assert_eq!(handle.samples_played(), 11_024);
    assert!(!handle.is_finished());
    mixer.render(&mut out[47_999..]);
    assert!(out.iter().all(|&value| value == 137));
    assert_eq!(handle.samples_played(), 11_025);
    assert!(handle.is_finished());
}

#[test]
fn rendering_is_chunk_independent_across_completion() {
    let samples: Arc<[u8]> = (0..113).map(|index| (index * 31) as u8).collect();
    let mut whole = SfxMixer::new(2, 11_025).unwrap();
    let mut split = SfxMixer::new(2, 11_025).unwrap();
    let whole_handle = whole.trigger(Arc::clone(&samples), 8_000).unwrap();
    let split_handle = split.trigger(samples, 8_000).unwrap();
    let mut expected = [0; 200];
    let mut actual = [0; 200];
    whole.render(&mut expected);
    for chunk in actual.chunks_mut(7) {
        split.render(chunk);
    }
    assert_eq!(actual, expected);
    assert_eq!(whole_handle.samples_played(), split_handle.samples_played());
    assert!(whole_handle.is_finished());
    assert!(split_handle.is_finished());
}

#[test]
fn invalid_inputs_do_not_steal_a_playing_voice() {
    for voices in [0, 1025, usize::MAX] {
        assert!(SfxMixer::new(voices, 8_000).is_err());
    }
    for rate in [0, 192_001, u32::MAX] {
        assert!(SfxMixer::new(1, rate).is_err());
    }
    assert!(SfxMixer::new(1024, 192_000).is_ok());
    let mut mixer = SfxMixer::new(1, 8_000).unwrap();
    let handle = mixer.trigger(vec![192; 8], 8_000).unwrap();
    for rate in [0, 192_001, u32::MAX] {
        assert!(mixer.trigger(vec![0], rate).is_err());
    }
    assert!(mixer.trigger([].as_slice(), 8_000).is_err());
    assert!(!handle.is_finished());
    assert_eq!(handle.samples_played(), 0);
    let mut out = [0];
    mixer.render(&mut out);
    assert_eq!(out, [192]);
}

#[test]
fn render_never_allocates_or_frees_including_source_completion_and_stop() {
    let mut mixer = SfxMixer::new(2, 8_000).unwrap();
    // Both sample allocations are owned solely by the mixer during measurement.
    let finished = mixer.trigger(vec![255; 3], 8_000).unwrap();
    let stopped = mixer.trigger(vec![0; 3], 8_000).unwrap();
    drop(stopped);
    let mut out = [0; 8];
    let allocations = allocation_counter::measure(|| {
        mixer.render(&mut out[..1]);
        finished.set_volume(0.5);
        mixer.render(&mut out[1..]);
    });
    assert_eq!(allocations.count_total, 0);
    assert_eq!(allocations.count_current, 0);
    assert_eq!(allocations.bytes_total, 0);
    assert_eq!(allocations.bytes_current, 0);
    assert!(finished.is_finished());
    assert_eq!(out, [255, 191, 191, 128, 128, 128, 128, 128]);
}

#[test]
fn completed_buffers_are_retained_until_replacement_and_mixer_drop_finishes_handles() {
    let samples: Arc<[u8]> = Arc::from([192_u8]);
    let mut mixer = SfxMixer::new(1, 8_000).unwrap();
    let completed = mixer.trigger(Arc::clone(&samples), 8_000).unwrap();
    mixer.render(&mut [0]);
    assert!(completed.is_finished());
    assert_eq!(Arc::strong_count(&samples), 2);
    let replacement = mixer.trigger(vec![255; 8], 8_000).unwrap();
    assert_eq!(Arc::strong_count(&samples), 1);
    assert!(!replacement.is_finished());
    drop(mixer);
    assert!(replacement.is_finished());
}
