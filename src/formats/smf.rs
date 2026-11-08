use crate::{Error, MidiMessage, MusicEvent, MusicTarget, PreparedMusic, Result, Tick, TimedEvent};
use midly::{MetaMessage, TrackEventKind};

const MAX_BYTES: usize = 64 * 1024 * 1024;
const MAX_EVENTS: usize = 1_000_000;
const MAX_TRACKS: usize = 1_024;

/// Standard MIDI Files, type 0 or synchronously merged type 1.
///
/// Requires an exact six-byte MThd header and declared MTrk chunks, each ending
/// with a zero-length EndOfTrack event and no trailing bytes. Type 2, RIFF wrappers,
/// raw F7 escapes, interleaved real-time bytes and unfinished SysEx are rejected.
/// Split F0/F7 SysEx packets are reassembled and emitted at their completion tick.
/// Tempo defaults to 500,000 microseconds per quarter note; SMPTE timing ignores
/// tempo and supports 24, 25, 30000/1001 and 30 frames per second.
///
/// Parsing is bounded to 64 MiB, 1,024 tracks, one million source events and
/// 65,536 bytes per complete SysEx message. Returned events own all payloads.
pub struct Smf;

impl Smf {
    /// Validate and prepare a MIDI file without retaining input references.
    pub fn load(bytes: &[u8]) -> Result<PreparedMusic> {
        if bytes.len() > MAX_BYTES {
            return Err(Error::TooLarge);
        }
        let mut file = Cursor::new(bytes);
        if file.take(4)? != b"MThd" || file.be32()? != 6 {
            return Err(Error::Format("SMF requires an exact MThd header"));
        }
        let format = file.be16()?;
        if format == 2 {
            return Err(Error::Format(
                "SMF type 2 asynchronous tracks are unsupported",
            ));
        }
        if format > 1 {
            return Err(Error::Format("unsupported SMF format"));
        }
        let count = usize::from(file.be16()?);
        if count == 0 || format == 0 && count != 1 {
            return Err(Error::Format("invalid SMF track count"));
        }
        if count > MAX_TRACKS {
            return Err(Error::TooLarge);
        }
        let mut clock = Clock::new(file.be16()?)?;
        let mut merged = Vec::new();
        let mut payload = Vec::new();
        let mut event_count = 0;
        for track in 0..count {
            if file.take(4)? != b"MTrk" {
                return Err(Error::Format("expected SMF track chunk"));
            }
            let length = usize::try_from(file.be32()?).map_err(|_| Error::TooLarge)?;
            let data = file.take(length)?;
            validate_track(data, &mut event_count)?;
            read_track(data, track, &mut merged, &mut payload)?;
        }
        if file.position != bytes.len() {
            return Err(Error::Format("trailing SMF data"));
        }
        merged.sort_unstable_by_key(|event| (event.tick, event.track, event.index));
        let mut events = Vec::new();
        let mut previous = 0;
        for event in merged {
            clock.advance(event.tick - previous)?;
            previous = event.tick;
            match event.kind {
                Kind::Tempo(tempo) => clock.tempo(tempo),
                Kind::Event(event) => events.push(TimedEvent {
                    at: clock.now,
                    event,
                }),
                Kind::End => {}
            }
        }
        PreparedMusic::new(MusicTarget::Midi, events, clock.now, payload)
    }
}

struct Cursor<'a> {
    data: &'a [u8],
    position: usize,
}
impl<'a> Cursor<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, position: 0 }
    }
    fn take(&mut self, count: usize) -> Result<&'a [u8]> {
        let end = self.position.checked_add(count).ok_or(Error::TooLarge)?;
        let bytes = self
            .data
            .get(self.position..end)
            .ok_or(Error::Format("truncated SMF data"))?;
        self.position = end;
        Ok(bytes)
    }
    fn byte(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }
    fn be16(&mut self) -> Result<u16> {
        Ok(u16::from_be_bytes(
            self.take(2)?.try_into().expect("two bytes"),
        ))
    }
    fn be32(&mut self) -> Result<u32> {
        Ok(u32::from_be_bytes(
            self.take(4)?.try_into().expect("four bytes"),
        ))
    }
    fn variable(&mut self) -> Result<u32> {
        let mut value = 0;
        for _ in 0..4 {
            let byte = self.byte()?;
            value = (value << 7) | u32::from(byte & 0x7f);
            if byte & 0x80 == 0 {
                return Ok(value);
            }
        }
        Err(Error::Format(
            "SMF variable-length value exceeds four bytes",
        ))
    }
}

// midly's default parser is intentionally tolerant. Validate framing first so
// truncation, malformed mandatory metadata and overlong VLQs cannot be discarded.
fn validate_track(data: &[u8], total: &mut usize) -> Result<()> {
    let mut input = Cursor::new(data);
    let mut running = None;
    while input.position < data.len() {
        *total += 1;
        if *total > MAX_EVENTS {
            return Err(Error::TooLarge);
        }
        input.variable()?;
        let next = *data
            .get(input.position)
            .ok_or(Error::Format("missing SMF event"))?;
        let status = if next < 0x80 {
            running.ok_or(Error::Format("SMF running status has no channel message"))?
        } else {
            input.byte()?
        };
        match status {
            0x80..=0xef => {
                running = Some(status);
                let length = if status & 0xe0 == 0xc0 { 1 } else { 2 };
                if input.take(length)?.iter().any(|&byte| byte >= 0x80) {
                    return Err(Error::Format("invalid SMF MIDI data byte"));
                }
            }
            0xf0 | 0xf7 => {
                running = None;
                let length = usize::try_from(input.variable()?).map_err(|_| Error::TooLarge)?;
                input.take(length)?;
            }
            0xff => {
                running = None;
                let kind = input.byte()?;
                if kind >= 0x80 {
                    return Err(Error::Format("invalid SMF meta type"));
                }
                let length = usize::try_from(input.variable()?).map_err(|_| Error::TooLarge)?;
                let bytes = input.take(length)?;
                if kind == 0x2f {
                    if length != 0 || input.position != data.len() {
                        return Err(Error::Format("SMF EndOfTrack must be empty and final"));
                    }
                    return Ok(());
                }
                if kind == 0x51 && (length != 3 || bytes == [0, 0, 0]) {
                    return Err(Error::Format(
                        "SMF tempo must encode nonzero microseconds in three bytes",
                    ));
                }
            }
            _ => return Err(Error::Format("unsupported SMF event status")),
        }
    }
    Err(Error::Format("SMF track requires EndOfTrack"))
}

struct Merged {
    tick: u64,
    track: usize,
    index: usize,
    kind: Kind,
}
enum Kind {
    Tempo(u32),
    Event(MusicEvent),
    End,
}

fn read_track(
    data: &[u8],
    track: usize,
    output: &mut Vec<Merged>,
    payload: &mut Vec<u8>,
) -> Result<()> {
    let mut tick = 0u64;
    let mut pending = None;
    let mut input = midly::EventIter::new(data);
    let mut index = 0;
    while !input.unread().is_empty() {
        let event = input
            .next()
            .ok_or(Error::Format("invalid SMF event"))?
            .map_err(|_| Error::Format("invalid SMF event"))?;
        tick = tick
            .checked_add(u64::from(event.delta.as_int()))
            .ok_or(Error::TooLarge)?;
        let kind = match event.kind {
            TrackEventKind::Midi { channel, message } => {
                if pending.is_some() {
                    return Err(Error::Format("MIDI channel message interrupts split SysEx"));
                }
                Some(Kind::Event(MusicEvent::Midi(channel_message(
                    channel.as_int(),
                    message,
                )?)))
            }
            TrackEventKind::SysEx(bytes) => {
                if pending.is_some() {
                    return Err(Error::Format("nested SMF SysEx message"));
                }
                pending = Some(vec![0xf0]);
                append_sysex(&mut pending, bytes, payload)?.map(Kind::Event)
            }
            TrackEventKind::Escape(bytes) => {
                if pending.is_none() {
                    return Err(Error::Format("raw SMF F7 escapes are unsupported"));
                }
                append_sysex(&mut pending, bytes, payload)?.map(Kind::Event)
            }
            TrackEventKind::Meta(MetaMessage::Tempo(value)) => Some(Kind::Tempo(value.as_int())),
            TrackEventKind::Meta(MetaMessage::EndOfTrack) => {
                if pending.is_some() {
                    return Err(Error::Format("unfinished SMF SysEx message"));
                }
                Some(Kind::End)
            }
            TrackEventKind::Meta(_) => None,
        };
        if let Some(kind) = kind {
            output.push(Merged {
                tick,
                track,
                index,
                kind,
            });
        }
        index += 1;
    }
    Ok(())
}

fn append_sysex(
    pending: &mut Option<Vec<u8>>,
    bytes: &[u8],
    payload: &mut Vec<u8>,
) -> Result<Option<MusicEvent>> {
    let message = pending.as_mut().expect("started SysEx");
    if message
        .len()
        .checked_add(bytes.len())
        .ok_or(Error::TooLarge)?
        > 65_536
    {
        return Err(Error::TooLarge);
    }
    if bytes
        .iter()
        .enumerate()
        .any(|(index, &byte)| byte >= 0x80 && !(byte == 0xf7 && index + 1 == bytes.len()))
    {
        return Err(Error::Format(
            "SMF SysEx contains status or real-time bytes",
        ));
    }
    message.extend_from_slice(bytes);
    if message.last() != Some(&0xf7) {
        return Ok(None);
    }
    let offset = u32::try_from(payload.len()).map_err(|_| Error::TooLarge)?;
    let length = u32::try_from(message.len()).map_err(|_| Error::TooLarge)?;
    payload.extend_from_slice(message);
    *pending = None;
    Ok(Some(MusicEvent::SysEx { offset, length }))
}

fn channel_message(channel: u8, message: midly::MidiMessage) -> Result<MidiMessage> {
    use midly::MidiMessage::*;
    let (status, first, second, length) = match message {
        NoteOff { key, vel } => (0x80, key.as_int(), vel.as_int(), 3),
        NoteOn { key, vel } => (0x90, key.as_int(), vel.as_int(), 3),
        Aftertouch { key, vel } => (0xa0, key.as_int(), vel.as_int(), 3),
        Controller { controller, value } => (0xb0, controller.as_int(), value.as_int(), 3),
        ProgramChange { program } => (0xc0, program.as_int(), 0, 2),
        ChannelAftertouch { vel } => (0xd0, vel.as_int(), 0, 2),
        PitchBend { bend } => (
            0xe0,
            (bend.0.as_int() & 0x7f) as u8,
            (bend.0.as_int() >> 7) as u8,
            3,
        ),
    };
    MidiMessage::new(&[status | channel, first, second][..length])
}

struct Clock {
    denominator: u128,
    factor: u128,
    ppq: bool,
    remainder: u128,
    now: Tick,
}
impl Clock {
    fn new(division: u16) -> Result<Self> {
        let (denominator, factor, ppq) = if division & 0x8000 == 0 {
            if division == 0 {
                return Err(Error::Format("SMF PPQ division must be nonzero"));
            }
            (
                u128::from(Tick::HZ_DENOMINATOR) * 1_000_000 * u128::from(division),
                u128::from(Tick::HZ_NUMERATOR) * 500_000,
                true,
            )
        } else {
            let subframes = u128::from(division & 0xff);
            if subframes == 0 {
                return Err(Error::Format("SMF SMPTE subframes must be nonzero"));
            }
            let (numerator, denominator) = match (division >> 8) as u8 as i8 {
                -24 => (24, 1),
                -25 => (25, 1),
                -29 => (30_000, 1_001),
                -30 => (30, 1),
                _ => return Err(Error::Format("invalid SMF SMPTE frame rate")),
            };
            (
                u128::from(Tick::HZ_DENOMINATOR) * numerator * subframes,
                u128::from(Tick::HZ_NUMERATOR) * denominator,
                false,
            )
        };
        Ok(Self {
            denominator,
            factor,
            ppq,
            remainder: 0,
            now: Tick::ZERO,
        })
    }
    fn advance(&mut self, ticks: u64) -> Result<()> {
        let numerator = u128::from(ticks)
            .checked_mul(self.factor)
            .and_then(|value| value.checked_add(self.remainder))
            .ok_or(Error::TooLarge)?;
        let delta = u64::try_from(numerator / self.denominator).map_err(|_| Error::TooLarge)?;
        self.remainder = numerator % self.denominator;
        self.now = self.now.checked_add(Tick(delta)).ok_or(Error::TooLarge)?;
        Ok(())
    }
    fn tempo(&mut self, tempo: u32) {
        if self.ppq {
            self.factor = u128::from(Tick::HZ_NUMERATOR) * u128::from(tempo);
        }
    }
}
