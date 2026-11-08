//! Buffer playback must preserve the public DSP protocol and render identically.
#![cfg(feature = "sb")]
use game_toolkit_dosaudio::{Error, prelude::*};
use std::sync::Arc;
mod common;

const CHANNEL: DmaChannel = DmaChannel::new(1).unwrap();

fn machine() -> (DosAudio, AudioEngine) {
    DosAudio::split(SoundConfig::sb2().latency(Tick::ZERO)).unwrap()
}

#[test]
fn buffer_pcm_matches_raw_dma_including_the_last_sample_hold_and_irq() {
    let (mut buffered, mut buffer_engine) = machine();
    let (mut manual, mut manual_engine) = machine();
    let sfx = buffered.play_pcm([0u8, 255].as_slice(), 8_000).unwrap();
    assert_eq!(sfx.sample_rate(), 8_000.0);
    assert!(buffered.next_dma_tick().is_none());
    assert!(buffered.dma_request(Tick::from_micros(125)).is_none());
    for byte in [0xd1, 0x40, 131, 0x14, 1, 0] {
        manual.outb(0x22c, byte);
    }
    for (index, sample) in [0, 255].into_iter().enumerate() {
        let at = Tick::from_micros((index as u64 + 1) * 125);
        buffered.try_advance_to(at).unwrap();
        manual.try_dma_write_at(at, CHANNEL, sample).unwrap();
        assert_eq!(sfx.samples_played(), index + 1);
        assert_eq!(buffered.irq_pending(at), index == 1);
        assert!(!sfx.is_finished());
    }
    let end = Tick::from_micros(375);
    buffered.try_advance_to(end).unwrap();
    manual.try_outb_at(end, 0x22c, 0xd3).unwrap();
    assert!(sfx.is_finished());
    let mut actual = [[0.0; 2]; 32];
    let mut expected = actual;
    buffer_engine.render(&mut actual);
    manual_engine.render(&mut expected);
    assert_eq!(actual, expected);
    assert!(actual.iter().any(|frame| frame[0] == -1.0));
    assert!(actual.iter().any(|frame| frame[0] > 0.9));
    assert_eq!(actual[31], [0.0; 2]);
    assert!(buffered.irq_pending(end)); // Automatic mute leaves the completion IRQ latched.
    buffered.inb(0x22e);
    assert!(!buffered.irq_pending(end));
}

#[test]
fn queue_overflow_preserves_the_entire_advance_and_port_transaction() {
    let (mut audio, mut engine) =
        DosAudio::split(SoundConfig::sb2().latency(Tick::ZERO).queue_capacity(3)).unwrap();
    let sfx = audio.play_pcm([0u8, 255].as_slice(), 8_000).unwrap();
    let end = Tick::from_micros(375);
    assert_eq!(audio.try_advance_to(end), Err(Error::QueueFull));
    assert_eq!(audio.try_outb_at(end, 0x22c, 0xd3), Err(Error::QueueFull));
    assert_eq!(audio.try_inb_at(end, 0x22e), Err(Error::QueueFull));
    assert_eq!(audio.now(), Tick::ZERO);
    assert_eq!(sfx.samples_played(), 0);
    assert!(!sfx.is_finished());
    assert!(!audio.irq_pending(end));
    engine.render(&mut [[0.0; 2]]);
    audio.try_advance_to(end).unwrap();
    assert!(sfx.is_finished());
    assert_eq!(sfx.samples_played(), 2);
    assert!(audio.irq_pending(end));
}

#[test]
fn stopping_or_dropping_the_last_clone_mutes_on_the_control_clock() {
    let (mut audio, mut engine) = machine();
    let sfx = audio
        .play_pcm([0u8, 255, 0, 255].as_slice(), 8_000)
        .unwrap();
    let clone = sfx.clone();
    drop(sfx);
    audio.advance_to(Tick::from_micros(125));
    assert_eq!(clone.samples_played(), 1);
    assert!(!clone.is_finished());
    drop(clone);
    audio.advance_to(Tick::from_micros(1_000));
    assert!(!audio.irq_pending(audio.now()));
    let mut frames = [[1.0; 2]; 48];
    engine.render(&mut frames);
    // Stop is stamped at 125 us, before any further DMA samples or full hold interval.
    assert_eq!(frames, [[0.0; 2]; 48]);

    let sfx = audio.play_pcm([255u8; 2].as_slice(), 8_000).unwrap();
    sfx.stop();
    assert!(sfx.is_finished());
    audio.advance_to(audio.now());
    assert_eq!(sfx.samples_played(), 0);
    assert!(audio.next_dma_tick().is_none());
}

#[test]
fn stop_queue_failure_keeps_the_buffer_owned_and_retryable() {
    let (mut audio, mut engine) =
        DosAudio::split(SoundConfig::sb2().latency(Tick::ZERO).queue_capacity(1)).unwrap();
    let samples: Arc<[u8]> = Arc::from([0u8, 255]);
    let sfx = audio.play_pcm(Arc::clone(&samples), 8_000).unwrap();
    sfx.stop();
    assert_eq!(audio.try_advance_to(Tick(10)), Err(Error::QueueFull));
    assert_eq!(audio.now(), Tick::ZERO);
    assert_eq!(Arc::strong_count(&samples), 2);
    engine.render(&mut [[0.0; 2]]);
    audio.try_advance_to(Tick(10)).unwrap();
    assert_eq!(Arc::strong_count(&samples), 1);
    assert_eq!(sfx.samples_played(), 0);
}

#[test]
fn explicit_dsp_programming_releases_the_buffer_but_fm_writes_do_not() {
    let (mut audio, _) = machine();
    let sfx = audio.play_pcm([0u8; 4].as_slice(), 8_000).unwrap();
    audio
        .try_outb_at(Tick::from_micros(125), 0x388, 0x20)
        .unwrap();
    audio.outb(0x389, 1);
    assert!(!sfx.is_finished());
    assert_eq!(sfx.samples_played(), 1);
    assert!(
        audio
            .try_dma_write_at(Tick::from_micros(250), CHANNEL, 255)
            .is_err()
    );
    audio.outb(0x22c, 0xd0); // Manual port I/O takes over the DSP and halts DMA.
    assert!(sfx.is_finished());
    audio.outb(0x22c, 0xd4);
    assert!(audio.next_dma_tick().is_some());
    audio
        .try_dma_write_at(audio.next_dma_tick().unwrap(), CHANNEL, 255)
        .unwrap();
}

#[test]
fn invalid_or_backpressured_starts_preserve_existing_playback() {
    let (mut audio, _) =
        DosAudio::split(SoundConfig::sb2().latency(Tick::ZERO).queue_capacity(1)).unwrap();
    let first = audio.play_pcm([128u8; 2].as_slice(), 8_000).unwrap();
    for rate in [0, 3_999, 23_001, u32::MAX] {
        assert!(audio.play_pcm([128u8].as_slice(), rate).is_err());
    }
    assert!(audio.play_pcm([].as_slice(), 8_000).is_err());
    assert!(audio.play_pcm(vec![128; 65_537], 8_000).is_err());
    assert!(matches!(
        audio.play_pcm([0u8].as_slice(), 8_000),
        Err(Error::QueueFull)
    ));
    assert!(!first.is_finished());
    assert_eq!(first.samples_played(), 0);
    let (mut absent, _) = DosAudio::split(SoundConfig::none()).unwrap();
    assert!(absent.play_pcm([128u8].as_slice(), 8_000).is_err());
}

#[test]
fn replacement_and_control_drop_finish_handles_and_release_samples_off_thread() {
    let (mut audio, mut engine) = machine();
    let samples: Arc<[u8]> = Arc::from([0u8, 255]);
    let first = audio.play_pcm(Arc::clone(&samples), 8_000).unwrap();
    assert_eq!(Arc::strong_count(&samples), 2);
    let second = audio.play_pcm([128u8].as_slice(), 11_025).unwrap();
    assert!(first.is_finished());
    assert_eq!(Arc::strong_count(&samples), 1);
    assert_eq!(second.sample_rate(), 1_000_000.0 / 91.0);
    drop(audio);
    assert!(second.is_finished());
    let allocation = allocation_counter::measure(|| engine.render(&mut [[0.0; 2]; 128]));
    assert_eq!(allocation.count_total, 0);
    assert_eq!(allocation.count_current, 0);
}

fn capture(chunks: &[u64], capacity: usize) -> Vec<u8> {
    let mut machine =
        OfflineMachine::new(SoundConfig::sb2().queue_capacity(capacity), 48_000).unwrap();
    let samples: Arc<[u8]> = (0..256)
        .map(|i| if i % 2 == 0 { 32 } else { 224 })
        .collect();
    let sfx = machine.play_pcm(samples, 11_025).unwrap();
    for &micros in chunks {
        machine.advance_to(Tick::from_micros(micros)).unwrap();
    }
    assert!(sfx.is_finished());
    assert_eq!(sfx.samples_played(), 256);
    assert!(machine.irq_request().unwrap().asserted);
    machine.finish().to_bytes().unwrap()
}

#[test]
fn offline_pcm_is_chunk_independent_and_runs_with_one_queue_slot() {
    let whole = capture(&[30_000], 16_384);
    assert_eq!(
        whole,
        capture(&[1, 7, 91, 2_000, 5_111, 13_000, 30_000], 16_384)
    );
    assert_eq!(whole, capture(&[30_000], 1));
    assert!(whole[44..].iter().any(|&byte| byte != 0));
}

#[test]
fn live_and_offline_buffer_output_match_without_callback_allocations() {
    let (mut audio, mut engine) = machine();
    let sfx = audio.play_pcm([0u8, 255].as_slice(), 8_000).unwrap();
    audio.advance_to(Tick::from_micros(1_000));
    let mut actual = [[0.0; 2]; 48];
    let allocation = allocation_counter::measure(|| {
        for frames in actual.chunks_mut(7) {
            engine.render(frames);
        }
    });
    assert_eq!(allocation.count_total, 0);
    assert_eq!(allocation.count_current, 0);
    assert!(sfx.is_finished());
    assert_eq!(engine.stats().late_commands, 0);
    let mut offline = OfflineMachine::new(SoundConfig::sb2(), 48_000).unwrap();
    let _handle = offline.play_pcm([0u8, 255].as_slice(), 8_000).unwrap();
    // Rounded crystal time needs one more tick for all 48 complete output samples.
    offline
        .advance_to(Tick::from_micros(1_000) + Tick(1))
        .unwrap();
    assert_eq!(offline.finish().frames(), actual);
}

#[test]
fn render_to_end_handles_pcm_alone_and_retains_a_complete_last_sample() {
    let mut offline = OfflineMachine::new(SoundConfig::sb2(), 48_000).unwrap();
    let handle = offline.play_pcm([255u8].as_slice(), 8_000).unwrap();
    let wav = offline.render_to_end().unwrap();
    assert!(handle.is_finished());
    assert_eq!(handle.samples_played(), 1);
    let energy: f64 = wav.frames().iter().map(|frame| f64::from(frame[0])).sum();
    assert!((energy - 6.0 * 127.0 / 128.0).abs() < 0.01);
    assert_eq!(*wav.frames().last().unwrap(), [0.0; 2]);
}

#[test]
fn held_reset_and_unrepresentable_end_are_rejected_transactionally() {
    let (mut audio, _) = machine();
    audio.outb(0x226, 1);
    assert!(audio.play_pcm([128u8].as_slice(), 8_000).is_err());
    audio.wait_us(3);
    audio.outb(0x226, 0);
    assert_eq!(audio.inb(0x22a), 0xaa);
    audio.advance_to(Tick::MAX);
    assert!(matches!(
        audio.play_pcm([128u8].as_slice(), 8_000),
        Err(Error::TooLarge)
    ));
    assert_eq!(audio.now(), Tick::MAX);
}

#[test]
fn render_to_end_waits_for_both_fm_music_and_pcm() {
    let mut offline = OfflineMachine::new(SoundConfig::sb2(), 48_000).unwrap();
    let music = offline.play(common::tune(OplChip::Opl2, 0, 0)).unwrap();
    let sound = offline.play_pcm(vec![192; 1_600], 8_000).unwrap();
    let wav = offline.render_to_end().unwrap();
    assert!(music.is_finished());
    assert!(sound.is_finished());
    assert_eq!(sound.samples_played(), 1_600);
    // 200.125 ms of PCM followed by the usual 250 ms release tail.
    assert!((21_604..=21_606).contains(&wav.frames().len()));
    assert!(wav.frames().iter().any(|frame| frame[0].abs() > 0.4));
}

#[test]
fn music_mute_does_not_mute_the_shared_cards_pcm() {
    let (mut ports, mut with_music) = machine();
    let (mut plain_ports, mut plain) = machine();
    let _music = with_music
        .play(
            common::tune(OplChip::Opl2, 0, 0),
            PlayOptions::default().volume(0.0),
        )
        .unwrap();
    let _sfx = ports.play_pcm([0u8, 255].as_slice(), 8_000).unwrap();
    let _plain_sfx = plain_ports.play_pcm([0u8, 255].as_slice(), 8_000).unwrap();
    ports.advance_to(Tick::from_micros(1_000));
    plain_ports.advance_to(Tick::from_micros(1_000));
    let mut actual = [[0.0; 2]; 48];
    let mut expected = actual;
    with_music.render(&mut actual);
    plain.render(&mut expected);
    assert_eq!(actual, expected);
}

#[test]
fn render_to_end_stops_at_the_current_clock_after_a_stop_request() {
    let mut offline = OfflineMachine::new(SoundConfig::sb2(), 48_000).unwrap();
    let sound = offline.play_pcm(vec![255; 65_536], 4_000).unwrap();
    sound.stop();
    let wav = offline.render_to_end().unwrap();
    assert!((11_999..=12_000).contains(&wav.frames().len()));
    assert_eq!(sound.samples_played(), 0);
    assert!(wav.frames().iter().all(|frame| *frame == [0.0; 2]));
}
