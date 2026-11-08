//! Musical helpers that program the installed OPL through its normal register path.

use crate::{DosAudio, Error, OplChip, Result};

/// The five register values describing one FM operator.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct OplOperator {
    /// Tremolo, vibrato, envelope mode, key scaling and frequency multiplier (20h).
    pub characteristics: u8,
    /// Key-scale level and total attenuation (40h).
    pub level: u8,
    /// Attack and decay rates (60h).
    pub attack_decay: u8,
    /// Sustain attenuation and release rate (80h).
    pub sustain_release: u8,
    /// Waveform selection (E0h).
    pub waveform: u8,
}

/// A two-operator instrument, including channel feedback and connection mode.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct OplPatch {
    /// First operator, used as a modulator in FM connection mode.
    pub modulator: OplOperator,
    /// Second operator, used as the carrier in FM connection mode.
    pub carrier: OplOperator,
    /// Feedback and connection bits, the low four bits of register C0h.
    pub feedback: u8,
}

impl OplPatch {
    /// Read an SBI instrument: `SBI\x1a`, a 32-byte name and 16 instrument bytes.
    /// Trailing metadata is accepted; the name and five reserved data bytes are ignored.
    pub fn from_sbi(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < 52 || &bytes[..4] != b"SBI\x1a" {
            return Err(Error::Format("invalid or truncated SBI instrument"));
        }
        let data = &bytes[36..52];
        let operator = |offset: usize| OplOperator {
            characteristics: data[offset],
            level: data[offset + 2],
            attack_decay: data[offset + 4],
            sustain_release: data[offset + 6],
            waveform: data[offset + 8],
        };
        Ok(Self {
            modulator: operator(0),
            carrier: operator(1),
            feedback: data[10],
        })
    }
}

/// An equal-tempered MIDI note, with A4 tuned to 440 Hz.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Note(u8);

impl Note {
    /// Middle C, MIDI note 60.
    pub const C4: Self = Self(60);
    /// Concert A, MIDI note 69.
    pub const A4: Self = Self(69);

    /// Accept MIDI note numbers 0 through 127.
    pub fn from_midi(note: u8) -> Result<Self> {
        if note > 127 {
            return Err(Error::Config("MIDI note must be in 0 through 127"));
        }
        Ok(Self(note))
    }

    /// Frequency before quantization to the OPL's F-number and block.
    pub fn frequency(self) -> f64 {
        440.0 * 2.0f64.powf((f64::from(self.0) - 69.0) / 12.0)
    }

    fn tuning(self) -> Result<(u16, u8)> {
        let native_rate = 315_000_000.0 / (22.0 * 288.0);
        let mut number = self.frequency() * 1_048_576.0 / native_rate;
        for block in 0..=7 {
            let rounded = number.round();
            if rounded <= 1023.0 {
                return Ok((rounded as u16, block));
            }
            number *= 0.5;
        }
        Err(Error::Config("note exceeds the OPL frequency range"))
    }
}

/// Percussion key bits in the first OPL bank's rhythm register.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Drum {
    /// Bass drum, using channel 6.
    BassDrum,
    /// Snare drum, using channel 7's carrier.
    Snare,
    /// Tom, using channel 8's modulator.
    Tom,
    /// Cymbal, using channel 8's carrier.
    Cymbal,
    /// Hi-hat, using channel 7's modulator.
    HiHat,
}

impl Drum {
    fn mask(self) -> u8 {
        match self {
            Self::BassDrum => 0x10,
            Self::Snare => 0x08,
            Self::Tom => 0x04,
            Self::Cymbal => 0x02,
            Self::HiHat => 0x01,
        }
    }
}

/// Borrowed musical access to the same FM hardware used by port I/O and music.
pub struct Opl<'a> {
    ports: &'a mut DosAudio,
    chip: OplChip,
}

impl<'a> Opl<'a> {
    pub(crate) fn new(ports: &'a mut DosAudio, chip: OplChip) -> Self {
        Self { ports, chip }
    }

    fn channel(&self, channel: u8) -> Result<(u16, u16)> {
        let channels = if self.chip == OplChip::Opl2 { 9 } else { 18 };
        if channel >= channels {
            return Err(Error::Config("OPL channel is outside the installed chip"));
        }
        let bank = u16::from(channel / 9) << 8;
        let local = usize::from(channel % 9);
        let operator = [0, 1, 2, 8, 9, 10, 16, 17, 18][local];
        Ok((bank | u16::from(channel % 9), bank | operator))
    }

    fn write(&mut self, writes: &[(u16, u8)]) -> Result<()> {
        let mut batch = [(0, 0); 13];
        let offset = usize::from(self.chip == OplChip::Opl3);
        if offset != 0 {
            batch[0] = (0x105, 1);
        }
        batch[offset..offset + writes.len()].copy_from_slice(writes);
        self.ports.opl_write(&batch[..offset + writes.len()])
    }

    /// Program a two-operator patch, preserving OPL3 output routing.
    /// Channels without explicit OPL3 routing default to both stereo outputs.
    pub fn set_patch(&mut self, channel: u8, patch: &OplPatch) -> Result<()> {
        let (channel_register, modulator) = self.channel(channel)?;
        let carrier = modulator + 3;
        let mut routing = 0;
        if self.chip == OplChip::Opl3 {
            routing = self.ports.opl_register(0xc0 | channel_register)? & 0xf0;
            if routing == 0 {
                routing = 0x30;
            }
        }
        let a = patch.modulator;
        let b = patch.carrier;
        let waveform_register = 0x01 | (modulator & 0x100);
        let waveform_enable = self.ports.opl_register(waveform_register)? | 0x20;
        self.write(&[
            (waveform_register, waveform_enable),
            (0x20 | modulator, a.characteristics),
            (0x20 | carrier, b.characteristics),
            (0x40 | modulator, a.level),
            (0x40 | carrier, b.level),
            (0x60 | modulator, a.attack_decay),
            (0x60 | carrier, b.attack_decay),
            (0x80 | modulator, a.sustain_release),
            (0x80 | carrier, b.sustain_release),
            (0xe0 | modulator, a.waveform),
            (0xe0 | carrier, b.waveform),
            (0xc0 | channel_register, routing | (patch.feedback & 0x0f)),
        ])?;
        self.ports.opl_set_patch_levels(channel, [a.level, b.level])
    }

    /// Start a note with MIDI velocity 1 through 127; zero releases the note.
    /// Pitch uses the lowest fitting block and the nearest F-number. Notes above
    /// the chip's maximum frequency return an error before changing registers.
    pub fn note_on(&mut self, channel: u8, note: Note, velocity: u8) -> Result<()> {
        let (channel_register, modulator) = self.channel(channel)?;
        if velocity > 127 {
            return Err(Error::Config("MIDI velocity must be in 0 through 127"));
        }
        if velocity == 0 {
            return self.note_off(channel);
        }
        let (number, block) = note.tuning()?;
        let levels = self.ports.opl_patch_levels(channel)?;
        let attenuation = ((127 - u16::from(velocity)) * 63 + 63) / 127;
        let attenuate =
            |level: u8| (level & 0xc0) | ((u16::from(level & 0x3f) + attenuation).min(63) as u8);
        let additive = self.ports.opl_register(0xc0 | channel_register)? & 1 != 0;
        let old_key = self.ports.opl_register(0xb0 | channel_register)?;
        self.write(&[
            (
                0x40 | modulator,
                if additive {
                    attenuate(levels[0])
                } else {
                    levels[0]
                },
            ),
            (0x40 | (modulator + 3), attenuate(levels[1])),
            (0xa0 | channel_register, number as u8),
            (
                0xb0 | channel_register,
                (old_key & 0xc0) | 0x20 | (block << 2) | (number >> 8) as u8,
            ),
        ])
    }

    /// Release a melodic key without changing its block or F-number.
    pub fn note_off(&mut self, channel: u8) -> Result<()> {
        let (channel_register, _) = self.channel(channel)?;
        let register = 0xb0 | channel_register;
        self.write(&[(register, self.ports.opl_register(register)? & !0x20)])
    }

    /// Enable percussion on bank zero; disabling also releases all drum keys.
    /// Tremolo and vibrato depth bits remain unchanged.
    pub fn rhythm(&mut self, enabled: bool) -> Result<()> {
        let value = self.ports.opl_register(0xbd)?;
        self.write(&[(0xbd, if enabled { value | 0x20 } else { value & !0x3f })])
    }

    /// Enable rhythm mode and retrigger one drum, preserving the other drum keys.
    pub fn hit(&mut self, drum: Drum) -> Result<()> {
        let value = self.ports.opl_register(0xbd)?;
        self.write(&[
            (0xbd, value & !drum.mask()),
            (0xbd, value | 0x20 | drum.mask()),
        ])
    }

    /// Release one drum without changing rhythm mode or the other drum keys.
    pub fn release(&mut self, drum: Drum) -> Result<()> {
        let value = self.ports.opl_register(0xbd)?;
        self.write(&[(0xbd, value & !drum.mask())])
    }
}
