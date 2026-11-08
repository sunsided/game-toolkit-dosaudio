//! SMF fixtures are original byte sequences built here, never third-party music.
#![cfg(feature = "formats")]
use game_toolkit_dosaudio::{Error, MusicEvent, MusicTarget, PreparedMusic, Tick, formats::Smf};

fn variable(mut value: u32) -> Vec<u8> {
    assert!(value <= 0x0fff_ffff);
    let mut bytes = vec![(value & 0x7f) as u8];
    value >>= 7;
    while value != 0 {
        bytes.push((value as u8 & 0x7f) | 0x80);
        value >>= 7;
    }
    bytes.reverse();
    bytes
}
fn track(events: &[(u32, Vec<u8>)], final_delay: u32) -> Vec<u8> {
    let mut bytes = Vec::new();
    for (delta, body) in events {
        bytes.extend(variable(*delta));
        bytes.extend(body);
    }
    bytes.extend(variable(final_delay));
    bytes.extend([0xff, 0x2f, 0]);
    bytes
}
fn file(format: u16, division: u16, tracks: &[Vec<u8>]) -> Vec<u8> {
    let mut bytes = b"MThd".to_vec();
    bytes.extend(6u32.to_be_bytes());
    bytes.extend(format.to_be_bytes());
    bytes.extend((tracks.len() as u16).to_be_bytes());
    bytes.extend(division.to_be_bytes());
    for data in tracks {
        bytes.extend(b"MTrk");
        bytes.extend((data.len() as u32).to_be_bytes());
        bytes.extend(data);
    }
    bytes
}
fn tempo(micros: u32) -> Vec<u8> {
    let raw = micros.to_be_bytes();
    vec![0xff, 0x51, 3, raw[1], raw[2], raw[3]]
}
fn packet(status: u8, bytes: &[u8]) -> Vec<u8> {
    let mut result = vec![status];
    result.extend(variable(bytes.len() as u32));
    result.extend(bytes);
    result
}
fn messages(music: &PreparedMusic) -> Vec<(Tick, Vec<u8>)> {
    music
        .events()
        .iter()
        .filter_map(|event| match event.event {
            MusicEvent::Midi(message) => Some((event.at, message.bytes().to_vec())),
            _ => None,
        })
        .collect()
}

#[test]
fn type_zero_uses_default_tempo_and_keeps_final_delay_and_zero_velocity_note_on() {
    let data = file(
        0,
        480,
        &[track(
            &[
                (0, vec![0xc1, 7]),
                (480, vec![0x91, 64, 0]),
                (240, vec![0x81, 64, 3]),
            ],
            240,
        )],
    );
    let music = Smf::load(&data).unwrap();
    assert_eq!(music.target(), MusicTarget::Midi);
    assert_eq!(music.loop_start(), Tick::ZERO);
    assert_eq!(music.duration(), Tick::from_micros(1_000_000));
    assert_eq!(
        messages(&music),
        [
            (Tick::ZERO, vec![0xc1, 7]),
            (Tick::from_micros(500_000), vec![0x91, 64, 0]),
            (Tick::from_micros(750_000), vec![0x81, 64, 3]),
        ]
    );
}

#[test]
fn type_one_merges_stably_and_applies_new_tempo_after_the_old_interval() {
    let conductor = track(&[(0, tempo(500_000)), (100, tempo(1_000_000))], 100);
    let notes = track(
        &[
            (0, vec![0xc0, 1]),
            (100, vec![0x90, 60, 100]),
            (100, vec![0x80, 60, 0]),
        ],
        100,
    );
    let other = track(&[(100, vec![0xc2, 2]), (0, vec![0xc2, 3])], 0);
    let music = Smf::load(&file(1, 100, &[conductor, notes, other])).unwrap();
    assert_eq!(
        messages(&music),
        [
            (Tick::ZERO, vec![0xc0, 1]),
            (Tick::from_micros(500_000), vec![0x90, 60, 100]),
            (Tick::from_micros(500_000), vec![0xc2, 2]),
            (Tick::from_micros(500_000), vec![0xc2, 3]),
            (Tick::from_micros(1_500_000), vec![0x80, 60, 0]),
        ]
    );
    assert_eq!(music.duration(), Tick::from_micros(2_500_000));
}

#[test]
fn simultaneous_tempo_changes_follow_track_order_and_fraction_survives_changes() {
    let music = Smf::load(&file(
        1,
        10,
        &[
            track(&[(10, tempo(1_000_000))], 10),
            track(&[(10, tempo(250_000))], 10),
            track(&[(20, vec![0xc0, 1])], 0),
        ],
    ))
    .unwrap();
    assert_eq!(
        messages(&music),
        [(Tick::from_micros(750_000), vec![0xc0, 1])]
    );
    let music = Smf::load(&file(
        0,
        3,
        &[track(
            &[
                (0, tempo(1)),
                (1, tempo(2)),
                (1, tempo(3)),
                (1, tempo(4)),
                (1, tempo(5)),
                (1, vec![0xc0, 1]),
            ],
            1,
        )],
    ))
    .unwrap();
    // 15/3 us at the note, 20/3 us at EOT; rounding each interval loses ticks.
    assert_eq!(music.events()[0].at, Tick(71));
    assert_eq!(music.duration(), Tick(95));
}

#[test]
fn all_smpte_rates_include_drop_frame_rational_and_ignore_tempo() {
    for (fps, ticks, micros) in [
        (-24i8, 240, 1_000_000),
        (-25, 250, 1_000_000),
        (-29, 300, 1_001_000),
        (-30, 300, 1_000_000),
    ] {
        let division = u16::from_be_bytes([fps as u8, 10]);
        let music = Smf::load(&file(
            0,
            division,
            &[track(&[(0, tempo(1)), (ticks, vec![0xc0, 1])], ticks)],
        ))
        .unwrap();
        assert_eq!(music.events()[0].at, Tick::from_micros(micros));
        assert_eq!(music.duration(), Tick::from_micros(micros * 2));
    }
}

#[test]
fn channel_encodings_and_zero_delta_running_status_are_preserved() {
    let events = [
        vec![0x83, 60, 9],
        vec![0x93, 60, 127],
        vec![61, 0],
        vec![0xa3, 60, 2],
        vec![0xb3, 7, 99],
        vec![0xc3, 5],
        vec![6],
        vec![0xd3, 10],
        vec![0xe3, 1, 2],
    ];
    let track = track(
        &events
            .iter()
            .cloned()
            .map(|body| (0, body))
            .collect::<Vec<_>>(),
        1,
    );
    let music = Smf::load(&file(0, 1, &[track])).unwrap();
    let expected = [
        vec![0x83, 60, 9],
        vec![0x93, 60, 127],
        vec![0x93, 61, 0],
        vec![0xa3, 60, 2],
        vec![0xb3, 7, 99],
        vec![0xc3, 5],
        vec![0xc3, 6],
        vec![0xd3, 10],
        vec![0xe3, 1, 2],
    ];
    assert_eq!(
        messages(&music)
            .into_iter()
            .map(|(_, bytes)| bytes)
            .collect::<Vec<_>>(),
        expected
    );
}

#[test]
fn unknown_metadata_is_ignored_but_its_delay_is_retained() {
    let data = file(
        0,
        100,
        &[track(
            &[
                (10, vec![0xff, 0x09, 3, b'a', b'b', b'c']),
                (0, vec![0xc0, 1]),
                (10, vec![0xff, 0x7f, 0]),
            ],
            10,
        )],
    );
    let music = Smf::load(&data).unwrap();
    assert_eq!(
        messages(&music),
        [(Tick::from_micros(50_000), vec![0xc0, 1])]
    );
    assert_eq!(music.duration(), Tick::from_micros(150_000));
}

#[test]
fn split_sysex_is_complete_owned_and_emitted_at_completion_tick() {
    let music = {
        let original = file(
            0,
            100,
            &[track(
                &[
                    (0, packet(0xf0, &[0x7d, 1])),
                    (10, tempo(500_000)),
                    (10, packet(0xf7, &[2, 3, 0xf7])),
                    (0, packet(0xf0, &[4, 0xf7])),
                    (0, vec![0x90, 60, 0]),
                ],
                10,
            )],
        );
        Smf::load(&original).unwrap()
    };
    assert_eq!(music.sysex(), [0xf0, 0x7d, 1, 2, 3, 0xf7, 0xf0, 4, 0xf7]);
    assert_eq!(music.events()[0].at, Tick::from_micros(100_000));
    assert_eq!(
        music.events()[0].event,
        MusicEvent::SysEx {
            offset: 0,
            length: 6
        }
    );
    assert_eq!(
        music.events()[1].event,
        MusicEvent::SysEx {
            offset: 6,
            length: 3
        }
    );
    assert_eq!(
        messages(&music),
        [(Tick::from_micros(100_000), vec![0x90, 60, 0])]
    );
    assert_eq!(music.duration(), Tick::from_micros(150_000));
}

#[test]
fn malformed_or_unsupported_sysex_and_realtime_fail_cleanly() {
    for events in [
        vec![(0, packet(0xf0, &[1]))],
        vec![(0, packet(0xf7, &[0x90, 60, 1]))],
        vec![(0, packet(0xf0, &[1, 0xf8, 0xf7]))],
        vec![(0, packet(0xf0, &[1, 0xf7, 2]))],
        vec![(0, packet(0xf0, &[1])), (0, packet(0xf0, &[2, 0xf7]))],
        vec![
            (0, packet(0xf0, &[1])),
            (0, vec![0x90, 60, 1]),
            (0, packet(0xf7, &[0xf7])),
        ],
        vec![(0, vec![0xf8])],
    ] {
        assert!(
            Smf::load(&file(0, 96, &[track(&events, 0)])).is_err(),
            "{events:?}"
        );
    }
    let mut maximum = vec![0x7f; 65_535];
    maximum[65_534] = 0xf7;
    let music = Smf::load(&file(0, 96, &[track(&[(0, packet(0xf0, &maximum))], 0)])).unwrap();
    assert_eq!(music.sysex().len(), 65_536);
    maximum.insert(0, 1);
    assert!(matches!(
        Smf::load(&file(0, 96, &[track(&[(0, packet(0xf0, &maximum))], 0)])),
        Err(Error::TooLarge)
    ));
}

#[test]
fn running_status_is_rejected_after_metadata_or_sysex_and_without_a_status() {
    for events in [
        vec![(0, vec![60, 1])],
        vec![
            (0, vec![0x90, 60, 1]),
            (0, vec![0xff, 1, 0]),
            (0, vec![61, 1]),
        ],
        vec![
            (0, vec![0x90, 60, 1]),
            (0, packet(0xf0, &[1, 0xf7])),
            (0, vec![61, 1]),
        ],
    ] {
        assert!(Smf::load(&file(0, 96, &[track(&events, 0)])).is_err());
    }
}

#[test]
fn headers_tracks_and_every_truncated_prefix_are_strict() {
    let valid = file(0, 96, &[track(&[(0, vec![0x90, 60, 1])], 1)]);
    for end in 0..valid.len() {
        assert!(Smf::load(&valid[..end]).is_err(), "prefix {end}");
    }
    for (index, value) in [
        (0, b'x'),
        (7, 7),
        (9, 2),
        (9, 3),
        (11, 0),
        (11, 2),
        (12, 0),
        (14, b'x'),
    ] {
        let mut bad = valid.clone();
        bad[index] = value;
        // A zero high division byte is already valid; make the low byte zero too.
        if index == 12 {
            bad[13] = 0;
        }
        assert!(Smf::load(&bad).is_err(), "header index {index}");
    }
    let mut trailing = valid.clone();
    trailing.push(0);
    assert!(Smf::load(&trailing).is_err());
    let mut length = valid;
    length[18..22].copy_from_slice(&u32::MAX.to_be_bytes());
    assert!(Smf::load(&length).is_err());
    for division in [
        0u16,
        u16::from_be_bytes([(-25i8) as u8, 0]),
        u16::from_be_bytes([(-27i8) as u8, 10]),
    ] {
        assert!(Smf::load(&file(0, division, &[track(&[], 0)])).is_err());
    }
}

#[test]
fn malformed_track_grammar_cannot_be_silently_truncated_by_midly() {
    for data in [
        vec![],
        vec![0, 0x90, 60, 1],
        vec![0x81],
        vec![0, 0x90, 60],
        vec![0x81, 0x80, 0x80, 0x80, 0, 0xff, 0x2f, 0],
        vec![0, 0x90, 0x80, 1, 0, 0xff, 0x2f, 0],
        vec![0, 0xff, 0x2f, 1, 0],
        vec![0, 0xff, 0x2f, 0, 0],
        vec![0, 0xff, 0x51, 2, 1, 2, 0, 0xff, 0x2f, 0],
        vec![0, 0xff, 0x51, 3, 0, 0, 0, 0, 0xff, 0x2f, 0],
        vec![0, 0xf0, 5, 1, 0xf7, 0, 0xff, 0x2f, 0],
    ] {
        assert!(
            Smf::load(&file(0, 96, std::slice::from_ref(&data))).is_err(),
            "{data:?}"
        );
    }
}

#[test]
fn event_track_and_time_limits_report_errors_without_overflow() {
    let mut tracks = file(1, 1, &[track(&[], 0)]);
    tracks[10..12].copy_from_slice(&1_025u16.to_be_bytes());
    assert!(matches!(Smf::load(&tracks), Err(Error::TooLarge)));
    let mut events = vec![(0, tempo(0xff_ffff))];
    events.extend((0..400).map(|_| (0x0fff_ffff, vec![0xc0, 1])));
    assert!(matches!(
        Smf::load(&file(0, 1, &[track(&events, 0)])),
        Err(Error::TooLarge)
    ));
    let mut many = Vec::with_capacity(4_000_008);
    for _ in 0..1_000_001 {
        many.extend([0, 0xff, 1, 0]);
    }
    many.extend([0, 0xff, 0x2f, 0]);
    assert!(matches!(
        Smf::load(&file(0, 96, &[many])),
        Err(Error::TooLarge)
    ));
}

#[test]
fn arbitrary_short_inputs_never_panic() {
    for length in 0..256 {
        let bytes: Vec<u8> = (0..length)
            .map(|index| (index * 131 + length * 17) as u8)
            .collect();
        assert!(Smf::load(&bytes).is_err());
    }
}
