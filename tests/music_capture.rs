//! Original synthesis traces capture application time and stay bounded in the callback.
#![cfg(feature = "opl")]

use game_toolkit_dosaudio::{Capture, Error, MusicCapture, prelude::*};
mod common;

fn event(at: Tick, register: u16, value: u8) -> TimedEvent {
    TimedEvent {
        at,
        event: MusicEvent::Opl { register, value },
    }
}
fn out(ports: &mut DosAudio, at: Tick, register: u16, value: u8) {
    let address = if register & 0x100 == 0 { 0x388 } else { 0x38a };
    ports.try_outb_at(at, address, register as u8).unwrap();
    ports.try_outb_at(at, address + 1, value).unwrap();
}
fn registers(capture: &Capture) -> Vec<(Tick, u16, u8)> {
    let mut address = [0u8; 2];
    let mut writes = Vec::new();
    for write in capture.writes() {
        assert!((0x388..=0x38b).contains(&write.port));
        let bank = usize::from(write.port >= 0x38a);
        if write.port & 1 == 0 {
            address[bank] = write.value;
        } else {
            writes.push((
                write.at,
                u16::from(address[bank]) | ((bank as u16) << 8),
                write.value,
            ));
        }
    }
    writes
}
fn render_rt(engine: &mut AudioEngine, frames: &mut [Frame]) {
    let allocations = allocation_counter::measure(|| engine.render(frames));
    assert_eq!(allocations.count_total, 0, "capture callback allocated");
    assert_eq!(
        allocations.count_current, 0,
        "capture callback freed a resource"
    );
    assert_eq!(allocations.bytes_current, 0);
}
fn finish(reader: &mut MusicCapture, engine: &mut AudioEngine) -> Capture {
    reader.stop();
    render_rt(engine, &mut [[0.0; 2]; 1]);
    assert!(reader.is_finished());
    let result = reader.try_finish().unwrap().unwrap();
    assert!(reader.try_finish().unwrap().is_none());
    result
}

#[test]
fn capture_unifies_manual_sequence_runtime_and_note_release_writes() {
    let (mut ports, mut engine) = DosAudio::split(SoundConfig::opl3().latency(Tick(500))).unwrap();
    let mut reader = engine.start_capture(256).unwrap();
    let sequence = Sequence::new(
        OplChip::Opl3,
        vec![
            event(Tick::ZERO, 0x20, 0x21),
            event(Tick::ZERO, 0xb0, 0x31),
            event(Tick(1_000), 0xa0, 0x66),
        ],
        Tick(5_000),
    )
    .unwrap();
    let _handle = engine.play(sequence, PlayOptions::default()).unwrap();
    let mut controller = engine.music_controller(4).unwrap();
    controller
        .send_event_at(
            Tick(1_500),
            MusicEvent::Opl {
                register: 0x43,
                value: 9,
            },
        )
        .unwrap();
    controller
        .send_event_at(
            Tick(1_500),
            MusicEvent::Opl {
                register: 0x43,
                value: 10,
            },
        )
        .unwrap();
    out(&mut ports, Tick(100), 0x20, 0x11);
    out(&mut ports, Tick(2_000), 0x1a0, 0x55);
    render_rt(&mut engine, &mut [[0.0; 2]; 24]);
    assert!(!reader.is_finished());
    assert!(reader.try_finish().unwrap().is_none());
    let capture_end = engine.now();
    let capture = finish(&mut reader, &mut engine);
    let writes = registers(&capture);
    for expected in [
        (Tick::ZERO, 0x20, 0x21),
        (Tick(600), 0x20, 0x11),
        (Tick(1_000), 0xa0, 0x66),
        (Tick(1_500), 0x43, 9),
        (Tick(1_500), 0x43, 10),
        (Tick(2_500), 0x1a0, 0x55),
        (Tick(5_000), 0xb0, 0x11),
    ] {
        assert!(
            writes.contains(&expected),
            "missing {expected:?}: {writes:?}"
        );
    }
    assert!(writes.windows(2).all(|pair| pair[0].0 <= pair[1].0));
    let same_tick: Vec<_> = writes
        .iter()
        .filter(|(at, r, _)| *at == Tick(1_500) && *r == 0x43)
        .map(|(_, _, value)| *value)
        .collect();
    assert_eq!(same_tick, [9, 10]);
    assert_eq!(capture.duration(), capture_end);
    assert!(writes.iter().all(|(at, _, _)| *at < capture.duration()));
}

#[test]
fn capture_records_actual_late_application_and_excludes_future_writes_after_stop() {
    let (mut ports, mut engine) =
        DosAudio::split(SoundConfig::adlib().latency(Tick::ZERO)).unwrap();
    engine.render(&mut [[0.0; 2]; 4]);
    let start = engine.now();
    let mut reader = engine.start_capture(8).unwrap();
    // The game clock trails output; capture must use application time, not Tick(100).
    out(&mut ports, Tick(100), 0xa0, 0x55);
    out(&mut ports, Tick(10_000), 0xa0, 0x66);
    render_rt(&mut engine, &mut [[0.0; 2]; 1]);
    let capture_end = engine.now();
    let capture = finish(&mut reader, &mut engine);
    assert_eq!(registers(&capture), [(start, 0xa0, 0x55)]);
    assert_eq!(capture.duration(), capture_end - start);
    assert!(engine.stats().late_commands > 0);
    render_rt(&mut engine, &mut [[0.0; 2]; 48]);
    assert!(reader.try_finish().unwrap().is_none());
}

#[test]
fn looping_pause_resume_and_stop_capture_only_register_changes_and_key_offs() {
    let (_ports, mut engine) = DosAudio::split(SoundConfig::adlib()).unwrap();
    let mut reader = engine.start_capture(256).unwrap();
    let sequence = Sequence::new(
        OplChip::Opl2,
        vec![
            event(Tick::ZERO, 0xb0, 0x31),
            event(Tick(1_000), 0xa0, 0x98),
        ],
        Tick(2_000),
    )
    .unwrap();
    let handle = engine.play(sequence, PlayOptions::looped()).unwrap();
    render_rt(&mut engine, &mut [[0.0; 2]; 16]);
    let pause_start = engine.now();
    handle.pause();
    render_rt(&mut engine, &mut [[0.0; 2]; 4]);
    let pause_end = engine.now();
    handle.resume();
    render_rt(&mut engine, &mut [[0.0; 2]; 4]);
    let stop_at = engine.now();
    handle.stop();
    render_rt(&mut engine, &mut [[0.0; 2]; 2]);
    let capture = finish(&mut reader, &mut engine);
    let writes = registers(&capture);
    for at in [Tick::ZERO, Tick(2_000), Tick(4_000)] {
        assert!(writes.contains(&(at, 0xb0, 0x31)));
    }
    assert!(
        writes
            .iter()
            .all(|(at, _, _)| *at < pause_start || *at >= pause_end)
    );
    assert!(writes.contains(&(stop_at, 0xb0, 0x11)));
    assert!(handle.is_finished());
}

#[test]
fn replacement_onset_precedes_same_tick_live_event_in_captured_fifo() {
    let (_ports, mut engine) = DosAudio::split(SoundConfig::adlib()).unwrap();
    let mut reader = engine.start_capture(128).unwrap();
    let mut controller = engine.music_controller(2).unwrap();
    let sequence = Sequence::new(
        OplChip::Opl2,
        vec![event(Tick::ZERO, 0xa0, 4)],
        Tick(10_000),
    )
    .unwrap();
    let _handle = controller
        .play_at(Tick(100), sequence, PlayOptions::default())
        .unwrap();
    controller
        .send_event_at(
            Tick(100),
            MusicEvent::Opl {
                register: 0xa0,
                value: 12,
            },
        )
        .unwrap();
    render_rt(&mut engine, &mut [[0.0; 2]; 2]);
    let capture = finish(&mut reader, &mut engine);
    let values: Vec<_> = registers(&capture)
        .into_iter()
        .filter(|(_, register, _)| *register == 0xa0)
        .collect();
    assert_eq!(values, [(Tick(100), 0xa0, 4), (Tick(100), 0xa0, 12)]);
}

#[test]
fn queue_overflow_is_latched_and_reports_one_explicit_error() {
    let (mut ports, mut engine) =
        DosAudio::split(SoundConfig::adlib().latency(Tick::ZERO)).unwrap();
    let mut reader = engine.start_capture(1).unwrap();
    out(&mut ports, Tick::ZERO, 0x20, 1);
    out(&mut ports, Tick::ZERO, 0x23, 2);
    render_rt(&mut engine, &mut [[0.0; 2]; 2]);
    assert!(reader.is_overflowed());
    assert!(reader.is_finished());
    assert_eq!(reader.try_finish().err(), Some(Error::CaptureOverflow));
    assert!(reader.try_finish().unwrap().is_none());
    render_rt(&mut engine, &mut [[0.0; 2]; 8]);
    assert!(reader.is_overflowed());
}

#[test]
fn draining_on_control_side_prevents_overflow_and_stop_waits_for_boundary() {
    let (mut ports, mut engine) =
        DosAudio::split(SoundConfig::adlib().latency(Tick::ZERO)).unwrap();
    let mut reader = engine.start_capture(1).unwrap();
    assert!(reader.try_finish().unwrap().is_none());
    for value in 0..8 {
        out(&mut ports, engine.now(), 0xa0, value);
        render_rt(&mut engine, &mut [[0.0; 2]; 1]);
        assert!(reader.try_finish().unwrap().is_none());
    }
    assert!(!reader.is_overflowed());
    reader.stop();
    reader.stop();
    assert!(!reader.is_finished());
    assert!(reader.try_finish().unwrap().is_none());
    engine.render(&mut []);
    assert!(!reader.is_finished());
    let capture_end = engine.now();
    let capture = finish(&mut reader, &mut engine);
    assert_eq!(registers(&capture).len(), 8);
    assert_eq!(capture.duration(), capture_end);
}

#[test]
fn capture_setup_validates_capacity_hardware_and_active_reader() {
    let (_ports, mut engine) = DosAudio::split(SoundConfig::adlib()).unwrap();
    for capacity in [0, 1_048_577] {
        assert!(matches!(
            engine.start_capture(capacity),
            Err(Error::Config(_))
        ));
    }
    let mut reader = engine.start_capture(8).unwrap();
    assert!(matches!(engine.start_capture(8), Err(Error::Config(_))));
    let _capture = finish(&mut reader, &mut engine);
    let _next = engine.start_capture(8).unwrap();
    let (_ports, mut engine) = DosAudio::split(SoundConfig::none()).unwrap();
    assert!(matches!(engine.start_capture(8), Err(Error::Config(_))));
}

#[test]
fn reader_stop_drop_and_overflow_do_not_free_callback_resources() {
    for stop_first in [false, true] {
        let (mut ports, mut engine) =
            DosAudio::split(SoundConfig::adlib().latency(Tick::ZERO)).unwrap();
        let reader = engine.start_capture(1).unwrap();
        out(&mut ports, Tick::ZERO, 0x20, 1);
        out(&mut ports, Tick::ZERO, 0x23, 2);
        if stop_first {
            reader.stop();
        }
        drop(reader);
        render_rt(&mut engine, &mut [[0.0; 2]; 64]);
        let at = ports.now();
        out(&mut ports, at, 0xa0, 4);
        render_rt(&mut engine, &mut [[0.0; 2]; 64]);
    }
}

#[cfg(feature = "formats")]
#[test]
fn dro_export_replays_normalized_banked_trace_and_matches_audio_with_ms_tolerance() {
    let (mut ports, mut engine) = DosAudio::split(SoundConfig::opl3().latency(Tick::ZERO)).unwrap();
    let mut reader = engine.start_capture(256).unwrap();
    let _handle = engine
        .play(
            common::tune(OplChip::Opl3, 0x100, 0x30),
            PlayOptions::default(),
        )
        .unwrap();
    out(&mut ports, Tick::from_micros(25_000), 0x143, 12);
    let mut original = vec![[0.0; 2]; 4_800];
    render_rt(&mut engine, &mut original);
    let capture = finish(&mut reader, &mut engine);
    let sequence = Dro::load(&capture.to_dro().unwrap()).unwrap();
    assert_eq!(sequence.chip(), OplChip::Opl3);
    let captured = registers(&capture);
    assert_eq!(sequence.events().len(), captured.len());
    for (event, (at, register, value)) in sequence.events().iter().zip(captured) {
        assert_eq!(event.event, MusicEvent::Opl { register, value });
        let error = event.at.0.abs_diff(at.0);
        assert!(
            error <= Tick::from_micros(500).0 + 1,
            "DRO quantization exceeded half a millisecond"
        );
    }
    let (_ports, mut replay) = DosAudio::split(SoundConfig::opl3()).unwrap();
    let _handle = replay.play(sequence, PlayOptions::default()).unwrap();
    let mut actual = vec![[0.0; 2]; original.len()];
    replay.render(&mut actual);
    let energy: f64 = original
        .iter()
        .flatten()
        .map(|value| f64::from(*value).powi(2))
        .sum();
    let error: f64 = original
        .iter()
        .zip(actual)
        .flat_map(|(a, b)| a.iter().zip(b))
        .map(|(a, b)| (f64::from(*a) - f64::from(b)).powi(2))
        .sum();
    assert!(energy > 0.001, "fixture must produce audible energy");
    // The original 50.123 ms key-off rounds to 50 ms in DRO; allow 8% relative RMS.
    assert!((error / energy).sqrt() < 0.08);
}

#[test]
fn stop_before_first_render_captures_an_empty_half_open_interval() {
    let (mut ports, mut engine) =
        DosAudio::split(SoundConfig::adlib().latency(Tick::ZERO)).unwrap();
    let mut reader = engine.start_capture(2).unwrap();
    out(&mut ports, Tick::ZERO, 0xa0, 0x55);
    reader.stop();
    render_rt(&mut engine, &mut [[0.0; 2]; 2]);
    assert!(reader.is_finished());
    let capture = reader.try_finish().unwrap().unwrap();
    assert_eq!(capture.duration(), Tick::ZERO);
    assert!(capture.writes().is_empty());
}

#[cfg(feature = "sb")]
#[test]
fn pro_broadcast_pair_captures_both_logical_chips_at_output_time() {
    let (mut ports, mut engine) =
        DosAudio::split(SoundConfig::sb_pro().latency(Tick(500))).unwrap();
    let mut reader = engine.start_capture(8).unwrap();
    out(&mut ports, Tick::ZERO, 0x20, 0x21);
    render_rt(&mut engine, &mut [[0.0; 2]; 4]);
    let capture = finish(&mut reader, &mut engine);
    assert_eq!(
        registers(&capture),
        [(Tick(500), 0x20, 0x21), (Tick(500), 0x120, 0x21)]
    );
}

#[test]
fn captured_trace_and_end_are_independent_of_callback_chunks() {
    fn capture(chunks: &[usize]) -> Capture {
        let (mut ports, mut engine) =
            DosAudio::split(SoundConfig::adlib().latency(Tick::ZERO)).unwrap();
        let mut reader = engine.start_capture(256).unwrap();
        let _handle = engine
            .play(common::tune(OplChip::Opl2, 0, 0), PlayOptions::default())
            .unwrap();
        out(&mut ports, Tick::from_micros(25_000), 0x43, 12);
        let mut buffer = vec![[0.0; 2]; 4_800];
        let mut offset = 0;
        for &chunk in chunks.iter().cycle() {
            let end = (offset + chunk).min(buffer.len());
            render_rt(&mut engine, &mut buffer[offset..end]);
            offset = end;
            if end == buffer.len() {
                break;
            }
        }
        finish(&mut reader, &mut engine)
    }
    let whole = capture(&[4_800]);
    let chunks = capture(&[1, 7, 31, 3, 257]);
    assert_eq!(whole.writes(), chunks.writes());
    assert_eq!(whole.duration(), chunks.duration());
}
