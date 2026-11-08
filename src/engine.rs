use crate::bus::{Command, TimedCommand};
#[cfg(feature = "opl")]
use crate::device::opl::OplVoice;
#[cfg(feature = "psg")]
use crate::device::psg::PsgVoice;
#[cfg(feature = "sb")]
use crate::device::sb::{DacState, SbControl};
use crate::device::speaker::SpeakerVoice;
#[cfg(feature = "mpu401")]
use crate::midi::voice::MidiVoice;
use crate::music::{MusicCapabilities, MusicRequest, MusicRuntime, Player, validate_play};
use crate::{
    Error, Frame, MusicEvent, MusicHandle, MusicTarget, PlayOptions, PreparedMusic, Result,
    SoundConfig, Tick,
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
    #[cfg(feature = "mpu401")]
    midi: Option<MidiVoice>,
    #[cfg(feature = "psg")]
    psg: Option<PsgVoice>,
    #[cfg(feature = "opl")]
    opl: Option<OplVoice>,
    #[cfg(feature = "sb")]
    dac: Option<DacState>,
    #[cfg(feature = "sb")]
    dac_filter: [f64; 2],
    player: Option<Player>,
    runtime: Option<MusicRuntime>,
    speaker_player: Option<Player>,
    speaker_runtime: Option<MusicRuntime>,
    capture: Option<crate::capture::stream::CaptureWriter>,
    event_time: Tick,
    stats: EngineStats,
}

impl AudioEngine {
    pub(crate) fn new(config: &SoundConfig, commands: Consumer<TimedCommand>) -> Result<Self> {
        #[cfg(feature = "mpu401")]
        let midi = config
            .midi
            .as_ref()
            .map(|backend| MidiVoice::new(backend, config.sample_rate))
            .transpose()?;
        Ok(Self {
            rate: config.sample_rate,
            latency: config.latency,
            commands,
            #[cfg(feature = "mpu401")]
            midi,
            #[cfg(feature = "psg")]
            psg: config.psg.then(PsgVoice::new),
            speaker: config
                .speaker
                .then(|| SpeakerVoice::new(config.sample_rate)),
            #[cfg(feature = "opl")]
            opl: config
                .opl
                .map(|chip| OplVoice::new(chip, config.backend.clone())),
            #[cfg(feature = "sb")]
            dac: config
                .sb
                .map(|resources| SbControl::new(resources, config.sb_model, config.dma16).output()),
            #[cfg(feature = "sb")]
            dac_filter: [0.0; 2],
            player: None,
            runtime: None,
            speaker_player: None,
            speaker_runtime: None,
            capture: None,
            event_time: Tick::ZERO,
            stats: EngineStats::default(),
        })
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

    /// Capture applied OPL writes from manual I/O, sequencers and live events.
    /// Create outside rendering; stop is acknowledged at the next frame start.
    /// The bounded writer stays owned by the engine after capture ends.
    pub fn start_capture(&mut self, capacity: usize) -> Result<crate::MusicCapture> {
        if self
            .capture
            .as_ref()
            .is_some_and(|writer| !writer.is_finished())
        {
            return Err(Error::Config("synthesis capture is already active"));
        }
        #[cfg(feature = "opl")]
        let chip = self.opl.as_ref().map(|voice| voice.chip);
        #[cfg(not(feature = "opl"))]
        let chip: Option<crate::OplChip> = None;
        let chip = chip.ok_or(Error::Config("synthesis capture requires OPL hardware"))?;
        let (capture, writer) = crate::capture::stream::stream(chip, self.now(), capacity)?;
        self.capture = Some(writer);
        Ok(capture)
    }

    /// Construct a unique runtime music sender before moving this engine to a stream.
    /// Allocates two bounded queues. Retired resources are reclaimed by the sender.
    pub fn music_controller(&mut self, capacity: usize) -> Result<crate::MusicController> {
        if self.runtime.is_some() {
            return Err(Error::Config("a music controller already exists"));
        }
        let capabilities = self.capabilities();
        if capabilities.opl.is_none()
            && !capabilities.midi
            && !capabilities.psg
            && !capabilities.speaker
        {
            return Err(Error::Config("runtime music requires installed hardware"));
        }
        let (controller, runtime) =
            crate::MusicController::new(capabilities, self.rate, self.now(), capacity)?;
        self.runtime = Some(runtime);
        Ok(controller)
    }

    /// Independently sequence a speaker melody or PWM stream alongside FM/MIDI music.
    /// Construction/replacement allocates only on the caller's thread.
    pub fn play_speaker(
        &mut self,
        music: impl Into<PreparedMusic>,
        options: PlayOptions,
    ) -> Result<crate::SpeakerHandle> {
        let music = music.into();
        if music.target != MusicTarget::Speaker {
            return Err(Error::Config("speaker playback requires speaker events"));
        }
        validate_play(self.capabilities(), &music, options, self.now())?;
        if let Some(old) = self.speaker_player.take() {
            old.state
                .finished
                .store(true, std::sync::atomic::Ordering::Release);
        }
        let handle = MusicHandle::new(options.volume, self.rate);
        self.speaker_player = Some(Player::new(music, options, &handle, self.now(), self.now()));
        Ok(crate::SpeakerHandle::new(handle))
    }

    /// Construct a bounded runtime sender dedicated to speaker melodies/PWM.
    pub fn speaker_controller(&mut self, capacity: usize) -> Result<crate::MusicController> {
        if self.speaker.is_none() || self.speaker_runtime.is_some() {
            return Err(Error::Config(
                "speaker is absent or its controller already exists",
            ));
        }
        let capabilities = MusicCapabilities {
            opl: None,
            midi: false,
            psg: false,
            speaker: true,
        };
        let (controller, runtime) =
            crate::MusicController::new(capabilities, self.rate, self.now(), capacity)?;
        self.speaker_runtime = Some(runtime);
        Ok(controller)
    }

    pub(crate) fn speaker_end(&self) -> Result<Option<Tick>> {
        let Some(player) = &self.speaker_player else {
            return Ok(None);
        };
        if player.options.looped {
            return Err(Error::Config(
                "render_to_end requires finite speaker playback",
            ));
        }
        player
            .origin
            .checked_add(player.sequence.duration)
            .map(Some)
            .ok_or(Error::TooLarge)
    }

    fn speaker_next(&self, at: Tick) -> Option<Tick> {
        let sequence = self.speaker_player.as_ref().and_then(Player::next);
        let runtime = self
            .speaker_runtime
            .as_ref()
            .and_then(|runtime| runtime.next(at, self.speaker_player.is_some()));
        sequence.into_iter().chain(runtime).min()
    }

    fn speaker_due(&mut self, at: Tick) -> bool {
        if self.speaker_player.is_none() && self.speaker_runtime.is_none() {
            return false;
        }
        std::mem::swap(&mut self.player, &mut self.speaker_player);
        std::mem::swap(&mut self.runtime, &mut self.speaker_runtime);
        let changed = self.runtime_due(at);
        self.sequence_due(at);
        std::mem::swap(&mut self.player, &mut self.speaker_player);
        std::mem::swap(&mut self.runtime, &mut self.speaker_runtime);
        changed
    }

    /// Install music before handing the engine to an audio stream.
    ///
    /// This method allocates the control handle and may drop an earlier sequence.
    /// The sequence starts at the engine's current output time. OPL3 can play an
    /// OPL2 sequence; dual OPL2 and OPL3 arrangements require matching hardware.
    /// Retain the returned handle while playing: its last drop stops the music.
    pub fn play(
        &mut self,
        sequence: impl Into<PreparedMusic>,
        options: PlayOptions,
    ) -> Result<MusicHandle> {
        self.play_at(sequence, options, self.now())
    }

    /// Install music at a future absolute output timestamp. Allocates like `play`.
    pub fn play_at(
        &mut self,
        sequence: impl Into<PreparedMusic>,
        options: PlayOptions,
        at: Tick,
    ) -> Result<MusicHandle> {
        if at < self.now() {
            return Err(Error::BackwardsTime);
        }
        let sequence = sequence.into();
        validate_play(self.capabilities(), &sequence, options, at)?;
        self.event_time = self.now();
        self.release_notes();
        let handle = MusicHandle::new(options.volume, self.rate);
        let now = self.now();
        self.player = Some(Player::new(sequence, options, &handle, at, now));
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

    fn capabilities(&self) -> MusicCapabilities {
        #[cfg(feature = "opl")]
        let opl = self.opl.as_ref().map(|voice| voice.chip);
        #[cfg(not(feature = "opl"))]
        let opl = None;
        MusicCapabilities {
            opl,
            midi: {
                #[cfg(feature = "mpu401")]
                {
                    self.midi.is_some()
                }
                #[cfg(not(feature = "mpu401"))]
                {
                    false
                }
            },
            psg: {
                #[cfg(feature = "psg")]
                {
                    self.psg.is_some()
                }
                #[cfg(not(feature = "psg"))]
                {
                    false
                }
            },
            speaker: self.speaker.is_some(),
        }
    }

    fn runtime_next(&self, now: Tick) -> Option<Tick> {
        self.runtime
            .as_ref()
            .and_then(|runtime| runtime.next(now, self.player.is_some()))
    }

    fn runtime_due(&mut self, at: Tick) -> bool {
        let mut changed = false;
        while self.runtime_next(at).is_some_and(|next| next <= at) {
            let request = self.runtime.as_mut().expect("scheduled runtime").pop();
            if request.at.is_some_and(|stamp| stamp < at) {
                self.stats.late_commands += 1;
            }
            match request.request {
                MusicRequest::Event(event) => self.music_event(event),
                MusicRequest::Replace(mut player) => {
                    self.release_notes();
                    if let Some(old) = self.player.take() {
                        old.state
                            .finished
                            .store(true, std::sync::atomic::Ordering::Release);
                        self.runtime
                            .as_mut()
                            .expect("scheduled runtime")
                            .retire(old);
                    }
                    player.origin = at;
                    player.last = at;
                    if at.checked_add(player.sequence.duration).is_none() {
                        player.ended = true;
                        player
                            .state
                            .finished
                            .store(true, std::sync::atomic::Ordering::Release);
                    }
                    player.prepare(at);
                    self.player = Some(player);
                    self.sequence_due(at);
                    changed = true;
                }
            }
        }
        changed
    }

    fn sequence_due(&mut self, at: Tick) {
        while self
            .player
            .as_ref()
            .and_then(Player::next)
            .is_some_and(|next| next <= at)
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
    }
    fn command_time(&self, command: &TimedCommand) -> Tick {
        command.at.checked_add(self.latency).unwrap_or(Tick::MAX)
    }

    fn apply(&mut self, command: Command) {
        match command {
            #[cfg(feature = "mpu401")]
            Command::MidiByte(byte) => {
                if let Some(midi) = &mut self.midi {
                    midi.byte(byte);
                }
            }
            #[cfg(feature = "psg")]
            Command::Psg(byte) => {
                if let Some(psg) = &mut self.psg {
                    psg.write(byte);
                }
            }

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
                self.write_opl(register, value);
            }
            #[cfg(feature = "opl")]
            Command::OplPair { first, second } => {
                self.write_opl(first.0, first.1);
                self.write_opl(second.0, second.1);
            }
        }
    }

    fn music_event(&mut self, event: MusicEvent) {
        match event {
            MusicEvent::SpeakerLevel(high) => {
                if let Some(speaker) = &mut self.speaker {
                    speaker.state.gate = false;
                    speaker.state.data = high;
                    speaker.state.epoch = self.event_time;
                }
            }

            MusicEvent::Speaker { divisor, enabled } => {
                if let Some(speaker) = &mut self.speaker {
                    speaker.state.divisor = if divisor == 0 {
                        65_536
                    } else {
                        u32::from(divisor)
                    };
                    speaker.state.epoch = self.event_time;
                    speaker.state.mode = 3;
                    speaker.state.gate = enabled;
                    speaker.state.data = enabled;
                }
            }
            MusicEvent::Midi(message) => {
                #[cfg(feature = "mpu401")]
                if let Some(midi) = &mut self.midi {
                    midi.message(message.bytes());
                }
                #[cfg(not(feature = "mpu401"))]
                let _ = message;
            }
            MusicEvent::SysEx { offset, length } => {
                #[cfg(feature = "mpu401")]
                if let (Some(midi), Some(player)) = (&mut self.midi, &self.player) {
                    midi.message(
                        &player.sequence.sysex[offset as usize..offset as usize + length as usize],
                    );
                }
                #[cfg(not(feature = "mpu401"))]
                let _ = (offset, length);
            }
            MusicEvent::Psg(byte) => {
                #[cfg(feature = "psg")]
                if let Some(psg) = &mut self.psg {
                    psg.write(byte);
                }
                #[cfg(not(feature = "psg"))]
                let _ = byte;
            }

            MusicEvent::Opl { register, value } => {
                #[cfg(feature = "opl")]
                self.write_opl(register, value);
                #[cfg(not(feature = "opl"))]
                let _ = (register, value);
            }
        }
    }

    #[cfg(feature = "opl")]
    fn write_opl(&mut self, register: u16, value: u8) {
        if let Some(opl) = &mut self.opl {
            if let Some(capture) = &mut self.capture {
                capture.record(self.event_time, register, value);
            }
            opl.write(register, value);
        }
    }

    fn release_notes(&mut self) {
        if self
            .player
            .as_ref()
            .is_some_and(|player| player.sequence.target == MusicTarget::Midi)
        {
            #[cfg(feature = "mpu401")]
            if let Some(midi) = &mut self.midi {
                midi.release();
            }
            return;
        }
        if self.player.as_ref().is_some_and(|player| {
            matches!(
                player.sequence.target,
                MusicTarget::Psg | MusicTarget::OplPsg(_)
            )
        }) {
            #[cfg(feature = "psg")]
            if let Some(psg) = &mut self.psg {
                psg.mute();
            }
            if self
                .player
                .as_ref()
                .is_some_and(|player| player.sequence.target == MusicTarget::Psg)
            {
                return;
            }
        }

        if self
            .player
            .as_ref()
            .is_some_and(|player| player.sequence.target == MusicTarget::Speaker)
        {
            if let Some(speaker) = &mut self.speaker {
                speaker.state.data = false;
            }
        } else {
            #[cfg(feature = "opl")]
            if let Some(opl) = &mut self.opl {
                let capture = &mut self.capture;
                let at = self.event_time;
                opl.release_notes(|register, value| {
                    if let Some(capture) = capture {
                        capture.record(at, register, value);
                    }
                });
            }
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
            self.event_time = Tick((start / denominator) as u64);
            if let Some(capture) = &mut self.capture {
                capture.boundary(self.event_time);
            }
            if let Some(player) = &mut self.player
                && player.prepare(Tick((start / denominator) as u64))
            {
                self.release_notes();
            }
            if let Some(player) = &mut self.speaker_player
                && player.prepare(self.event_time)
                && let Some(speaker) = &mut self.speaker
            {
                speaker.state.data = false;
            }
            let mut speaker_gain = self.speaker_player.as_mut().map_or(1.0, Player::gain);
            let mut gain = self.player.as_mut().map_or(1.0, Player::gain);
            let mut cursor = start;
            let mut speaker_area = 0.0;
            #[cfg(any(feature = "mpu401", feature = "psg"))]
            let mut extra_area = [0.0f64; 2];
            #[cfg(not(any(feature = "mpu401", feature = "psg")))]
            let extra_area = [0.0f64; 2];
            #[cfg(feature = "sb")]
            let mut dac_area = [0.0; 2];
            #[cfg(feature = "opl")]
            let mut opl_area = [0.0f64; 2];
            #[cfg(not(feature = "opl"))]
            let opl_area = [0.0f64; 2];
            while cursor < end {
                self.event_time = Tick((cursor / denominator) as u64);
                if self.runtime_due(Tick((cursor / denominator) as u64)) {
                    gain = self.player.as_mut().map_or(1.0, Player::gain);
                }
                if self.speaker_due(self.event_time) {
                    speaker_gain = self.speaker_player.as_mut().map_or(1.0, Player::gain);
                }
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
                #[cfg(feature = "mpu401")]
                if let Some(midi) = &mut self.midi
                    && u128::from(midi.next.0) * denominator <= cursor
                {
                    midi.step();
                }
                #[cfg(feature = "psg")]
                if let Some(psg) = &mut self.psg
                    && u128::from(psg.next.0) * denominator <= cursor
                {
                    psg.step();
                }
                let mut next = end;
                if let Some(at) = self.speaker_next(self.event_time) {
                    next = next.min(u128::from(at.0) * denominator);
                }
                if let Some(at) = self.runtime_next(Tick((cursor / denominator) as u64)) {
                    next = next.min(u128::from(at.0) * denominator);
                }
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
                #[cfg(feature = "mpu401")]
                if let Some(midi) = &self.midi {
                    next = next.min(u128::from(midi.next.0) * denominator);
                }
                #[cfg(feature = "psg")]
                if let Some(psg) = &self.psg {
                    next = next.min(u128::from(psg.next.0) * denominator);
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
                    for (area, sample) in dac_area.iter_mut().zip(dac.output()) {
                        *area += sample * (to - from);
                    }
                }
                #[cfg(feature = "opl")]
                if let Some(opl) = &self.opl {
                    for (channel, (area, value)) in opl_area.iter_mut().zip(opl.held).enumerate() {
                        #[cfg(feature = "sb")]
                        let value = f64::from(value)
                            * f64::from(self.dac.map_or(1.0, |dac| dac.fm_gain[channel]));
                        #[cfg(not(feature = "sb"))]
                        let value = {
                            let _ = channel;
                            f64::from(value)
                        };
                        *area += value
                            * (to - from)
                            * if self.runtime.is_some()
                                && self.player.as_ref().is_none_or(|p| {
                                    matches!(
                                        p.sequence.target,
                                        MusicTarget::Opl(_) | MusicTarget::OplPsg(_)
                                    )
                                })
                            {
                                gain
                            } else {
                                1.0
                            };
                    }
                }
                #[cfg(feature = "mpu401")]
                if let Some(midi) = &self.midi {
                    let gain = if self
                        .player
                        .as_ref()
                        .is_some_and(|p| p.sequence.target == MusicTarget::Midi)
                    {
                        gain
                    } else {
                        1.0
                    };
                    for (area, sample) in extra_area.iter_mut().zip(midi.held) {
                        *area += f64::from(sample) * (to - from) * gain;
                    }
                }
                #[cfg(feature = "psg")]
                if let Some(psg) = &self.psg {
                    let gain = if self.player.as_ref().is_some_and(|p| {
                        matches!(p.sequence.target, MusicTarget::Psg | MusicTarget::OplPsg(_))
                    }) {
                        gain
                    } else {
                        1.0
                    };
                    for (area, sample) in extra_area.iter_mut().zip(psg.held) {
                        *area += f64::from(sample) * (to - from) * gain;
                    }
                }
                cursor = next;
            }
            let width = Tick::HZ_NUMERATOR as f64 / denominator as f64;
            let speaker = self
                .speaker
                .as_mut()
                .map_or(0.0, |speaker| speaker.filter(speaker_area / width));
            let speaker_gain = if self.speaker_player.is_some() {
                speaker_gain
            } else if self
                .player
                .as_ref()
                .is_some_and(|p| p.sequence.target == MusicTarget::Speaker)
            {
                gain
            } else {
                1.0
            };
            let speaker = speaker * speaker_gain as f32;
            #[cfg(feature = "sb")]
            let pcm = core::array::from_fn::<_, 2, _>(|channel| {
                let mean = dac_area[channel] / width;
                let hz = self.dac.map_or(0, |dac| dac.filter_hz);
                if hz != 0 {
                    let alpha =
                        1.0 - (-std::f64::consts::TAU * f64::from(hz) / f64::from(self.rate)).exp();
                    self.dac_filter[channel] += alpha * (mean - self.dac_filter[channel]);
                    self.dac_filter[channel] as f32
                } else {
                    self.dac_filter[channel] = mean;
                    mean as f32
                }
            });
            #[cfg(not(feature = "sb"))]
            let pcm = [0.0; 2];
            let gain = if self.runtime.is_some()
                || self.player.as_ref().is_some_and(|p| {
                    !matches!(
                        p.sequence.target,
                        MusicTarget::Opl(_) | MusicTarget::OplPsg(_)
                    )
                }) {
                1.0
            } else {
                gain
            };
            *frame = [
                (speaker
                    + pcm[0]
                    + (extra_area[0] / width) as f32
                    + (opl_area[0] / width * gain) as f32)
                    .clamp(-1.0, 1.0),
                (speaker
                    + pcm[1]
                    + (extra_area[1] / width) as f32
                    + (opl_area[1] / width * gain) as f32)
                    .clamp(-1.0, 1.0),
            ];
            self.stats.frames += 1;
            if let Some(runtime) = &self.runtime {
                runtime.publish_time(self.now());
            }
            if let Some(runtime) = &self.speaker_runtime {
                runtime.publish_time(self.now());
            }
        }
    }
}

#[cfg(feature = "toolkit")]
impl game_toolkit_audio::StereoStream for AudioEngine {
    fn fill(&mut self, out: &mut [[f32; 2]]) {
        self.render(out);
    }
}
