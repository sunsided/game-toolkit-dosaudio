//! DOS-era sound devices sharing one timestamped hardware timeline.
//!
//! Game code owns [`DosAudio`]; an audio callback owns [`AudioEngine`]. Port
//! commands cross a bounded queue with [`Tick`] timestamps from
//! `game-toolkit-dosbus`. Music is prepared before playback and sequenced on the
//! audio thread. [`OfflineMachine`] uses the same renderer without audio I/O.
//!
//! ```
//! use game_toolkit_dosaudio::{OfflineMachine, SoundConfig};
//! use std::time::Duration;
//!
//! let mut machine = OfflineMachine::new(SoundConfig::none().speaker(true), 48_000)?;
//! machine.sound(440.0)?;
//! machine.render_for(Duration::from_millis(100))?;
//! machine.nosound()?;
//! machine.render_for(Duration::from_millis(20))?;
//! let wav_bytes = machine.finish().to_bytes()?;
//! assert_eq!(&wav_bytes[..4], b"RIFF");
//! # Ok::<(), game_toolkit_dosaudio::Error>(())
//! ```
#![forbid(unsafe_code)]

mod bus;
mod capture;
mod config;
mod device;
#[cfg(feature = "sb")]
mod dma;
mod engine;
mod error;
#[cfg(feature = "formats")]
pub mod formats;
pub mod midi;
pub mod music;
mod offline;
mod speaker;
pub use speaker::{Melody, SpeakerHandle, speaker_pwm};
#[cfg(feature = "opl")]
mod opl;
#[cfg(feature = "sb")]
pub mod sfx;

pub use bus::DosAudio;
pub use capture::{Capture, MusicCapture, PortWrite};
pub use config::{OplBackend, OplChip, SbModel, SoundBlasterConfig, SoundConfig};
#[cfg(feature = "opl")]
pub use device::opl::OplCore;
#[cfg(feature = "psg")]
pub use device::psg::Psg;
#[cfg(feature = "sb")]
pub use dma::{ConventionalMemory, DmaCompletion, DmaController, DmaUnit};
pub use engine::{AudioEngine, EngineStats};
pub use error::{Error, Result};
pub use game_toolkit_dosbus::{
    DmaChannel, DmaDirection, DmaRequest, DmaWidth, IoDevice, IrqLine, IrqRequest, PortRange, Tick,
    TickAccumulator,
};
#[cfg(feature = "gm")]
pub use midi::{GmSynth, SoundFont};
pub use midi::{MidiBackend, MidiMessage, MidiSynth};
pub use music::{
    MusicController, MusicEvent, MusicHandle, MusicTarget, PlayOptions, PreparedMusic, Sequence,
    TimedEvent,
};
pub use offline::{OfflineMachine, Wav};
#[cfg(feature = "opl")]
pub use opl::{
    Drum, Note, Opl, OplBank, OplBankEntry, OplBankKey, OplInstrument, OplMidiDriver, OplOperator,
    OplPatch,
};
#[cfg(feature = "sb")]
pub use sfx::{SfxHandle, SfxMixer, SfxVoiceHandle};

/// A stereo PCM frame, with each channel in -1 through 1.
pub type Frame = [f32; 2];

/// Common machine, device and music types.
pub mod prelude {
    #[cfg(feature = "formats")]
    pub use crate::formats::{Dro, Imf, Smf, Vgm};
    pub use crate::{
        AudioEngine, DmaChannel, DosAudio, Frame, IoDevice, MusicController, MusicEvent,
        MusicHandle, MusicTarget, OfflineMachine, OplBackend, OplChip, PlayOptions, PreparedMusic,
        SbModel, Sequence, SoundBlasterConfig, SoundConfig, Tick, TimedEvent,
    };
    #[cfg(feature = "sb")]
    pub use crate::{
        ConventionalMemory, DmaController, DmaUnit, SfxHandle, SfxMixer, SfxVoiceHandle,
    };
    #[cfg(feature = "opl")]
    pub use crate::{
        Drum, Note, Opl, OplBank, OplBankEntry, OplBankKey, OplInstrument, OplMidiDriver,
        OplOperator, OplPatch,
    };
}
