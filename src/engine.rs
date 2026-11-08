use crate::bus::{Command, TimedCommand};
#[cfg(feature = "opl")]
use crate::device::opl::OplVoice;
#[cfg(feature = "sb")]
use crate::device::sb::DacState;
use crate::device::speaker::SpeakerVoice;
use crate::music::Player;
use crate::{
    Error, Frame, MusicEvent, MusicHandle, PlayOptions, Result, Sequence, SoundConfig, Tick,
};
use rtrb::Consumer;

/// Observable synthesis counters, independent of callback buffer size.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct EngineStats {
    /// Total output frames produced.
    pub frames: u64,
    /// Port commands delivered after their intended output timestamp.
    pub late_commands: u64,
}

/// Audio-thread synthesis, also used by offline rendering.
///
/// Construction and `play` allocate. `render` neither allocates nor locks. Output
/// time starts at zero; the host must pace its machine clock against playback.
/// Late commands take effect at the current output position and are counted.
/// The configured fixed latency absorbs jitter but does not synchronize clocks.
pub struct AudioEngine {
    rate: u32,
    latency: Tick,
    commands: Consumer<TimedCommand>,
    speaker: Option<SpeakerVoice>,
    #[cfg(feature = "opl")]
    opl: Option<OplVoice>,
    #[cfg(feature = "sb")]
    dac: Option<DacState>,
    player: Option<Player>,
    stats: EngineStats,
}

impl AudioEngine {
    pub(crate) fn new(config: &SoundConfig, commands: Consumer<TimedCommand>) -> Self {
        Self {
            rate: config.sample_rate,
            latency: config.latency,
            commands,
            speaker: config
                .speaker
                .then(|| SpeakerVoice::new(config.sample_rate)),
            #[cfg(feature = "opl")]
            opl: config
                .opl
                .map(|chip| OplVoice::new(chip, config.backend.clone())),
            #[cfg(feature = "sb")]
            dac: config.sb.map(|_| DacState::default()),
            player: None,
            stats: EngineStats::default(),
        }
    }

    /// Output rate in frames per second.
    pub fn sample_rate(&self) -> u32 {
        self.rate
    }
    /// Current output time, rounded down to a crystal tick.
    pub fn now(&self) -> Tick {
        Tick(
            (u128::from(self.stats.frames) * u128::from(Tick::HZ_NUMERATOR) / self.denominator())
                as u64,
        )
    }
    /// Current synthesis counters.
    pub fn stats(&self) -> EngineStats {
        self.stats
    }

    /// Install music before handing the engine to an audio stream.
    ///
    /// This method allocates the control handle and may drop an earlier sequence.
    /// The sequence starts at the engine's current output time. OPL3 can play an
    /// OPL2 sequence; dual OPL2 and OPL3 arrangements require matching hardware.
    /// Retain the returned handle while playing: its last drop stops the music.
    pub fn play(&mut self, sequence: Sequence, options: PlayOptions) -> Result<MusicHandle> {
        self.play_at(sequence, options, self.now())
    }

    /// Install music at a future absolute output timestamp. Allocates like `play`.
    pub fn play_at(
        &mut self,
        sequence: Sequence,
        options: PlayOptions,
        at: Tick,
    ) -> Result<MusicHandle> {
        if at < self.now() {
            return Err(Error::BackwardsTime);
        }
        at.checked_add(sequence.duration).ok_or(Error::TooLarge)?;
        #[cfg(feature = "opl")]
        let chip = self.opl.as_ref().map(|voice| voice.chip);
        #[cfg(not(feature = "opl"))]
        let chip: Option<crate::OplChip> = None;
        if chip.is_none()
            || !(chip == Some(sequence.chip)
                || chip == Some(crate::OplChip::Opl3) && sequence.chip == crate::OplChip::Opl2)
        {
            return Err(Error::Config(
                "sequence does not match the installed OPL hardware",
            ));
        }
        if options.looped && sequence.duration == Tick::ZERO {
            return Err(Error::Config("a zero-duration sequence cannot loop"));
        }
        if !options.volume.is_finite() {
            return Err(Error::Config("music volume must be finite"));
        }
        self.release_notes();
        let handle = MusicHandle::new(options.volume);
        let now = self.now();
        self.player = Some(Player {
            sequence,
            state: std::sync::Arc::clone(&handle.state),
            options,
            origin: at,
            last: now,
            index: 0,
            ended: false,
            was_paused: false,
        });
        Ok(handle)
    }

    pub(crate) fn music_end(&self) -> Result<Option<Tick>> {
        let Some(player) = self.player.as_ref() else {
            return Ok(None);
        };
        if player.options.looped {
            return Err(Error::Config(
                "offline render_to_end requires non-looping music",
            ));
        }
        player
            .origin
            .checked_add(player.sequence.duration)
            .map(Some)
            .ok_or(Error::TooLarge)
    }

    fn denominator(&self) -> u128 {
        u128::from(self.rate) * u128::from(Tick::HZ_DENOMINATOR)
    }
    fn command_time(&self, command: &TimedCommand) -> Tick {
        command.at.checked_add(self.latency).unwrap_or(Tick::MAX)
    }

    fn apply(&mut self, command: Command) {
        match command {
            #[cfg(feature = "sb")]
            Command::Dac(state) => self.dac = Some(state),
            Command::Speaker(mut state) => {
                state.epoch = state.epoch.checked_add(self.latency).unwrap_or(Tick::MAX);
                if let Some(speaker) = &mut self.speaker {
                    speaker.state = state;
                }
            }
            #[cfg(feature = "opl")]
            Command::Opl { register, value } => {
                if let Some(opl) = &mut self.opl {
                    opl.write(register, value);
                }
            }
        }
    }

    fn music_event(&mut self, event: MusicEvent) {
        match event {
            MusicEvent::Opl { register, value } => {
                #[cfg(feature = "opl")]
                if let Some(opl) = &mut self.opl {
                    opl.write(register, value);
                }
                #[cfg(not(feature = "opl"))]
                let _ = (register, value);
            }
        }
    }

    fn release_notes(&mut self) {
        #[cfg(feature = "opl")]
        if let Some(opl) = &mut self.opl {
            opl.release_notes();
        }
    }

    /// Overwrite a buffer with the next stereo frames, without allocating.
    ///
    /// Each sample interval is split at port commands, music events and native
    /// OPL boundaries. Speaker pulses, held DSP DAC values and native FM frames are integrated
    /// over those intervals, including commands inside a single output sample.
    /// Commands at the sample's end belong to the following sample.
    pub fn render(&mut self, out: &mut [Frame]) {
        let denominator = self.denominator();
        for frame in out {
            let start = u128::from(self.stats.frames) * u128::from(Tick::HZ_NUMERATOR);
            let end = start + u128::from(Tick::HZ_NUMERATOR);
            if let Some(player) = &mut self.player
                && player.prepare(Tick((start / denominator) as u64))
            {
                self.release_notes();
            }
            let gain = self.player.as_ref().map_or(1.0, Player::gain);
            let mut cursor = start;
            let mut speaker_area = 0.0;
            #[cfg(feature = "sb")]
            let mut dac_area = 0.0;
            #[cfg(feature = "opl")]
            let mut opl_area = [0.0f64; 2];
            #[cfg(not(feature = "opl"))]
            let opl_area = [0.0f64; 2];
            while cursor < end {
                while let Ok(command) = self.commands.peek() {
                    let stamp = u128::from(self.command_time(command).0) * denominator;
                    if stamp > cursor {
                        break;
                    }
                    if stamp < cursor {
                        self.stats.late_commands += 1;
                    }
                    let command = self.commands.pop().expect("peeked command");
                    self.apply(command.command);
                }
                while self
                    .player
                    .as_ref()
                    .and_then(Player::next)
                    .is_some_and(|at| u128::from(at.0) * denominator <= cursor)
                {
                    let player = self.player.as_mut().expect("scheduled player");
                    let event = player.pop();
                    let ended = player.ended;
                    if let Some(event) = event {
                        self.music_event(event);
                    }
                    if ended {
                        self.release_notes();
                    }
                }
                #[cfg(feature = "opl")]
                if let Some(opl) = &mut self.opl
                    && u128::from(opl.next.0) * denominator <= cursor
                {
                    opl.step();
                }
                let mut next = end;
                if let Ok(command) = self.commands.peek() {
                    next = next.min(u128::from(self.command_time(command).0) * denominator);
                }
                if let Some(at) = self.player.as_ref().and_then(Player::next) {
                    next = next.min(u128::from(at.0) * denominator);
                }
                #[cfg(feature = "opl")]
                if let Some(opl) = &self.opl {
                    next = next.min(u128::from(opl.next.0) * denominator);
                }
                // A producer may publish an already-late command between peeks.
                if next <= cursor {
                    continue;
                }
                let from = cursor as f64 / denominator as f64;
                let to = next as f64 / denominator as f64;
                if let Some(speaker) = &self.speaker {
                    speaker_area += speaker.state.integral(from, to);
                }
                #[cfg(feature = "sb")]
                if let Some(dac) = self.dac {
                    dac_area += dac.output() * (to - from);
                }
                #[cfg(feature = "opl")]
                if let Some(opl) = &self.opl {
                    for (area, value) in opl_area.iter_mut().zip(opl.held) {
                        *area += f64::from(value) * (to - from);
                    }
                }
                cursor = next;
            }
            let width = Tick::HZ_NUMERATOR as f64 / denominator as f64;
            let speaker = self
                .speaker
                .as_mut()
                .map_or(0.0, |speaker| speaker.filter(speaker_area / width));
            #[cfg(feature = "sb")]
            let speaker = speaker + (dac_area / width) as f32;
            *frame = [
                (speaker + (opl_area[0] / width * gain) as f32).clamp(-1.0, 1.0),
                (speaker + (opl_area[1] / width * gain) as f32).clamp(-1.0, 1.0),
            ];
            self.stats.frames += 1;
        }
    }
}

#[cfg(feature = "toolkit")]
impl game_toolkit_audio::StereoStream for AudioEngine {
    fn fill(&mut self, out: &mut [[f32; 2]]) {
        self.render(out);
    }
}
