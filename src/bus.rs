#[cfg(feature = "opl")]
use crate::device::opl::OplControl;
#[cfg(feature = "sb")]
use crate::device::sb::{DacState, SbControl};
use crate::device::speaker::{SpeakerControl, SpeakerState};
#[cfg(feature = "sb")]
use crate::sfx::{PcmPlan, PcmPlayback};
use crate::{
    AudioEngine, Capture, Error, IoDevice, PortRange, PortWrite, Result, SoundConfig, Tick,
};
use rtrb::Producer;

#[derive(Clone, Copy, Debug)]
pub(crate) enum Command {
    Speaker(SpeakerState),
    #[cfg(feature = "sb")]
    Dac(DacState),
    #[cfg(feature = "opl")]
    Opl {
        register: u16,
        value: u8,
    },
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct TimedCommand {
    pub at: Tick,
    pub command: Command,
}

/// Game-thread hardware control, including port reads and device timers.
///
/// There is one owner of port I/O. Implementing [`IoDevice`] allows moving this
/// object directly into a host bus. `outb`/`inb` do not charge time themselves;
/// use `advance`, `wait_us`, or timestamps supplied by the host.
///
/// Fallible writes leave state unchanged when the queue is full. The infallible
/// helpers and [`IoDevice::outb`] panic on overflow rather than lose commands.
pub struct DosAudio {
    now: Tick,
    ports: Vec<PortRange>,
    speaker: Option<SpeakerControl>,
    #[cfg(feature = "opl")]
    opl: Option<OplControl>,
    #[cfg(feature = "sb")]
    sb: Option<SbControl>,
    #[cfg(feature = "sb")]
    pcm: Option<PcmPlayback>,
    commands: Producer<TimedCommand>,
    capture: Option<Capture>,
    latency: Tick,
}

impl DosAudio {
    /// Allocate the bounded command queue and construct both thread halves.
    pub fn split(config: SoundConfig) -> Result<(Self, AudioEngine)> {
        config.validate()?;
        let (producer, consumer) = rtrb::RingBuffer::new(config.queue_capacity);
        let mut ports = Vec::new();
        if config.speaker {
            ports.extend([PortRange::new(0x42, 0x43), PortRange::single(0x61)]);
        }
        if let Some(chip) = config.opl {
            ports.push(PortRange::new(
                0x388,
                if chip == crate::OplChip::Opl2 {
                    0x389
                } else {
                    0x38b
                },
            ));
        }
        if let Some(sb) = config.sb {
            ports.extend([
                PortRange::single(sb.base + 6),
                PortRange::new(sb.base + 8, sb.base + 0x0a),
                PortRange::single(sb.base + 0x0c),
                PortRange::single(sb.base + 0x0e),
            ]);
        }
        let control = Self {
            now: Tick::ZERO,
            ports,
            speaker: config.speaker.then(SpeakerControl::default),
            #[cfg(feature = "opl")]
            opl: config.opl.map(OplControl::new),
            #[cfg(feature = "sb")]
            sb: config.sb.map(SbControl::new),
            #[cfg(feature = "sb")]
            pcm: None,
            commands: producer,
            capture: None,
            latency: config.latency,
        };
        Ok((control, AudioEngine::new(&config, consumer)))
    }

    /// Current machine time, initially zero.
    pub fn now(&self) -> Tick {
        self.now
    }

    /// Advance machine time without consuming wall time.
    pub fn advance(&mut self, duration: Tick) {
        self.advance_to(self.now + duration);
    }

    /// Advance to an absolute timestamp, servicing an attached PCM buffer.
    /// Panics on backwards time or command queue overflow.
    pub fn advance_to(&mut self, at: Tick) {
        self.try_advance_to(at).expect("sound time advance failed");
    }

    /// Advance and enqueue every due buffer DMA sample, or change nothing.
    ///
    /// A full queue requires rendering queued audio, or advancing in smaller
    /// intervals. Infallible `advance`/`advance_to` and bus calls panic instead.
    pub fn try_advance_to(&mut self, at: Tick) -> Result<()> {
        self.check_time(at)?;
        #[cfg(feature = "sb")]
        if let Some(plan) = self.plan_pcm(at) {
            self.reserve(plan.commands)?;
            self.commit_pcm(at, plan);
        }
        self.set_time(at);
        Ok(())
    }

    fn check_time(&self, at: Tick) -> Result<()> {
        if at < self.now {
            return Err(Error::BackwardsTime);
        }
        at.checked_add(self.latency).ok_or(Error::TooLarge)?;
        Ok(())
    }

    fn reserve(&mut self, count: usize) -> Result<()> {
        if self.commands.slots() < count {
            return Err(Error::QueueFull);
        }
        Ok(())
    }

    fn set_time(&mut self, at: Tick) {
        #[cfg(feature = "opl")]
        if let Some(opl) = &mut self.opl {
            opl.sync(at);
        }
        self.now = at;
    }

    /// Busy-wait for an integer number of emulated microseconds.
    pub fn wait_us(&mut self, micros: u64) {
        self.advance(Tick::from_micros(micros));
    }

    /// Write at current machine time. Panics on command queue overflow.
    pub fn outb(&mut self, port: u16, value: u8) {
        self.try_outb_at(self.now, port, value)
            .expect("sound port write failed");
    }

    /// Read at current machine time. Unattached ports read FFh.
    pub fn inb(&mut self, port: u16) -> u8 {
        self.try_inb_at(self.now, port)
            .expect("sound port read failed")
    }

    /// Timestamped port write, with transactional queue overflow handling.
    pub fn try_outb_at(&mut self, at: Tick, port: u16, value: u8) -> Result<()> {
        self.check_time(at)?;
        #[cfg(feature = "sb")]
        let plan = self.plan_pcm(at);
        let mut command = None;
        let mut next_speaker = self.speaker;
        if matches!(port, 0x42 | 0x43 | 0x61)
            && let Some(speaker) = &mut next_speaker
            && let Some(state) = speaker.write(at, port, value)
        {
            command = Some(Command::Speaker(state));
        }
        #[cfg(feature = "opl")]
        let mut next_opl = self.opl;
        #[cfg(feature = "opl")]
        if let Some(opl_port) = self.opl_port(port)
            && let Some(opl) = &mut next_opl
            && let Some((register, value)) = opl.write(at, opl_port, value)
        {
            command = Some(Command::Opl { register, value });
        }
        #[cfg(feature = "sb")]
        let mut next_sb = plan.map_or(self.sb, |plan| Some(plan.sb));
        #[cfg(feature = "sb")]
        if let Some(sb) = &mut next_sb
            && (sb.resources.base..=sb.resources.base + 0x0f).contains(&port)
            && let Some(state) = sb.write(at, port, value)
        {
            command = Some(Command::Dac(state));
        }
        let needed = usize::from(command.is_some());
        #[cfg(feature = "sb")]
        let needed = needed + plan.map_or(0, |plan| plan.commands);
        self.reserve(needed)?;
        #[cfg(feature = "sb")]
        if let Some(plan) = plan {
            self.commit_pcm(at, plan);
        }
        if let Some(command) = command {
            self.push(at, command).expect("reserved port command");
        }
        self.speaker = next_speaker;
        #[cfg(feature = "opl")]
        {
            self.opl = next_opl;
        }
        #[cfg(feature = "sb")]
        {
            self.sb = next_sb;
            if let Some(sb) = &self.sb
                && matches!(port.checked_sub(sb.resources.base), Some(6 | 0x0c))
            {
                self.pcm = None; // Explicit DSP programming takes ownership.
            }
        }
        self.set_time(at);
        if let Some(capture) = &mut self.capture {
            capture.writes.push(PortWrite { at, port, value });
        }
        Ok(())
    }

    /// Play one 1..=65,536-byte unsigned mono PCM buffer through the installed DSP.
    ///
    /// Starting allocates and replaces the previous DSP transfer. Requested rates
    /// are 4,000..=23,000 Hz, rounded to the nearest integer-microsecond period;
    /// the returned handle reports the actual rate. Retain it while playing.
    /// Advance the control clock to supply bytes automatically. Buffer data and
    /// handles remain on the control side; the audio thread receives only DAC values.
    ///
    /// Raw writes to the DSP reset/command ports cancel buffer ownership. FM and
    /// speaker I/O continue alongside PCM. Failed starts leave existing playback intact.
    #[cfg(feature = "sb")]
    pub fn play_pcm(
        &mut self,
        samples: impl Into<std::sync::Arc<[u8]>>,
        sample_rate: u32,
    ) -> Result<crate::SfxHandle> {
        if !(4_000..=23_000).contains(&sample_rate) {
            return Err(Error::Config("PCM sample rate must be 4,000..=23,000 Hz"));
        }
        let mut sb = self
            .sb
            .ok_or(Error::Config("Sound Blaster is not installed"))?;
        let samples = samples.into();
        if samples.is_empty() || samples.len() > 65_536 {
            return Err(Error::Config("PCM buffer must contain 1..=65,536 bytes"));
        }
        let period = ((1_000_000 + sample_rate / 2) / sample_rate) as u16;
        let (state, end) = sb.begin_pcm(self.now, samples.len() as u32, (256 - period) as u8)?;
        end.checked_add(self.latency).ok_or(Error::TooLarge)?;
        self.reserve(1)?;
        let handle = crate::SfxHandle::new(period);
        self.push(self.now, Command::Dac(state))
            .expect("reserved PCM start");
        self.pcm = Some(PcmPlayback::new(samples, &handle, end));
        self.sb = Some(sb);
        Ok(handle)
    }

    #[cfg(feature = "sb")]
    fn plan_pcm(&self, at: Tick) -> Option<PcmPlan> {
        self.pcm
            .as_ref()
            .map(|pcm| pcm.plan(self.sb.expect("buffer owns DSP"), self.now, at))
    }

    #[cfg(feature = "sb")]
    fn commit_pcm(&mut self, at: Tick, plan: PcmPlan) {
        let commands = &mut self.commands;
        let finished = self.pcm.as_mut().expect("prepared buffer").commit(
            plan,
            self.sb.expect("buffer owns DSP"),
            self.now,
            at,
            |at, state| {
                commands
                    .push(TimedCommand {
                        at,
                        command: Command::Dac(state),
                    })
                    .expect("reserved PCM command");
            },
        );
        self.sb = Some(plan.sb);
        if finished {
            self.pcm = None;
        }
    }

    #[cfg(feature = "sb")]
    pub(crate) fn next_pcm_tick(&self) -> Option<Tick> {
        self.pcm
            .as_ref()
            .map(|pcm| pcm.next(self.sb.as_ref().expect("buffer owns DSP"), self.now))
    }

    #[cfg(feature = "sb")]
    pub(crate) fn pcm_end(&self) -> Option<Tick> {
        self.pcm.as_ref().map(|pcm| pcm.end(self.now))
    }

    fn push(&mut self, at: Tick, command: Command) -> Result<()> {
        self.commands
            .push(TimedCommand { at, command })
            .map_err(|_| Error::QueueFull)
    }

    #[cfg(feature = "opl")]
    fn opl_port(&self, port: u16) -> Option<u16> {
        #[cfg(feature = "sb")]
        if let Some(sb) = &self.sb
            && (sb.resources.base + 8..=sb.resources.base + 9).contains(&port)
        {
            return Some(0x388 + port - (sb.resources.base + 8));
        }
        (0x388..=0x38b).contains(&port).then_some(port)
    }

    /// Earliest external DSP byte request, or `None` when stopped, paused, or buffer-owned.
    ///
    /// Service each byte at this tick to preserve the DSP's fractional clock
    /// phase. The timestamp can precede `now()` if the host has fallen behind.
    /// In that case service at `now()`; subsequent requests resume from there.
    #[cfg(feature = "sb")]
    pub fn next_dma_tick(&self) -> Option<Tick> {
        if self.pcm.is_some() {
            return None;
        }
        self.sb.as_ref().and_then(SbControl::next_dma_tick)
    }

    /// Supply one unsigned PCM byte from the host's DMA controller.
    ///
    /// The host owns masking, memory, page wrapping and terminal count. Call at
    /// `next_dma_tick()` after checking DRQ. Early, paused, unmatched and backwards
    /// transfers fail. Queue overflow leaves time, DSP state and IRQ unchanged,
    /// so the host should advance its DMA address only after this succeeds.
    /// Samples receive the same fixed output latency as port writes.
    #[cfg(feature = "sb")]
    pub fn try_dma_write_at(
        &mut self,
        at: Tick,
        channel: crate::DmaChannel,
        value: u8,
    ) -> Result<()> {
        self.check_time(at)?;
        if self.pcm.is_some() {
            return Err(Error::Config("PCM buffer owns Sound Blaster DMA"));
        }
        let mut sb = self
            .sb
            .ok_or(Error::Config("Sound Blaster is not installed"))?;
        let state = sb.dma_write(at, channel, value)?;
        self.push(at, Command::Dac(state))?;
        self.sb = Some(sb);
        self.set_time(at);
        Ok(())
    }

    /// Timestamped port read. Rejects backwards time without changing state.
    pub fn try_inb_at(&mut self, at: Tick, port: u16) -> Result<u8> {
        self.try_advance_to(at)?;
        if matches!(port, 0x42 | 0x61)
            && let Some(speaker) = &mut self.speaker
        {
            return Ok(speaker.read(at, port));
        }
        #[cfg(feature = "opl")]
        if let Some(opl_port) = self.opl_port(port)
            && let Some(opl) = &mut self.opl
        {
            return Ok(opl.read(at, opl_port));
        }
        #[cfg(feature = "sb")]
        if let Some(sb) = &mut self.sb
            && (sb.resources.base..=sb.resources.base + 0x0f).contains(&port)
        {
            return Ok(sb.read(port));
        }
        Ok(0xff)
    }

    /// Play a square-wave PC speaker tone through the emulated ports.
    ///
    /// Frequency must be finite and positive. The divisor is clamped to the PIT
    /// range, including zero as the encoding for 65,536.
    pub fn sound(&mut self, hz: f64) -> Result<()> {
        if !hz.is_finite() || hz <= 0.0 {
            return Err(Error::Config("invalid speaker frequency"));
        }
        if self.speaker.is_none() {
            return Err(Error::Config("PC speaker is not installed"));
        }
        // Reserve the complete sequence so overflow cannot leave half a divisor.
        let needed = 3;
        // A concurrent last-handle drop can add one stop command during this sequence.
        #[cfg(feature = "sb")]
        let needed = needed + usize::from(self.pcm.is_some());
        self.reserve(needed)?;
        let divisor = (Tick::HZ_NUMERATOR as f64 / (Tick::HZ_DENOMINATOR as f64 * 12.0 * hz))
            .round()
            .clamp(1.0, 65_536.0) as u32 as u16;
        self.try_outb_at(self.now, 0x43, 0xb6)?;
        self.try_outb_at(self.now, 0x42, divisor as u8)?;
        self.try_outb_at(self.now, 0x42, (divisor >> 8) as u8)?;
        let bits = self.inb(0x61);
        self.try_outb_at(self.now, 0x61, bits | 3)
    }

    /// Disconnect the PC speaker through port 61h.
    pub fn nosound(&mut self) -> Result<()> {
        let bits = self.inb(0x61);
        self.try_outb_at(self.now, 0x61, bits & !3)
    }

    /// Consume this control half as the object attached to a host's bus.
    pub fn bus_device(self) -> impl IoDevice {
        self
    }

    /// Begin recording game-thread port writes. Returns an error if already active.
    pub fn start_capture(&mut self) -> Result<()> {
        if self.capture.is_some() {
            return Err(Error::Config("a capture is already active"));
        }
        #[cfg(feature = "opl")]
        let (chip, address) = self
            .opl
            .as_ref()
            .map_or((None, [0; 2]), |opl| (Some(opl.chip), opl.addresses()));
        #[cfg(not(feature = "opl"))]
        let (chip, address) = (None, [0; 2]);
        self.capture = Some(Capture {
            start: self.now,
            end: self.now,
            chip,
            address,
            opl_alias: self.opl_alias(),
            writes: Vec::new(),
        });
        Ok(())
    }

    fn opl_alias(&self) -> Option<u16> {
        #[cfg(feature = "sb")]
        if let Some(sb) = &self.sb {
            return Some(sb.resources.base + 8);
        }
        None
    }

    /// End the active capture, including idle time through the current timestamp.
    pub fn finish_capture(&mut self) -> Option<Capture> {
        self.capture.take().map(|mut capture| {
            capture.end = self.now;
            capture
        })
    }
}

impl IoDevice for DosAudio {
    fn ports(&self) -> &[PortRange] {
        &self.ports
    }
    fn advance_to(&mut self, at: Tick) {
        DosAudio::advance_to(self, at);
    }
    fn outb(&mut self, at: Tick, port: u16, value: u8) {
        self.try_outb_at(at, port, value)
            .expect("sound port write failed");
    }
    fn inb(&mut self, at: Tick, port: u16) -> u8 {
        self.try_inb_at(at, port).expect("sound port read failed")
    }

    fn irq_request(&self, _at: Tick) -> Option<crate::IrqRequest> {
        #[cfg(feature = "sb")]
        if let Some(sb) = &self.sb {
            return Some(sb.irq_request());
        }
        None
    }

    fn dma_request(&self, _at: Tick) -> Option<crate::DmaRequest> {
        #[cfg(feature = "sb")]
        if self.pcm.is_some() {
            return None;
        }
        #[cfg(feature = "sb")]
        if let Some(sb) = &self.sb {
            return sb.dma_request(_at);
        }
        None
    }
}
