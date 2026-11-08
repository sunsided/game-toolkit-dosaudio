use super::{
    MusicCapabilities, MusicEvent, MusicHandle, MusicTarget, PlayOptions, Player, PreparedMusic,
    validate_play,
};
use crate::{Error, OplChip, Result, Tick};
use std::sync::Arc;
mod restore;

/// A soundtrack arrangement and its required synthesis family.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
#[repr(u8)]
#[non_exhaustive]
pub enum MusicDevice {
    /// Nine-channel AdLib.
    Opl2 = 0,
    /// Two hard-panned OPL2 chips.
    DualOpl2 = 1,
    /// OPL3.
    Opl3 = 2,
    /// SoundFont General MIDI.
    GeneralMidi = 3,
    /// CM-32L/MT-32 MIDI arrangements.
    Mt32 = 4,
    /// A caller-supplied MIDI synthesizer.
    CustomMidi = 5,
    /// Tandy PSG.
    Psg = 6,
    /// PC speaker.
    Speaker = 7,
    /// Prepared native synthie PCM.
    Native = 8,
}
impl MusicDevice {
    pub(crate) const ALL: [Self; 9] = [
        Self::Opl2,
        Self::DualOpl2,
        Self::Opl3,
        Self::GeneralMidi,
        Self::Mt32,
        Self::CustomMidi,
        Self::Psg,
        Self::Speaker,
        Self::Native,
    ];
    pub(crate) fn from_code(code: u32) -> Option<Self> {
        Self::ALL.get(code as usize).copied()
    }
    fn matches(self, target: MusicTarget) -> bool {
        matches!(
            (self, target),
            (Self::Opl2, MusicTarget::Opl(OplChip::Opl2))
                | (Self::DualOpl2, MusicTarget::Opl(OplChip::DualOpl2))
                | (Self::Opl3, MusicTarget::Opl(OplChip::Opl3))
                | (
                    Self::GeneralMidi | Self::Mt32 | Self::CustomMidi,
                    MusicTarget::Midi
                )
                | (Self::Psg, MusicTarget::Psg)
                | (Self::Speaker, MusicTarget::Speaker)
                | (Self::Native, MusicTarget::Native)
        )
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Checkpoint {
    pub at: Tick,
    pub index: usize,
    pub events: Vec<MusicEvent>,
}
#[derive(Clone, Debug)]
pub(crate) struct Arrangement {
    pub device: MusicDevice,
    pub music: PreparedMusic,
    pub checkpoints: Vec<Checkpoint>,
}

/// Alternative arrangements sharing an exact elapsed-tick duration and loop point.
///
/// Insert and validate on the control thread. Compact device-state checkpoints
/// are prepared there, so switching performs no file parsing, history scans or
/// heap reclamation in render. Checkpoints restore register/controller/key state;
/// sounding notes retrigger envelopes, rather than preserving oscillator phase.
/// Different tempos are allowed only as arrangements on the same elapsed-time
/// timeline; mismatched durations/loop positions fail instead of rescaling.
///
/// An arrangement is bounded to 4,096 event-time checkpoints, 1,024 restore
/// messages per checkpoint, and one million restore messages overall. MIDI
/// restoration retains up to 128 distinct held keys and 16 SysEx messages totalling
/// 65,536 bytes since its last reset. Files exceeding these limits fail during
/// insertion. Overlapping same-key MIDI notes are rejected for switching.
/// OPL+PSG mixed logs are not a single-device arrangement.
#[derive(Clone, Debug, Default)]
pub struct Soundtrack {
    pub(crate) arrangements: Vec<Arrangement>,
    fallbacks: [Option<MusicDevice>; 9],
}
impl Soundtrack {
    /// Empty soundtrack. Missing arrangements fail selection unless a fallback resolves.
    pub fn new() -> Self {
        Self::default()
    }
    /// Add or replace an arrangement, preparing all restore checkpoints first.
    /// Failure leaves this soundtrack unchanged.
    pub fn insert(&mut self, device: MusicDevice, music: impl Into<PreparedMusic>) -> Result<()> {
        let music = music.into();
        if !device.matches(music.target()) {
            return Err(Error::Config(
                "soundtrack device does not match arrangement target",
            ));
        }
        if self.arrangements.iter().any(|a| {
            a.device != device
                && (a.music.duration != music.duration || a.music.loop_start != music.loop_start)
        }) {
            return Err(Error::Config(
                "soundtrack arrangements require equal durations and loop points",
            ));
        }
        let checkpoints = restore::checkpoints(&music, device)?;
        let arrangement = Arrangement {
            device,
            music,
            checkpoints,
        };
        match self
            .arrangements
            .binary_search_by_key(&device, |a| a.device)
        {
            Ok(i) => self.arrangements[i] = arrangement,
            Err(i) => self.arrangements.insert(i, arrangement),
        }
        Ok(())
    }
    /// Set an explicit fallback edge. Cycles fail without changing existing edges.
    pub fn fallback(&mut self, from: MusicDevice, to: MusicDevice) -> Result<()> {
        let mut next = Some(to);
        for _ in 0..9 {
            match next {
                Some(device) if device == from => {
                    return Err(Error::Config("cyclic soundtrack fallback"));
                }
                Some(device) => next = self.fallbacks[device as usize],
                None => {
                    self.fallbacks[from as usize] = Some(to);
                    return Ok(());
                }
            }
        }
        Err(Error::Config("cyclic soundtrack fallback"))
    }
    /// Prepare native, OPL and speaker arrangements from synthie 0.8.0.
    #[cfg(feature = "synthie")]
    pub fn synthie(song: &synthie::pattern::Song, chip: OplChip, sample_rate: u32) -> Result<Self> {
        super::synthie::soundtrack(song, chip, sample_rate)
    }

    /// Durations and checkpoint data stay owned by the soundtrack throughout playback.
    pub fn arrangement(&self, device: MusicDevice) -> Option<&PreparedMusic> {
        self.find(device).map(|a| &a.music)
    }
    pub(crate) fn find(&self, device: MusicDevice) -> Option<&Arrangement> {
        self.arrangements
            .binary_search_by_key(&device, |a| a.device)
            .ok()
            .map(|i| &self.arrangements[i])
    }
    pub(crate) fn select(
        &self,
        preferred: MusicDevice,
        capabilities: MusicCapabilities,
    ) -> Option<MusicDevice> {
        let mut device = preferred;
        for _ in 0..9 {
            if self.find(device).is_some_and(|a| {
                capabilities.accepts(a.music.target)
                    && (!matches!(
                        device,
                        MusicDevice::GeneralMidi | MusicDevice::Mt32 | MusicDevice::CustomMidi
                    ) || capabilities.midi_device == Some(device))
            }) {
                return Some(device);
            }
            device = self.fallbacks[device as usize]?;
        }
        None
    }
}

pub(crate) fn player(
    soundtrack: Arc<Soundtrack>,
    preferred: MusicDevice,
    capabilities: MusicCapabilities,
    rate: u32,
    options: PlayOptions,
    at: Tick,
    last: Tick,
) -> Result<(MusicHandle, Player)> {
    let device = soundtrack
        .select(preferred, capabilities)
        .ok_or(Error::Config(
            "no usable soundtrack arrangement or configured fallback",
        ))?;
    let music = soundtrack
        .find(device)
        .expect("selected arrangement")
        .music
        .clone();
    validate_play(capabilities, &music, options, at)?;
    let mut handle = MusicHandle::new(options.volume, rate);
    let state = Arc::get_mut(&mut handle.state).expect("new handle");
    state
        .device
        .store(device as u32, std::sync::atomic::Ordering::Relaxed);
    for requested in MusicDevice::ALL {
        state.device_routes[requested as usize] = soundtrack
            .select(requested, capabilities)
            .map_or(u8::MAX, |d| d as u8);
    }
    let mut player = Player::new(music, options, &handle, at, last);
    player.soundtrack = Some(soundtrack);
    Ok((handle, player))
}
