//! PLAY compilation and independent speaker playback through the public API.

use game_toolkit_dosaudio::{
    Melody, MusicEvent, MusicTarget, PreparedMusic, prelude::*, speaker_pwm,
};

fn ticks(numerator: u64, denominator: u64) -> Tick {
    Tick(
        (u128::from(Tick::HZ_NUMERATOR) * u128::from(numerator)
            / (u128::from(Tick::HZ_DENOMINATOR) * u128::from(denominator))) as u64,
    )
}

fn prepared(source: &str) -> PreparedMusic {
    Melody::parse(source).unwrap().into()
}

fn pitches(music: &PreparedMusic) -> Vec<u16> {
    music
        .events()
        .iter()
        .filter_map(|event| match event.event {
            MusicEvent::Speaker {
                divisor,
                enabled: true,
            } => Some(divisor),
            _ => None,
        })
        .collect()
}

#[test]
fn play_defaults_lengths_rests_and_dots_have_exact_cumulative_duration() {
    assert_eq!(Melody::parse("C").unwrap().duration(), ticks(1, 2));
    assert_eq!(Melody::parse("L4 C8 D").unwrap().duration(), ticks(3, 4));
    assert_eq!(
        Melody::parse("T120 O3 L4 C P8 D8.").unwrap().duration(),
        ticks(9, 8)
    );
    assert_eq!(
        Melody::parse("T120 L4 C.. P8.").unwrap().duration(),
        ticks(3, 2)
    );
    let rests = prepared("N0 P4");
    assert_eq!(rests.target(), MusicTarget::Speaker);
    assert_eq!(rests.duration(), ticks(1, 1));
    assert!(
        rests
            .events()
            .iter()
            .all(|event| matches!(event.event, MusicEvent::Speaker { enabled: false, .. }))
    );
}

#[test]
fn articulation_preserves_note_lengths_and_inserts_the_correct_gaps() {
    let music = prepared("T120 O3 L4 MN C ML D MS E");
    let off: Vec<_> = music
        .events()
        .iter()
        .filter_map(|event| match event.event {
            MusicEvent::Speaker { enabled: false, .. } => Some(event.at),
            _ => None,
        })
        .collect();
    assert_eq!(off, [ticks(7, 16), ticks(1, 1), ticks(11, 8), ticks(3, 2)]);
    assert_eq!(music.duration(), ticks(3, 2));
}

#[test]
fn pitches_octaves_accidentals_and_absolute_notes_follow_play_notation() {
    assert_eq!(pitches(&prepared("O3 A")), pitches(&prepared("N46")));
    let a = pitches(&prepared("O3 A"))[0];
    let hz = Tick::HZ_NUMERATOR as f64 / (Tick::HZ_DENOMINATOR as f64 * 12.0 * f64::from(a));
    assert!((hz - 440.0).abs() < 0.2);
    assert_eq!(
        pitches(&prepared("O3 C# C+ D-")),
        [pitches(&prepared("O3 C#"))[0]; 3]
    );
    let shifted = pitches(&prepared("O2 B > C < B"));
    assert_eq!(shifted[0], shifted[2]);
    assert!(shifted[1] < shifted[0]);
    assert_eq!(
        pitches(&prepared("O0 C O6 B")),
        pitches(&prepared("N1 N84"))
    );
}

#[test]
fn repeated_notes_and_tempo_changes_retain_fractional_crystal_ticks() {
    let source = format!("T137 L7 ML {}", "C".repeat(1_000));
    let melody = Melody::parse(&source).unwrap();
    assert_eq!(melody.duration(), ticks(240_000, 137 * 7));
    assert_ne!(melody.duration(), Tick(ticks(240, 137 * 7).0 * 1_000));
    assert_eq!(
        Melody::parse("T120 L4 C T240 C").unwrap().duration(),
        ticks(3, 4)
    );
}

#[test]
fn modes_are_case_insensitive_and_foreground_policy_is_explicitly_asynchronous() {
    let foreground = prepared("MF O3 C4 D8");
    let background = prepared("mb o 3 c4 d8");
    assert_eq!(foreground.events(), background.events());
    assert_eq!(foreground.duration(), background.duration());
    assert_eq!(Melody::parse("").unwrap().duration(), Tick::ZERO);
}

#[test]
fn malformed_unsupported_and_out_of_range_play_input_is_rejected() {
    for source in [
        "é",
        "Xsong$",
        "O=VAR",
        "T",
        "L",
        "P",
        "N",
        "M",
        "MX",
        "O7C",
        "O0<C",
        "O6>C",
        "T31C",
        "T256C",
        "L0C",
        "L65C",
        "C0",
        "C65",
        "P0",
        "P65",
        "N85",
        "N0#",
        "C.........",
        "O0C-",
        "O6B#",
        "C;D",
        "123",
        "T999999999999999999999999C",
    ] {
        assert!(Melody::parse(source).is_err(), "accepted {source:?}");
    }
}

#[test]
fn pwm_events_preserve_duty_area_phase_and_final_disconnect() {
    let music = speaker_pwm(&[0, 64, 128, 255], 1_000).unwrap();
    assert_eq!(music.target(), MusicTarget::Speaker);
    assert_eq!(music.duration(), ticks(4, 1_000));
    let mut high = false;
    let mut previous = Tick::ZERO;
    let mut high_ticks = 0;
    for timed in music.events() {
        if high {
            high_ticks += timed.at.0 - previous.0;
        }
        let MusicEvent::SpeakerLevel(level) = timed.event else {
            panic!("PWM emitted PIT tone");
        };
        high = level;
        previous = timed.at;
    }
    assert!(!high);
    assert_eq!(previous, music.duration());
    let ideal = Tick::HZ_NUMERATOR as f64 / (Tick::HZ_DENOMINATOR as f64 * 1_000.0)
        * (64.0 + 128.0 + 255.0)
        / 255.0;
    assert!((high_ticks as f64 - ideal).abs() < 4.0);
    let silence = speaker_pwm(&[0; 4], 44_100).unwrap();
    assert!(
        silence
            .events()
            .iter()
            .all(|event| event.event == MusicEvent::SpeakerLevel(false))
    );
    assert_eq!(speaker_pwm(&[], 8_000).unwrap().duration(), Tick::ZERO);
}

#[test]
fn pwm_checks_input_bounds_before_preparing_events() {
    assert!(speaker_pwm(&[128], 0).is_err());
    assert!(speaker_pwm(&[128], 192_001).is_err());
    assert!(speaker_pwm(&vec![0; 1_000_001], 8_000).is_err());
}

fn render(music: PreparedMusic, chunks: &[usize]) -> Vec<Frame> {
    let (_, mut engine) =
        DosAudio::split(SoundConfig::none().speaker(true).latency(Tick::ZERO)).unwrap();
    let _handle = engine.play_speaker(music, PlayOptions::default()).unwrap();
    let mut frames = vec![[0.0; 2]; 4_800];
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

#[test]
fn melody_is_audible_in_tune_and_independent_of_callback_chunks() {
    let melody = prepared("ML T120 O3 A4");
    let frames = render(melody.clone(), &[4_800]);
    assert_eq!(frames, render(melody, &[1, 7, 83, 511]));
    assert!(frames.iter().any(|frame| frame[0].abs() > 0.01));
    let edges = frames
        .windows(2)
        .filter(|pair| pair[0][0] <= 0.0 && pair[1][0] > 0.0)
        .count();
    assert!(
        (42..=45).contains(&edges),
        "unexpected pitch: {edges} rising edges"
    );
}

#[test]
fn pwm_is_audible_chunk_independent_and_silent_for_zero_duty() {
    let pwm = speaker_pwm(&[0, 64, 128, 192, 255, 192, 128, 64].repeat(64), 8_000).unwrap();
    let frames = render(pwm.clone(), &[4_800]);
    assert_eq!(frames, render(pwm, &[1, 3, 37, 511]));
    assert!(frames.iter().any(|frame| frame[0].abs() > 0.01));
    assert!(
        render(speaker_pwm(&[0; 512], 8_000).unwrap(), &[4_800])
            .iter()
            .all(|frame| *frame == [0.0; 2])
    );
}

#[test]
fn play_speaker_at_waits_for_its_start_tick_and_rejects_the_past() {
    let (_, mut engine) =
        DosAudio::split(SoundConfig::none().speaker(true).latency(Tick::ZERO)).unwrap();
    let start_frame = engine.sample_rate() as usize / 20;
    let _handle = engine
        .play_speaker_at(
            prepared("ML T120 O3 A4"),
            PlayOptions::default(),
            ticks(1, 20),
        )
        .unwrap();
    let mut frames = vec![[0.0; 2]; 2 * start_frame];
    engine.render(&mut frames);
    assert!(
        frames[..start_frame - 1]
            .iter()
            .all(|frame| *frame == [0.0; 2])
    );
    assert!(
        frames[start_frame..]
            .iter()
            .any(|frame| frame[0].abs() > 0.01)
    );
    assert!(matches!(
        engine.play_speaker_at(prepared("A"), PlayOptions::default(), Tick::ZERO),
        Err(game_toolkit_dosaudio::Error::BackwardsTime)
    ));
}

#[test]
fn pit_owns_timer_ports_irq0_latches_and_the_refresh_bit() {
    let (mut dos, _engine) = DosAudio::split(SoundConfig::none().speaker(true)).unwrap();
    let second = Tick(Tick::HZ_NUMERATOR / Tick::HZ_DENOMINATOR);
    assert_eq!(dos.pit().unwrap().irq0_edges(Tick::ZERO, second), 18);
    assert_eq!(dos.inb(0x61) & 0x10, 0);
    dos.advance(Tick(216)); // one channel 1 refresh period
    assert_eq!(dos.inb(0x61) & 0x10, 0x10);

    dos.outb(0x43, 0x36);
    dos.outb(0x40, 0x9c);
    dos.outb(0x40, 0x2e); // divisor 11932: 100 Hz
    let start = dos.now();
    let period = Tick(11_932 * 12);
    assert_eq!(dos.pit().unwrap().next_irq0(start), Some(start + period));
    assert_eq!(
        dos.pit()
            .unwrap()
            .irq0_edges(start, start + Tick(period.0 * 100)),
        100
    );
    dos.outb(0x43, 0x00); // latch channel 0
    assert_eq!([dos.inb(0x40), dos.inb(0x40)], [0x9c, 0x2e]);
    assert_eq!(
        dos.speaker_tone(),
        None,
        "channel 0 leaves the speaker alone"
    );

    dos.outb(0x43, 0xb0); // channel 2 mode 0 is a one-shot, not a tone
    dos.outb(0x42, 0xa9);
    dos.outb(0x42, 0x04);
    dos.outb(0x61, 0x03);
    assert_eq!(dos.speaker_tone(), None);

    let (absent, _engine) = DosAudio::split(SoundConfig::none()).unwrap();
    assert!(absent.pit().is_none());
}

#[test]
fn speaker_tone_reports_gated_pit_tones_only() {
    let (mut dos, _engine) = DosAudio::split(SoundConfig::none().speaker(true)).unwrap();
    assert_eq!(dos.speaker_tone(), None);
    dos.outb(0x43, 0xb6);
    dos.outb(0x42, 0xa9);
    dos.outb(0x42, 0x04); // divisor 1193
    dos.outb(0x61, 0x03);
    let hz = dos.speaker_tone().unwrap();
    assert!((hz - 1_000.15).abs() < 0.01, "{hz}");
    dos.outb(0x61, 0x02); // gate off: bit-banged data
    assert_eq!(dos.speaker_tone(), None);
    dos.outb(0x61, 0x01);
    assert_eq!(dos.speaker_tone(), None);
    let (absent, _engine) = DosAudio::split(SoundConfig::none()).unwrap();
    assert_eq!(absent.speaker_tone(), None);
}

#[test]
fn speaker_handle_tone_follows_notes_rests_pause_and_end() {
    let (_, mut engine) =
        DosAudio::split(SoundConfig::none().speaker(true).latency(Tick::ZERO)).unwrap();
    let melody = prepared("ML T120 O3 A4 P4");
    let divisor = u32::from(pitches(&melody)[0]);
    let expected =
        Tick::HZ_NUMERATOR as f64 / (Tick::HZ_DENOMINATOR * 12) as f64 / f64::from(divisor);
    let handle = engine.play_speaker(melody, PlayOptions::default()).unwrap();
    let quarter = engine.sample_rate() as usize / 2;
    let mut frames = vec![[0.0; 2]; quarter / 2];
    engine.render(&mut frames);
    let hz = handle.tone().unwrap();
    assert!((hz - expected).abs() < 1e-9, "{hz}");
    handle.pause();
    assert_eq!(handle.tone(), None);
    handle.resume();
    engine.render(&mut frames);
    engine.render(&mut frames);
    assert_eq!(handle.tone(), None, "rest");
    engine.render(&mut vec![[0.0; 2]; quarter]);
    assert!(handle.is_finished());
    assert_eq!(handle.tone(), None);

    let pwm = speaker_pwm(&[255, 0].repeat(64), 8_000).unwrap();
    let handle = engine.play_speaker(pwm, PlayOptions::default()).unwrap();
    engine.render(&mut frames[..16]);
    assert_eq!(handle.tone(), None);
}

#[test]
fn speaker_handles_pause_resume_volume_stop_and_clone_lifetime() {
    let (_, mut engine) = DosAudio::split(SoundConfig::none().speaker(true)).unwrap();
    let handle = engine
        .play_speaker(Melody::parse("ML O3 A1").unwrap(), PlayOptions::default())
        .unwrap();
    let clone = handle.clone();
    let mut frames = [[0.0; 2]; 128];
    engine.render(&mut frames);
    handle.pause();
    assert!(handle.is_paused());
    engine.render(&mut frames);
    let position = handle.position();
    engine.render(&mut frames);
    assert_eq!(handle.position(), position);
    assert!(frames.iter().all(|frame| *frame == [0.0; 2]));
    handle.resume();
    engine.render(&mut frames);
    assert!(handle.position() > position);
    handle.set_volume(0.0);
    engine.render(&mut frames);
    assert!(frames.iter().all(|frame| *frame == [0.0; 2]));
    drop(handle);
    assert!(!clone.is_finished());
    clone.stop();
    assert!(clone.is_finished());
    engine.render(&mut frames);
}

#[test]
fn melody_and_pwm_first_use_and_controls_render_without_allocation_or_deallocation() {
    for music in [
        prepared("ML O3 A4"),
        speaker_pwm(&[64, 128, 255, 0].repeat(64), 8_000).unwrap(),
    ] {
        let (_, mut engine) = DosAudio::split(SoundConfig::none().speaker(true)).unwrap();
        let handle = engine.play_speaker(music, PlayOptions::default()).unwrap();
        let mut frames = [[0.0; 2]; 1_024];
        let allocations = allocation_counter::measure(|| {
            engine.render(&mut frames);
            handle.pause();
            engine.render(&mut frames);
            handle.resume();
            engine.render(&mut frames);
            handle.stop();
            engine.render(&mut frames);
        });
        assert_eq!(allocations.count_total, 0);
        assert_eq!(allocations.count_current, 0);
    }
}

#[cfg(feature = "opl")]
#[test]
fn fm_and_speaker_handles_control_independent_tracks() {
    use game_toolkit_dosaudio::OplCore;
    use std::sync::Arc;
    struct Constant;
    impl OplCore for Constant {
        fn write(&mut self, _: u16, _: u8) {}
        fn generate(&mut self) -> Frame {
            [0.25; 2]
        }
    }
    let config = SoundConfig::adlib()
        .speaker(true)
        .latency(Tick::ZERO)
        .opl_backend(OplBackend::Custom(Arc::new(|| Box::new(Constant))));
    let (_, mut engine) = DosAudio::split(config).unwrap();
    let fm = engine
        .play(
            Sequence::new(OplChip::Opl2, Vec::new(), ticks(1, 1)).unwrap(),
            PlayOptions::default(),
        )
        .unwrap();
    let speaker = engine
        .play_speaker(Melody::parse("ML O3 A1").unwrap(), PlayOptions::default())
        .unwrap();
    let mut frames = [[0.0; 2]; 128];
    speaker.pause();
    engine.render(&mut frames);
    assert!(frames.iter().all(|frame| *frame == [0.25; 2]));
    speaker.resume();
    fm.pause();
    engine.render(&mut frames);
    assert!(frames.iter().any(|frame| frame[0].abs() > 0.01));
    assert!(fm.is_paused());
    assert!(!speaker.is_paused());
    assert!(speaker.position() > Tick::ZERO);
}
