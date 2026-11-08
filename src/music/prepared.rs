use super::{MusicEvent, Sequence, TimedEvent};
use crate::{Error, OplChip, Result, Tick};
use std::sync::Arc;

/// Device arrangement required by a prepared soundtrack.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum MusicTarget {
    /// Yamaha FM hardware, preserving the OPL2, dual OPL2 or OPL3 distinction.
    Opl(OplChip),
    /// A MIDI synthesizer receiving complete short messages and SysEx.
    Midi,
    /// An SN76489 programmable sound generator.
    Psg,
    /// PIT channel 2 and the PC speaker.
    Speaker,
    /// Yamaha FM and SN76489 writes sharing one timeline.
    OplPsg(OplChip),
    /// Prepared native stereo PCM (synthie feature).
    Native,
}

/// Immutable, validated device events and owned SysEx storage.
///
/// Construction and cloning occur outside rendering. Clones share event and
/// payload allocations. SysEx events reference complete F0..F7 messages of at
/// most 65,536 bytes, including framing. Interior bytes must be seven-bit data;
/// interleaved MIDI real-time messages must be separate [`MusicEvent::Midi`]
/// events. Unreferenced payload bytes are retained but are not interpreted.
#[derive(Clone, Debug)]
pub struct PreparedMusic {
    pub(crate) events: Arc<[TimedEvent]>,
    pub(crate) duration: Tick,
    pub(crate) sysex: Arc<[u8]>,
    pub(crate) loop_start: Tick,
    pub(crate) loop_index: usize,
    pub(crate) target: MusicTarget,
    pub(crate) pcm: Option<Arc<NativePcm>>,
}

impl PreparedMusic {
    /// Validate sorted events, their target, and every referenced SysEx range.
    /// Equal timestamps preserve their supplied order; events at the end are valid.
    /// Zero duration is valid for finite music, with default loop start zero.
    pub fn new(
        target: MusicTarget,
        events: Vec<TimedEvent>,
        duration: Tick,
        sysex: impl Into<Arc<[u8]>>,
    ) -> Result<Self> {
        let sysex = sysex.into();
        let mut previous = Tick::ZERO;
        for timed in &events {
            if timed.at < previous || timed.at > duration {
                return Err(Error::Format("invalid prepared event ordering or duration"));
            }
            previous = timed.at;
            let compatible = matches!(
                (target, timed.event),
                (
                    MusicTarget::Opl(_) | MusicTarget::OplPsg(_),
                    MusicEvent::Opl { .. }
                ) | (
                    MusicTarget::Midi,
                    MusicEvent::Midi(_) | MusicEvent::SysEx { .. }
                ) | (
                    MusicTarget::Psg | MusicTarget::OplPsg(_),
                    MusicEvent::Psg(_)
                ) | (
                    MusicTarget::Speaker,
                    MusicEvent::Speaker { .. } | MusicEvent::SpeakerLevel(_)
                )
            );
            if !compatible {
                return Err(Error::Format(
                    "prepared event does not match its music target",
                ));
            }
            match timed.event {
                MusicEvent::Opl { register, .. } => {
                    let chip = match target {
                        MusicTarget::Opl(chip) | MusicTarget::OplPsg(chip) => chip,
                        _ => unreachable!("compatible OPL target"),
                    };
                    if register > 0x1ff || chip == OplChip::Opl2 && register > 0xff {
                        return Err(Error::Format(
                            "prepared OPL register does not match its chip",
                        ));
                    }
                }
                MusicEvent::SysEx { offset, length } => {
                    if !(2..=65_536).contains(&length) {
                        return Err(Error::Format("SysEx length must be 2..=65,536 bytes"));
                    }
                    let end = offset.checked_add(length).ok_or(Error::TooLarge)?;
                    let start = usize::try_from(offset).map_err(|_| Error::TooLarge)?;
                    let end = usize::try_from(end).map_err(|_| Error::TooLarge)?;
                    let bytes = sysex
                        .get(start..end)
                        .ok_or(Error::Format("SysEx range exceeds owned payload storage"))?;
                    if bytes[0] != 0xf0
                        || bytes[bytes.len() - 1] != 0xf7
                        || bytes[1..bytes.len() - 1].iter().any(|&byte| byte > 0x7f)
                    {
                        return Err(Error::Format(
                            "SysEx requires F0/F7 framing and seven-bit payload",
                        ));
                    }
                }
                _ => {}
            }
        }
        Ok(Self {
            events: events.into(),
            duration,
            sysex,
            loop_start: Tick::ZERO,
            loop_index: 0,
            target,
            pcm: None,
        })
    }

    /// Set the repeat region's start strictly before the track's duration.
    /// An explicit loop start at the end, including zero on an empty track, fails.
    pub fn with_loop_start(self, at: Tick) -> Result<Self> {
        let index = self.events.partition_point(|event| event.at < at);
        self.with_loop_boundary(at, index)
    }

    // A file loop offset can distinguish intro and repeat writes at the same
    // timestamp. Preserve its event boundary separately from its clock position.
    pub(crate) fn with_loop_boundary(mut self, at: Tick, index: usize) -> Result<Self> {
        if at >= self.duration {
            return Err(Error::Format(
                "loop start must precede prepared music duration",
            ));
        }
        if index > self.events.len()
            || index > 0 && self.events[index - 1].at > at
            || self.events.get(index).is_some_and(|event| event.at < at)
        {
            return Err(Error::Format(
                "loop event boundary does not match its timestamp",
            ));
        }
        self.loop_start = at;
        self.loop_index = index;
        Ok(self)
    }

    /// Events in playback order, retaining FIFO order at equal timestamps.
    pub fn events(&self) -> &[TimedEvent] {
        &self.events
    }
    /// End of the soundtrack's final delay, excluding any release tail.
    pub fn duration(&self) -> Tick {
        self.duration
    }
    /// Required device arrangement.
    pub fn target(&self) -> MusicTarget {
        self.target
    }
    /// Owned storage referenced by complete SysEx events.
    pub fn sysex(&self) -> &[u8] {
        &self.sysex
    }
    /// Start of the repeat region, zero unless explicitly supplied.
    pub fn loop_start(&self) -> Tick {
        self.loop_start
    }
}

impl From<Sequence> for PreparedMusic {
    fn from(sequence: Sequence) -> Self {
        Self {
            events: sequence.events,
            duration: sequence.duration,
            sysex: Arc::from([]),
            loop_start: Tick::ZERO,
            loop_index: 0,
            target: MusicTarget::Opl(sequence.chip),
            pcm: None,
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct NativePcm {
    pub frames: Arc<[crate::Frame]>,
    pub rate: u32,
}
impl PreparedMusic {
    /// Prepare finite native PCM outside rendering. Available for synthie output
    /// and shared soundtrack switching; playback supports the usual handles.
    #[cfg(feature = "synthie")]
    pub fn from_pcm(frames: impl Into<Arc<[crate::Frame]>>, sample_rate: u32) -> Result<Self> {
        let frames = frames.into();
        if !(8_000..=192_000).contains(&sample_rate) || frames.len() > 16_000_000 {
            return Err(Error::Config(
                "native PCM requires 8–192 kHz and at most 16 million frames",
            ));
        }
        if frames.iter().flatten().any(|v| !v.is_finite()) {
            return Err(Error::Format("native PCM contains non-finite samples"));
        }
        let duration = crate::TickAccumulator::new(u64::from(sample_rate))
            .checked_advance(frames.len() as u64)
            .ok_or(Error::TooLarge)?;
        let mut music = Self::new(MusicTarget::Native, vec![], duration, [])?;
        music.pcm = Some(Arc::new(NativePcm {
            frames,
            rate: sample_rate,
        }));
        Ok(music)
    }
}
