//! Generic prepared music validates device targets and owned event/payload metadata.
use game_toolkit_dosaudio::music::{MusicTarget, PreparedMusic};
use game_toolkit_dosaudio::{Error, MidiMessage, MusicEvent, OplChip, Sequence, Tick, TimedEvent};
use std::sync::Arc;

fn event(at: u64, event: MusicEvent) -> TimedEvent {
    TimedEvent {
        at: Tick(at),
        event,
    }
}
fn prepare(
    target: MusicTarget,
    events: Vec<TimedEvent>,
    duration: u64,
) -> game_toolkit_dosaudio::Result<PreparedMusic> {
    PreparedMusic::new(target, events, Tick(duration), Vec::<u8>::new())
}
fn sysex(offset: u32, length: u32, bytes: &[u8]) -> game_toolkit_dosaudio::Result<PreparedMusic> {
    PreparedMusic::new(
        MusicTarget::Midi,
        vec![event(0, MusicEvent::SysEx { offset, length })],
        Tick(10),
        bytes,
    )
}

#[test]
fn all_device_targets_preserve_metadata_and_equal_tick_fifo() {
    let note = MidiMessage::new(&[0x90, 60, 100]).unwrap();
    for (target, events, payload) in [
        (
            MusicTarget::Opl(OplChip::Opl2),
            vec![
                event(
                    0,
                    MusicEvent::Opl {
                        register: 0xa0,
                        value: 1,
                    },
                ),
                event(
                    0,
                    MusicEvent::Opl {
                        register: 0xa0,
                        value: 2,
                    },
                ),
            ],
            vec![],
        ),
        (
            MusicTarget::Midi,
            vec![
                event(0, MusicEvent::Midi(note)),
                event(
                    5,
                    MusicEvent::SysEx {
                        offset: 0,
                        length: 3,
                    },
                ),
            ],
            vec![0xf0, 0x7d, 0xf7],
        ),
        (
            MusicTarget::Psg,
            vec![
                event(0, MusicEvent::Psg(0x90)),
                event(0, MusicEvent::Psg(0x3f)),
            ],
            vec![],
        ),
        (
            MusicTarget::Speaker,
            vec![
                event(
                    0,
                    MusicEvent::Speaker {
                        divisor: 0,
                        enabled: true,
                    },
                ),
                event(
                    5,
                    MusicEvent::Speaker {
                        divisor: u16::MAX,
                        enabled: false,
                    },
                ),
            ],
            vec![],
        ),
        (
            MusicTarget::OplPsg(OplChip::Opl3),
            vec![
                event(0, MusicEvent::Psg(0x90)),
                event(
                    0,
                    MusicEvent::Opl {
                        register: 0x1a0,
                        value: 3,
                    },
                ),
            ],
            vec![],
        ),
    ] {
        let music = PreparedMusic::new(target, events.clone(), Tick(5), payload.clone()).unwrap();
        assert_eq!(music.events(), events);
        assert_eq!(music.sysex(), payload);
        assert_eq!(music.target(), target);
        assert_eq!(music.duration(), Tick(5));
        assert_eq!(music.loop_start(), Tick::ZERO);
    }
}

#[test]
fn each_target_rejects_other_devices_including_mixed_target_midi_and_speaker() {
    let midi = MidiMessage::new(&[0xc0, 3]).unwrap();
    let events = [
        MusicEvent::Opl {
            register: 0xa0,
            value: 1,
        },
        MusicEvent::Midi(midi),
        MusicEvent::SysEx {
            offset: 0,
            length: 2,
        },
        MusicEvent::Psg(0xff),
        MusicEvent::Speaker {
            divisor: 1,
            enabled: true,
        },
    ];
    for (target, accepted) in [
        (
            MusicTarget::Opl(OplChip::Opl2),
            [true, false, false, false, false],
        ),
        (MusicTarget::Midi, [false, true, true, false, false]),
        (MusicTarget::Psg, [false, false, false, true, false]),
        (MusicTarget::Speaker, [false, false, false, false, true]),
        (
            MusicTarget::OplPsg(OplChip::DualOpl2),
            [true, false, false, true, false],
        ),
    ] {
        for (event, expected) in events.into_iter().zip(accepted) {
            let result = PreparedMusic::new(
                target,
                vec![TimedEvent {
                    at: Tick::ZERO,
                    event,
                }],
                Tick(1),
                [0xf0, 0xf7],
            );
            assert_eq!(result.is_ok(), expected, "{target:?}: {event:?}");
        }
    }
}

#[test]
fn opl_register_limits_apply_to_pure_and_combined_targets() {
    for chip in [OplChip::Opl2, OplChip::DualOpl2, OplChip::Opl3] {
        for target in [MusicTarget::Opl(chip), MusicTarget::OplPsg(chip)] {
            for register in [0, 0xff, 0x100, 0x1ff, 0x200, u16::MAX] {
                let accepted = register <= 0x1ff && (chip != OplChip::Opl2 || register <= 0xff);
                let result = prepare(
                    target,
                    vec![event(
                        0,
                        MusicEvent::Opl {
                            register,
                            value: 0xff,
                        },
                    )],
                    1,
                );
                assert_eq!(result.is_ok(), accepted, "{target:?}: {register:#x}");
            }
        }
    }
}

#[test]
fn decreasing_and_out_of_duration_events_fail_without_reordering_equal_ticks() {
    for events in [
        vec![event(2, MusicEvent::Psg(1)), event(1, MusicEvent::Psg(2))],
        vec![event(4, MusicEvent::Psg(1))],
    ] {
        assert!(matches!(
            prepare(MusicTarget::Psg, events, 3),
            Err(Error::Format(_))
        ));
    }
    let events = vec![
        event(0, MusicEvent::Psg(3)),
        event(3, MusicEvent::Psg(2)),
        event(3, MusicEvent::Psg(1)),
    ];
    assert_eq!(
        prepare(MusicTarget::Psg, events.clone(), 3)
            .unwrap()
            .events(),
        events
    );
    let maximal = prepare(
        MusicTarget::Psg,
        vec![event(u64::MAX, MusicEvent::Psg(0))],
        u64::MAX,
    )
    .unwrap();
    assert_eq!(maximal.duration(), Tick::MAX);
}

#[test]
fn sysex_ranges_allow_shared_storage_and_require_complete_seven_bit_messages() {
    let bytes = [0xff, 0xf0, 0, 0x7f, 0xf7, 0xfe, 0xf0, 0xf7];
    let events = vec![
        event(
            0,
            MusicEvent::SysEx {
                offset: 1,
                length: 4,
            },
        ),
        event(
            1,
            MusicEvent::SysEx {
                offset: 6,
                length: 2,
            },
        ),
    ];
    let music = PreparedMusic::new(MusicTarget::Midi, events.clone(), Tick(2), bytes).unwrap();
    assert_eq!(music.sysex(), bytes);
    assert_eq!(music.events(), events);
    for bytes in [
        vec![0, 0, 0xf7],
        vec![0xf0, 0, 0],
        vec![0xf0, 0x80, 0xf7],
        vec![0xf0, 0xf0, 0xf7],
        vec![0xf0, 0xf7, 0xf7],
        vec![0xf0, 0xf8, 0xf7],
        vec![0xf0, 0xff, 0xf7],
    ] {
        assert!(sysex(0, bytes.len() as u32, &bytes).is_err(), "{bytes:?}");
    }
    assert!(sysex(0, 2, &[0xf0, 0xf7]).is_ok());
    // Real-time MIDI is a separate complete event rather than an interior SysEx byte.
    let clock = MidiMessage::new(&[0xf8]).unwrap();
    let events = vec![
        event(
            0,
            MusicEvent::SysEx {
                offset: 0,
                length: 2,
            },
        ),
        event(0, MusicEvent::Midi(clock)),
    ];
    assert!(PreparedMusic::new(MusicTarget::Midi, events, Tick(1), [0xf0, 0xf7]).is_ok());
}

#[test]
fn sysex_lengths_offsets_and_integer_overflow_are_checked_before_slicing() {
    for (offset, length) in [
        (0, 0),
        (0, 1),
        (0, 65_537),
        (0, u32::MAX),
        (1, 2),
        (3, 2),
        (u32::MAX - 2, 2),
    ] {
        assert!(
            sysex(offset, length, &[0xf0, 0xf7]).is_err(),
            "offset={offset}, length={length}"
        );
    }
    for offset in [u32::MAX, u32::MAX - 1] {
        assert!(matches!(
            sysex(offset, 2, &[0xf0, 0xf7]),
            Err(Error::TooLarge)
        ));
    }
    let mut maximum = vec![0x7f; 65_536];
    maximum[0] = 0xf0;
    maximum[65_535] = 0xf7;
    assert!(sysex(0, 65_536, &maximum).is_ok());
    maximum.insert(1, 0);
    assert!(sysex(0, 65_537, &maximum).is_err());
}

#[test]
fn repeat_region_is_metadata_strictly_before_duration_and_zero_duration_stays_finite() {
    let music = prepare(
        MusicTarget::Psg,
        vec![event(0, MusicEvent::Psg(1)), event(5, MusicEvent::Psg(2))],
        10,
    )
    .unwrap();
    for start in [Tick::ZERO, Tick(5), Tick(9)] {
        let region = music.clone().with_loop_start(start).unwrap();
        assert_eq!(region.loop_start(), start);
        assert_eq!(region.events(), music.events());
        assert_eq!(region.duration(), Tick(10));
    }
    for start in [Tick(10), Tick(11), Tick::MAX] {
        assert!(music.clone().with_loop_start(start).is_err());
    }
    let empty = prepare(MusicTarget::Psg, Vec::new(), 0).unwrap();
    assert_eq!(empty.loop_start(), Tick::ZERO);
    assert!(empty.with_loop_start(Tick::ZERO).is_err());
    let at_zero = prepare(
        MusicTarget::Speaker,
        vec![event(
            0,
            MusicEvent::Speaker {
                divisor: 0,
                enabled: false,
            },
        )],
        0,
    )
    .unwrap();
    assert_eq!(at_zero.duration(), Tick::ZERO);
}

#[test]
fn clones_and_loop_metadata_share_event_and_owned_payload_allocations() {
    let storage: Arc<[u8]> = Arc::from([0xf0, 0x7d, 1, 0xf7]);
    let music = PreparedMusic::new(
        MusicTarget::Midi,
        vec![event(
            0,
            MusicEvent::SysEx {
                offset: 0,
                length: 4,
            },
        )],
        Tick(10),
        Arc::clone(&storage),
    )
    .unwrap();
    assert_eq!(music.sysex().as_ptr(), storage.as_ptr());
    assert_eq!(Arc::strong_count(&storage), 2);
    let clone = music.clone().with_loop_start(Tick(5)).unwrap();
    assert_eq!(clone.events().as_ptr(), music.events().as_ptr());
    assert_eq!(clone.sysex().as_ptr(), storage.as_ptr());
    assert_eq!(Arc::strong_count(&storage), 3);
    drop(music);
    assert_eq!(clone.sysex(), storage.as_ref());
    assert_eq!(clone.loop_start(), Tick(5));
    assert_eq!(Arc::strong_count(&storage), 2);
}

#[test]
fn existing_opl_sequences_convert_without_copying_or_changing_their_public_contract() {
    for chip in [OplChip::Opl2, OplChip::DualOpl2, OplChip::Opl3] {
        let register = if chip == OplChip::Opl2 { 0xa0 } else { 0x1a0 };
        let sequence = Sequence::new(
            chip,
            vec![event(
                0,
                MusicEvent::Opl {
                    register,
                    value: 0x98,
                },
            )],
            Tick(10),
        )
        .unwrap();
        let original_events = sequence.events().as_ptr();
        let prepared = PreparedMusic::from(sequence.clone());
        assert_eq!(sequence.chip(), chip);
        assert_eq!(prepared.target(), MusicTarget::Opl(chip));
        assert_eq!(prepared.duration(), sequence.duration());
        assert_eq!(prepared.events(), sequence.events());
        assert_eq!(prepared.events().as_ptr(), original_events);
        assert!(prepared.sysex().is_empty());
        assert_eq!(prepared.loop_start(), Tick::ZERO);
    }
}
