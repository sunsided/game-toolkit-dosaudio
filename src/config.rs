use crate::{Error, Result, Tick};

/// Sound Blaster playback hardware and DSP generation.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[non_exhaustive]
pub enum SbModel {
    /// Original Sound Blaster, DSP 1.05 and one OPL2.
    Sb1,
    /// Sound Blaster 2.0, DSP 2.01 and one OPL2.
    #[default]
    Sb2,
    /// First Sound Blaster Pro, DSP 3.00 and dual OPL2.
    SbPro,
    /// Sound Blaster Pro 2, DSP 3.02 and OPL3.
    SbPro2,
    /// Sound Blaster 16, DSP 4.05 and OPL3.
    Sb16,
}

impl SbModel {
    /// Version returned by the DSP E1h command.
    pub const fn dsp_version(self) -> (u8, u8) {
        match self {
            Self::Sb1 => (1, 5),
            Self::Sb2 => (2, 1),
            Self::SbPro => (3, 0),
            Self::SbPro2 => (3, 2),
            Self::Sb16 => (4, 5),
        }
    }
    /// Installed FM chips.
    pub const fn opl_chip(self) -> OplChip {
        match self {
            Self::Sb1 | Self::Sb2 => OplChip::Opl2,
            Self::SbPro => OplChip::DualOpl2,
            Self::SbPro2 | Self::Sb16 => OplChip::Opl3,
        }
    }
    pub(crate) const fn has_mixer(self) -> bool {
        matches!(self, Self::SbPro | Self::SbPro2 | Self::Sb16)
    }
}

/// ISA resources shared by Sound Blaster playback models.
///
/// Select the DSP generation and word DMA separately on [`SoundConfig`].
/// The host may service DRQ or attach an emulated memory DMA controller.
/// Resources are validated when constructing the machine.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SoundBlasterConfig {
    /// Base I/O address, aligned to 10h in 200h through 280h.
    pub base: u16,
    /// ISA interrupt line: 2, 3, 5 or 7; SB16 also supports 10.
    pub irq: u8,
    /// Byte-wide DMA channel: 0, 1 or 3.
    pub dma: u8,
}

impl Default for SoundBlasterConfig {
    fn default() -> Self {
        Self {
            base: 0x220,
            irq: 5,
            dma: 1,
        }
    }
}

/// Installed Yamaha FM hardware.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OplChip {
    /// Nine-channel mono YM3812.
    Opl2,
    /// Two independent YM3812 chips, routed left and right.
    DualOpl2,
    /// YMF262, initially in its OPL2 compatibility mode.
    Opl3,
}

/// FM synthesis implementation selected at machine creation.
#[derive(Clone, Default)]
pub enum OplBackend {
    /// MIT-licensed clean-room core.
    #[default]
    Oplon,
    /// LGPL-2.1-or-later reference core, compiled with the `nuked` feature.
    #[cfg(feature = "nuked")]
    Nuked,
    /// A factory invoked once per physical chip, before rendering starts.
    ///
    /// Dual OPL2 calls it twice, producing independent cores. A custom core's
    /// methods must follow the allocation and native-rate contract of `OplCore`.
    #[cfg(feature = "opl")]
    Custom(std::sync::Arc<dyn Fn() -> Box<dyn crate::OplCore> + Send + Sync>),
}

impl core::fmt::Debug for OplBackend {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            Self::Oplon => "Oplon",
            #[cfg(feature = "nuked")]
            Self::Nuked => "Nuked",
            #[cfg(feature = "opl")]
            Self::Custom(_) => "Custom",
        })
    }
}

/// Hardware and output configuration. Builders are validated by `DosAudio::split`.
#[derive(Clone, Debug)]
pub struct SoundConfig {
    pub(crate) opl: Option<OplChip>,
    pub(crate) sb: Option<SoundBlasterConfig>,
    pub(crate) sb_model: SbModel,
    pub(crate) dma16: u8,
    pub(crate) backend: OplBackend,
    pub(crate) speaker: bool,
    pub(crate) psg: bool,
    pub(crate) midi: Option<crate::MidiBackend>,
    pub(crate) midi_port: u16,
    pub(crate) sample_rate: u32,
    pub(crate) queue_capacity: usize,
    pub(crate) latency: Tick,
}

impl Default for SoundConfig {
    fn default() -> Self {
        Self::adlib()
    }
}

impl SoundConfig {
    /// No installed sound hardware.
    pub fn none() -> Self {
        Self {
            opl: None,
            sb: None,
            sb_model: SbModel::Sb2,
            dma16: 5,
            backend: OplBackend::Oplon,
            speaker: false,
            psg: false,
            midi: None,
            midi_port: 0x330,
            sample_rate: 48_000,
            queue_capacity: 16_384,
            latency: Tick::from_micros(10_000),
        }
    }

    /// An AdLib with an OPL2 at ports 388h/389h.
    pub fn adlib() -> Self {
        Self {
            opl: Some(OplChip::Opl2),
            ..Self::none()
        }
    }

    /// Sound Blaster 2.0 at 220h, IRQ 5, DMA 1, with its OPL2.
    pub fn sb2() -> Self {
        Self::none().sound_blaster(SoundBlasterConfig::default())
    }

    /// An original Sound Blaster using the default ISA resources.
    pub fn sb1() -> Self {
        Self::none().sb_model(SbModel::Sb1)
    }
    /// A first-generation Sound Blaster Pro with dual OPL2.
    pub fn sb_pro() -> Self {
        Self::none().sb_model(SbModel::SbPro)
    }
    /// A Sound Blaster Pro 2 with OPL3.
    pub fn sb_pro2() -> Self {
        Self::none().sb_model(SbModel::SbPro2)
    }
    /// A Sound Blaster 16, with byte DMA 1 and word DMA 5.
    pub fn sb16() -> Self {
        Self::none().sb_model(SbModel::Sb16)
    }

    /// Select a DSP model, installing default ISA resources if no card is present.
    #[must_use]
    pub fn sb_model(mut self, model: SbModel) -> Self {
        self.sb_model = model;
        self.sb.get_or_insert_with(SoundBlasterConfig::default);
        self.opl = Some(model.opl_chip());
        self
    }

    /// Select SB16 DMA: word channel 5/6/7, or the same byte channel as D.
    #[must_use]
    pub fn dma16(mut self, channel: u8) -> Self {
        self.dma16 = channel;
        self
    }

    /// Parse a BLASTER value containing A (hex), I, D, T and optional H fields.
    ///
    /// A/I/D/T are required; T values 1/2/3/4/6 select SB1/Pro/SB2/Pro2/SB16.
    /// H is valid only for SB16 and defaults to 5. Unknown or duplicate fields
    /// are errors. Resource and feature validation also runs at construction.
    pub fn from_blaster(value: &str) -> Result<Self> {
        let mut fields = [None; 5];
        for field in value.split_ascii_whitespace() {
            if !field.is_ascii() {
                return Err(Error::Config("BLASTER fields must be ASCII"));
            }
            let (key, number) = field.split_at(1);
            let index = match key.to_ascii_uppercase().as_str() {
                "A" => 0,
                "I" => 1,
                "D" => 2,
                "T" => 3,
                "H" => 4,
                _ => return Err(Error::Config("unsupported BLASTER field")),
            };
            if fields[index].is_some() {
                return Err(Error::Config("duplicate BLASTER field"));
            }
            let number = if index == 0 {
                number
                    .strip_prefix("0x")
                    .or_else(|| number.strip_prefix("0X"))
                    .unwrap_or(number)
            } else {
                number
            };
            fields[index] = Some(
                u16::from_str_radix(number, if index == 0 { 16 } else { 10 })
                    .map_err(|_| Error::Config("invalid BLASTER field value"))?,
            );
        }
        let [Some(base), Some(irq), Some(dma), Some(card), high] = fields else {
            return Err(Error::Config("BLASTER requires A, I, D and T fields"));
        };
        let model = match card {
            1 => SbModel::Sb1,
            2 => SbModel::SbPro,
            3 => SbModel::Sb2,
            4 => SbModel::SbPro2,
            6 => SbModel::Sb16,
            _ => return Err(Error::Config("unsupported BLASTER card type")),
        };
        if high.is_some() && model != SbModel::Sb16 {
            return Err(Error::Config("BLASTER H requires an SB16"));
        }
        let resources = SoundBlasterConfig {
            base,
            irq: u8::try_from(irq).map_err(|_| Error::Config("invalid BLASTER IRQ"))?,
            dma: u8::try_from(dma).map_err(|_| Error::Config("invalid BLASTER DMA"))?,
        };
        let config = Self::none().sb_model(model).sound_blaster(resources).dma16(
            u8::try_from(high.unwrap_or(5))
                .map_err(|_| Error::Config("invalid BLASTER high DMA"))?,
        );
        config.validate()?;
        Ok(config)
    }

    /// Install a Sound Blaster 2.0 and its OPL2 using the supplied ISA resources.
    #[must_use]
    pub fn sound_blaster(mut self, resources: SoundBlasterConfig) -> Self {
        self.sb = Some(resources);
        self.opl = Some(self.sb_model.opl_chip());
        self
    }

    /// An OPL3 at ports 388h through 38Bh.
    pub fn opl3() -> Self {
        Self {
            opl: Some(OplChip::Opl3),
            ..Self::none()
        }
    }

    /// Two OPL2 chips at ports 388h through 38Bh.
    pub fn dual_opl2() -> Self {
        Self {
            opl: Some(OplChip::DualOpl2),
            ..Self::none()
        }
    }

    /// Install or remove PIT channel 2 and the PC speaker.
    #[must_use]
    pub fn speaker(mut self, enabled: bool) -> Self {
        self.speaker = enabled;
        self
    }

    /// Install an MPU-401 UART and MIDI synthesis backend.
    #[must_use]
    pub fn mpu401(mut self, backend: crate::MidiBackend) -> Self {
        self.midi = Some(backend);
        self
    }
    /// General MIDI using an MPU-401 UART and caller-supplied SoundFont.
    #[cfg(feature = "gm")]
    pub fn gm(font: std::sync::Arc<crate::SoundFont>) -> Self {
        Self::none().mpu401(crate::MidiBackend::Gm(font))
    }
    /// Relocate the MPU-401 to an aligned base in 300h through 360h.
    #[must_use]
    pub fn midi_port(mut self, base: u16) -> Self {
        self.midi_port = base;
        self
    }
    /// Install or remove the Tandy SN76489 PSG at C0h.
    #[must_use]
    pub fn psg(mut self, enabled: bool) -> Self {
        self.psg = enabled;
        self
    }

    /// Select the FM synthesis backend.
    #[must_use]
    pub fn opl_backend(mut self, backend: OplBackend) -> Self {
        self.backend = backend;
        self
    }

    /// Output sample rate, from 8,000 through 192,000 Hz.
    #[must_use]
    pub fn sample_rate(mut self, sample_rate: u32) -> Self {
        self.sample_rate = sample_rate;
        self
    }

    /// Fixed command queue capacity; must be nonzero.
    #[must_use]
    pub fn queue_capacity(mut self, capacity: usize) -> Self {
        self.queue_capacity = capacity;
        self
    }

    /// Delay timestamped port writes by this much output time.
    ///
    /// Live playback defaults to 10 ms. Offline machines force zero latency.
    /// Increase this for a host with a larger audio callback or game-frame jitter.
    #[must_use]
    pub fn latency(mut self, latency: Tick) -> Self {
        self.latency = latency;
        self
    }

    pub(crate) fn validate(&self) -> Result<()> {
        if self.midi.is_some() {
            if !cfg!(feature = "mpu401") {
                return Err(Error::Config("MPU-401 requires the mpu401 feature"));
            }
            if !(0x300..=0x360).contains(&self.midi_port) || self.midi_port & 0xf != 0 {
                return Err(Error::Config("invalid MPU-401 base"));
            }
        }
        if self.psg && !cfg!(feature = "psg") {
            return Err(Error::Config("PSG requires the psg feature"));
        }
        if let Some(sb) = self.sb {
            if !cfg!(feature = "sb") {
                return Err(Error::Config("Sound Blaster requires the sb feature"));
            }
            if !(0x200..=0x280).contains(&sb.base) || sb.base & 0x0f != 0 {
                return Err(Error::Config("invalid Sound Blaster base address"));
            }
            if !matches!(sb.irq, 2 | 3 | 5 | 7 | 10) || !matches!(sb.dma, 0 | 1 | 3) {
                return Err(Error::Config("invalid Sound Blaster IRQ or DMA channel"));
            }
            if sb.irq == 10 && self.sb_model != SbModel::Sb16 {
                return Err(Error::Config("IRQ 10 requires an SB16"));
            }
            if !(5..=7).contains(&self.dma16)
                && !(self.sb_model == SbModel::Sb16 && self.dma16 == sb.dma)
            {
                return Err(Error::Config("word DMA channel must be 5, 6 or 7"));
            }
        }
        if !(8_000..=192_000).contains(&self.sample_rate) {
            return Err(Error::Config("sample rate must be 8,000..=192,000 Hz"));
        }
        if self.queue_capacity == 0 || self.queue_capacity > 1_048_576 {
            return Err(Error::Config("queue capacity must be 1..=1,048,576"));
        }
        if self.opl.is_some() && !cfg!(feature = "opl") {
            return Err(Error::Config("OPL hardware requires the opl feature"));
        }
        Ok(())
    }
}
