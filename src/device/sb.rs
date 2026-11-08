use crate::{
    DmaChannel, DmaDirection, DmaRequest, DmaWidth, Error, IrqLine, IrqRequest, Result, SbModel,
    SoundBlasterConfig, Tick,
};
mod mixer;
use mixer::Mixer;

#[derive(Clone, Copy, Debug)]
pub(crate) struct DacState {
    samples: [i16; 2],
    enabled: bool,
    pub fm_gain: [f32; 2],
    pcm_gain: [f32; 2],
    pub filter_hz: u32,
}
impl Default for DacState {
    fn default() -> Self {
        Self {
            samples: [0; 2],
            enabled: false,
            fm_gain: [1.0; 2],
            pcm_gain: [1.0; 2],
            filter_hz: 0,
        }
    }
}
impl DacState {
    pub fn output(self) -> [f64; 2] {
        if !self.enabled {
            return [0.0; 2];
        }
        core::array::from_fn(|c| f64::from(self.samples[c]) / 32768.0 * f64::from(self.pcm_gain[c]))
    }
}

#[derive(Clone, Copy, Default)]
struct Format {
    word: bool,
    signed: bool,
    stereo: bool,
}
#[derive(Clone, Copy)]
struct Transfer {
    remaining: u32,
    auto_init: bool,
    direct_rate: bool,
    high_speed: bool,
    deadline: u128,
    period: u128,
    denominator: u128,
    paused: Option<u128>,
    format: Format,
    right: bool,
    low: Option<u8>,
}

/// Timestamped playback DSP. The host or an attached buffer supplies DMA units.
#[derive(Clone, Copy)]
pub(crate) struct SbControl {
    pub resources: SoundBlasterConfig,
    pub model: SbModel,
    mixer: Mixer,
    reset_since: Option<Tick>,
    command: u8,
    parameters: [u8; 3],
    received: usize,
    needed: usize,
    replies: [u8; 16],
    reply_head: usize,
    reply_len: usize,
    time_constant: u8,
    sample_rate: u16,
    block_size: u32,
    transfer: Option<Transfer>,
    irq8: bool,
    irq16: bool,
    blocks: u64,
    dac: DacState,
}
impl SbControl {
    pub fn new(resources: SoundBlasterConfig, model: SbModel, dma16: u8) -> Self {
        Self {
            resources,
            model,
            mixer: Mixer::new(model, resources.irq, resources.dma, dma16),
            reset_since: None,
            command: 0,
            parameters: [0; 3],
            received: 0,
            needed: 0,
            replies: [0; 16],
            reply_head: 0,
            reply_len: 0,
            time_constant: 156,
            sample_rate: 22_050,
            block_size: 65_536,
            transfer: None,
            irq8: false,
            irq16: false,
            blocks: 0,
            dac: DacState::default(),
        }
    }
    pub fn output(&self) -> DacState {
        let mut dac = self.dac;
        dac.pcm_gain = self.mixer.pcm_gain();
        dac.fm_gain = self.mixer.fm_gain();
        // Simple analog approximations; retain the published SB2 ideal DAC.
        dac.filter_hz = match self.model {
            SbModel::Sb1 => 3_800,
            SbModel::SbPro | SbModel::SbPro2 if self.mixer.filter_enabled() => 3_200,
            SbModel::Sb16 => u32::from(self.sample_rate) / 2,
            _ => 0,
        };
        dac
    }
    fn reply(&mut self, value: u8) {
        if self.reply_len < self.replies.len() {
            self.replies[(self.reply_head + self.reply_len) % self.replies.len()] = value;
            self.reply_len += 1;
        }
    }
    // Approximate the supported rate ranges with integer-microsecond periods.
    // Clamp out-of-range time constants instead of accepting impossible clocks:
    // normal mono 44..250 us, high-speed mono 23..43 us, Pro stereo 23..45 us
    // per scalar channel sample. SB16's direct clock is clamped by command 41h.
    fn clock(&self, format: Format, direct_rate: bool, high_speed: bool) -> (u128, u128) {
        if direct_rate {
            return (
                u128::from(Tick::HZ_NUMERATOR),
                u128::from(Tick::HZ_DENOMINATOR)
                    * u128::from(self.sample_rate)
                    * if format.stereo { 2 } else { 1 },
            );
        }
        let micros = 256 - u128::from(self.time_constant);
        let micros = if format.stereo {
            micros.clamp(23, 45)
        } else if high_speed {
            micros.clamp(23, 43)
        } else {
            micros.clamp(44, 250)
        };
        (micros * u128::from(Tick::HZ_NUMERATOR), 22_000_000)
    }

    fn start(
        &mut self,
        at: Tick,
        count: u32,
        auto_init: bool,
        format: Format,
        direct_rate: bool,
        high_speed: bool,
    ) {
        let (period, denominator) = self.clock(format, direct_rate, high_speed);
        self.transfer = Some(Transfer {
            remaining: count,
            auto_init,
            direct_rate,
            high_speed,
            deadline: u128::from(at.0) * denominator + period,
            period,
            denominator,
            paused: None,
            format,
            right: false,
            low: None,
        });
    }

    fn start_legacy(&mut self, at: Tick, count: u32, auto_init: bool, high_speed: bool) {
        let format = Format {
            stereo: self.mixer.stereo(),
            ..Format::default()
        };
        self.start(at, count, auto_init, format, false, high_speed);
    }

    fn update_rate(&mut self, direct_rate: bool) {
        let Some(transfer) = self.transfer.filter(|t| t.direct_rate == direct_rate) else {
            return;
        };
        let (period, denominator) = self.clock(transfer.format, direct_rate, transfer.high_speed);
        let transfer = self.transfer.as_mut().expect("active transfer");
        // Keep the pending request and paused duration in absolute crystal time.
        // Split whole/fractional ticks to avoid overflowing on distant timestamps.
        fn rescale(value: u128, previous: u128, next: u128) -> u128 {
            value / previous * next + value % previous * next / previous
        }
        transfer.deadline = rescale(transfer.deadline, transfer.denominator, denominator);
        transfer.paused = transfer
            .paused
            .map(|left| rescale(left, transfer.denominator, denominator));
        transfer.period = period;
        transfer.denominator = denominator;
    }

    pub fn begin_pcm(
        &mut self,
        at: Tick,
        count: u32,
        time_constant: u8,
    ) -> Result<(DacState, Tick)> {
        if self.reset_since.is_some() {
            return Err(Error::Config("Sound Blaster DSP is held in reset"));
        }
        self.needed = 0;
        self.received = 0;
        self.time_constant = time_constant;
        self.dac.samples = [0; 2];
        self.dac.enabled = true;
        self.irq8 = false;
        self.irq16 = false;
        // The convenience byte API remains mono regardless of the Pro mixer switch.
        self.start(at, count, false, Format::default(), false, false);
        let transfer = self.transfer.expect("new PCM transfer");
        let end =
            u128::from(at.0) * transfer.denominator + (u128::from(count) + 1) * transfer.period;
        let end = Tick(u64::try_from(end / transfer.denominator).map_err(|_| Error::TooLarge)?);
        Ok((self.output(), end))
    }
    pub fn stop_pcm(&mut self) -> DacState {
        self.transfer = None;
        self.dac.samples = [0; 2];
        self.dac.enabled = false;
        self.output()
    }
    fn pause(&mut self, at: Tick, word: bool, enabled: bool) {
        if let Some(t) = &mut self.transfer
            && t.format.word == word
        {
            if enabled {
                if let Some(left) = t.paused.take() {
                    t.deadline = u128::from(at.0) * t.denominator + left;
                }
            } else if t.paused.is_none() {
                t.paused = Some(t.deadline.saturating_sub(u128::from(at.0) * t.denominator));
            }
        }
    }
    pub fn write(&mut self, at: Tick, port: u16, value: u8) -> Option<DacState> {
        match port - self.resources.base {
            4 if self.model.has_mixer() => {
                self.mixer.set_address(value);
                return None;
            }
            5 if self.model.has_mixer() => {
                self.mixer.write(value);
                if let Some(irq) = self.mixer.irq() {
                    self.resources.irq = irq;
                }
                if let Some(dma) = self.mixer.dma8() {
                    self.resources.dma = dma;
                }
                return Some(self.output());
            }
            6 => {
                if value & 1 != 0 && self.reset_since.is_none() {
                    let mixer = self.mixer; // DSP reset preserves the separately addressed mixer.
                    *self = Self::new(self.resources, self.model, 5);
                    self.mixer = mixer;
                    self.reset_since = Some(at);
                    return Some(self.output());
                }
                if value & 1 == 0
                    && let Some(since) = self.reset_since.take()
                    && at - since >= Tick::from_micros(3)
                {
                    self.reply(0xaa);
                }
                return None;
            }
            0x0c if self.reset_since.is_none() => {}
            _ => return None,
        }
        if self.needed != 0 {
            self.parameters[self.received] = value;
            self.received += 1;
            if self.received != self.needed {
                return None;
            }
            self.needed = 0;
            let count = u32::from(u16::from_le_bytes([self.parameters[0], self.parameters[1]])) + 1;
            match self.command {
                0x10 => {
                    self.dac.samples = [(i16::from(self.parameters[0]) - 128) << 8; 2];
                    return Some(self.output());
                }
                0x14 => self.start_legacy(at, count, false, false),
                0x40 => {
                    self.time_constant = self.parameters[0];
                    self.update_rate(false);
                }
                0x41 if self.model == SbModel::Sb16 => {
                    self.sample_rate = u16::from_be_bytes([self.parameters[0], self.parameters[1]])
                        .clamp(5_000, 45_000);
                    self.update_rate(true);
                    return Some(self.output());
                }
                0x48 if self.model != SbModel::Sb1 => self.block_size = count,
                0xe0 => self.reply(!self.parameters[0]),
                0xb0..=0xcf if self.model == SbModel::Sb16 && self.command & 8 == 0 => {
                    let count =
                        u32::from(u16::from_le_bytes([self.parameters[1], self.parameters[2]])) + 1;
                    let format = Format {
                        word: self.command < 0xc0,
                        signed: self.parameters[0] & 0x10 != 0,
                        stereo: self.parameters[0] & 0x20 != 0,
                    };
                    self.block_size = count;
                    self.dac.enabled = true;
                    self.start(at, count, self.command & 4 != 0, format, true, false);
                    return Some(self.output());
                }
                _ => {} // Recognized but unsupported input/recording commands consume parameters.
            }
            return None;
        }
        self.command = value;
        self.received = 0;
        self.needed = match value {
            0x10 | 0x40 | 0xe0 => 1,
            0x14 | 0x41 | 0x42 | 0x48 | 0x80 => 2,
            0xb0..=0xcf if self.model == SbModel::Sb16 => 3,
            _ => 0,
        };
        match value {
            0x1c if self.model != SbModel::Sb1 => {
                self.start_legacy(at, self.block_size, true, false)
            }
            0x90 | 0x91 if self.model != SbModel::Sb1 => {
                self.start_legacy(at, self.block_size, value == 0x90, true)
            }
            0xd0 => self.pause(at, false, false),
            0xd4 => self.pause(at, false, true),
            0xd5 if self.model == SbModel::Sb16 => self.pause(at, true, false),
            0xd6 if self.model == SbModel::Sb16 => self.pause(at, true, true),
            0xda | 0xd9 => {
                if let Some(t) = &mut self.transfer
                    && t.format.word == (value == 0xd9)
                {
                    t.auto_init = false;
                }
            }
            0xd1 | 0xd3 => {
                self.dac.enabled = value == 0xd1;
                return Some(self.output());
            }
            0xd8 => self.reply(if self.dac.enabled { 0xff } else { 0 }),
            0xe1 => {
                let (major, minor) = self.model.dsp_version();
                self.reply(major);
                self.reply(minor);
            }
            0xf2 => self.irq8 = true,
            0xf3 if self.model == SbModel::Sb16 => self.irq16 = true,
            _ => {}
        }
        None
    }
    pub fn read(&mut self, port: u16) -> u8 {
        match port - self.resources.base {
            4 if self.model.has_mixer() => 0xff,
            5 if self.model.has_mixer() => self.mixer.read(self.irq8, self.irq16),
            0x0a if self.reply_len != 0 => {
                let value = self.replies[self.reply_head];
                self.reply_head = (self.reply_head + 1) % self.replies.len();
                self.reply_len -= 1;
                value
            }
            0x0c => {
                if self.reset_since.is_some() {
                    0x80
                } else {
                    0
                }
            }
            0x0e => {
                self.irq8 = false;
                if self.reply_len != 0 { 0x80 } else { 0 }
            }
            0x0f if self.model == SbModel::Sb16 => {
                self.irq16 = false;
                0xff
            }
            _ => 0xff,
        }
    }
    pub fn irq_request(&self) -> IrqRequest {
        IrqRequest {
            line: IrqLine::new(self.resources.irq).expect("validated IRQ"),
            asserted: self.mixer.irq().is_some() && (self.irq8 || self.irq16),
        }
    }
    pub fn next_dma_tick(&self) -> Option<Tick> {
        let t = self.transfer?;
        if t.paused.is_some() {
            return None;
        }
        u64::try_from(t.deadline / t.denominator).ok().map(Tick)
    }
    pub fn completed_blocks(&self) -> u64 {
        self.blocks
    }
    pub fn next_block_tick(&self) -> Option<Tick> {
        let t = self.transfer?;
        if t.paused.is_some() {
            return None;
        }
        u64::try_from((t.deadline + u128::from(t.remaining - 1) * t.period) / t.denominator)
            .ok()
            .map(Tick)
    }
    pub fn dma_request(&self, at: Tick) -> Option<DmaRequest> {
        if self.next_dma_tick()? > at {
            return None;
        }
        Some(DmaRequest {
            channel: self.dma_channel()?,
            direction: DmaDirection::MemoryToDevice,
        })
    }
    /// Selected channel for the active format, including while paused, or byte DMA when idle.
    pub(crate) fn dma_channel(&self) -> Option<DmaChannel> {
        let number = if self.transfer.is_some_and(|t| t.format.word) {
            self.mixer.dma16()?
        } else {
            self.mixer.dma8()?
        };
        DmaChannel::new(number)
    }
    fn check_dma(&self, at: Tick, channel: DmaChannel) -> Result<()> {
        if self.dma_request(at).is_none_or(|r| r.channel != channel) {
            return Err(Error::Config("no matching Sound Blaster DMA request"));
        }
        Ok(())
    }
    pub fn dma_write(&mut self, at: Tick, channel: DmaChannel, value: u8) -> Result<DacState> {
        self.check_dma(at, channel)?;
        let t = self.transfer.as_mut().expect("asserted DRQ");
        if t.format.word {
            if channel.width() != DmaWidth::Byte {
                return Err(Error::Config("word DMA requires a 16-bit transfer unit"));
            }
            if let Some(low) = t.low.take() {
                self.sample(at, u16::from_le_bytes([low, value]))
            } else {
                t.low = Some(value);
                Ok(self.output())
            }
        } else {
            self.sample(at, u16::from(value))
        }
    }
    pub fn dma_write16(&mut self, at: Tick, channel: DmaChannel, value: u16) -> Result<DacState> {
        self.check_dma(at, channel)?;
        let t = self.transfer.expect("asserted DRQ");
        if !t.format.word || channel.width() != DmaWidth::Word {
            return Err(Error::Config("DSP is not requesting word-wide DMA"));
        }
        self.sample(at, value)
    }
    fn sample(&mut self, at: Tick, value: u16) -> Result<DacState> {
        let t = self.transfer.as_mut().expect("asserted DRQ");
        let sample = match (t.format.word, t.format.signed) {
            (true, true) => value as i16,
            (true, false) => value.wrapping_sub(32768) as i16,
            (false, true) => (value as u8 as i8 as i16) << 8,
            (false, false) => (value as i16 - 128) << 8,
        };
        if t.format.stereo {
            self.dac.samples[usize::from(t.right)] = sample;
            t.right = !t.right;
        } else {
            self.dac.samples = [sample; 2];
        }
        t.remaining -= 1;
        if t.remaining == 0 {
            self.blocks = self.blocks.wrapping_add(1);
            if t.format.word {
                self.irq16 = true;
            } else {
                self.irq8 = true;
            }
            if t.auto_init {
                t.remaining = self.block_size;
            } else {
                self.transfer = None;
                return Ok(self.output());
            }
        }
        t.deadline = t.deadline.max(u128::from(at.0) * t.denominator) + t.period;
        Ok(self.output())
    }
}
