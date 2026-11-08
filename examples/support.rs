#![allow(dead_code)]
use game_toolkit_dosaudio::prelude::*;

/// An original unsigned 8-bit chirp, rising from 220 to 880 Hz over 600 ms.
pub fn pcm() -> std::sync::Arc<[u8]> {
    (0..4_800)
        .map(|index| {
            let time = f64::from(index) / 8_000.0;
            let envelope = (f64::from(index.min(4_799 - index)) / 160.0).min(1.0);
            let phase = std::f64::consts::TAU * (220.0 * time + 550.0 * time * time);
            (128.0 + 96.0 * envelope * phase.sin()).round() as u8
        })
        .collect()
}

pub fn tune() -> Sequence {
    let mut events = Vec::new();
    let patch = [
        (0x20, 1),
        (0x23, 1),
        (0x40, 0x25),
        (0x43, 0),
        (0x60, 0xf3),
        (0x63, 0xf2),
        (0x80, 0x77),
        (0x83, 0x77),
        (0xc0, 0),
    ];
    for (register, value) in patch {
        events.push(TimedEvent {
            at: Tick::ZERO,
            event: MusicEvent::Opl { register, value },
        });
    }
    for (index, fnum) in [342u16, 384, 432, 456, 512, 576, 512, 456]
        .into_iter()
        .enumerate()
    {
        let start = index as u64 * 150_000;
        for (register, value) in [(0xa0, fnum as u8), (0xb0, 0x30 | (fnum >> 8) as u8)] {
            events.push(TimedEvent {
                at: Tick::from_micros(start),
                event: MusicEvent::Opl { register, value },
            });
        }
        events.push(TimedEvent {
            at: Tick::from_micros(start + 120_000),
            event: MusicEvent::Opl {
                register: 0xb0,
                value: 0x10 | (fnum >> 8) as u8,
            },
        });
    }
    Sequence::new(OplChip::Opl2, events, Tick::from_micros(1_200_000)).unwrap()
}
