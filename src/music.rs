//! Immutable, timestamped music and playback controls.
use crate::{Error, OplChip, Result, Tick};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering},
};
use std::time::Duration;

const NO_GAIN_REQUEST: u64 = u64::MAX;
mod runtime;
pub use runtime::MusicController;
pub(crate) use runtime::{MusicCapabilities, MusicRequest, MusicRuntime, validate_play};
mod prepared;
pub use prepared::{MusicTarget, PreparedMusic};
pub(crate) mod soundtrack;
pub use soundtrack::{MusicDevice, Soundtrack};
#[cfg(feature = "synthie")]
pub mod synthie;

/// A register write emitted by a prepared music sequence.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum MusicEvent {
    /// Write an OPL register, including its bank bit.
    Opl {
        /// Register address, 000h through 1FFh.
        register: u16,
        /// Register value.
        value: u8,
    },
    /// A complete short MIDI message.
    Midi(crate::MidiMessage),
    /// Complete SysEx bytes in the prepared music's owned payload storage.
    SysEx {
        /// First payload byte.
        offset: u32,
        /// Number of payload bytes, including F0/F7.
        length: u32,
    },
    /// One SN76489 latch/data byte.
    Psg(u8),
    /// PIT channel-2 square wave. Zero divisor encodes 65,536.
    Speaker {
        /// PIT divisor.
        divisor: u16,
        /// Whether the speaker is connected.
        enabled: bool,
    },
    /// Direct binary speaker level with the PIT gate disconnected (PWM).
    SpeakerLevel(bool),
}

/// One event's absolute position relative to the beginning of a sequence.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TimedEvent {
    /// Position in crystal cycles.
    pub at: Tick,
    /// Register operation at that position.
    pub event: MusicEvent,
}

/// Prepared music, allocated and validated before entering the audio callback.
#[derive(Clone, Debug)]
pub struct Sequence {
    pub(crate) events: Arc<[TimedEvent]>,
    pub(crate) duration: Tick,
    pub(crate) chip: OplChip,
}

impl Sequence {
    /// Construct a sorted sequence. Equal timestamps retain their supplied order.
    pub fn new(chip: OplChip, events: Vec<TimedEvent>, duration: Tick) -> Result<Self> {
        let mut last = Tick::ZERO;
        for event in &events {
            let MusicEvent::Opl { register, .. } = event.event else {
                return Err(Error::Format(
                    "OPL sequence contains another device's event",
                ));
            };
            if event.at < last || event.at > duration || register > 0x1ff {
                return Err(Error::Format(
                    "invalid sequence ordering, duration or OPL register",
                ));
            }
            if chip == OplChip::Opl2 && register > 0xff {
                return Err(Error::Format(
                    "an OPL2 sequence cannot address the second register bank",
                ));
            }
            last = event.at;
        }
        Ok(Self {
            events: events.into(),
            duration,
            chip,
        })
    }
    /// Events in playback order.
    pub fn events(&self) -> &[TimedEvent] {
        &self.events
    }
    /// Time through the final delay, excluding any release tail.
    pub fn duration(&self) -> Tick {
        self.duration
    }
    /// FM hardware the sequence was written for.
    pub fn chip(&self) -> OplChip {
        self.chip
    }
}

/// Music playback options, applied when installing a sequence on an engine.
#[derive(Clone, Copy, Debug)]
pub struct PlayOptions {
    pub(crate) looped: bool,
    pub(crate) volume: f32,
}
impl Default for PlayOptions {
    fn default() -> Self {
        Self {
            looped: false,
            volume: 1.0,
        }
    }
}
impl PlayOptions {
    /// Loop the complete sequence. Zero-duration loops are rejected.
    pub fn looped() -> Self {
        Self {
            looped: true,
            ..Self::default()
        }
    }
    /// Set linear volume, clamped to 0 through 1.
    #[must_use]
    pub fn volume(mut self, volume: f32) -> Self {
        self.volume = volume.clamp(0.0, 1.0);
        self
    }
}

pub(crate) struct MusicState {
    pub device: AtomicU32,
    pub switch_request: AtomicU32,
    pub device_routes: [u8; 9],
    pub paused: AtomicBool,
    pub stopped: AtomicBool,
    pub finished: AtomicBool,
    gain_request: AtomicU64,
    sample_rate: u32,
    pub position: AtomicU64,
    /// PIT divisor of the sounding speaker note, or 0 when none sounds.
    pub tone: AtomicU32,
    handles: AtomicUsize,
}

/// Cloneable music controls; dropping the last handle requests a stop.
///
/// Control changes are observed at output-frame boundaries. They do not allocate
/// or acquire locks. Volume affects the OPL output shared by music and port I/O.
pub struct MusicHandle {
    pub(crate) state: Arc<MusicState>,
}
impl MusicHandle {
    pub(crate) fn new(volume: f32, sample_rate: u32) -> Self {
        Self {
            state: Arc::new(MusicState {
                device: AtomicU32::new(u32::MAX),
                switch_request: AtomicU32::new(u32::MAX),
                device_routes: [u8::MAX; 9],
                paused: AtomicBool::new(false),
                stopped: AtomicBool::new(false),
                finished: AtomicBool::new(false),
                gain_request: AtomicU64::new(u64::from(volume.to_bits())),
                sample_rate,
                position: AtomicU64::new(0),
                tone: AtomicU32::new(0),
                handles: AtomicUsize::new(1),
            }),
        }
    }
    /// Request an available soundtrack device (or its explicit fallback).
    /// Applies at an output-frame boundary, retaining position, pause, fade and
    /// loop state. Ordinary single-arrangement handles cannot switch devices.
    pub fn switch_device(&self, device: MusicDevice) -> Result<()> {
        if self.is_finished() {
            return Err(Error::Config("finished music cannot switch devices"));
        }
        let selected = self.state.device_routes[device as usize];
        if selected == u8::MAX {
            return Err(Error::Config(
                "soundtrack device or fallback is unavailable",
            ));
        }
        self.state
            .switch_request
            .store(u32::from(selected), Ordering::Release);
        Ok(())
    }
    /// Last applied soundtrack device, or none for ordinary prepared playback.
    pub fn device(&self) -> Option<MusicDevice> {
        MusicDevice::from_code(self.state.device.load(Ordering::Acquire))
    }

    /// Pause sequencing and mute the FM output.
    pub fn pause(&self) {
        self.state.paused.store(true, Ordering::Release);
    }
    /// Resume sequencing at its current position.
    pub fn resume(&self) {
        self.state.paused.store(false, Ordering::Release);
    }
    /// Stop playback permanently and release the FM notes.
    pub fn stop(&self) {
        self.state.stopped.store(true, Ordering::Release);
    }
    /// Set linear FM output gain, clamped to 0 through 1.
    pub fn set_volume(&self, volume: f32) {
        let volume = if volume.is_finite() {
            volume.clamp(0.0, 1.0)
        } else {
            0.0
        };
        self.request_gain(volume, 0);
    }
    /// Fade the shared FM output to a finite target, clamped to 0 through 1.
    ///
    /// Requests are observed at output-frame boundaries; the latest volume or
    /// fade request wins. Duration rounds up to whole output frames and must fit
    /// in `u32` frames. A zero duration sets the gain immediately.
    ///
    /// For an N-frame fade, the first frame uses the current gain and the target
    /// is reached after N rendered frames. Pausing freezes the ramp. A new fade
    /// starts from the current ramp gain, including when it interrupts a fade.
    pub fn fade_volume(&self, target: f32, duration: Duration) -> Result<()> {
        if !target.is_finite() {
            return Err(Error::Config("music fade target must be finite"));
        }
        let scaled = duration
            .as_nanos()
            .checked_mul(u128::from(self.state.sample_rate))
            .ok_or(Error::TooLarge)?;
        let frames = u32::try_from(scaled.div_ceil(1_000_000_000)).map_err(|_| Error::TooLarge)?;
        self.request_gain(target.clamp(0.0, 1.0), frames);
        Ok(())
    }
    fn request_gain(&self, target: f32, frames: u32) {
        let request = (u64::from(frames) << 32) | u64::from(target.to_bits());
        self.state.gain_request.store(request, Ordering::Release);
    }
    /// Whether the engine has reached the end or a stop was requested.
    pub fn is_finished(&self) -> bool {
        self.state.finished.load(Ordering::Acquire) || self.state.stopped.load(Ordering::Acquire)
    }
    /// Whether playback is paused.
    pub fn is_paused(&self) -> bool {
        self.state.paused.load(Ordering::Acquire)
    }
    /// Position relative to the current loop's start.
    pub fn position(&self) -> Tick {
        Tick(self.state.position.load(Ordering::Acquire))
    }
}
impl Clone for MusicHandle {
    fn clone(&self) -> Self {
        self.state.handles.fetch_add(1, Ordering::Relaxed);
        Self {
            state: Arc::clone(&self.state),
        }
    }
}
impl Drop for MusicHandle {
    fn drop(&mut self) {
        if self.state.handles.fetch_sub(1, Ordering::AcqRel) == 1 {
            self.stop();
        }
    }
}

pub(crate) struct Player {
    pub sequence: PreparedMusic,
    pub soundtrack: Option<Arc<Soundtrack>>,
    pub state: Arc<MusicState>,
    pub options: PlayOptions,
    pub origin: Tick,
    pub index: usize,
    pub last: Tick,
    pub ended: bool,
    pub was_paused: bool,
    gain: GainRamp,
}

impl Player {
    pub fn new(
        sequence: PreparedMusic,
        options: PlayOptions,
        handle: &MusicHandle,
        origin: Tick,
        last: Tick,
    ) -> Self {
        Self {
            sequence,
            soundtrack: None,
            state: Arc::clone(&handle.state),
            options,
            origin,
            index: 0,
            last,
            ended: false,
            was_paused: false,
            gain: GainRamp::new(options.volume),
        }
    }

    pub fn next(&self) -> Option<Tick> {
        if self.ended || self.was_paused {
            return None;
        }
        Some(
            self.origin
                + self
                    .sequence
                    .events
                    .get(self.index)
                    .map_or(self.sequence.duration, |event| event.at),
        )
    }

    pub fn prepare(&mut self, at: Tick) -> bool {
        let paused = self.state.paused.load(Ordering::Acquire);
        if self.was_paused {
            self.origin += at - self.last;
        }
        self.was_paused = paused;
        self.last = at;
        if !self.ended && self.state.stopped.load(Ordering::Acquire) {
            self.ended = true;
            self.state.finished.store(true, Ordering::Release);
            return true;
        }
        self.state.position.store(
            at.0.saturating_sub(self.origin.0)
                .min(self.sequence.duration.0),
            Ordering::Release,
        );
        false
    }

    pub fn pop(&mut self) -> Option<MusicEvent> {
        if let Some(event) = self.sequence.events.get(self.index) {
            self.index += 1;
            Some(event.event)
        } else {
            if self.options.looped {
                self.origin += self.sequence.duration - self.sequence.loop_start;
                self.index = self.sequence.loop_index;
            } else {
                self.ended = true;
                self.state.finished.store(true, Ordering::Release);
            }
            None
        }
    }

    pub fn gain(&mut self) -> f64 {
        let request = self
            .state
            .gain_request
            .swap(NO_GAIN_REQUEST, Ordering::Acquire);
        if request != NO_GAIN_REQUEST {
            self.gain
                .request(f32::from_bits(request as u32), (request >> 32) as u32);
        }
        if self.was_paused {
            0.0
        } else {
            self.gain.next()
        }
    }
}

struct GainRamp {
    current: f64,
    start: f64,
    target: f64,
    frames: u32,
    elapsed: u32,
}

impl GainRamp {
    fn new(volume: f32) -> Self {
        let current = f64::from(volume);
        Self {
            current,
            start: current,
            target: current,
            frames: 0,
            elapsed: 0,
        }
    }

    fn request(&mut self, target: f32, frames: u32) {
        self.start = self.current;
        self.target = f64::from(target);
        self.frames = frames;
        self.elapsed = 0;
        if frames == 0 {
            self.current = self.target;
        }
    }

    fn next(&mut self) -> f64 {
        let gain = self.current;
        if self.elapsed < self.frames {
            self.elapsed += 1;
            self.current = if self.elapsed == self.frames {
                self.target
            } else {
                self.start
                    + (self.target - self.start) * f64::from(self.elapsed) / f64::from(self.frames)
            };
        }
        gain
    }
}
