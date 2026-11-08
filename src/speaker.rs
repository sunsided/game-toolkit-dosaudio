//! Prepared QBasic PLAY melodies and binary PC-speaker pulse-width modulation.

use crate::{Error, MusicEvent, MusicHandle, MusicTarget, PreparedMusic, Result, Tick, TimedEvent};
use std::sync::Arc;

/// Cloneable controls for an independently sequenced PC-speaker track.
/// Dropping the last clone requests a stop, just like a music handle.
#[derive(Clone)]
pub struct SpeakerHandle(MusicHandle);

impl SpeakerHandle {
    pub(crate) fn new(handle: MusicHandle) -> Self {
        Self(handle)
    }
    /// Pause sequencing and mute speaker output.
    pub fn pause(&self) {
        self.0.pause();
    }
    /// Resume sequencing at the paused position.
    pub fn resume(&self) {
        self.0.resume();
    }
    /// Stop this track permanently.
    pub fn stop(&self) {
        self.0.stop();
    }
    /// Set the track's linear speaker gain, clamped to 0 through 1.
    pub fn set_volume(&self, volume: f32) {
        self.0.set_volume(volume);
    }
    /// Whether the track ended or a stop was requested.
    pub fn is_finished(&self) -> bool {
        self.0.is_finished()
    }
    /// Whether sequencing is paused.
    pub fn is_paused(&self) -> bool {
        self.0.is_paused()
    }
    /// Position relative to the current loop's start.
    pub fn position(&self) -> Tick {
        self.0.position()
    }
    /// Frequency in hertz of the note this track sounds, as of the last render.
    ///
    /// `None` during rests and PWM streams, while paused, and once the track has
    /// finished or stopped. Speaker port I/O is reported by
    /// [`DosAudio::speaker_tone`](crate::DosAudio::speaker_tone) instead.
    pub fn tone(&self) -> Option<f64> {
        let divisor = self.0.state.tone.load(std::sync::atomic::Ordering::Relaxed);
        (divisor != 0 && !self.is_paused() && !self.is_finished())
            .then(|| crate::device::speaker::pit_hz(divisor))
    }
}

/// A finite, prepared subset of the QBasic PLAY music macro language.
///
/// Supports A-G with `#`/`+`/`-`, O0-6, octave shifts `<`/`>`, T32-255,
/// L1-64, per-note lengths, P1-64 rests, N0-84 and MN/ML/MS articulation.
/// Defaults are octave 4, tempo 120, length 4 and normal articulation. PLAY's
/// octave 3 starts at middle C; N1 is octave 0's C, and N0 is a rest.
///
/// Up to eight dots follow a note or rest. Each dot multiplies its duration by
/// 3/2, following PLAY's repeated-dot convention. MF and MB are accepted but
/// produce identical asynchronous tracks: the caller uses the returned handle
/// to decide when to continue. Variables, substring macros and non-ASCII input
/// are unsupported. Letter case is ignored; whitespace is accepted between
/// commands and their arguments.
///
/// Absolute event times retain rational fractional ticks between notes and
/// round down only when emitting timestamps. Input and event counts are limited
/// to one million; rational arithmetic or timestamp overflow returns `TooLarge`.
/// Pitch is rounded to the nearest PIT divisor and clamped to its hardware range.
///
/// PLAY command reference: <https://scruss.com/qbasic_hlp/T003E.html>.
#[derive(Clone, Debug)]
pub struct Melody(PreparedMusic);

impl Melody {
    /// Compile a complete PLAY string before handing it to an audio callback.
    pub fn parse(source: &str) -> Result<Self> {
        if !source.is_ascii() {
            return Err(Error::Format("PLAY input must be ASCII"));
        }
        if source.len() > 1_000_000 {
            return Err(Error::TooLarge);
        }
        Parser::new(source.as_bytes()).parse().map(Self)
    }
    /// Track duration including rests and articulation gaps.
    pub fn duration(&self) -> Tick {
        self.0.duration()
    }
}

impl From<Melody> for PreparedMusic {
    fn from(melody: Melody) -> Self {
        melody.0
    }
}

#[derive(Clone, Copy)]
struct Fraction {
    numerator: u128,
    denominator: u128,
}

fn gcd(mut a: u128, mut b: u128) -> u128 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

impl Fraction {
    fn new(numerator: u128, denominator: u128) -> Self {
        let common = gcd(numerator, denominator);
        Self {
            numerator: numerator / common,
            denominator: denominator / common,
        }
    }
    fn add(self, other: Self) -> Result<Self> {
        let common = gcd(self.denominator, other.denominator);
        let a = other.denominator / common;
        let b = self.denominator / common;
        let numerator = self
            .numerator
            .checked_mul(a)
            .and_then(|n| {
                other
                    .numerator
                    .checked_mul(b)
                    .and_then(|m| n.checked_add(m))
            })
            .ok_or(Error::TooLarge)?;
        let denominator = self.denominator.checked_mul(a).ok_or(Error::TooLarge)?;
        Ok(Self::new(numerator, denominator))
    }
    fn multiply(self, numerator: u128, denominator: u128) -> Result<Self> {
        let a = gcd(self.numerator, denominator);
        let b = gcd(numerator, self.denominator);
        Ok(Self::new(
            (self.numerator / a)
                .checked_mul(numerator / b)
                .ok_or(Error::TooLarge)?,
            (self.denominator / b)
                .checked_mul(denominator / a)
                .ok_or(Error::TooLarge)?,
        ))
    }
    fn tick(self) -> Result<Tick> {
        u64::try_from(self.numerator / self.denominator)
            .map(Tick)
            .map_err(|_| Error::TooLarge)
    }
}

struct Parser<'a> {
    source: &'a [u8],
    cursor: usize,
    octave: u8,
    tempo: u16,
    length: u16,
    gate: (u128, u128),
    now: Fraction,
    events: Vec<TimedEvent>,
}

impl<'a> Parser<'a> {
    fn new(source: &'a [u8]) -> Self {
        Self {
            source,
            cursor: 0,
            octave: 4,
            tempo: 120,
            length: 4,
            gate: (7, 8),
            now: Fraction::new(0, 1),
            events: Vec::new(),
        }
    }
    fn peek(&mut self) -> Option<u8> {
        while self
            .source
            .get(self.cursor)
            .is_some_and(u8::is_ascii_whitespace)
        {
            self.cursor += 1;
        }
        self.source.get(self.cursor).map(u8::to_ascii_uppercase)
    }
    fn number(&mut self, required: bool) -> Result<Option<u16>> {
        self.peek();
        let start = self.cursor;
        let mut number = 0u16;
        while let Some(&byte) = self.source.get(self.cursor) {
            if !byte.is_ascii_digit() {
                break;
            }
            number = number
                .checked_mul(10)
                .and_then(|n| n.checked_add(u16::from(byte - b'0')))
                .ok_or(Error::Format("PLAY number is out of range"))?;
            self.cursor += 1;
        }
        if self.cursor == start {
            if required {
                Err(Error::Format("PLAY command requires a number"))
            } else {
                Ok(None)
            }
        } else {
            Ok(Some(number))
        }
    }
    fn bounded(&mut self, lower: u16, upper: u16) -> Result<u16> {
        let value = self.number(true)?.expect("required number");
        if !(lower..=upper).contains(&value) {
            return Err(Error::Format("PLAY number is out of range"));
        }
        Ok(value)
    }
    fn dots(&mut self) -> Result<u32> {
        let mut dots = 0;
        while self.peek() == Some(b'.') {
            self.cursor += 1;
            dots += 1;
            if dots > 8 {
                return Err(Error::Format("PLAY permits at most eight dots"));
            }
        }
        Ok(dots)
    }
    fn push(&mut self, at: Tick, divisor: u16, enabled: bool) -> Result<()> {
        if self.events.len() >= 1_000_000 {
            return Err(Error::TooLarge);
        }
        self.events.push(TimedEvent {
            at,
            event: MusicEvent::Speaker { divisor, enabled },
        });
        Ok(())
    }
    fn note(&mut self, pitch: Option<u8>, length: u16) -> Result<()> {
        let dots = self.dots()?;
        let duration = Fraction::new(
            u128::from(Tick::HZ_NUMERATOR) * 240 * 3u128.pow(dots),
            u128::from(Tick::HZ_DENOMINATOR)
                * u128::from(self.tempo)
                * u128::from(length)
                * 2u128.pow(dots),
        );
        let end = self.now.add(duration)?;
        // Check the final timestamp even for rests, before recording events.
        end.tick()?;
        if let Some(pitch) = pitch {
            let hz = 440.0 * 2.0f64.powf((f64::from(pitch) - 69.0) / 12.0);
            let divisor = (Tick::HZ_NUMERATOR as f64 / (Tick::HZ_DENOMINATOR as f64 * 12.0 * hz))
                .round()
                .clamp(1.0, 65_536.0) as u32 as u16;
            self.push(self.now.tick()?, divisor, true)?;
            let gate_end = self
                .now
                .add(duration.multiply(self.gate.0, self.gate.1)?)?
                .tick()?;
            self.push(gate_end, divisor, false)?;
        } else {
            self.push(self.now.tick()?, 0, false)?;
        }
        self.now = end;
        Ok(())
    }
    fn parse(mut self) -> Result<PreparedMusic> {
        while let Some(command) = self.peek() {
            self.cursor += 1;
            match command {
                b'O' => self.octave = self.bounded(0, 6)? as u8,
                b'<' => {
                    self.octave = self
                        .octave
                        .checked_sub(1)
                        .ok_or(Error::Format("PLAY octave is below zero"))?
                }
                b'>' => {
                    if self.octave == 6 {
                        return Err(Error::Format("PLAY octave exceeds six"));
                    }
                    self.octave += 1;
                }
                b'T' => self.tempo = self.bounded(32, 255)?,
                b'L' => self.length = self.bounded(1, 64)?,
                b'P' => {
                    let length = self.bounded(1, 64)?;
                    self.note(None, length)?;
                }
                b'N' => {
                    let number = self.bounded(0, 84)?;
                    self.note((number != 0).then_some((number + 23) as u8), self.length)?;
                }
                b'M' => {
                    let mode = self
                        .peek()
                        .ok_or(Error::Format("PLAY mode requires a suffix"))?;
                    self.cursor += 1;
                    match mode {
                        b'N' => self.gate = (7, 8),
                        b'L' => self.gate = (1, 1),
                        b'S' => self.gate = (3, 4),
                        b'B' | b'F' => {}
                        _ => return Err(Error::Format("unsupported PLAY mode")),
                    }
                }
                b'A'..=b'G' => {
                    let semitone = match command {
                        b'C' => 0,
                        b'D' => 2,
                        b'E' => 4,
                        b'F' => 5,
                        b'G' => 7,
                        b'A' => 9,
                        b'B' => 11,
                        _ => unreachable!(),
                    };
                    let mut pitch = 24 + i16::from(self.octave) * 12 + semitone;
                    match self.peek() {
                        Some(b'#' | b'+') => {
                            pitch += 1;
                            self.cursor += 1;
                        }
                        Some(b'-') => {
                            pitch -= 1;
                            self.cursor += 1;
                        }
                        _ => {}
                    }
                    if !(24..=107).contains(&pitch) {
                        return Err(Error::Format("PLAY accidental exceeds its octave range"));
                    }
                    let length = self.number(false)?.unwrap_or(self.length);
                    if !(1..=64).contains(&length) {
                        return Err(Error::Format("PLAY note length is out of range"));
                    }
                    self.note(Some(pitch as u8), length)?;
                }
                _ => return Err(Error::Format("unsupported PLAY command")),
            }
        }
        let duration = self.now.tick()?;
        self.push(duration, 0, false)?;
        PreparedMusic::new(
            MusicTarget::Speaker,
            self.events,
            duration,
            Arc::<[u8]>::from([]),
        )
    }
}

/// Prepare binary speaker pulses with duty cycle `sample / 255` per input period.
///
/// Samples are unsigned duty values, not signed PCM: 0 is disconnected and 255
/// is continuously high for that period. The final event always disconnects the
/// speaker. Rate must be 1 through 192,000 Hz, with at most one million samples.
/// Input-period and pulse-end positions retain cumulative phase using integer
/// rational arithmetic and round down to crystal ticks only at event emission.
pub fn speaker_pwm(samples: &[u8], sample_rate: u32) -> Result<PreparedMusic> {
    if !(1..=192_000).contains(&sample_rate) {
        return Err(Error::Config(
            "speaker PWM rate must be 1 through 192,000 Hz",
        ));
    }
    if samples.len() > 1_000_000 {
        return Err(Error::TooLarge);
    }
    let denominator = u128::from(Tick::HZ_DENOMINATOR) * u128::from(sample_rate);
    let tick = |numerator: u128, denominator: u128| -> Result<Tick> {
        u64::try_from(numerator / denominator)
            .map(Tick)
            .map_err(|_| Error::TooLarge)
    };
    let duration = tick(
        samples.len() as u128 * u128::from(Tick::HZ_NUMERATOR),
        denominator,
    )?;
    let mut events = Vec::with_capacity(samples.len() * 2 + 1);
    for (index, &sample) in samples.iter().enumerate() {
        let start = tick(index as u128 * u128::from(Tick::HZ_NUMERATOR), denominator)?;
        events.push(TimedEvent {
            at: start,
            event: MusicEvent::SpeakerLevel(sample != 0),
        });
        if sample != 0 && sample != 255 {
            let pulse_end = tick(
                (index as u128 * 255 + u128::from(sample)) * u128::from(Tick::HZ_NUMERATOR),
                denominator * 255,
            )?;
            events.push(TimedEvent {
                at: pulse_end,
                event: MusicEvent::SpeakerLevel(false),
            });
        }
    }
    events.push(TimedEvent {
        at: duration,
        event: MusicEvent::SpeakerLevel(false),
    });
    PreparedMusic::new(
        MusicTarget::Speaker,
        events,
        duration,
        Arc::<[u8]>::from([]),
    )
}
