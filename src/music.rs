//! Immutable, timestamped music and playback controls.
use crate::{Error, OplChip, Result, Tick};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering},
};

/// A register write emitted by a prepared music sequence.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MusicEvent {
    /// Write an OPL register, including its bank bit.
    Opl {
        /// Register address, 000h through 1FFh.
        register: u16,
        /// Register value.
        value: u8,
    },
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
            let MusicEvent::Opl { register, .. } = event.event;
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
    pub paused: AtomicBool,
    pub stopped: AtomicBool,
    pub finished: AtomicBool,
    pub volume: AtomicU32,
    pub position: AtomicU64,
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
    pub(crate) fn new(volume: f32) -> Self {
        Self {
            state: Arc::new(MusicState {
                paused: AtomicBool::new(false),
                stopped: AtomicBool::new(false),
                finished: AtomicBool::new(false),
                volume: AtomicU32::new(volume.to_bits()),
                position: AtomicU64::new(0),
                handles: AtomicUsize::new(1),
            }),
        }
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
        self.state.volume.store(volume.to_bits(), Ordering::Release);
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
    pub sequence: Sequence,
    pub state: Arc<MusicState>,
    pub options: PlayOptions,
    pub origin: Tick,
    pub index: usize,
    pub last: Tick,
    pub ended: bool,
    pub was_paused: bool,
}

impl Player {
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
                self.origin += self.sequence.duration;
                self.index = 0;
            } else {
                self.ended = true;
                self.state.finished.store(true, Ordering::Release);
            }
            None
        }
    }

    pub fn gain(&self) -> f64 {
        if self.was_paused {
            0.0
        } else {
            f64::from(f32::from_bits(self.state.volume.load(Ordering::Acquire)))
        }
    }
}
