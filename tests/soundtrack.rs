//! Multi-device selection and state reconstruction through public playback APIs.
#![cfg(feature = "opl")]
mod common;
use game_toolkit_dosaudio::{Error, Melody, prelude::*};
use std::sync::Arc;
fn fm() -> PreparedMusic {
    let source = common::tune(OplChip::Opl2, 0, 0x30);
    let mut events: Vec<_> = source
        .events()
        .iter()
        .filter(|e| e.at == Tick::ZERO)
        .copied()
        .collect();
    events.push(TimedEvent {
        at: Tick::from_micros(900_000),
        event: MusicEvent::Opl {
            register: 0xb0,
            value: 0x11,
        },
    });
    PreparedMusic::new(
        MusicTarget::Opl(OplChip::Opl2),
        events,
        Tick::from_micros(1_000_000),
        [],
    )
    .unwrap()
}
fn speaker() -> PreparedMusic {
    PreparedMusic::new(
        MusicTarget::Speaker,
        vec![
            TimedEvent {
                at: Tick::ZERO,
                event: MusicEvent::Speaker {
                    divisor: 2712,
                    enabled: true,
                },
            },
            TimedEvent {
                at: Tick::from_micros(900_000),
                event: MusicEvent::Speaker {
                    divisor: 0,
                    enabled: false,
                },
            },
        ],
        Tick::from_micros(1_000_000),
        [],
    )
    .unwrap()
}
fn soundtrack() -> Arc<Soundtrack> {
    let mut s = Soundtrack::new();
    s.insert(MusicDevice::Opl2, fm()).unwrap();
    s.insert(MusicDevice::Speaker, speaker()).unwrap();
    s.fallback(MusicDevice::Mt32, MusicDevice::Opl2).unwrap();
    Arc::new(s)
}

#[test]
fn selection_fallbacks_missing_hardware_and_cycles_are_explicit() {
    let mut s = Soundtrack::new();
    s.insert(MusicDevice::Speaker, speaker()).unwrap();
    s.fallback(MusicDevice::Mt32, MusicDevice::GeneralMidi)
        .unwrap();
    s.fallback(MusicDevice::GeneralMidi, MusicDevice::Speaker)
        .unwrap();
    assert!(s.fallback(MusicDevice::Speaker, MusicDevice::Mt32).is_err());
    let (_, mut engine) = DosAudio::split(SoundConfig::none().speaker(true)).unwrap();
    let handle = engine
        .play_soundtrack(Arc::new(s), MusicDevice::Mt32, PlayOptions::default())
        .unwrap();
    assert_eq!(handle.device(), Some(MusicDevice::Speaker));
    assert!(handle.switch_device(MusicDevice::Opl3).is_err());
    assert!(handle.switch_device(MusicDevice::Mt32).is_ok());
    let (_, mut engine) = DosAudio::split(SoundConfig::none()).unwrap();
    assert!(
        engine
            .play_soundtrack(soundtrack(), MusicDevice::Opl2, PlayOptions::default())
            .is_err()
    );
}
#[test]
fn mismatched_target_duration_loop_and_empty_tracks_fail_transactionally() {
    let mut s = Soundtrack::new();
    s.insert(MusicDevice::Opl2, fm()).unwrap();
    assert!(s.insert(MusicDevice::Speaker, fm()).is_err());
    let short: PreparedMusic = Melody::parse("T120 O3 C").unwrap().into();
    assert!(s.insert(MusicDevice::Speaker, short).is_err());
    assert!(
        s.insert(
            MusicDevice::Speaker,
            speaker().with_loop_start(Tick(1)).unwrap()
        )
        .is_err()
    );
    assert!(s.arrangement(MusicDevice::Opl2).is_some());
    assert!(s.arrangement(MusicDevice::Speaker).is_none());
    let (_, mut e) = DosAudio::split(SoundConfig::adlib()).unwrap();
    assert!(
        e.play_soundtrack(
            Arc::new(Soundtrack::new()),
            MusicDevice::Opl2,
            PlayOptions::default()
        )
        .is_err()
    );
}
#[test]
fn repeated_switches_keep_position_pause_fade_loop_and_callback_resources() {
    let (_, mut e) =
        DosAudio::split(SoundConfig::adlib().speaker(true).latency(Tick::ZERO)).unwrap();
    let handle = e
        .play_soundtrack(soundtrack(), MusicDevice::Opl2, PlayOptions::looped())
        .unwrap();
    let mut frames = [[0.0; 2]; 2400];
    e.render(&mut frames);
    let before = handle.position();
    assert!(before > Tick::ZERO);
    handle
        .fade_volume(0.5, std::time::Duration::from_millis(200))
        .unwrap();
    for device in [
        MusicDevice::Speaker,
        MusicDevice::Opl2,
        MusicDevice::Speaker,
    ] {
        handle.switch_device(device).unwrap();
        let usage = allocation_counter::measure(|| e.render(&mut frames));
        assert_eq!(usage.count_total, 0);
        assert_eq!(usage.count_current, 0);
        assert_eq!(handle.device(), Some(device));
        assert!(frames.iter().flatten().any(|v| v.abs() > 0.001));
    }
    assert!(handle.position() > before);
    handle.pause();
    e.render(&mut frames);
    let paused = handle.position();
    handle.switch_device(MusicDevice::Opl2).unwrap();
    e.render(&mut frames);
    assert_eq!(handle.position(), paused);
    assert!(frames.iter().flatten().all(|v| *v == 0.0));
    handle.resume();
    e.render(&mut frames);
    assert!(handle.position() > paused);
    assert!(!handle.is_finished());
    let mut long = vec![[0.0; 2]; 48_000];
    e.render(&mut long);
    assert!(!handle.is_finished());
    assert!(handle.position() < Tick::from_micros(1_000_000));
    handle.stop();
    e.render(&mut frames);
    assert!(handle.switch_device(MusicDevice::Speaker).is_err());
}
#[test]
fn switching_at_equal_output_positions_is_independent_of_callback_chunks() {
    let run = |chunk: usize| {
        let (_, mut e) =
            DosAudio::split(SoundConfig::adlib().speaker(true).latency(Tick::ZERO)).unwrap();
        let handle = e
            .play_soundtrack(soundtrack(), MusicDevice::Opl2, PlayOptions::default())
            .unwrap();
        let mut result = vec![[0.0; 2]; 24_000];
        for (index, segment) in result.chunks_mut(4800).enumerate() {
            if index % 2 == 1 {
                handle.switch_device(MusicDevice::Speaker).unwrap();
            } else if index > 0 {
                handle.switch_device(MusicDevice::Opl2).unwrap();
            }
            for part in segment.chunks_mut(chunk) {
                e.render(part);
            }
        }
        result
    };
    assert_eq!(run(4800), run(137));
}
#[test]
fn runtime_soundtracks_use_existing_reclamation_and_queue_backpressure() {
    let (_, mut e) = DosAudio::split(SoundConfig::adlib().speaker(true)).unwrap();
    let mut control = e.music_controller(1).unwrap();
    let h = control
        .play_soundtrack(soundtrack(), MusicDevice::Mt32, PlayOptions::default())
        .unwrap();
    assert!(matches!(
        control.play_soundtrack(soundtrack(), MusicDevice::Opl2, PlayOptions::default()),
        Err(Error::QueueFull)
    ));
    e.render(&mut [[0.0; 2]; 1]);
    h.switch_device(MusicDevice::Speaker).unwrap();
    let usage = allocation_counter::measure(|| e.render(&mut [[0.0; 2]; 128]));
    assert_eq!(usage.count_total, 0);
    assert_eq!(usage.count_current, 0);
    assert_eq!(h.device(), Some(MusicDevice::Speaker));
}

#[cfg(feature = "mpu401")]
mod midi {
    use super::*;
    use game_toolkit_dosaudio::{Frame, MidiBackend, MidiMessage, MidiSynth};
    use std::sync::atomic::{AtomicU8, Ordering};
    struct Synth {
        on: bool,
        program: u8,
        onset: Arc<AtomicU8>,
    }
    impl MidiSynth for Synth {
        fn sample_rate(&self) -> u32 {
            48_000
        }
        fn message(&mut self, d: &[u8]) {
            match d[0] {
                0xff => {
                    self.on = false;
                    self.program = 0;
                }
                0xc0 => self.program = d[1],
                0x90 => {
                    self.on = d[2] != 0;
                    self.onset.store(self.program, Ordering::Relaxed);
                }
                0x80 => self.on = false,
                0xb0 if [120, 123].contains(&d[1]) => self.on = false,
                _ => {}
            }
        }
        fn render(&mut self, out: &mut [Frame]) {
            out.fill(if self.on { [0.25, 0.0] } else { [0.0; 2] });
        }
    }
    #[test]
    fn midi_selection_requires_the_configured_backend_kind() {
        let backend = MidiBackend::Custom(Arc::new(|_| {
            Ok(Box::new(Synth {
                on: false,
                program: 0,
                onset: Arc::new(AtomicU8::new(255)),
            }))
        }));
        let (_, mut engine) = DosAudio::split(SoundConfig::none().mpu401(backend)).unwrap();
        let music = PreparedMusic::new(MusicTarget::Midi, vec![], Tick(1000), []).unwrap();
        let mut soundtrack = Soundtrack::new();
        soundtrack
            .insert(MusicDevice::GeneralMidi, music.clone())
            .unwrap();
        soundtrack.insert(MusicDevice::CustomMidi, music).unwrap();
        assert!(
            engine
                .play_soundtrack(
                    Arc::new(soundtrack.clone()),
                    MusicDevice::GeneralMidi,
                    PlayOptions::default()
                )
                .is_err()
        );
        soundtrack
            .fallback(MusicDevice::GeneralMidi, MusicDevice::CustomMidi)
            .unwrap();
        let handle = engine
            .play_soundtrack(
                Arc::new(soundtrack),
                MusicDevice::GeneralMidi,
                PlayOptions::default(),
            )
            .unwrap();
        assert_eq!(handle.device(), Some(MusicDevice::CustomMidi));
    }

    #[test]
    fn switching_into_midi_restores_held_instrument_and_mutes_old_fm_tail() {
        let onset = Arc::new(AtomicU8::new(255));
        let shared = onset.clone();
        let backend = MidiBackend::Custom(Arc::new(move |_| {
            Ok(Box::new(Synth {
                on: false,
                program: 0,
                onset: shared.clone(),
            }))
        }));
        let (_, mut e) =
            DosAudio::split(SoundConfig::adlib().mpu401(backend).latency(Tick::ZERO)).unwrap();
        let mut s = (*soundtrack()).clone();
        let msgs = [
            (0, &[0xc0, 40][..]),
            (0, &[0x90, 60, 100][..]),
            (100_000, &[0xc0, 64][..]),
            (900_000, &[0x80, 60, 0][..]),
        ];
        let events = msgs
            .into_iter()
            .map(|(t, d)| TimedEvent {
                at: Tick::from_micros(t),
                event: MusicEvent::Midi(MidiMessage::new(d).unwrap()),
            })
            .collect();
        s.insert(
            MusicDevice::CustomMidi,
            PreparedMusic::new(MusicTarget::Midi, events, Tick::from_micros(1_000_000), [])
                .unwrap(),
        )
        .unwrap();
        let h = e
            .play_soundtrack(Arc::new(s), MusicDevice::Opl2, PlayOptions::default())
            .unwrap();
        e.render(&mut vec![[0.0; 2]; 12_000]);
        h.switch_device(MusicDevice::CustomMidi).unwrap();
        let mut frames = [[0.0; 2]; 480];
        let usage = allocation_counter::measure(|| e.render(&mut frames));
        assert_eq!(usage.count_total, 0);
        assert_eq!(usage.count_current, 0);
        assert_eq!(onset.load(Ordering::Relaxed), 40);
        assert!(frames[2..].iter().all(|f| *f == [0.25, 0.0]));
        assert!(h.position() > Tick::from_micros(250_000));
    }
}

#[test]
fn switching_opl3_and_opl2_restores_compatibility_mode_and_routing() {
    let mut mono = fm();
    let events = mono
        .events()
        .iter()
        .map(|e| TimedEvent {
            at: e.at,
            event: match e.event {
                MusicEvent::Opl { register: 0xc0, .. } => MusicEvent::Opl {
                    register: 0xc0,
                    value: 0,
                },
                event => event,
            },
        })
        .collect();
    mono =
        PreparedMusic::new(MusicTarget::Opl(OplChip::Opl2), events, mono.duration(), []).unwrap();
    let mut stereo = vec![TimedEvent {
        at: Tick::ZERO,
        event: MusicEvent::Opl {
            register: 0x105,
            value: 1,
        },
    }];
    stereo.extend(mono.events().iter().map(|e| TimedEvent {
        at: e.at,
        event: match e.event {
            MusicEvent::Opl { register: 0xc0, .. } => MusicEvent::Opl {
                register: 0xc0,
                value: 0x20,
            },
            event => event,
        },
    }));
    let stereo =
        PreparedMusic::new(MusicTarget::Opl(OplChip::Opl3), stereo, mono.duration(), []).unwrap();
    let mut s = Soundtrack::new();
    s.insert(MusicDevice::Opl2, mono).unwrap();
    s.insert(MusicDevice::Opl3, stereo).unwrap();
    let (_, mut e) = DosAudio::split(SoundConfig::opl3().latency(Tick::ZERO)).unwrap();
    let h = e
        .play_soundtrack(Arc::new(s), MusicDevice::Opl3, PlayOptions::default())
        .unwrap();
    let mut frames = [[0.0; 2]; 2400];
    e.render(&mut frames);
    assert!(frames.iter().all(|f| f[0] == 0.0));
    h.switch_device(MusicDevice::Opl2).unwrap();
    e.render(&mut frames);
    assert!(frames[64..].iter().any(|f| f[0].abs() > 0.001));
    assert!(frames[64..].iter().all(|f| f[0] == f[1]));
    h.switch_device(MusicDevice::Opl3).unwrap();
    e.render(&mut frames);
    assert!(frames[64..].iter().all(|f| f[0] == 0.0));
    assert!(frames[64..].iter().any(|f| f[1].abs() > 0.001));
}

#[test]
fn excessive_checkpoint_history_is_rejected_during_preparation() {
    let events = (1..=4096)
        .map(|i| TimedEvent {
            at: Tick(i),
            event: MusicEvent::Opl {
                register: 0x40,
                value: (i & 63) as u8,
            },
        })
        .collect();
    let music =
        PreparedMusic::new(MusicTarget::Opl(OplChip::Opl2), events, Tick(5000), []).unwrap();
    let mut s = Soundtrack::new();
    assert!(matches!(
        s.insert(MusicDevice::Opl2, music),
        Err(Error::TooLarge)
    ));
    assert!(s.arrangement(MusicDevice::Opl2).is_none());
}

#[test]
fn switching_resets_opl_patch_registers_omitted_by_the_new_arrangement() {
    struct Core(u8);
    impl game_toolkit_dosaudio::OplCore for Core {
        fn write(&mut self, register: u16, value: u8) {
            if register == 0xe0 {
                self.0 = value;
            }
        }
        fn generate(&mut self) -> game_toolkit_dosaudio::Frame {
            [f32::from(self.0) / 10.0; 2]
        }
    }
    let duration = Tick::from_micros(1_000_000);
    let mut s = Soundtrack::new();
    s.insert(
        MusicDevice::Opl3,
        PreparedMusic::new(
            MusicTarget::Opl(OplChip::Opl3),
            vec![TimedEvent {
                at: Tick::ZERO,
                event: MusicEvent::Opl {
                    register: 0xe0,
                    value: 7,
                },
            }],
            duration,
            [],
        )
        .unwrap(),
    )
    .unwrap();
    s.insert(
        MusicDevice::Opl2,
        PreparedMusic::new(MusicTarget::Opl(OplChip::Opl2), vec![], duration, []).unwrap(),
    )
    .unwrap();
    let (_, mut engine) = DosAudio::split(
        SoundConfig::opl3()
            .opl_backend(OplBackend::Custom(Arc::new(|| Box::new(Core(0)))))
            .latency(Tick::ZERO),
    )
    .unwrap();
    let handle = engine
        .play_soundtrack(Arc::new(s), MusicDevice::Opl3, PlayOptions::default())
        .unwrap();
    let mut frames = [[0.0; 2]; 100];
    engine.render(&mut frames);
    assert!(frames.iter().any(|f| f[0] > 0.5));
    handle.switch_device(MusicDevice::Opl2).unwrap();
    let usage = allocation_counter::measure(|| engine.render(&mut frames));
    assert_eq!(usage.count_total, 0);
    assert_eq!(usage.count_current, 0);
    assert!(frames[2..].iter().all(|f| *f == [0.0; 2]));
}

#[test]
fn switching_at_activation_loop_boundary_and_natural_end_is_defined() {
    let (_, mut engine) =
        DosAudio::split(SoundConfig::adlib().speaker(true).latency(Tick::ZERO)).unwrap();
    let mut controller = engine.music_controller(2).unwrap();
    let handle = controller
        .play_soundtrack(soundtrack(), MusicDevice::Opl2, PlayOptions::looped())
        .unwrap();
    // An options-menu change can arrive before the queued player activates.
    handle.switch_device(MusicDevice::Speaker).unwrap();
    engine.render(&mut [[0.0; 2]; 1]);
    assert_eq!(handle.device(), Some(MusicDevice::Speaker));
    engine.render(&mut vec![[0.0; 2]; 47_999]);
    handle.switch_device(MusicDevice::Opl2).unwrap();
    let mut frames = [[0.0; 2]; 480];
    engine.render(&mut frames);
    assert_eq!(handle.device(), Some(MusicDevice::Opl2));
    assert!(!handle.is_finished());
    assert!(handle.position() < Tick::from_micros(20_000));
    assert!(frames.iter().flatten().any(|v| v.abs() > 0.001));
    drop(controller);
    let finite = engine
        .play_soundtrack(soundtrack(), MusicDevice::Speaker, PlayOptions::default())
        .unwrap();
    engine.render(&mut vec![[0.0; 2]; 48_001]);
    assert!(finite.is_finished());
    assert!(finite.switch_device(MusicDevice::Opl2).is_err());
}
