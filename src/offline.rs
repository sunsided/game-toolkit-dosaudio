use crate::{
    AudioEngine, DosAudio, Error, Frame, MusicHandle, PlayOptions, Result, Sequence, SoundConfig,
    Tick,
};
use std::time::Duration;

/// Stereo 16-bit PCM WAV output from a deterministic offline render.
pub struct Wav {
    rate: u32,
    frames: Vec<Frame>,
}
impl Wav {
    /// Output sample rate.
    pub fn sample_rate(&self) -> u32 {
        self.rate
    }
    /// Captured frames before conversion to signed PCM.
    pub fn frames(&self) -> &[Frame] {
        &self.frames
    }
    /// Encode a RIFF/WAVE file with two interleaved 16-bit channels.
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        let length = self
            .frames
            .len()
            .checked_mul(4)
            .and_then(|n| u32::try_from(n).ok())
            .ok_or(Error::TooLarge)?;
        let riff_length = length.checked_add(36).ok_or(Error::TooLarge)?;
        let mut bytes = Vec::with_capacity(
            usize::try_from(length)
                .map_err(|_| Error::TooLarge)?
                .checked_add(44)
                .ok_or(Error::TooLarge)?,
        );
        bytes.extend(b"RIFF");
        bytes.extend(riff_length.to_le_bytes());
        bytes.extend(b"WAVEfmt ");
        bytes.extend(16u32.to_le_bytes());
        bytes.extend(1u16.to_le_bytes());
        bytes.extend(2u16.to_le_bytes());
        bytes.extend(self.rate.to_le_bytes());
        bytes.extend((self.rate * 4).to_le_bytes());
        bytes.extend(4u16.to_le_bytes());
        bytes.extend(16u16.to_le_bytes());
        bytes.extend(b"data");
        bytes.extend(length.to_le_bytes());
        for frame in &self.frames {
            for &value in frame {
                let pcm = (value.clamp(-1.0, 1.0) * 32767.0).round() as i16;
                bytes.extend(pcm.to_le_bytes());
            }
        }
        Ok(bytes)
    }
}

/// Both hardware halves on one thread, with no audio device or window.
///
/// Writes and time advances use the same queue and renderer as live output.
/// Output is accumulated as time advances. Music sequencers run in the engine,
/// so their tempo is independent of those advances and the chosen render chunk.
pub struct OfflineMachine {
    control: DosAudio,
    engine: AudioEngine,
    frames: Vec<Frame>,
}
impl OfflineMachine {
    /// Create an offline machine at the supplied sample rate, with zero latency.
    pub fn new(config: SoundConfig, sample_rate: u32) -> Result<Self> {
        let (control, engine) =
            DosAudio::split(config.sample_rate(sample_rate).latency(Tick::ZERO))?;
        Ok(Self {
            control,
            engine,
            frames: Vec::new(),
        })
    }
    /// Current emulated machine time.
    pub fn now(&self) -> Tick {
        self.control.now()
    }
    /// Read a port at current machine time.
    pub fn inb(&mut self, port: u16) -> u8 {
        self.control.inb(port)
    }
    /// Write a port at current machine time.
    pub fn outb(&mut self, port: u16, value: u8) -> Result<()> {
        self.control.try_outb_at(self.now(), port, value)
    }
    /// Earliest outstanding Sound Blaster DMA request.
    #[cfg(feature = "sb")]
    pub fn next_dma_tick(&self) -> Option<Tick> {
        self.control.next_dma_tick()
    }
    /// Deliver a host DMA byte at current machine time, after advancing to its request.
    #[cfg(feature = "sb")]
    pub fn dma_write(&mut self, channel: crate::DmaChannel, value: u8) -> Result<()> {
        self.control.try_dma_write_at(self.now(), channel, value)
    }
    /// Current Sound Blaster IRQ line and level, without acknowledging it.
    #[cfg(feature = "sb")]
    pub fn irq_request(&self) -> Option<crate::IrqRequest> {
        crate::IoDevice::irq_request(&self.control, self.now())
    }
    /// Start a PC speaker tone through the PIT registers.
    pub fn sound(&mut self, hz: f64) -> Result<()> {
        self.control.sound(hz)
    }
    /// Disconnect the PC speaker.
    pub fn nosound(&mut self) -> Result<()> {
        self.control.nosound()
    }
    /// Install a finite, non-looping soundtrack at current output time.
    pub fn play(&mut self, sequence: Sequence) -> Result<MusicHandle> {
        self.engine
            .play_at(sequence, PlayOptions::default(), self.now())
    }
    /// Start an unsigned mono PCM buffer, serviced automatically as time advances.
    /// Keep its handle alive through rendering; the last byte gets a full hold interval.
    #[cfg(feature = "sb")]
    pub fn play_pcm(
        &mut self,
        samples: impl Into<std::sync::Arc<[u8]>>,
        sample_rate: u32,
    ) -> Result<crate::SfxHandle> {
        self.control.play_pcm(samples, sample_rate)
    }
    /// Advance both halves and capture every complete output sample up to `at`.
    pub fn advance_to(&mut self, at: Tick) -> Result<()> {
        if at < self.now() {
            return Err(Error::BackwardsTime);
        }
        let target = u128::from(at.0)
            * u128::from(self.engine.sample_rate())
            * u128::from(Tick::HZ_DENOMINATOR)
            / u128::from(Tick::HZ_NUMERATOR);
        let target = u64::try_from(target).map_err(|_| Error::TooLarge)?;
        let extra = target.saturating_sub(self.engine.stats().frames);
        let extra = usize::try_from(extra).map_err(|_| Error::TooLarge)?;
        // A RIFF file has a 32-bit size. Reject impossible captures before allocation.
        if self
            .frames
            .len()
            .checked_add(extra)
            .ok_or(Error::TooLarge)?
            > (u32::MAX as usize - 36) / 4
        {
            return Err(Error::TooLarge);
        }
        loop {
            #[cfg(feature = "sb")]
            let next = self.control.next_pcm_tick().map_or(at, |next| next.min(at));
            #[cfg(not(feature = "sb"))]
            let next = at;
            // Render complete frames before the next DMA event, then publish it.
            // Rendering a partial frame early could miss later events in it.
            self.render_until(next);
            self.control.try_advance_to(next)?;
            if next == at {
                break;
            }
        }
        Ok(())
    }

    fn render_until(&mut self, at: Tick) {
        let target = u128::from(at.0)
            * u128::from(self.engine.sample_rate())
            * u128::from(Tick::HZ_DENOMINATOR)
            / u128::from(Tick::HZ_NUMERATOR);
        let mut buffer = [[0.0; 2]; 512];
        let mut remaining = (target as u64).saturating_sub(self.engine.stats().frames);
        while remaining > 0 {
            let count = remaining.min(buffer.len() as u64) as usize;
            self.engine.render(&mut buffer[..count]);
            self.frames.extend_from_slice(&buffer[..count]);
            remaining -= count as u64;
        }
    }
    /// Advance by a duration, retaining the existing machine timestamp.
    pub fn render_for(&mut self, duration: Duration) -> Result<()> {
        let delta = Tick::checked_from_duration(duration).ok_or(Error::TooLarge)?;
        self.advance_to(self.now().checked_add(delta).ok_or(Error::TooLarge)?)
    }
    /// Render through finite music and/or PCM and a 250 ms release tail.
    ///
    /// Keep playback handles alive until this call has completed.
    pub fn render_to_end(mut self) -> Result<Wav> {
        #[cfg(feature = "sb")]
        let pcm_end = self.control.pcm_end();
        #[cfg(not(feature = "sb"))]
        let pcm_end = None;
        let until = self
            .engine
            .music_end()?
            .into_iter()
            .chain(pcm_end)
            .max()
            .ok_or(Error::Config("no music or PCM buffer is installed"))?
            .checked_add(Tick::from_micros(250_000))
            .ok_or(Error::TooLarge)?;
        self.advance_to(until.max(self.now()))?;
        Ok(self.finish())
    }
    /// Finish a manually timed capture without adding any samples.
    pub fn finish(self) -> Wav {
        Wav {
            rate: self.engine.sample_rate(),
            frames: self.frames,
        }
    }
}
