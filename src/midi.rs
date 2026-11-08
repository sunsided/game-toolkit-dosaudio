//! MIDI messages and synthesis adapters shared by ports and prepared music.
use crate::{Error, Frame, Result};
use std::sync::Arc;
#[cfg(feature = "gm")]
mod gm;
#[cfg(feature = "mt32")]
mod mt32;
#[cfg(feature = "mt32")]
pub use mt32::{Mt32Model, Mt32Roms, Mt32Synth};
#[cfg(feature = "mpu401")]
pub(crate) mod voice;
#[cfg(feature = "gm")]
pub use gm::GmSynth;
#[cfg(feature = "gm")]
pub use rustysynth::SoundFont;

/// Synthesis factory, evaluated once outside the audio callback.
#[derive(Clone)]
#[non_exhaustive]
pub enum MidiBackend {
    /// A user-supplied real-time adapter, receiving the requested output rate.
    Custom(Arc<dyn Fn(u32) -> Result<Box<dyn MidiSynth>> + Send + Sync>),
    /// General MIDI synthesis using a caller-supplied SF2.
    #[cfg(feature = "gm")]
    Gm(Arc<SoundFont>),
    /// CM-32L synthesis with caller-supplied, validated ROMs.
    #[cfg(feature = "mt32")]
    Mt32(Arc<Mt32Roms>),
}
impl core::fmt::Debug for MidiBackend {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Custom(_) => f.write_str("Custom"),
            #[cfg(feature = "gm")]
            Self::Gm(_) => f.write_str("Gm"),
            #[cfg(feature = "mt32")]
            Self::Mt32(_) => f.write_str("Mt32"),
        }
    }
}
#[cfg(feature = "mpu401")]
impl MidiBackend {
    pub(crate) fn device(&self) -> crate::MusicDevice {
        match self {
            Self::Custom(_) => crate::MusicDevice::CustomMidi,
            #[cfg(feature = "gm")]
            Self::Gm(_) => crate::MusicDevice::GeneralMidi,
            #[cfg(feature = "mt32")]
            Self::Mt32(_) => crate::MusicDevice::Mt32,
        }
    }
    pub(crate) fn build(&self, rate: u32) -> Result<Box<dyn MidiSynth>> {
        match self {
            Self::Custom(factory) => factory(rate),
            #[cfg(feature = "gm")]
            Self::Gm(font) => Ok(Box::new(GmSynth::with_soundfont(Arc::clone(font), rate)?)),
            #[cfg(feature = "mt32")]
            Self::Mt32(roms) => Ok(Box::new(Mt32Synth::new(Arc::clone(roms))?)),
        }
    }
}

/// A complete short MIDI message (channel, system common, or real-time).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MidiMessage {
    bytes: [u8; 3],
    length: u8,
}
impl MidiMessage {
    /// Validate a complete message. SysEx belongs to prepared music's byte storage.
    pub fn new(bytes: &[u8]) -> Result<Self> {
        let Some(&status) = bytes.first() else {
            return Err(Error::Format("empty MIDI message"));
        };
        let length = match status {
            0x80..=0xbf | 0xe0..=0xef | 0xf2 => 3,
            0xc0..=0xdf | 0xf1 | 0xf3 => 2,
            0xf6 | 0xf8..=0xff => 1,
            _ => return Err(Error::Format("invalid short MIDI status")),
        };
        if bytes.len() != length || bytes[1..].iter().any(|&b| b >= 0x80) {
            return Err(Error::Format("invalid MIDI length or data byte"));
        }
        let mut data = [0; 3];
        data[..length].copy_from_slice(bytes);
        Ok(Self {
            bytes: data,
            length: length as u8,
        })
    }
    /// Complete status and data bytes.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes[..usize::from(self.length)]
    }
}

/// A synthesizer prepared outside rendering, with bounded real-time operations.
///
/// Both methods must avoid allocation, deallocation, locks, and blocking. Messages
/// are complete short messages or complete F0..F7 SysEx. Keep owned resources in
/// the adapter until it is destroyed outside the callback.
pub trait MidiSynth: Send {
    /// Native synthesis sample rate.
    fn sample_rate(&self) -> u32;
    /// Apply a complete MIDI message at the current native output position.
    fn message(&mut self, bytes: &[u8]);
    /// Generate the next native stereo frames.
    fn render(&mut self, out: &mut [Frame]);
    /// Messages dropped by an optional bounded backend queue (zero by default).
    fn dropped_messages(&self) -> u64 {
        0
    }
}
