use super::{MusicEvent, MusicHandle, MusicTarget, PlayOptions, Player, PreparedMusic};
use crate::{Error, OplChip, Result, Tick};
use rtrb::{Consumer, Producer};
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

pub(crate) enum MusicRequest {
    Replace(Player),
    Event(MusicEvent),
}
pub(crate) struct ScheduledRequest {
    pub at: Option<Tick>,
    pub request: MusicRequest,
}

/// Unique game-thread sender for replacing music and submitting live events.
///
/// Construct before handing the engine to a stream. The bounded queues transfer
/// prepared players to the callback and retired players back to this controller;
/// only controller operations reclaim their allocations. Requests are FIFO.
/// An immediate request behind a scheduled request waits for that request.
pub struct MusicController {
    requests: Producer<ScheduledRequest>,
    retired: Consumer<Player>,
    clock: Arc<AtomicU64>,
    capabilities: MusicCapabilities,
    rate: u32,
    last_scheduled: Tick,
}

pub(crate) struct MusicRuntime {
    requests: Consumer<ScheduledRequest>,
    retired: Producer<Player>,
    clock: Arc<AtomicU64>,
}

#[derive(Clone, Copy)]
pub(crate) struct MusicCapabilities {
    pub opl: Option<OplChip>,
    pub midi: bool,
    pub psg: bool,
    pub speaker: bool,
}
impl MusicCapabilities {
    fn has_opl(self, needed: OplChip) -> bool {
        self.opl == Some(needed) || self.opl == Some(OplChip::Opl3) && needed == OplChip::Opl2
    }
    pub fn accepts(self, target: MusicTarget) -> bool {
        match target {
            MusicTarget::Opl(c) => self.has_opl(c),
            MusicTarget::Midi => self.midi,
            MusicTarget::Psg => self.psg,
            MusicTarget::Speaker => self.speaker,
            MusicTarget::OplPsg(c) => self.has_opl(c) && self.psg,
        }
    }
}
pub(crate) fn validate_play(
    capabilities: MusicCapabilities,
    sequence: &PreparedMusic,
    options: PlayOptions,
    at: Tick,
) -> Result<()> {
    at.checked_add(sequence.duration).ok_or(Error::TooLarge)?;
    if !capabilities.accepts(sequence.target) {
        return Err(Error::Config(
            "music target does not match installed hardware",
        ));
    }
    if options.looped && sequence.duration == sequence.loop_start {
        return Err(Error::Config("a zero-duration loop cannot play"));
    }
    if !options.volume.is_finite() {
        return Err(Error::Config("music volume must be finite"));
    }
    Ok(())
}

impl MusicController {
    pub(crate) fn new(
        capabilities: MusicCapabilities,
        rate: u32,
        at: Tick,
        capacity: usize,
    ) -> Result<(Self, MusicRuntime)> {
        if capacity == 0 || capacity > 1_048_576 {
            return Err(Error::Config("music queue capacity must be 1..=1,048,576"));
        }
        let (requests, incoming) = rtrb::RingBuffer::new(capacity);
        let (outgoing, retired) = rtrb::RingBuffer::new(capacity);
        let clock = Arc::new(AtomicU64::new(at.0));
        Ok((
            Self {
                requests,
                retired,
                clock: Arc::clone(&clock),
                capabilities,
                rate,
                last_scheduled: at,
            },
            MusicRuntime {
                requests: incoming,
                retired: outgoing,
                clock,
            },
        ))
    }
    /// Last completed output-frame timestamp, rounded down to a crystal tick.
    pub fn now(&self) -> Tick {
        Tick(self.clock.load(Ordering::Acquire))
    }
    /// Reclaim completed/replaced players on the calling thread. Returns the count.
    pub fn reclaim(&mut self) -> usize {
        let mut count = 0;
        while let Ok(player) = self.retired.pop() {
            drop(player);
            count += 1;
        }
        count
    }
    /// Start prepared music at the next observed output position. Allocates here.
    pub fn play(
        &mut self,
        sequence: impl Into<PreparedMusic>,
        options: PlayOptions,
    ) -> Result<MusicHandle> {
        self.enqueue_play(sequence.into(), options, None)
    }
    /// Replace music at an absolute output timestamp; scheduled times are nondecreasing.
    /// Late delivery starts at the current output position and increments late_commands.
    pub fn play_at(
        &mut self,
        at: Tick,
        sequence: impl Into<PreparedMusic>,
        options: PlayOptions,
    ) -> Result<MusicHandle> {
        self.enqueue_play(sequence.into(), options, Some(at))
    }
    fn check_stamp(&self, at: Option<Tick>) -> Result<()> {
        if let Some(at) = at
            && (at < self.now() || at < self.last_scheduled)
        {
            return Err(Error::BackwardsTime);
        }
        Ok(())
    }
    fn enqueue_play(
        &mut self,
        sequence: PreparedMusic,
        options: PlayOptions,
        at: Option<Tick>,
    ) -> Result<MusicHandle> {
        self.reclaim();
        self.check_stamp(at)?;
        validate_play(
            self.capabilities,
            &sequence,
            options,
            at.unwrap_or(self.now()),
        )?;
        let handle = MusicHandle::new(options.volume, self.rate);
        let player = Player::new(
            sequence,
            options,
            &handle,
            at.unwrap_or(self.now()),
            self.now(),
        );
        self.requests
            .push(ScheduledRequest {
                at,
                request: MusicRequest::Replace(player),
            })
            .map_err(|_| Error::QueueFull)?;
        if let Some(at) = at {
            self.last_scheduled = at;
        }
        Ok(handle)
    }
    /// Submit a live chip event at the next output position. No allocation.
    pub fn send_event(&mut self, event: MusicEvent) -> Result<()> {
        self.enqueue_event(None, event)
    }
    /// Submit a live event at a nondecreasing absolute output timestamp.
    pub fn send_event_at(&mut self, at: Tick, event: MusicEvent) -> Result<()> {
        self.enqueue_event(Some(at), event)
    }
    fn enqueue_event(&mut self, at: Option<Tick>, event: MusicEvent) -> Result<()> {
        self.reclaim();
        self.check_stamp(at)?;
        match event {
            MusicEvent::Opl { register, .. }
                if register > 0x1ff
                    || (self.capabilities.opl == Some(OplChip::Opl2) && register > 0xff)
                    || self.capabilities.opl.is_none() =>
            {
                return Err(Error::Config(
                    "live event register does not match installed OPL",
                ));
            }
            MusicEvent::Midi(_) if !self.capabilities.midi => {
                return Err(Error::Config("MIDI synth is not installed"));
            }
            MusicEvent::Psg(_) if !self.capabilities.psg => {
                return Err(Error::Config("PSG is not installed"));
            }
            MusicEvent::Speaker { .. } | MusicEvent::SpeakerLevel(_)
                if !self.capabilities.speaker =>
            {
                return Err(Error::Config("speaker is not installed"));
            }
            MusicEvent::SysEx { .. } => {
                return Err(Error::Config(
                    "live SysEx must be supplied as owned prepared music",
                ));
            }
            _ => {}
        }
        self.requests
            .push(ScheduledRequest {
                at,
                request: MusicRequest::Event(event),
            })
            .map_err(|_| Error::QueueFull)?;
        if let Some(at) = at {
            self.last_scheduled = at;
        }
        Ok(())
    }
}

impl MusicRuntime {
    pub fn next(&self, now: Tick, has_player: bool) -> Option<Tick> {
        let next = self.requests.peek().ok()?;
        if matches!(next.request, MusicRequest::Replace(_))
            && has_player
            && self.retired.slots() == 0
        {
            return None;
        }
        Some(next.at.unwrap_or(now))
    }
    pub fn pop(&mut self) -> ScheduledRequest {
        self.requests.pop().expect("peeked music request")
    }
    pub fn retire(&mut self, player: Player) {
        self.retired
            .push(player)
            .unwrap_or_else(|_| panic!("reserved music retirement slot"));
    }
    pub fn publish_time(&self, at: Tick) {
        self.clock.store(at.0, Ordering::Release);
    }
}
