//! Original FM fixtures shared by integration tests.
#![allow(dead_code)]
use game_toolkit_dosaudio::{MusicEvent, OplChip, Sequence, Tick, TimedEvent};

/// A short original phrase with explicit instrument and key-on/key-off writes.
pub fn tune(chip: OplChip, bank: u16, pan: u8) -> Sequence {
    let registers = [
        (0x20, 1),
        (0x23, 1),
        (0x40, 0x3f),
        (0x43, 0),
        (0x60, 0xf0),
        (0x63, 0xf0),
        (0x80, 0x77),
        (0x83, 0x77),
        (0xc0, pan | 1),
        (0xa0, 0x98),
        (0xb0, 0x31),
    ];
    let mut events = Vec::new();
    if chip == OplChip::Opl3 {
        events.push(TimedEvent {
            at: Tick::ZERO,
            event: MusicEvent::Opl {
                register: 0x105,
                value: 1,
            },
        });
    }
    events.extend(registers.into_iter().map(|(register, value)| TimedEvent {
        at: Tick::ZERO,
        event: MusicEvent::Opl {
            register: register | bank,
            value,
        },
    }));
    events.push(TimedEvent {
        at: Tick::from_micros(50_123),
        event: MusicEvent::Opl {
            register: 0xb0 | bank,
            value: 0x11,
        },
    });
    Sequence::new(chip, events, Tick::from_micros(100_000)).unwrap()
}
