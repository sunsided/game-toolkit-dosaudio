use crate::{Error, Result, Tick};

/// ISA resources for a Sound Blaster 2.0 (DSP 2.01, unsigned 8-bit mono).
///
/// The host owns the DMA controller and supplies bytes when DRQ is asserted.
/// Resources are validated when constructing the machine.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SoundBlasterConfig {
    /// Base I/O address, aligned to 10h in 200h through 280h.
    pub base: u16,
    /// ISA interrupt line: 2, 3, 5 or 7.
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
    pub(crate) backend: OplBackend,
    pub(crate) speaker: bool,
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
            backend: OplBackend::Oplon,
            speaker: false,
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

    /// Install a Sound Blaster 2.0 and its OPL2 using the supplied ISA resources.
    #[must_use]
    pub fn sound_blaster(mut self, resources: SoundBlasterConfig) -> Self {
        self.sb = Some(resources);
        self.opl = Some(OplChip::Opl2);
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
        if let Some(sb) = self.sb {
            if !cfg!(feature = "sb") {
                return Err(Error::Config("Sound Blaster requires the sb feature"));
            }
            if !(0x200..=0x280).contains(&sb.base) || sb.base & 0x0f != 0 {
                return Err(Error::Config("invalid Sound Blaster base address"));
            }
            if !matches!(sb.irq, 2 | 3 | 5 | 7) || !matches!(sb.dma, 0 | 1 | 3) {
                return Err(Error::Config("invalid Sound Blaster IRQ or DMA channel"));
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
