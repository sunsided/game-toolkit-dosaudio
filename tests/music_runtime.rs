//! Public runtime requests transfer prepared music without callback resource destruction.
#![cfg(feature = "opl")]

use game_toolkit_dosaudio::{Error, MusicController, OplCore, prelude::*};
use std::sync::Arc;

const RATE: u32 = 48_000;

struct RegisterCore(f32);
impl OplCore for RegisterCore {
    fn write(&mut self, register: u16, value: u8) {
        if register == 0xa0 {
            self.0 = f32::from(value) / 16.0;
        } else if register == 0xb0 && value & 0x20 == 0 {
            self.0 = 0.0;
        }
    }
    fn generate(&mut self) -> Frame {
        [self.0; 2]
    }
}

fn config() -> SoundConfig {
    SoundConfig::adlib()
        .sample_rate(RATE)
        .latency(Tick::ZERO)
        .opl_backend(OplBackend::Custom(Arc::new(|| Box::new(RegisterCore(0.0)))))
}
fn write(register: u16, value: u8) -> MusicEvent {
    MusicEvent::Opl { register, value }
}
fn track(value: u8) -> Sequence {
    Sequence::new(
        OplChip::Opl2,
        vec![TimedEvent {
            at: Tick::ZERO,
            event: write(0xa0, value),
        }],
        Tick::from_micros(1_000_000),
    )
    .unwrap()
}
fn machine(capacity: usize) -> (DosAudio, AudioEngine, MusicController) {
    let (ports, mut engine) = DosAudio::split(config()).unwrap();
    let controller = engine.music_controller(capacity).unwrap();
    (ports, engine, controller)
}
fn assert_level(frame: Frame, expected: f32) {
    for value in frame {
        assert!((value - expected).abs() < 1e-6, "{value} != {expected}");
    }
}
fn render_rt(engine: &mut AudioEngine, frames: &mut [Frame]) {
    let allocations = allocation_counter::measure(|| engine.render(frames));
    assert_eq!(allocations.count_total, 0, "callback allocated");
    assert_eq!(allocations.count_current, 0, "callback freed a resource");
    assert_eq!(allocations.bytes_current, 0);
}

#[test]
fn prepared_replacements_finish_old_handles_and_reclaim_only_on_control_side() {
    let (_ports, mut engine, mut controller) = machine(2);
    let first = controller.play(track(4), PlayOptions::default()).unwrap();
    let mut frames = [[0.0; 2]; 8];
    render_rt(&mut engine, &mut frames);
    assert_level(frames[7], 0.25);
    assert!(!first.is_finished());
    let second = controller.play(track(12), PlayOptions::default()).unwrap();
    render_rt(&mut engine, &mut frames);
    assert_level(frames[7], 0.75);
    assert!(first.is_finished());
    assert!(!second.is_finished());
    let reclamation = allocation_counter::measure(|| assert_eq!(controller.reclaim(), 1));
    assert_eq!(reclamation.count_total, 0);
    assert!(
        reclamation.count_current < 0,
        "retired sequence was not freed on control side"
    );
    assert!(reclamation.bytes_current < 0);
    assert_eq!(controller.reclaim(), 0);
}

#[test]
fn scheduled_replacement_weights_both_gains_inside_one_output_sample() {
    let (_ports, mut engine, mut controller) = machine(2);
    let first = controller
        .play(track(8), PlayOptions::default().volume(0.25))
        .unwrap();
    let mut warmup = [[0.0; 2]; 4];
    engine.render(&mut warmup);
    assert_level(warmup[3], 0.125);
    let width = Tick::HZ_NUMERATOR as f64 / (f64::from(RATE) * Tick::HZ_DENOMINATOR as f64);
    let frame_start = engine.stats().frames as f64 * width;
    let at = engine.now() + Tick(100);
    let second = controller
        .play_at(at, track(8), PlayOptions::default().volume(0.75))
        .unwrap();
    let mut frames = [[0.0; 2]; 3];
    render_rt(&mut engine, &mut frames);
    let old_fraction = (at.0 as f64 - frame_start) / width;
    let expected = 0.125 * old_fraction + 0.375 * (1.0 - old_fraction);
    assert_level(frames[0], expected as f32);
    assert_level(frames[2], 0.375);
    assert!(first.is_finished());
    assert!(!second.is_finished());
    assert_eq!(engine.stats().late_commands, 0);
}

#[test]
fn live_events_are_allocation_free_and_equal_timestamps_preserve_fifo() {
    let (_ports, mut engine, mut controller) = machine(4);
    let handle = controller.play(track(4), PlayOptions::default()).unwrap();
    engine.render(&mut [[0.0; 2]; 4]);
    let at = controller.now() + Tick(100);
    let allocations = allocation_counter::measure(|| {
        controller.send_event_at(at, write(0xa0, 8)).unwrap();
        controller.send_event_at(at, write(0xa0, 12)).unwrap();
    });
    assert_eq!(allocations.count_total, 0);
    assert_eq!(allocations.count_current, 0);
    let mut frames = [[0.0; 2]; 8];
    render_rt(&mut engine, &mut frames);
    assert_level(frames[7], 0.75);
    controller.send_event(write(0xa0, 2)).unwrap();
    render_rt(&mut engine, &mut frames);
    assert_level(frames[7], 0.125);
    assert!(!handle.is_finished());
}

#[test]
fn live_event_after_equal_timestamp_replacement_wins_in_fifo_order() {
    let (_ports, mut engine, mut controller) = machine(2);
    let handle = controller
        .play_at(Tick::ZERO, track(4), PlayOptions::default())
        .unwrap();
    controller
        .send_event_at(Tick::ZERO, write(0xa0, 12))
        .unwrap();
    let mut frames = [[0.0; 2]; 4];
    render_rt(&mut engine, &mut frames);
    assert_level(frames[3], 0.75);
    assert!(!handle.is_finished());
}

#[test]
fn queue_overflow_keeps_current_track_and_does_not_advance_schedule_watermark() {
    let (_ports, mut engine, mut controller) = machine(1);
    let current = controller.play(track(4), PlayOptions::default()).unwrap();
    let mut frames = [[0.0; 2]; 40];
    engine.render(&mut frames[..4]);
    controller
        .send_event_at(Tick(10_000), write(0x20, 1))
        .unwrap();
    assert!(matches!(
        controller.play_at(Tick(20_000), track(12), PlayOptions::default()),
        Err(Error::QueueFull)
    ));
    assert_eq!(
        controller.send_event_at(Tick(20_000), write(0xa0, 12)),
        Err(Error::QueueFull)
    );
    assert_eq!(
        controller.send_event_at(Tick(9_000), write(0xa0, 12)),
        Err(Error::BackwardsTime)
    );
    assert!(!current.is_finished());
    render_rt(&mut engine, &mut frames);
    assert_level(frames[39], 0.25);
    // Failed requests at 20,000 ticks must not reject this earlier valid request.
    let replacement = controller
        .play_at(Tick(15_000), track(8), PlayOptions::default())
        .unwrap();
    render_rt(&mut engine, &mut frames);
    assert!(current.is_finished());
    assert!(!replacement.is_finished());
    assert_level(frames[39], 0.5);
}

#[test]
fn invalid_requests_preserve_playback_and_allow_earlier_valid_followups() {
    let (_ports, mut engine, mut controller) = machine(4);
    let current = controller.play(track(4), PlayOptions::default()).unwrap();
    let mut frames = [[0.0; 2]; 8];
    engine.render(&mut frames);
    let now = controller.now();
    assert!(matches!(
        controller.play_at(Tick::ZERO, track(12), PlayOptions::default()),
        Err(Error::BackwardsTime)
    ));
    assert_eq!(
        controller.send_event_at(Tick::ZERO, write(0xa0, 12)),
        Err(Error::BackwardsTime)
    );
    for register in [0x100, 0x200, u16::MAX] {
        assert!(matches!(
            controller.send_event_at(now + Tick(2_000), write(register, 12)),
            Err(Error::Config(_))
        ));
    }
    let wrong_chip = Sequence::new(OplChip::Opl3, Vec::new(), Tick(100)).unwrap();
    assert!(matches!(
        controller.play(wrong_chip, PlayOptions::default()),
        Err(Error::Config(_))
    ));
    let empty = Sequence::new(OplChip::Opl2, Vec::new(), Tick::ZERO).unwrap();
    assert!(matches!(
        controller.play(empty, PlayOptions::looped()),
        Err(Error::Config(_))
    ));
    assert!(matches!(
        controller.play(track(12), PlayOptions::default().volume(f32::NAN)),
        Err(Error::Config(_))
    ));
    assert!(matches!(
        controller.play_at(Tick::MAX, track(12), PlayOptions::default()),
        Err(Error::TooLarge)
    ));
    assert!(!current.is_finished());
    render_rt(&mut engine, &mut frames);
    assert_level(frames[7], 0.25);
    controller
        .send_event_at(controller.now() + Tick(100), write(0xa0, 8))
        .unwrap();
    render_rt(&mut engine, &mut frames);
    assert_level(frames[7], 0.5);
    assert!(!current.is_finished());
}

#[test]
fn controller_construction_validates_capacity_hardware_and_uniqueness() {
    let (_ports, mut engine) = DosAudio::split(config()).unwrap();
    for capacity in [0, 1_048_577] {
        assert!(matches!(
            engine.music_controller(capacity),
            Err(Error::Config(_))
        ));
    }
    let _controller = engine.music_controller(1).unwrap();
    assert!(matches!(engine.music_controller(1), Err(Error::Config(_))));
    let (_ports, mut engine) = DosAudio::split(SoundConfig::none()).unwrap();
    assert!(matches!(engine.music_controller(1), Err(Error::Config(_))));
}

#[test]
fn published_clock_is_last_completed_frame_floor_and_empty_render_does_not_move_it() {
    let (_ports, mut engine, controller) = machine(1);
    assert_eq!(controller.now(), Tick::ZERO);
    for count in [1usize, 7, 31] {
        engine.render(&mut vec![[0.0; 2]; count]);
        let expected = Tick(
            (u128::from(engine.stats().frames) * u128::from(Tick::HZ_NUMERATOR)
                / (u128::from(RATE) * u128::from(Tick::HZ_DENOMINATOR))) as u64,
        );
        assert_eq!(controller.now(), expected);
        assert_eq!(controller.now(), engine.now());
        engine.render(&mut []);
        assert_eq!(controller.now(), expected);
    }
}

#[test]
fn scheduled_music_and_live_events_are_independent_of_callback_chunks() {
    fn capture(chunks: &[usize]) -> Vec<Frame> {
        let (_ports, mut engine, mut controller) = machine(8);
        let first = controller
            .play_at(Tick::ZERO, track(4), PlayOptions::default())
            .unwrap();
        controller.send_event_at(Tick(500), write(0xa0, 8)).unwrap();
        let second = controller
            .play_at(Tick(1_700), track(12), PlayOptions::default().volume(0.5))
            .unwrap();
        controller
            .send_event_at(Tick(3_100), write(0xa0, 4))
            .unwrap();
        let third = controller
            .play_at(Tick(5_200), track(8), PlayOptions::default().volume(0.75))
            .unwrap();
        let mut frames = vec![[0.0; 2]; 128];
        let mut offset = 0;
        for &chunk in chunks.iter().cycle() {
            let end = (offset + chunk).min(frames.len());
            render_rt(&mut engine, &mut frames[offset..end]);
            offset = end;
            if end == frames.len() {
                break;
            }
        }
        assert!(first.is_finished());
        assert!(second.is_finished());
        assert!(!third.is_finished());
        assert_level(frames[127], 0.375);
        frames
    }
    assert_eq!(capture(&[128]), capture(&[1, 7, 3, 31, 2]));
}

#[test]
fn repeated_replacement_stop_and_controller_drop_never_allocate_or_free_in_render() {
    let (_ports, mut engine, mut controller) = machine(2);
    let mut current = controller.play(track(4), PlayOptions::default()).unwrap();
    let mut frames = [[0.0; 2]; 8];
    render_rt(&mut engine, &mut frames);
    for value in [8, 12, 2, 4] {
        let replacement = controller
            .play(track(value), PlayOptions::default())
            .unwrap();
        render_rt(&mut engine, &mut frames);
        assert!(current.is_finished());
        current = replacement;
        assert_level(frames[7], f32::from(value) / 16.0);
    }
    current.stop();
    render_rt(&mut engine, &mut frames);
    assert_level(frames[7], 0.0);
    let pending = controller.play(track(12), PlayOptions::default()).unwrap();
    let last = controller.play(track(8), PlayOptions::default()).unwrap();
    // Fill the incoming queue, then abandon its producer and retirement consumer.
    // Both replacements must retain old resources in the surviving callback queues.
    drop(controller);
    render_rt(&mut engine, &mut frames);
    assert!(pending.is_finished());
    assert_level(frames[7], 0.5);
    drop(pending);
    drop(last);
    render_rt(&mut engine, &mut frames);
    assert_level(frames[7], 0.0);
}

#[test]
fn full_retirement_queue_retains_players_until_controller_reclaims() {
    let (_ports, mut engine) = DosAudio::split(config()).unwrap();
    let original = engine.play(track(2), PlayOptions::default()).unwrap();
    let mut controller = engine.music_controller(2).unwrap();
    let first = controller.play(track(4), PlayOptions::default()).unwrap();
    let second = controller.play(track(8), PlayOptions::default()).unwrap();
    let mut frames = [[0.0; 2]; 8];
    render_rt(&mut engine, &mut frames);
    assert!(original.is_finished());
    assert!(first.is_finished());
    assert!(!second.is_finished());
    assert_level(frames[7], 0.5);
    render_rt(&mut engine, &mut frames);
    assert_eq!(controller.reclaim(), 2);
    assert_eq!(controller.reclaim(), 0);
}

#[test]
fn immediate_request_behind_scheduled_replacement_waits_in_fifo() {
    let (_ports, mut engine) = DosAudio::split(config()).unwrap();
    let original = engine.play(track(8), PlayOptions::default()).unwrap();
    let mut controller = engine.music_controller(2).unwrap();
    let scheduled = controller
        .play_at(Tick(5_000), track(4), PlayOptions::default())
        .unwrap();
    controller.send_event(write(0xa0, 12)).unwrap();
    let mut early = [[0.0; 2]; 8];
    render_rt(&mut engine, &mut early);
    assert_level(early[7], 0.5);
    assert!(!original.is_finished());
    let mut later = [[0.0; 2]; 16];
    render_rt(&mut engine, &mut later);
    assert_level(later[15], 0.75);
    assert!(original.is_finished());
    assert!(!scheduled.is_finished());
}
