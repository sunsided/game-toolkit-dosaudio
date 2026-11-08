use crate::{
    DmaChannel, DmaDirection, DmaRequest, Error, IrqLine, IrqRequest, Result, SoundBlasterConfig,
    Tick,
};

// One microsecond is exactly HZ_NUMERATOR / DENOMINATOR crystal ticks.
const DENOMINATOR: u128 = 22_000_000;

#[derive(Clone, Copy, Debug)]
pub(crate) struct DacState {
    sample: u8,
    enabled: bool,
}

impl Default for DacState {
    fn default() -> Self {
        Self {
            sample: 128,
            enabled: false,
        }
    }
}

impl DacState {
    pub fn output(self) -> f64 {
        if self.enabled {
            (f64::from(self.sample) - 128.0) / 128.0
        } else {
            0.0
        }
    }
}

#[derive(Clone, Copy)]
struct Transfer {
    remaining: u32,
    auto_init: bool,
    // A rational crystal timestamp; retaining the fraction prevents DMA drift.
    deadline: u128,
    paused: Option<u128>,
}

/// The DSP has no memory or audio-thread state. A host DMA controller services
/// each request through DosAudio::try_dma_write_at at the advertised timestamp.
#[derive(Clone, Copy)]
pub(crate) struct SbControl {
    pub resources: SoundBlasterConfig,
    reset_since: Option<Tick>,
    command: u8,
    parameters: [u8; 2],
    received: usize,
    needed: usize,
    replies: [u8; 16],
    reply_head: usize,
    reply_len: usize,
    time_constant: u8,
    block_size: u32,
    transfer: Option<Transfer>,
    irq: bool,
    dac: DacState,
}

impl SbControl {
    pub fn new(resources: SoundBlasterConfig) -> Self {
        Self {
            resources,
            reset_since: None,
            command: 0,
            parameters: [0; 2],
            received: 0,
            needed: 0,
            replies: [0; 16],
            reply_head: 0,
            reply_len: 0,
            time_constant: 156,
            block_size: 65_536,
            transfer: None,
            irq: false,
            dac: DacState::default(),
        }
    }

    fn reply(&mut self, value: u8) {
        if self.reply_len < self.replies.len() {
            self.replies[(self.reply_head + self.reply_len) % self.replies.len()] = value;
            self.reply_len += 1;
        }
    }

    fn period(&self) -> u128 {
        (256 - u128::from(self.time_constant)) * u128::from(Tick::HZ_NUMERATOR)
    }

    fn start(&mut self, at: Tick, count: u32, auto_init: bool) {
        self.transfer = Some(Transfer {
            remaining: count,
            auto_init,
            deadline: u128::from(at.0) * DENOMINATOR + self.period(),
            paused: None,
        });
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
        self.dac = DacState {
            sample: 128,
            enabled: true,
        };
        self.irq = false;
        self.start(at, count, false);
        // Include one complete hold interval for the last byte, then mute.
        let end = u128::from(at.0) * DENOMINATOR + (u128::from(count) + 1) * self.period();
        let end = Tick(u64::try_from(end / DENOMINATOR).map_err(|_| Error::TooLarge)?);
        Ok((self.dac, end))
    }

    pub fn stop_pcm(&mut self) -> DacState {
        self.transfer = None;
        self.dac = DacState::default();
        self.dac
    }

    pub fn write(&mut self, at: Tick, port: u16, value: u8) -> Option<DacState> {
        match port - self.resources.base {
            6 => {
                if value & 1 != 0 && self.reset_since.is_none() {
                    *self = Self::new(self.resources);
                    self.reset_since = Some(at);
                    return Some(self.dac);
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
            let count = u32::from(u16::from_le_bytes(self.parameters)) + 1;
            match self.command {
                0x10 => {
                    self.dac.sample = self.parameters[0];
                    return Some(self.dac);
                }
                0x14 => self.start(at, count, false),
                0x40 => self.time_constant = self.parameters[0],
                0x48 => self.block_size = count,
                0xe0 => self.reply(!self.parameters[0]),
                _ => unreachable!("only supported commands collect parameters"),
            }
            return None;
        }
        self.command = value;
        self.received = 0;
        self.needed = match value {
            0x10 | 0x40 | 0xe0 => 1,
            0x14 | 0x48 => 2,
            _ => 0,
        };
        match value {
            0x1c => self.start(at, self.block_size, true),
            0xd0 => {
                if let Some(transfer) = &mut self.transfer
                    && transfer.paused.is_none()
                {
                    transfer.paused = Some(
                        transfer
                            .deadline
                            .saturating_sub(u128::from(at.0) * DENOMINATOR),
                    );
                }
            }
            0xd4 => {
                if let Some(transfer) = &mut self.transfer
                    && let Some(left) = transfer.paused.take()
                {
                    transfer.deadline = u128::from(at.0) * DENOMINATOR + left;
                }
            }
            0xda => {
                if let Some(transfer) = &mut self.transfer {
                    transfer.auto_init = false;
                }
            }
            0xd1 | 0xd3 => {
                self.dac.enabled = value == 0xd1;
                return Some(self.dac);
            }
            0xd8 => self.reply(if self.dac.enabled { 0xff } else { 0 }),
            0xe1 => {
                self.reply(2);
                self.reply(1);
            }
            0xf2 => self.irq = true,
            _ => {} // Unsupported DSP commands are ignored.
        }
        None
    }

    pub fn read(&mut self, port: u16) -> u8 {
        match port - self.resources.base {
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
                self.irq = false;
                if self.reply_len != 0 { 0x80 } else { 0 }
            }
            _ => 0xff,
        }
    }

    pub fn irq_request(&self) -> IrqRequest {
        IrqRequest {
            line: IrqLine::new(self.resources.irq).expect("validated IRQ"),
            asserted: self.irq,
        }
    }

    pub fn next_dma_tick(&self) -> Option<Tick> {
        let transfer = self.transfer?;
        if transfer.paused.is_some() {
            return None;
        }
        u64::try_from(transfer.deadline / DENOMINATOR)
            .ok()
            .map(Tick)
    }

    pub fn dma_request(&self, at: Tick) -> Option<DmaRequest> {
        (self.next_dma_tick()? <= at).then(|| DmaRequest {
            channel: DmaChannel::new(self.resources.dma).expect("validated DMA"),
            direction: DmaDirection::MemoryToDevice,
        })
    }

    pub fn dma_write(&mut self, at: Tick, channel: DmaChannel, value: u8) -> Result<DacState> {
        if self
            .dma_request(at)
            .is_none_or(|request| request.channel != channel)
        {
            return Err(Error::Config("no matching Sound Blaster DMA request"));
        }
        let period = self.period();
        let transfer = self.transfer.as_mut().expect("asserted DRQ");
        self.dac.sample = value;
        transfer.remaining -= 1;
        if transfer.remaining == 0 {
            self.irq = true;
            if transfer.auto_init {
                transfer.remaining = self.block_size;
            } else {
                self.transfer = None;
                return Ok(self.dac);
            }
        }
        // A late host stalls the transfer; never emit retroactive samples. An
        // on-time host retains the exact fractional phase across all requests.
        transfer.deadline = transfer.deadline.max(u128::from(at.0) * DENOMINATOR) + period;
        Ok(self.dac)
    }
}
