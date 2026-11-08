use crate::SbModel;

/// Indexed CT1345/CT1745 controls, without analog-input emulation.
///
/// Register definitions and reset levels: Creative's Sound Blaster Series
/// Hardware Programming Guide, chapter 4 and section 2-7:
/// https://pdos.csail.mit.edu/6.828/2018/readings/hardware/SoundBlaster.pdf
/// Reserved bits read as zero; unknown registers read FF and ignore writes.
/// Board-specific mirrors and reserved-bit readback are not modeled.
#[derive(Clone, Copy)]
pub(super) struct Mixer {
    model: SbModel,
    address: u8,
    registers: [u8; 0x83],
    fixed_irq: u8,
    fixed_dma: u8,
}

impl Mixer {
    pub(super) fn new(model: SbModel, irq: u8, dma: u8, dma16: u8) -> Self {
        let mut mixer = Self {
            model,
            address: 0,
            registers: [0; 0x83],
            fixed_irq: irq,
            fixed_dma: dma,
        };
        mixer.registers[0x80] = match irq {
            2 | 9 => 1,
            5 => 2,
            7 => 4,
            10 => 8,
            _ => 0,
        };
        mixer.registers[0x81] = match dma {
            0 | 1 | 3 => 1 << dma,
            _ => 0,
        } | match dma16 {
            5..=7 => 1 << dma16,
            _ => 0,
        };
        mixer.reset();
        mixer
    }

    fn reset(&mut self) {
        // The mixer reset does not change the board's ISA resource selection.
        let irq = self.registers[0x80];
        let dma = self.registers[0x81];
        self.registers = [0; 0x83];
        self.registers[0x80] = irq;
        self.registers[0x81] = dma;
        if self.model == SbModel::Sb16 {
            self.registers[0x30..=0x35].fill(0xc0);
            self.registers[0x3c] = 0x1f;
            self.registers[0x3d] = 0x15;
            self.registers[0x3e] = 0x0b;
            self.registers[0x44..=0x47].fill(0x80);
        } else if self.is_pro() {
            self.registers[0x04] = 0x88;
            self.registers[0x22] = 0x88;
            self.registers[0x26] = 0x88;
        }
    }

    fn is_pro(&self) -> bool {
        matches!(self.model, SbModel::SbPro | SbModel::SbPro2)
    }

    fn has_mixer(&self) -> bool {
        self.is_pro() || self.model == SbModel::Sb16
    }

    pub(super) fn set_address(&mut self, address: u8) {
        self.address = address;
    }

    fn legacy_pair(address: u8) -> Option<usize> {
        match address {
            0x22 => Some(0x30),
            0x04 => Some(0x32),
            0x26 => Some(0x34),
            0x28 => Some(0x36),
            0x2e => Some(0x38),
            _ => None,
        }
    }

    fn mask(&self, address: u8) -> Option<u8> {
        if !self.has_mixer() {
            return None;
        }
        if self.is_pro() {
            return match address {
                0x00 => Some(0),
                0x04 | 0x22 | 0x26 | 0x28 | 0x2e => Some(0xee),
                0x0a => Some(0x06),
                0x0c => Some(0x2e),
                0x0e => Some(0x22),
                _ => None,
            };
        }
        match address {
            0x00 => Some(0),
            0x04 | 0x22 | 0x26 | 0x28 | 0x2e => Some(0xff),
            0x0a => Some(0x07),
            0x30..=0x3a => Some(0xf8),
            0x3b | 0x3f..=0x42 => Some(0xc0),
            0x3c => Some(0x1f),
            0x3d | 0x3e => Some(0x7f),
            0x43 => Some(1),
            0x44..=0x47 => Some(0xf0),
            0x80 => Some(0x0f),
            0x81 => Some(0xeb),
            0x82 => Some(0x03),
            _ => None,
        }
    }

    pub(super) fn read(&self, irq8: bool, irq16: bool) -> u8 {
        let Some(mask) = self.mask(self.address) else {
            return 0xff;
        };
        if self.model == SbModel::Sb16 {
            if self.address == 0x82 {
                return u8::from(irq8) | (u8::from(irq16) << 1);
            }
            if let Some(left) = Self::legacy_pair(self.address) {
                return (self.registers[left] & 0xf0) | (self.registers[left + 1] >> 4);
            }
            if self.address == 0x0a {
                return self.registers[0x3a] >> 5;
            }
        }
        self.registers[usize::from(self.address)] & mask
    }

    pub(super) fn write(&mut self, value: u8) {
        let Some(mask) = self.mask(self.address) else {
            return;
        };
        if self.address == 0 {
            self.reset();
        } else if self.model == SbModel::Sb16 && self.address == 0x82 {
            // Interrupt status is read-only; DSP acknowledge ports clear it.
        } else if self.model == SbModel::Sb16 {
            if let Some(left) = Self::legacy_pair(self.address) {
                self.registers[left] = Self::expand_legacy(value >> 4) << 3;
                self.registers[left + 1] = Self::expand_legacy(value & 15) << 3;
            } else if self.address == 0x0a {
                self.registers[0x3a] = (value & 7) << 5;
            } else {
                self.registers[usize::from(self.address)] = value & mask;
            }
        } else {
            self.registers[usize::from(self.address)] = value & mask;
        }
    }

    fn expand_legacy(nibble: u8) -> u8 {
        // Canonicalize both interfaces to native five-bit levels. Keep the
        // documented CC <-> C0 reset, while allowing legacy FF to reach unity.
        // Creative's legacy/native dB tables disagree by 2 dB at reset; these
        // aliases therefore approximate intermediate legacy attenuation.
        if nibble == 15 { 31 } else { nibble << 1 }
    }

    fn gain(&self, source: usize) -> [f32; 2] {
        if !self.has_mixer() {
            return [1.0; 2];
        }
        if self.model == SbModel::Sb16 {
            core::array::from_fn(|channel| {
                Self::native_gain(self.registers[0x30 + channel] >> 3)
                    * Self::native_gain(self.registers[source + channel] >> 3)
            })
        } else {
            let source = match source {
                0x32 => 0x04,
                _ => 0x26,
            };
            core::array::from_fn(|channel| {
                let shift = if channel == 0 { 5 } else { 1 };
                Self::pro_gain((self.registers[0x22] >> shift) & 7)
                    * Self::pro_gain((self.registers[source] >> shift) & 7)
            })
        }
    }

    fn native_gain(level: u8) -> f32 {
        // Treat level zero as mute; remaining levels follow CT1745's 2 dB
        // steps. This omits analog leakage/noise at the minimum setting.
        if level == 0 {
            0.0
        } else {
            10.0_f32.powf((f32::from(level) - 31.0) / 10.0)
        }
    }

    fn pro_gain(level: u8) -> f32 {
        // CT1345 is documented in approximate 4 dB steps, with a nonuniform
        // lowest setting. Use mute at zero and 4 dB steps up to unity at seven;
        // this gives -12 dB at default four versus the documented circa -11.
        if level == 0 {
            0.0
        } else {
            10.0_f32.powf((f32::from(level) - 7.0) / 5.0)
        }
    }

    pub(super) fn pcm_gain(&self) -> [f32; 2] {
        self.gain(0x32)
    }

    pub(super) fn fm_gain(&self) -> [f32; 2] {
        self.gain(0x34)
    }

    pub(super) fn stereo(&self) -> bool {
        self.is_pro() && self.registers[0x0e] & 2 != 0
    }

    pub(super) fn filter_enabled(&self) -> bool {
        self.is_pro() && self.registers[0x0e] & 0x20 == 0
    }

    fn single_selection(bits: u8) -> Option<u8> {
        // Invalid multiple selections have no active resource in this model.
        (bits.count_ones() == 1).then(|| bits.trailing_zeros() as u8)
    }

    pub(super) fn irq(&self) -> Option<u8> {
        if self.model != SbModel::Sb16 {
            return Some(self.fixed_irq);
        }
        match Self::single_selection(self.registers[0x80])? {
            0 => Some(2),
            1 => Some(5),
            2 => Some(7),
            3 => Some(10),
            _ => None,
        }
    }

    pub(super) fn dma8(&self) -> Option<u8> {
        if self.model != SbModel::Sb16 {
            return Some(self.fixed_dma);
        }
        Self::single_selection(self.registers[0x81] & 0x0b)
    }

    pub(super) fn dma16(&self) -> Option<u8> {
        if self.model != SbModel::Sb16 {
            return None;
        }
        let high = self.registers[0x81] & 0xe0;
        if high == 0 {
            self.dma8()
        } else {
            Self::single_selection(high)
        }
    }
}
