// Copyright (C) 2021-2026 Geoff Hill <geoff@geoffhill.org>
// Copyright (C) 2003-2026 Dean Beeler, Jerome Fisher, Sergey V. Mikayev
//
// This program is free software: you can redistribute it and/or modify it
// under the terms of the GNU Lesser General Public License as published by
// the Free Software Foundation, either version 2.1 of the License, or (at
// your option) any later version. Read COPYING.LESSER.txt for details.

use super::midi::Event;
use std::boxed::Box;

const MIDI_QUEUE_SIZE: usize = 4096;
const MIDI_QUEUE_MASK: usize = 0xfff;

#[derive(Debug)]
pub struct MidiQueue {
    items: Box<[Option<Event>; MIDI_QUEUE_SIZE]>,
    given: usize,
    taken: usize,
}

impl MidiQueue {
    pub fn new() -> MidiQueue {
        const INIT: Option<Event> = None;
        MidiQueue {
            items: Box::new([INIT; MIDI_QUEUE_SIZE]),
            given: 0,
            taken: 0,
        }
    }

    pub fn push(&mut self, value: Event) -> bool {
        let (given, taken) = (self.given, self.taken);

        if given.wrapping_sub(taken) >= MIDI_QUEUE_SIZE {
            return false;
        }

        let slot = given & MIDI_QUEUE_MASK;
        // assert_eq!(self.items[slot], None);
        self.items[slot] = Some(value);
        self.given = given.wrapping_add(1);
        true
    }

    pub fn peek(&self) -> Option<&Event> {
        let (given, taken) = (self.given, self.taken);

        if given == taken {
            return None;
        }

        let slot = taken & MIDI_QUEUE_MASK;
        self.items[slot].as_ref()
    }

    pub fn pop(&mut self) -> Option<Event> {
        let (given, taken) = (self.given, self.taken);

        if given == taken {
            return None;
        }

        let slot = taken & MIDI_QUEUE_MASK;
        let value = self.items[slot].take();
        self.taken = taken.wrapping_add(1);
        value
    }
}
