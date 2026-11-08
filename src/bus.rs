#[cfg(feature = "mpu401")]
use crate::device::mpu401::MpuControl;
#[cfg(feature = "opl")]
use crate::device::opl::OplControl;
#[cfg(feature = "sb")]
use crate::device::sb::{DacState, SbControl};
use crate::device::speaker::{SpeakerControl, SpeakerState};
#[cfg(feature = "sb")]
use crate::dma::playback::{MemoryPlan, MemoryPlayback};
#[cfg(feature = "sb")]
use crate::sfx::{PcmPlan, PcmPlayback};
use crate::{
    AudioEngine, Capture, Error, IoDevice, PortRange, PortWrite, Result, SoundConfig, Tick,
};
use rtrb::Producer;

#[derive(Clone, Copy, Debug)]
pub(crate) enum Command {
    Speaker(SpeakerState),
    #[cfg(feature = "mpu401")]
    MidiByte(u8),
    #[cfg(feature = "psg")]
    Psg(u8),
    #[cfg(feature = "sb")]
    Dac(DacState),
    #[cfg(feature = "opl")]
    Opl {
        register: u16,
        value: u8,
    },
    #[cfg(feature = "opl")]
    OplPair {
        first: (u16, u8),
        second: (u16, u8),
    },
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct TimedCommand {
    pub at: Tick,
    pub command: Command,
}

#[cfg(feature = "sb")]
#[derive(Clone, Copy)]
enum AutomaticPlan {
    Buffer(PcmPlan),
    Memory(MemoryPlan),
}
#[cfg(feature = "sb")]
impl AutomaticPlan {
    fn sb(self) -> SbControl {
        match self {
            Self::Buffer(p) => p.sb,
            Self::Memory(p) => p.sb,
        }
    }
    fn commands(self) -> usize {
        match self {
            Self::Buffer(p) => p.commands,
            Self::Memory(p) => p.commands,
        }
    }
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
    #[cfg(feature = "mpu401")]
    mpu: Option<MpuControl>,
    #[cfg(feature = "psg")]
    psg: bool,
    #[cfg(feature = "opl")]
    opl: Option<OplControl>,
    #[cfg(feature = "sb")]
    sb: Option<SbControl>,
    #[cfg(feature = "sb")]
    pcm: Option<PcmPlayback>,
    #[cfg(feature = "sb")]
    memory_dma: Option<MemoryPlayback>,
    #[cfg(feature = "sb")]
    queue_capacity: usize,
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
        if config.midi.is_some() {
            ports.push(PortRange::new(config.midi_port, config.midi_port + 1));
        }
        if config.psg {
            ports.push(PortRange::single(0xc0));
        }
        if config.speaker {
            ports.extend([PortRange::new(0x40, 0x43), PortRange::single(0x61)]);
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
            if config.sb_model.has_mixer() {
                ports.push(PortRange::new(sb.base, sb.base + 5));
            }
            ports.extend([
                PortRange::single(sb.base + 6),
                PortRange::new(sb.base + 8, sb.base + 0x0a),
                PortRange::single(sb.base + 0x0c),
                PortRange::single(sb.base + 0x0e),
            ]);
            if config.sb_model == crate::SbModel::Sb16 {
                ports.push(PortRange::single(sb.base + 0x0f));
            }
        }
        let control = Self {
            now: Tick::ZERO,
            ports,
            speaker: config.speaker.then(SpeakerControl::default),
            #[cfg(feature = "mpu401")]
            mpu: config
                .midi
                .as_ref()
                .map(|_| MpuControl::new(config.midi_port)),
            #[cfg(feature = "psg")]
            psg: config.psg,
            #[cfg(feature = "opl")]
            opl: config.opl.map(OplControl::new),
            #[cfg(feature = "sb")]
            sb: config
                .sb
                .map(|resources| SbControl::new(resources, config.sb_model, config.dma16)),
            #[cfg(feature = "sb")]
            pcm: None,
            #[cfg(feature = "sb")]
            memory_dma: None,
            #[cfg(feature = "sb")]
            queue_capacity: config.queue_capacity,
            commands: producer,
            capture: None,
            latency: config.latency,
        };
        Ok((control, AudioEngine::new(&config, consumer)?))
    }

    /// Current machine time, initially zero.
    pub fn now(&self) -> Tick {
        self.now
    }

    /// Frequency in hertz of the PIT tone that speaker port I/O currently plays.
    ///
    /// `None` when the speaker is absent or port 61h has the PIT gate (bit 0) or
    /// speaker data (bit 1) clear, which includes bit-banged output. Melodies from
    /// [`AudioEngine::play_speaker`] run on the engine and are not reflected.
    pub fn speaker_tone(&self) -> Option<f64> {
        let state = self.speaker.as_ref()?.state;
        (state.gate && state.data && state.mode != 0)
            .then(|| crate::device::speaker::pit_hz(state.divisor))
    }

    /// The machine's 8253 timer, present with the PC speaker.
    ///
    /// With the speaker installed this object owns ports 40h-43h: route them only
    /// here and read IRQ 0 timing from this timer instead of keeping a second one.
    /// `None` without a speaker; the host then owns its own
    /// [`Pit`](game_toolkit_dosbus::Pit).
    pub fn pit(&self) -> Option<&game_toolkit_dosbus::Pit> {
        self.speaker.as_ref().map(|speaker| &speaker.pit)
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
        if let Some(plan) = self.plan_pcm(at)? {
            self.reserve(plan.commands())?;
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
        let plan = self.plan_pcm(at)?;
        let mut command = None;
        #[cfg(feature = "mpu401")]
        let mut next_mpu = self.mpu;
        #[cfg(feature = "mpu401")]
        if let Some(mpu) = &mut next_mpu
            && (mpu.base..=mpu.base + 1).contains(&port)
            && let Some(byte) = mpu.write(port, value)
        {
            command = Some(Command::MidiByte(byte));
        }
        #[cfg(feature = "psg")]
        if self.psg && port == 0xc0 {
            command = Some(Command::Psg(value));
        }
        // Pit is plain data without heap storage; cloning keeps the write transactional.
        let mut next_speaker = self.speaker.clone();
        if matches!(port, 0x40..=0x43 | 0x61)
            && let Some(speaker) = &mut next_speaker
            && let Some(state) = speaker.write(at, port, value)
        {
            command = Some(Command::Speaker(state));
        }
        #[cfg(feature = "opl")]
        let mut next_opl = self.opl;
        #[cfg(feature = "opl")]
        if let Some(opl) = &mut next_opl {
            let writes = self
                .opl_ports(port)
                .map(|port| port.and_then(|port| opl.write(at, port, value)));
            match writes {
                [Some(first), Some(second)] => command = Some(Command::OplPair { first, second }),
                [Some((register, value)), None] | [None, Some((register, value))] => {
                    command = Some(Command::Opl { register, value })
                }
                _ => {}
            }
        }
        #[cfg(feature = "sb")]
        let mut next_sb = plan.map_or(self.sb, |plan| Some(plan.sb()));
        #[cfg(feature = "sb")]
        let mut cancel_pcm = false;
        #[cfg(feature = "sb")]
        if let Some(sb) = &mut next_sb
            && (sb.resources.base..=sb.resources.base + 0x0f).contains(&port)
            && let Some(state) = sb.write(at, port, value)
        {
            command = Some(Command::Dac(state));
        }
        #[cfg(feature = "sb")]
        if self.pcm.is_some()
            && let Some(sb) = &mut next_sb
            && sb.dma_channel().is_none()
        {
            command = Some(Command::Dac(sb.stop_pcm()));
            cancel_pcm = true;
        }
        let needed = usize::from(command.is_some());
        #[cfg(feature = "sb")]
        let needed = needed + plan.map_or(0, AutomaticPlan::commands);
        self.reserve(needed)?;
        #[cfg(feature = "sb")]
        if let Some(plan) = plan {
            self.commit_pcm(at, plan);
        }
        #[cfg(feature = "sb")]
        if matches!(plan, Some(AutomaticPlan::Memory(_))) {
            // Read shared memory only during the committed walk. Recreate the
            // port result from its actual final DAC state, not the preview's bytes.
            next_sb = self.sb;
            if let Some(sb) = &mut next_sb
                && (sb.resources.base..=sb.resources.base + 0x0f).contains(&port)
                && let Some(state) = sb.write(at, port, value)
            {
                command = Some(Command::Dac(state));
            }
        }
        if let Some(command) = command {
            self.push(at, command).expect("reserved port command");
        }
        self.speaker = next_speaker;
        #[cfg(feature = "mpu401")]
        {
            self.mpu = next_mpu;
        }
        #[cfg(feature = "opl")]
        {
            self.opl = next_opl;
        }
        #[cfg(feature = "sb")]
        {
            self.sb = next_sb;
            if let Some(memory) = &mut self.memory_dma {
                memory.controller.outb(port, value);
            }
            if let Some(sb) = &self.sb
                && (cancel_pcm || matches!(port.checked_sub(sb.resources.base), Some(6 | 0x0c)))
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
        let period = ((1_000_000 + sample_rate / 2) / sample_rate).clamp(44, 250) as u16;
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
    fn plan_pcm(&self, at: Tick) -> Result<Option<AutomaticPlan>> {
        if let Some(pcm) = &self.pcm {
            return Ok(Some(AutomaticPlan::Buffer(pcm.plan(
                self.sb.expect("buffer owns DSP"),
                self.now,
                at,
            ))));
        }
        if let (Some(memory), Some(sb)) = (&self.memory_dma, self.sb) {
            return memory
                .plan(sb, self.now, at)
                .map(|p| Some(AutomaticPlan::Memory(p)));
        }
        Ok(None)
    }

    #[cfg(feature = "sb")]
    fn commit_pcm(&mut self, at: Tick, plan: AutomaticPlan) {
        let commands = &mut self.commands;
        let mut emit = |at, state| {
            commands
                .push(TimedCommand {
                    at,
                    command: Command::Dac(state),
                })
                .expect("reserved DMA command");
        };
        if let AutomaticPlan::Memory(_) = plan {
            self.memory_dma
                .as_mut()
                .expect("prepared memory DMA")
                .commit(self.sb.as_mut().expect("installed DSP"), self.now, at, emit)
                .expect("preflighted memory DMA");
            return;
        }
        let AutomaticPlan::Buffer(plan) = plan else {
            unreachable!()
        };
        let finished = self.pcm.as_mut().expect("prepared buffer").commit(
            plan,
            self.sb.expect("buffer owns DSP"),
            self.now,
            at,
            &mut emit,
        );
        self.sb = Some(plan.sb);
        if finished {
            self.pcm = None;
        }
    }

    #[cfg(feature = "sb")]
    pub(crate) fn next_pcm_tick(&self) -> Option<Tick> {
        if let Some(pcm) = &self.pcm {
            return Some(pcm.next(self.sb.as_ref().expect("buffer owns DSP"), self.now));
        }
        self.memory_dma
            .as_ref()
            .and_then(|memory| self.sb.as_ref().and_then(|sb| memory.next(sb, self.now)))
    }

    /// Attach shared conventional memory and an internal 8237 controller.
    /// Call before attaching this object to a host bus: this expands its port map.
    /// Memory bytes are atomic; coordinate writes with advances for coherent samples.
    #[cfg(feature = "sb")]
    pub fn attach_dma(&mut self, memory: std::sync::Arc<crate::ConventionalMemory>) -> Result<()> {
        #[cfg(feature = "psg")]
        if self.psg {
            return Err(Error::Config(
                "Tandy PSG overlaps the internal AT DMA controller",
            ));
        }
        if self.memory_dma.is_some() {
            return Err(Error::Config("DMA memory is already attached"));
        }
        if self.sb.is_none() {
            return Err(Error::Config("Sound Blaster is not installed"));
        }
        let dma = MemoryPlayback::new(memory, self.queue_capacity);
        self.ports.extend_from_slice(dma.controller.ports());
        self.memory_dma = Some(dma);
        Ok(())
    }

    /// Pop one memory-DMA block completion. This does not acknowledge the ISA IRQ.
    /// Drain before retrying IrqQueueFull; refill memory before advancing past a block.
    #[cfg(feature = "sb")]
    pub fn poll_dma_completion(&mut self) -> Option<crate::DmaCompletion> {
        self.memory_dma.as_mut().and_then(MemoryPlayback::poll)
    }

    /// Next predicted DSP block boundary; masks/controller terminal count may stall it.
    #[cfg(feature = "sb")]
    pub fn next_dma_block_tick(&self) -> Option<Tick> {
        self.sb.as_ref().and_then(SbControl::next_block_tick)
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
    fn opl_ports(&self, port: u16) -> [Option<u16>; 2] {
        #[cfg(feature = "sb")]
        let card = self.sb.as_ref().map(|sb| (sb.resources.base, sb.model));
        #[cfg(not(feature = "sb"))]
        let card = None;
        map_opl_ports(port, card)
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
        if self.pcm.is_some() || self.memory_dma.is_some() {
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

    /// Supply one SB16 word-wide DMA unit; bit interpretation follows the DSP mode.
    /// Queue overflow leaves the DSP, IRQ, and machine timestamp unchanged.
    #[cfg(feature = "sb")]
    pub fn try_dma_write16_at(
        &mut self,
        at: Tick,
        channel: crate::DmaChannel,
        value: u16,
    ) -> Result<()> {
        self.check_time(at)?;
        if self.pcm.is_some() || self.memory_dma.is_some() {
            return Err(Error::Config("PCM buffer owns Sound Blaster DMA"));
        }
        let mut sb = self
            .sb
            .ok_or(Error::Config("Sound Blaster is not installed"))?;
        let state = sb.dma_write16(at, channel, value)?;
        self.push(at, Command::Dac(state))?;
        self.sb = Some(sb);
        self.set_time(at);
        Ok(())
    }

    /// Timestamped port read. Rejects backwards time without changing state.
    pub fn try_inb_at(&mut self, at: Tick, port: u16) -> Result<u8> {
        self.try_advance_to(at)?;
        #[cfg(feature = "mpu401")]
        if let Some(mpu) = &mut self.mpu
            && (mpu.base..=mpu.base + 1).contains(&port)
        {
            return Ok(mpu.read(port));
        }
        #[cfg(feature = "sb")]
        if let Some(memory) = &mut self.memory_dma
            && memory
                .controller
                .ports()
                .iter()
                .any(|range| range.contains(port))
        {
            return Ok(memory.controller.inb(port));
        }
        if matches!(port, 0x40..=0x42 | 0x61)
            && let Some(speaker) = &mut self.speaker
        {
            return Ok(speaker.read(at, port));
        }
        #[cfg(feature = "opl")]
        if let Some(opl_port) = self.opl_ports(port)[0]
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
    /// Borrow tone/noise helpers for the installed Tandy SN76489.
    #[cfg(feature = "psg")]
    pub fn psg(&mut self) -> Result<crate::Psg<'_>> {
        if !self.psg {
            return Err(Error::Config("PSG is not installed"));
        }
        Ok(crate::Psg::new(self))
    }
    #[cfg(feature = "psg")]
    pub(crate) fn psg_write(&mut self, bytes: &[u8]) -> Result<()> {
        if !self.psg {
            return Err(Error::Config("PSG is not installed"));
        }
        let needed = bytes.len();
        #[cfg(feature = "sb")]
        let needed = needed
            + self.plan_pcm(self.now)?.map_or(0, AutomaticPlan::commands)
            + usize::from(self.pcm.is_some());
        self.reserve(needed)?;
        for &byte in bytes {
            self.try_outb_at(self.now, 0xc0, byte)?;
        }
        Ok(())
    }

    /// Borrow musical patch, note, and rhythm helpers for the installed OPL chips.
    #[cfg(feature = "opl")]
    pub fn opl(&mut self) -> Result<crate::Opl<'_>> {
        let chip = self
            .opl
            .as_ref()
            .ok_or(Error::Config("OPL hardware is not installed"))?
            .chip;
        Ok(crate::Opl::new(self, chip))
    }

    #[cfg(feature = "opl")]
    pub(crate) fn opl_register(&self, register: u16) -> Result<u8> {
        let opl = self
            .opl
            .as_ref()
            .ok_or(Error::Config("OPL hardware is not installed"))?;
        if register > 0x1ff || opl.chip == crate::OplChip::Opl2 && register > 0xff {
            return Err(Error::Config("OPL register is not installed"));
        }
        Ok(opl.register(register))
    }

    #[cfg(feature = "opl")]
    pub(crate) fn opl_patch_levels(&self, channel: u8) -> Result<[u8; 2]> {
        let opl = self
            .opl
            .as_ref()
            .ok_or(Error::Config("OPL hardware is not installed"))?;
        if channel
            >= if opl.chip == crate::OplChip::Opl2 {
                9
            } else {
                18
            }
        {
            return Err(Error::Config("invalid OPL channel"));
        }
        Ok(opl.patch_levels(usize::from(channel)))
    }

    #[cfg(feature = "opl")]
    pub(crate) fn opl_set_patch_levels(&mut self, channel: u8, levels: [u8; 2]) -> Result<()> {
        self.opl_patch_levels(channel)?;
        self.opl
            .as_mut()
            .expect("validated OPL")
            .set_patch_levels(usize::from(channel), levels);
        Ok(())
    }

    #[cfg(feature = "opl")]
    pub(crate) fn opl_write(&mut self, writes: &[(u16, u8)]) -> Result<()> {
        for &(register, _) in writes {
            self.opl_register(register)?;
        }
        let needed = writes.len();
        #[cfg(feature = "sb")]
        let needed = needed
            + self.plan_pcm(self.now)?.map_or(0, AutomaticPlan::commands)
            + usize::from(self.pcm.is_some());
        self.reserve(needed)?;
        for &(register, value) in writes {
            let port = 0x388 + (register >> 8) * 2;
            #[cfg(feature = "sb")]
            let port = if let Some(sb) = &self.sb
                && sb.model == crate::SbModel::SbPro
            {
                sb.resources.base + (register >> 8) * 2
            } else {
                port
            };
            self.try_outb_at(self.now, port, register as u8)?;
            self.try_outb_at(self.now, port + 1, value)?;
        }
        Ok(())
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
            card: self.opl_card(),
            writes: Vec::new(),
        });
        Ok(())
    }

    fn opl_card(&self) -> Option<(u16, crate::SbModel)> {
        #[cfg(feature = "sb")]
        if let Some(sb) = &self.sb {
            return Some((sb.resources.base, sb.model));
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

pub(crate) fn map_opl_ports(port: u16, card: Option<(u16, crate::SbModel)>) -> [Option<u16>; 2] {
    if let Some((base, model)) = card {
        let common = (base + 8..=base + 9).contains(&port);
        if model == crate::SbModel::SbPro && (common || (0x388..=0x389).contains(&port)) {
            let data = port & 1;
            return [Some(0x388 + data), Some(0x38a + data)];
        }
        if model.has_mixer() && (base..=base + 3).contains(&port) {
            return [Some(0x388 + port - base), None];
        }
        if common {
            return [Some(0x388 + port - base - 8), None];
        }
    }
    [(0x388..=0x38b).contains(&port).then_some(port), None]
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
        if self.pcm.is_some() || self.memory_dma.is_some() {
            return None;
        }
        #[cfg(feature = "sb")]
        if let Some(sb) = &self.sb {
            return sb.dma_request(_at);
        }
        None
    }
}
