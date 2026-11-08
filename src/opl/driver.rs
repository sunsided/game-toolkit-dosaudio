use super::{OplBank, OplBankKey, OplInstrument, channel_address, frequency_tuning, patch_writes};
use crate::{
    DosAudio, Error, MidiMessage, MusicEvent, MusicTarget, OplChip, PreparedMusic, Result, Tick,
    TimedEvent,
};
use std::sync::Arc;

#[derive(Clone, Copy)]
struct Channel {
    program: u8,
    msb: u8,
    lsb: u8,
    volume: u8,
    expression: u8,
    pan: u8,
    sustain: bool,
    bend: u16,
    bend_range: u8,
    bend_cents: u8,
    rpn: [u8; 2],
}
impl Default for Channel {
    fn default() -> Self {
        Self {
            program: 0,
            msb: 0,
            lsb: 0,
            volume: 100,
            expression: 127,
            pan: 64,
            sustain: false,
            bend: 8192,
            bend_range: 2,
            bend_cents: 0,
            rpn: [127, 127],
        }
    }
}
#[derive(Clone, Copy)]
struct Voice {
    channel: u8,
    key: u8,
    velocity: u8,
    part: u8,
    instrument: OplInstrument,
    age: u64,
    key_down: bool,
}
#[derive(Clone)]
struct State {
    channels: [Channel; 16],
    voices: [Option<Voice>; 18],
    registers: [u8; 512],
    age: u64,
    initialized: bool,
}
impl Default for State {
    fn default() -> Self {
        Self {
            channels: [Channel::default(); 16],
            voices: [None; 18],
            registers: [0; 512],
            age: 0,
            initialized: false,
        }
    }
}

/// General MIDI messages translated to the installed OPL's two-operator voices.
///
/// OPL2 has 9 voices; dual OPL2 and OPL3 have 18. Layered instruments consume
/// two. Idle slots are used first, then the oldest complete note is stolen.
/// Repeated same-key note-offs release the oldest still-held instance.
/// Channel 10 uses fixed-pitch bank percussion on ordinary melodic OPL voices;
/// hardware rhythm mode is reserved for the explicit `Opl` drum helpers.
///
/// Handles program/bank select, volume, expression, sustain, pitch bend (default
/// +/-2 semitones; RPN0 bend sensitivity), pan, all-notes-off, all-sound-off and
/// reset-controllers. Velocity, volume and expression multiply as linear gain,
/// quantized to 0.75 dB OPL attenuation steps. Aftertouch, modulation and other
/// controllers are ignored.
/// OPL3 pan is left/both/right at 32/96; dual OPL2 assigns new notes to a physical
/// side using those thresholds, with center notes using either chip. Active
/// dual-OPL2 notes retain their physical side. Pitches outside hardware range are
/// clamped. OPL2 waveforms mask to two bits, OPL3 to three.
///
/// Construction, bank changes and [`Self::prepare`] belong outside rendering.
/// [`Self::message`] and [`Self::write_message`] allocate/free nothing. The driver
/// owns FM registers while in use: raw writes or other FM helpers can invalidate
/// its cached state. Reset/reinitialize the driver before resuming ownership.
///
/// ```
/// use game_toolkit_dosaudio::{MidiMessage, OplChip, OplMidiDriver, MusicEvent,
///     MusicTarget, PreparedMusic, Tick, TimedEvent};
/// let midi = PreparedMusic::new(MusicTarget::Midi, vec![
///     TimedEvent { at: Tick::ZERO,
///         event: MusicEvent::Midi(MidiMessage::new(&[0x90, 60, 100])?) },
///     TimedEvent { at: Tick::from_micros(100_000),
///         event: MusicEvent::Midi(MidiMessage::new(&[0x80, 60, 0])?) },
/// ], Tick::from_micros(150_000), [])?;
/// let driver = OplMidiDriver::with_starter_bank(OplChip::Opl2);
/// let fm = driver.prepare(&midi)?;
/// assert_eq!(fm.target(), MusicTarget::Opl(OplChip::Opl2));
/// # Ok::<(), game_toolkit_dosaudio::Error>(())
/// ```
#[derive(Clone)]
pub struct OplMidiDriver {
    chip: OplChip,
    bank: Arc<OplBank>,
    state: State,
}

impl OplMidiDriver {
    /// Use a caller-owned validated bank; its final drop must occur off callback.
    pub fn new(chip: OplChip, bank: Arc<OplBank>) -> Self {
        Self {
            chip,
            bank,
            state: State::default(),
        }
    }

    /// Use the original small starter bank (not a complete GM sound set).
    pub fn with_starter_bank(chip: OplChip) -> Self {
        Self::new(chip, Arc::new(OplBank::starter()))
    }

    /// Installed hardware expected by this driver.
    pub fn chip(&self) -> OplChip {
        self.chip
    }

    fn count(&self) -> usize {
        if self.chip == OplChip::Opl2 { 9 } else { 18 }
    }
    fn write(&mut self, register: u16, value: u8, emit: &mut impl FnMut(u16, u8)) {
        self.state.registers[usize::from(register)] = value;
        emit(register, value);
    }
    fn reset(&mut self, emit: &mut impl FnMut(u16, u8)) {
        self.state = State::default();
        if self.chip == OplChip::Opl3 {
            self.write(0x105, 1, emit);
            self.write(0x104, 0, emit);
        }
        for bank in 0..if self.chip == OplChip::Opl2 { 1 } else { 2 } {
            self.write(bank * 256 + 1, 0x20, emit);
            self.write(bank * 256 + 0xbd, self.bank.depth, emit);
        }
        for i in 0..self.count() {
            let (channel, op) = channel_address(self.chip, i as u8).expect("bounded voice");
            self.write(0xb0 | channel, 0, emit);
            self.write(0x40 | op, 63, emit);
            self.write(0x40 | (op + 3), 63, emit);
        }
        self.state.initialized = true;
    }

    /// Emit bounded register writes for one complete short MIDI message. First
    /// use initializes the chip. A MIDI FFh reset restores all driver defaults.
    /// The callback must accept every write, without blocking or allocating if
    /// this method is used in real time. Use `write_message` for port backpressure.
    pub fn message(&mut self, message: MidiMessage, mut emit: impl FnMut(u16, u8)) {
        let bytes = message.bytes();
        let status = bytes[0];
        if !self.state.initialized || status == 0xff {
            self.reset(&mut emit);
        }
        if status >= 0xf0 {
            return;
        }
        let channel = status & 15;
        match status & 0xf0 {
            0x80 => self.note_off(channel, bytes[1], &mut emit),
            0x90 if bytes[2] == 0 => self.note_off(channel, bytes[1], &mut emit),
            0x90 => self.note_on(channel, bytes[1], bytes[2], &mut emit),
            0xb0 => self.control(channel, bytes[1], bytes[2], &mut emit),
            0xc0 => self.state.channels[usize::from(channel)].program = bytes[1],
            0xe0 => {
                self.state.channels[usize::from(channel)].bend =
                    u16::from(bytes[1]) | (u16::from(bytes[2]) << 7);
                self.update_channel(channel, false, &mut emit);
            }
            _ => {}
        }
    }

    /// Apply a supported complete universal GM1/GM2 System On message. Other
    /// SysEx is ignored; GM2 synthesis features are not implied by its reset.
    pub fn sysex(&mut self, bytes: &[u8], mut emit: impl FnMut(u16, u8)) {
        if matches!(bytes,[0xf0,0x7e,device,9,1|3,0xf7] if *device<128) {
            self.reset(&mut emit);
        }
    }

    /// Send a message through the normal control-side OPL port path at `ports.now()`.
    /// The entire message and driver state commit together; `QueueFull` leaves
    /// both untouched. Fixed port latency, capture and chip aliases still apply.
    pub fn write_message(&mut self, ports: &mut DosAudio, message: MidiMessage) -> Result<()> {
        if ports.opl()?.chip != self.chip {
            return Err(Error::Config(
                "OPL MIDI driver chip does not match installed hardware",
            ));
        }
        let mut preview = self.clone();
        // Covers dual-chip initialization plus a layered onset, or refreshing
        // all 18 active voices for a controller change, without heap storage.
        let mut writes = [(0, 0); 128];
        let mut len = 0;
        preview.message(message, |reg, value| {
            writes[len] = (reg, value);
            len += 1;
        });
        ports.opl_write(&writes[..len])?;
        self.state = preview.state;
        Ok(())
    }

    fn end_voice(&mut self, index: usize, hard: bool, emit: &mut impl FnMut(u16, u8)) {
        let (channel, op) = channel_address(self.chip, index as u8).expect("bounded voice");
        self.write(
            0xb0 | channel,
            self.state.registers[usize::from(0xb0 | channel)] & !0x20,
            emit,
        );
        if hard {
            for operator in [op, op + 3] {
                self.write(
                    0x40 | operator,
                    (self.state.registers[usize::from(0x40 | operator)] & 0xc0) | 63,
                    emit,
                );
            }
        }
        self.state.voices[index] = None;
    }
    fn end_group(&mut self, age: u64, hard: bool, emit: &mut impl FnMut(u16, u8)) {
        for i in 0..self.count() {
            if self.state.voices[i].is_some_and(|v| v.age == age) {
                self.end_voice(i, hard, emit);
            }
        }
    }
    fn note_off(&mut self, channel: u8, key: u8, emit: &mut impl FnMut(u16, u8)) {
        let age = self
            .state
            .voices
            .iter()
            .flatten()
            .filter(|v| v.channel == channel && v.key == key && v.key_down)
            .map(|v| v.age)
            .min();
        if let Some(age) = age {
            if self.state.channels[usize::from(channel)].sustain {
                for voice in self
                    .state
                    .voices
                    .iter_mut()
                    .flatten()
                    .filter(|v| v.age == age)
                {
                    voice.key_down = false;
                }
            } else {
                self.end_group(age, false, emit);
            }
        }
    }
    fn eligible(&self, index: usize, pan: u8) -> bool {
        self.chip != OplChip::DualOpl2
            || (32..96).contains(&pan)
            || if pan < 32 { index < 9 } else { index >= 9 }
    }
    fn note_on(&mut self, channel: u8, key: u8, velocity: u8, emit: &mut impl FnMut(u16, u8)) {
        let state = self.state.channels[usize::from(channel)];
        let Some(instrument) = self.bank.resolve(OplBankKey {
            msb: state.msb,
            lsb: state.lsb,
            program: if channel == 9 { key } else { state.program },
            percussion: channel == 9,
        }) else {
            return;
        };
        let needed = if instrument.second.is_some() { 2 } else { 1 };
        while (0..self.count())
            .filter(|&i| self.eligible(i, state.pan) && self.state.voices[i].is_none())
            .count()
            < needed
        {
            let age = (0..self.count())
                .filter(|&i| self.eligible(i, state.pan))
                .filter_map(|i| self.state.voices[i].map(|v| v.age))
                .min()
                .expect("finite voice capacity");
            self.end_group(age, false, emit);
        }
        // Rebase ages before overflow while preserving strict note order, without
        // allocating. This path cannot be reached by realistic playback lengths.
        if self.state.age == u64::MAX {
            let old = self.state.voices;
            for voice in self.state.voices.iter_mut().flatten() {
                voice.age = old.iter().flatten().filter(|v| v.age < voice.age).count() as u64;
            }
            self.state.age = 18;
        }
        let age = self.state.age;
        self.state.age += 1;
        for part in 0..needed {
            let i = (0..self.count())
                .find(|&i| self.eligible(i, state.pan) && self.state.voices[i].is_none())
                .expect("reserved voice slots");
            self.state.voices[i] = Some(Voice {
                channel,
                key,
                velocity,
                part: part as u8,
                instrument,
                age,
                key_down: true,
            });
            let (register, op) = channel_address(self.chip, i as u8).expect("bounded voice");
            // Always clear the hardware key before reusing a released envelope.
            self.write(
                0xb0 | register,
                self.state.registers[usize::from(0xb0 | register)] & !0x20,
                emit,
            );
            let mut patch = if part == 0 {
                instrument.patch
            } else {
                instrument.second.expect("layered patch")
            };
            let mask = if self.chip == OplChip::Opl3 { 7 } else { 3 };
            patch.modulator.waveform &= mask;
            patch.carrier.waveform &= mask;
            let pan = self.routing(state.pan);
            let waveform = self.state.registers[usize::from(1 | (op & 0x100))] | 0x20;
            for (reg, value) in patch_writes(register, op, &patch, pan, waveform) {
                self.write(reg, value, emit);
            }
            self.update_voice(i, true, emit);
        }
    }
    fn routing(&self, pan: u8) -> u8 {
        if self.chip != OplChip::Opl3 {
            0
        } else if pan < 32 {
            0x10
        } else if pan >= 96 {
            0x20
        } else {
            0x30
        }
    }
    fn update_voice(&mut self, i: usize, levels: bool, emit: &mut impl FnMut(u16, u8)) {
        let Some(voice) = self.state.voices[i] else {
            return;
        };
        let state = self.state.channels[usize::from(voice.channel)];
        let (channel, op) = channel_address(self.chip, i as u8).expect("bounded voice");
        let patch = if voice.part == 0 {
            voice.instrument.patch
        } else {
            voice.instrument.second.expect("layered patch")
        };
        if levels {
            let velocity = (i16::from(voice.velocity) + i16::from(voice.instrument.velocity_offset))
                .clamp(1, 127) as u32;
            let amplitude = velocity * u32::from(state.volume) * u32::from(state.expression);
            let maximum = 127_u32.pow(3);
            let attenuation = if amplitude == 0 {
                63
            } else {
                // MIDI gain is linear amplitude; OPL total level is 0.75 dB/step.
                (-20.0 * (f64::from(amplitude) / f64::from(maximum)).log10() / 0.75)
                    .round()
                    .clamp(0.0, 63.0) as u32
            };
            let level =
                |raw: u8| (raw & 0xc0) | ((u32::from(raw & 63) + attenuation).min(63) as u8);
            self.write(
                0x40 | op,
                if patch.feedback & 1 != 0 {
                    level(patch.modulator.level)
                } else {
                    patch.modulator.level
                },
                emit,
            );
            self.write(0x40 | (op + 3), level(patch.carrier.level), emit);
            self.write(
                0xc0 | channel,
                (patch.feedback & 15) | self.routing(state.pan),
                emit,
            );
        }
        let offset = voice.instrument.note_offsets[usize::from(voice.part)];
        let detune = if voice.part == 1 {
            f64::from(i16::from(voice.instrument.detune).div_euclid(2)) / 32.0
        } else {
            0.0
        };
        let bend = (f64::from(state.bend) - 8192.0) / 8192.0
            * (f64::from(state.bend_range) + f64::from(state.bend_cents) / 100.0);
        let note = f64::from(voice.instrument.fixed_note.unwrap_or(voice.key))
            + f64::from(offset)
            + detune
            + bend;
        let frequency = 440.0 * 2_f64.powf((note - 69.0) / 12.0);
        let (number, block) =
            frequency_tuning(frequency.max(315_000_000.0 / (22.0 * 288.0 * 1_048_576.0)))
                .unwrap_or((1023, 7));
        self.write(0xa0 | channel, number as u8, emit);
        self.write(
            0xb0 | channel,
            0x20 | (block << 2) | (number >> 8) as u8,
            emit,
        );
    }
    fn update_channel(&mut self, channel: u8, levels: bool, emit: &mut impl FnMut(u16, u8)) {
        for i in 0..self.count() {
            if self.state.voices[i].is_some_and(|v| v.channel == channel) {
                self.update_voice(i, levels, emit);
            }
        }
    }
    fn control(&mut self, channel: u8, controller: u8, value: u8, emit: &mut impl FnMut(u16, u8)) {
        let index = usize::from(channel);
        match controller {
            0 => self.state.channels[index].msb = value,
            32 => self.state.channels[index].lsb = value,
            7 | 11 | 10 => {
                match controller {
                    7 => self.state.channels[index].volume = value,
                    11 => self.state.channels[index].expression = value,
                    _ => self.state.channels[index].pan = value,
                }
                self.update_channel(channel, true, emit);
            }
            64 => {
                self.state.channels[index].sustain = value >= 64;
                if value < 64 {
                    for i in 0..self.count() {
                        if self.state.voices[i].is_some_and(|v| v.channel == channel && !v.key_down)
                        {
                            self.end_voice(i, false, emit);
                        }
                    }
                }
            }
            100 => self.state.channels[index].rpn[1] = value,
            101 => self.state.channels[index].rpn[0] = value,
            6 | 38 if self.state.channels[index].rpn == [0, 0] => {
                if controller == 6 {
                    self.state.channels[index].bend_range = value;
                } else {
                    self.state.channels[index].bend_cents = value.min(99);
                }
                self.update_channel(channel, false, emit);
            }
            120 | 123 => {
                for i in 0..self.count() {
                    if self.state.voices[i].is_some_and(|v| v.channel == channel) {
                        if controller == 123 && self.state.channels[index].sustain {
                            self.state.voices[i]
                                .as_mut()
                                .expect("active voice")
                                .key_down = false;
                        } else {
                            self.end_voice(i, controller == 120, emit);
                        }
                    }
                }
            }
            121 => {
                let old = self.state.channels[index];
                self.state.channels[index] = Channel {
                    program: old.program,
                    msb: old.msb,
                    lsb: old.lsb,
                    volume: old.volume,
                    pan: old.pan,
                    ..Channel::default()
                };
                for i in 0..self.count() {
                    if self.state.voices[i].is_some_and(|v| v.channel == channel && !v.key_down) {
                        self.end_voice(i, false, emit);
                    }
                }
                self.update_channel(channel, true, emit);
            }
            _ => {}
        }
    }

    fn snapshot(&self, emit: &mut impl FnMut(u16, u8)) {
        if self.chip == OplChip::Opl3 {
            emit(0x105, 1);
            emit(0x104, 0);
        }
        for i in 0..self.count() {
            let (channel, _) = channel_address(self.chip, i as u8).expect("bounded voice");
            emit(
                0xb0 | channel,
                self.state.registers[usize::from(0xb0 | channel)] & !0x20,
            );
        }
        for bank in 0..if self.chip == OplChip::Opl2 { 1 } else { 2 } {
            for register in [1, 0xbd] {
                let reg = bank * 256 + register;
                emit(reg, self.state.registers[usize::from(reg)]);
            }
        }
        for i in 0..self.count() {
            let (channel, op) = channel_address(self.chip, i as u8).expect("bounded voice");
            for base in [0x20, 0x40, 0x60, 0x80, 0xe0] {
                for operator in [op, op + 3] {
                    let reg = base | operator;
                    emit(reg, self.state.registers[usize::from(reg)]);
                }
            }
            for base in [0xc0, 0xa0] {
                let reg = base | channel;
                emit(reg, self.state.registers[usize::from(reg)]);
            }
        }
        for i in 0..self.count() {
            let (channel, _) = channel_address(self.chip, i as u8).expect("bounded voice");
            let reg = 0xb0 | channel;
            emit(reg, self.state.registers[usize::from(reg)]);
        }
    }

    /// Compile a MIDI arrangement to immutable writes for the installed OPL.
    /// Starts with fresh MIDI state, leaving this driver unchanged. Retains event
    /// FIFO, duration and exact intro/loop event boundary. A register snapshot at
    /// the repeat point restores chip state each loop (crossing notes retrigger
    /// their envelope). End-of-track releases keys. Unknown SysEx is ignored.
    /// Preparation allocates here; subsequent engine playback uses the existing
    /// allocation-free OPL sequencer. Output is limited to one million writes.
    pub fn prepare(&self, midi: &PreparedMusic) -> Result<PreparedMusic> {
        if midi.target() != MusicTarget::Midi {
            return Err(Error::Config("OPL MIDI preparation requires MIDI music"));
        }
        let mut driver = Self::new(self.chip, Arc::clone(&self.bank));
        let mut events = Vec::new();
        let mut overflow = false;
        let mut append = |at: Tick, reg, value| {
            if events.len() == 1_000_000 {
                overflow = true;
            } else {
                events.push(TimedEvent {
                    at,
                    event: MusicEvent::Opl {
                        register: reg,
                        value,
                    },
                });
            }
        };
        driver.reset(&mut |r, v| append(Tick::ZERO, r, v));
        let mut repeat_index = 0;
        // The snapshot sits before the source's loop event, including boundaries
        // that divide multiple source messages sharing the same crystal tick.
        for index in 0..=midi.events.len() {
            if index == midi.loop_index {
                repeat_index = events.len();
                driver.snapshot(&mut |r, v| {
                    if events.len() == 1_000_000 {
                        overflow = true;
                    } else {
                        events.push(TimedEvent {
                            at: midi.loop_start,
                            event: MusicEvent::Opl {
                                register: r,
                                value: v,
                            },
                        });
                    }
                });
            }
            let Some(event) = midi.events.get(index) else {
                break;
            };
            let mut emit = |r, v| {
                if events.len() == 1_000_000 {
                    overflow = true;
                } else {
                    events.push(TimedEvent {
                        at: event.at,
                        event: MusicEvent::Opl {
                            register: r,
                            value: v,
                        },
                    });
                }
            };
            match event.event {
                MusicEvent::Midi(message) => driver.message(message, &mut emit),
                MusicEvent::SysEx { offset, length } => driver.sysex(
                    &midi.sysex[offset as usize..(offset + length) as usize],
                    &mut emit,
                ),
                _ => unreachable!("validated MIDI arrangement"),
            }
            if overflow {
                return Err(Error::TooLarge);
            }
        }
        for i in 0..driver.count() {
            if driver.state.voices[i].is_some() {
                driver.end_voice(i, false, &mut |r, v| {
                    events.push(TimedEvent {
                        at: midi.duration,
                        event: MusicEvent::Opl {
                            register: r,
                            value: v,
                        },
                    })
                });
            }
        }
        if overflow || events.len() > 1_000_000 {
            return Err(Error::TooLarge);
        }
        let music = PreparedMusic::new(MusicTarget::Opl(self.chip), events, midi.duration, [])?;
        if midi.duration > midi.loop_start {
            music.with_loop_boundary(midi.loop_start, repeat_index)
        } else {
            Ok(music)
        }
    }
}
