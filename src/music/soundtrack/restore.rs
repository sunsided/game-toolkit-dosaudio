use super::*;
use crate::MidiMessage;
use std::collections::BTreeMap;

#[derive(Clone)]
struct Note {
    channel: u8,
    key: u8,
    held: bool,
    id: u64,
}
#[derive(Default)]
struct MidiState {
    notes: Vec<Note>,
    history: Vec<(MusicEvent, Option<u64>)>,
    sustain: [bool; 16],
    next_id: u64,
    sysex_count: usize,
    sysex_bytes: u32,
}
impl MidiState {
    fn event(&mut self, event: MusicEvent, storage: &[u8], device: MusicDevice) -> Result<()> {
        let mut note_id = None;
        match event {
            MusicEvent::SysEx { offset, length } => {
                let bytes = &storage[offset as usize..(offset + length) as usize];
                let gm = matches!(bytes, [0xf0, 0x7e, _, 9, 1 | 3, 0xf7]);
                let roland = bytes.len() >= 8
                    && bytes[1] == 0x41
                    && bytes[2] <= 0x10
                    && bytes[3] == 0x16
                    && bytes[4] == 0x12
                    && bytes[5] == 0x7f
                    && bytes[5..bytes.len() - 1]
                        .iter()
                        .fold(0u8, |n, b| n.wrapping_add(*b))
                        & 127
                        == 0;
                if device == MusicDevice::GeneralMidi && gm || device == MusicDevice::Mt32 && roland
                {
                    *self = Self::default();
                }
                self.sysex_count += 1;
                self.sysex_bytes = self
                    .sysex_bytes
                    .checked_add(length)
                    .ok_or(Error::TooLarge)?;
                if self.sysex_count > 16 || self.sysex_bytes > 65_536 {
                    return Err(Error::TooLarge);
                }
            }
            MusicEvent::Midi(message) => {
                let d = message.bytes();
                let channel = d[0] & 15;
                let i = usize::from(channel);
                match d[0] & 0xf0 {
                    0x80 | 0x90 if d[0] & 0xf0 == 0x80 || d[2] == 0 => {
                        let Some(index) = self
                            .notes
                            .iter()
                            .position(|n| n.channel == channel && n.key == d[1] && n.held)
                        else {
                            return Ok(());
                        };
                        note_id = Some(self.notes[index].id);
                        if self.sustain[i] {
                            self.notes[index].held = false;
                        } else {
                            self.notes.remove(index);
                        }
                    }
                    0x90 => {
                        if self.notes.len() == 128 {
                            return Err(Error::TooLarge);
                        }
                        if self
                            .notes
                            .iter()
                            .any(|n| n.channel == channel && n.key == d[1])
                        {
                            return Err(Error::Format(
                                "overlapping same-key MIDI notes are not switchable",
                            ));
                        }
                        let id = self.next_id;
                        self.next_id += 1;
                        note_id = Some(id);
                        self.notes.push(Note {
                            channel,
                            key: d[1],
                            held: true,
                            id,
                        });
                    }
                    0xb0 => match d[1] {
                        64 => {
                            self.sustain[i] = d[2] >= 64;
                            if !self.sustain[i] {
                                self.notes.retain(|n| n.channel != channel || n.held);
                            }
                        }
                        120 => self.notes.retain(|n| n.channel != channel),
                        123 => {
                            if self.sustain[i] {
                                for n in self.notes.iter_mut().filter(|n| n.channel == channel) {
                                    n.held = false;
                                }
                            } else {
                                self.notes.retain(|n| n.channel != channel);
                            }
                        }
                        121 => {
                            self.sustain[i] = false;
                            self.notes.retain(|n| n.channel != channel || n.held);
                        }
                        _ => {}
                    },
                    0xc0 if device == MusicDevice::Mt32 && channel != 9 => {
                        // CM-32L program changes cancel melodic voices/pedal;
                        // GM program changes leave already sounding voices alone.
                        self.notes.retain(|n| n.channel != channel);
                        self.sustain[i] = false;
                    }
                    0xf0 if d[0] == 0xff => {
                        *self = Self::default();
                        return Ok(());
                    }
                    _ => {}
                }
            }
            _ => return Ok(()),
        }
        self.history.push((event, note_id));
        // Remove completed note events while retaining every state-changing
        // message in original order. In particular, program/RPN/SysEx writes
        // cannot be reordered or blindly deduplicated across one another.
        self.history
            .retain(|(_, id)| id.is_none_or(|id| self.notes.iter().any(|n| n.id == id)));
        if self.history.len() >= 1024 {
            return Err(Error::TooLarge);
        }
        Ok(())
    }
    fn snapshot(&self) -> Vec<MusicEvent> {
        std::iter::once(MusicEvent::Midi(MidiMessage::new(&[0xff]).unwrap()))
            .chain(self.history.iter().map(|(event, _)| *event))
            .collect()
    }
}

pub(super) fn checkpoints(music: &PreparedMusic, device: MusicDevice) -> Result<Vec<Checkpoint>> {
    let mut opl = BTreeMap::<u16, u8>::new();
    if let MusicTarget::Opl(chip) = music.target {
        // A newly selected arrangement starts from hardware reset values, even
        // for registers it never writes. Otherwise the previous patch leaks in.
        for bank in 0..if chip == OplChip::Opl2 { 1 } else { 2 } {
            for register in [1, 8, 0xbd] {
                opl.insert(bank * 256 + register, 0);
            }
            for channel in 0..9 {
                for base in [0xa0, 0xb0, 0xc0] {
                    opl.insert(bank * 256 + base + channel, 0);
                }
            }
            for slot in [
                0, 1, 2, 3, 4, 5, 8, 9, 10, 11, 12, 13, 16, 17, 18, 19, 20, 21,
            ] {
                for base in [0x20, 0x40, 0x60, 0x80, 0xe0] {
                    opl.insert(bank * 256 + base + slot, 0);
                }
            }
        }
        if chip == OplChip::Opl3 {
            opl.insert(0x104, 0);
            opl.insert(0x105, 0);
        }
    }
    let mut psg = [0u16, 15, 0, 15, 0, 15, 0, 15];
    let mut latch = 0;
    let mut speaker = MusicEvent::Speaker {
        divisor: 0,
        enabled: false,
    };
    let mut midi = MidiState::default();
    let snapshot =
        |opl: &BTreeMap<u16, u8>, psg: &[u16; 8], speaker: MusicEvent, midi: &MidiState| {
            let mut events = Vec::new();
            match music.target {
                MusicTarget::Opl(chip) => {
                    if chip == OplChip::Opl3 {
                        events.push(MusicEvent::Opl {
                            register: 0x105,
                            value: 1,
                        });
                        events.push(MusicEvent::Opl {
                            register: 0x104,
                            value: *opl.get(&0x104).unwrap_or(&0),
                        });
                    }
                    for bank in 0..if chip == OplChip::Opl2 { 1 } else { 2 } {
                        for register in [1, 8, 0xbd] {
                            events.push(MusicEvent::Opl {
                                register: bank * 256 + register,
                                value: 0,
                            });
                        }

                        for ch in 0..9 {
                            events.push(MusicEvent::Opl {
                                register: bank * 256 + 0xb0 + ch,
                                value: 0,
                            });
                        }
                    }
                    for (&register, &value) in opl {
                        if !(0xb0..=0xb8).contains(&(register & 255)) {
                            events.push(MusicEvent::Opl { register, value });
                        }
                    }
                    for (&register, &value) in opl {
                        if (0xb0..=0xb8).contains(&(register & 255)) {
                            events.push(MusicEvent::Opl { register, value });
                        }
                    }
                }
                MusicTarget::Midi => events = midi.snapshot(),
                MusicTarget::Speaker => events.push(speaker),
                MusicTarget::Psg => {
                    for (r, &value) in psg.iter().enumerate() {
                        events.push(MusicEvent::Psg(
                            0x80 | ((r as u8) << 4) | (value as u8 & 15),
                        ));
                        if r % 2 == 0 && r < 6 {
                            events.push(MusicEvent::Psg((value >> 4) as u8));
                        }
                    }
                }
                MusicTarget::Native => {}
                _ => {}
            }
            events
        };
    let mut out = vec![Checkpoint {
        at: Tick::ZERO,
        index: 0,
        events: snapshot(&opl, &psg, speaker, &midi),
    }];
    let mut total = out[0].events.len();
    let mut index = 0;
    while index < music.events.len() {
        let at = music.events[index].at;
        while index < music.events.len() && music.events[index].at == at {
            match music.events[index].event {
                MusicEvent::Opl { register, value } => {
                    opl.insert(register, value);
                }
                MusicEvent::Psg(byte) => {
                    if byte & 128 != 0 {
                        latch = usize::from((byte >> 4) & 7);
                    }
                    psg[latch] = if latch % 2 == 0 && latch < 6 {
                        if byte & 128 != 0 {
                            (psg[latch] & 0x3f0) | u16::from(byte & 15)
                        } else {
                            (psg[latch] & 15) | (u16::from(byte & 63) << 4)
                        }
                    } else {
                        u16::from(byte & 15)
                    };
                }
                event @ (MusicEvent::Speaker { .. } | MusicEvent::SpeakerLevel(_)) => {
                    speaker = event
                }
                event => midi.event(event, &music.sysex, device)?,
            }
            index += 1;
        }
        let events = snapshot(&opl, &psg, speaker, &midi);
        total += events.len();
        if events.len() > 1024 || total > 1_000_000 || out.len() == 4096 {
            return Err(Error::TooLarge);
        }
        if at == Tick::ZERO {
            out.clear();
        }
        out.push(Checkpoint { at, index, events });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(bytes: &[u8]) -> MusicEvent {
        MusicEvent::Midi(MidiMessage::new(bytes).unwrap())
    }

    #[test]
    fn midi_restore_keeps_sysex_program_controller_and_note_order() {
        let storage = [0xf0, 0x7d, 1, 0xf7];
        let sysex = MusicEvent::SysEx {
            offset: 0,
            length: 4,
        };
        // Program, patch upload, a held note, then a new program and RPN/CC121.
        // Reordering either the upload or the note changes what is heard.
        let events = [
            msg(&[0xc0, 40]),
            sysex,
            msg(&[0x90, 60, 100]),
            msg(&[0xc0, 64]),
            msg(&[0xb0, 101, 0]),
            msg(&[0xb0, 100, 0]),
            msg(&[0xb0, 6, 12]),
            msg(&[0xb0, 121, 0]),
            msg(&[0xb0, 6, 2]),
            msg(&[0x90, 62, 100]),
            msg(&[0x80, 62, 0]),
        ];
        let mut state = MidiState::default();
        for event in events {
            state
                .event(event, &storage, MusicDevice::GeneralMidi)
                .unwrap();
        }
        let mut expected = vec![msg(&[0xff])];
        expected.extend_from_slice(&events[..9]);
        assert_eq!(state.snapshot(), expected);
    }

    #[test]
    fn mt32_program_change_and_reset_discard_obsolete_note_state() {
        let mut state = MidiState::default();
        for event in [msg(&[0x91, 60, 100]), msg(&[0xc1, 40])] {
            state.event(event, &[], MusicDevice::Mt32).unwrap();
        }
        assert_eq!(state.snapshot(), vec![msg(&[0xff]), msg(&[0xc1, 40])]);
        state.event(msg(&[0xff]), &[], MusicDevice::Mt32).unwrap();
        state
            .event(msg(&[0x91, 62, 100]), &[], MusicDevice::Mt32)
            .unwrap();
        assert_eq!(state.snapshot(), vec![msg(&[0xff]), msg(&[0x91, 62, 100])]);
    }
}
