//! Tandy-style SN76489-compatible PSG, using a 15-bit noise shift register.
//!
//! The white-noise taps are bits 0 and 4, fed into bit 14; periodic noise
//! recirculates bit 0. Noise-control writes reset the seed to 4000h and restart
//! its divider phase. Tone divisor zero means 1024, rather than the Sega PSG's
//! high-frequency special case. Power-on state is deterministic and muted.
//!
//! Register protocol, attenuation and Tandy noise taps:
//! <https://www.smspower.org/Development/SN76489>.

use crate::{DosAudio, Error, Frame, Result, Tick};

const LEVELS: [f32; 16] = [
    1.0,
    0.794_328_2,
    0.630_957_37,
    0.501_187_2,
    0.398_107_17,
    0.316_227_76,
    0.251_188_64,
    0.199_526_24,
    0.158_489_32,
    0.125_892_54,
    0.1,
    0.079_432_82,
    0.063_095_73,
    0.050_118_722,
    0.039_810_717,
    0.0,
];

pub(crate) struct PsgVoice {
    pub held: Frame,
    pub next: Tick,
    latched: u8,
    tone: [u16; 3],
    remaining: [u16; 3],
    high: [bool; 3],
    attenuation: [u8; 4],
    noise: u8,
    noise_remaining: u16,
    noise_high: bool,
    shift: u16,
}

impl PsgVoice {
    pub fn new() -> Self {
        Self {
            held: [0.0; 2],
            next: Tick::ZERO,
            latched: 0,
            tone: [0; 3],
            remaining: [1; 3],
            high: [true; 3],
            attenuation: [15; 4],
            noise: 0,
            noise_remaining: 16,
            noise_high: false,
            shift: 0x4000,
        }
    }

    pub fn write(&mut self, value: u8) {
        let latch = value & 0x80 != 0;
        if latch {
            self.latched = (value >> 4) & 7;
        }
        let channel = usize::from(self.latched / 2);
        if self.latched & 1 != 0 {
            self.attenuation[channel] = value & 0x0f;
        } else if channel < 3 {
            self.tone[channel] = if latch {
                (self.tone[channel] & 0x3f0) | u16::from(value & 0x0f)
            } else {
                (self.tone[channel] & 0x00f) | (u16::from(value & 0x3f) << 4)
            };
        } else {
            self.noise = value & 7;
            self.shift = 0x4000;
            self.noise_remaining = self.noise_period();
            self.noise_high = false;
        }
        self.update_output();
    }

    fn noise_period(&self) -> u16 {
        16 << (self.noise & 3).min(2)
    }

    fn shift_noise(&mut self) {
        let feedback = if self.noise & 4 != 0 {
            (self.shift ^ (self.shift >> 4)) & 1
        } else {
            self.shift & 1
        };
        self.shift = (self.shift >> 1) | (feedback << 14);
    }

    pub fn step(&mut self) {
        let mut tone_two_falling = false;
        for channel in 0..3 {
            self.remaining[channel] -= 1;
            if self.remaining[channel] == 0 {
                self.remaining[channel] = if self.tone[channel] == 0 {
                    1024
                } else {
                    self.tone[channel]
                };
                self.high[channel] = !self.high[channel];
                if channel == 2 && !self.high[channel] {
                    tone_two_falling = true;
                }
            }
        }
        if self.noise & 3 == 3 {
            if tone_two_falling {
                self.shift_noise();
            }
        } else {
            self.noise_remaining -= 1;
            if self.noise_remaining == 0 {
                self.noise_remaining = self.noise_period();
                self.noise_high = !self.noise_high;
                if !self.noise_high {
                    self.shift_noise();
                }
            }
        }
        self.update_output();
        // Input crystal / 4, followed by the chip's / 16 prescaler.
        self.next += Tick(64);
    }

    pub fn mute(&mut self) {
        self.attenuation.fill(15);
        self.held = [0.0; 2];
    }

    fn update_output(&mut self) {
        let mut sum = 0.0;
        for channel in 0..4 {
            let high = if channel == 3 {
                self.shift & 1 != 0
            } else {
                self.high[channel]
            };
            let level = LEVELS[usize::from(self.attenuation[channel])] * 0.25;
            sum += if high { level } else { -level };
        }
        self.held = [sum; 2];
    }
}

/// Borrowed tone, noise and attenuation controls for the installed Tandy PSG.
pub struct Psg<'a> {
    ports: &'a mut DosAudio,
}

impl<'a> Psg<'a> {
    pub(crate) fn new(ports: &'a mut DosAudio) -> Self {
        Self { ports }
    }

    /// Play tone channel 0 through 2, with attenuation 0 (loudest) through 15 (mute).
    /// Frequency must be finite and positive. The nearest divisor is clamped to
    /// 1 through 1024; divisor 1024 is encoded as zero, as on the modeled chip.
    pub fn tone(&mut self, channel: u8, hz: f64, attenuation: u8) -> Result<()> {
        if channel > 2 || !hz.is_finite() || hz <= 0.0 || attenuation > 15 {
            return Err(Error::Config(
                "invalid PSG tone channel, frequency or attenuation",
            ));
        }
        let period = (315_000_000.0 / (22.0 * 4.0 * 32.0 * hz))
            .round()
            .clamp(1.0, 1024.0) as u16
            & 0x3ff;
        self.ports.psg_write(&[
            0x80 | (channel << 5) | (period as u8 & 0x0f),
            (period >> 4) as u8,
            0x90 | (channel << 5) | attenuation,
        ])
    }

    /// Set noise mode/rate (three control bits) and attenuation 0 through 15.
    /// Bit 2 selects white noise; low bits select fixed half-periods 16/32/64,
    /// or tone channel 2's falling edges. Rewriting control restarts the noise.
    pub fn noise(&mut self, control: u8, attenuation: u8) -> Result<()> {
        if control > 7 || attenuation > 15 {
            return Err(Error::Config("invalid PSG noise control or attenuation"));
        }
        self.ports.psg_write(&[0xe0 | control, 0xf0 | attenuation])
    }

    /// Mute tone channel 0 through 2, or noise channel 3.
    pub fn off(&mut self, channel: u8) -> Result<()> {
        if channel > 3 {
            return Err(Error::Config("invalid PSG channel"));
        }
        self.ports.psg_write(&[0x9f | (channel << 5)])
    }
}
