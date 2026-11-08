//! Validated instrument banks. Parsing and mutation belong to the control thread.
use super::{OplOperator, OplPatch};
use crate::{Error, Result};

mod readers;
mod starter;

/// An address in an OPL MIDI instrument bank. Programs and drum keys are zero-based.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct OplBankKey {
    /// MIDI bank-select MSB (CC0), 0 through 127.
    pub msb: u8,
    /// MIDI bank-select LSB (CC32), 0 through 127.
    pub lsb: u8,
    /// Melodic program or channel-10 drum key, 0 through 127.
    pub program: u8,
    /// Whether the address belongs to the percussion set.
    pub percussion: bool,
}

/// A two-operator instrument, optionally layered with a second independent voice.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct OplInstrument {
    /// First voice's operator registers.
    pub patch: OplPatch,
    /// Optional second two-operator voice (not true four-operator synthesis).
    pub second: Option<OplPatch>,
    /// Semitone transposition of each voice, limited to -127 through 127.
    pub note_offsets: [i16; 2],
    /// Second-voice detune in OP2-compatible units; two units are 1/32 semitone.
    /// Odd values round toward negative infinity, matching the file formats.
    pub detune: i8,
    /// A fixed MIDI pitch instead of the incoming key, used by percussion.
    pub fixed_note: Option<u8>,
    /// Signed offset applied to note-on velocity before controller gain.
    pub velocity_offset: i8,
}

impl From<OplPatch> for OplInstrument {
    fn from(patch: OplPatch) -> Self {
        Self {
            patch,
            ..Self::default()
        }
    }
}

impl OplInstrument {
    fn validate(&self) -> Result<()> {
        if self.fixed_note.is_some_and(|key| key > 127)
            || self.note_offsets.iter().any(|n| !(-127..=127).contains(n))
        {
            return Err(Error::Format(
                "unsupported OPL instrument pitch or transposition",
            ));
        }
        for patch in [Some(self.patch), self.second].into_iter().flatten() {
            if patch.feedback > 15 || patch.modulator.waveform > 7 || patch.carrier.waveform > 7 {
                return Err(Error::Format("invalid OPL instrument register bits"));
            }
        }
        Ok(())
    }
}

/// One named instrument in a sorted bank.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OplBankEntry {
    /// MIDI bank, program and percussion address.
    pub key: OplBankKey,
    /// Source name (lossy UTF-8 for legacy bank encodings).
    pub name: String,
    /// Validated instrument register data.
    pub instrument: OplInstrument,
}

/// An owned, sorted bank with allocation-free binary-search lookup.
///
/// File readers never import a historical bank implicitly. [`Self::starter`]
/// supplies a small original starter set; loaded banks contain only their file's
/// entries. The MIDI driver falls back to bank 0's matching program, then program
/// 0 (drum 36 for percussion); absent fallbacks remain silent.
#[derive(Clone, Debug, Default)]
pub struct OplBank {
    entries: Vec<OplBankEntry>,
    pub(super) depth: u8,
}

impl OplBank {
    /// An empty bank, with no fallback instruments.
    pub fn new() -> Self {
        Self::default()
    }

    /// A small original bank under this crate's triple license.
    /// This supplies six melodic patches and five fixed-pitch percussion patches;
    /// it is not a complete General MIDI sound set.
    pub fn starter() -> Self {
        starter::bank()
    }

    /// Insert or replace an instrument on the calling thread. Invalid entries
    /// leave the bank unchanged. A bank holds at most 32,768 entries.
    pub fn insert(&mut self, entry: OplBankEntry) -> Result<()> {
        if entry.key.msb > 127 || entry.key.lsb > 127 || entry.key.program > 127 {
            return Err(Error::Format(
                "OPL bank address must contain seven-bit MIDI values",
            ));
        }
        entry.instrument.validate()?;
        match self.entries.binary_search_by_key(&entry.key, |e| e.key) {
            Ok(index) => self.entries[index] = entry,
            Err(index) => {
                if self.entries.len() == 32_768 {
                    return Err(Error::TooLarge);
                }
                self.entries.insert(index, entry);
            }
        }
        Ok(())
    }

    /// Look up an exact bank address without applying fallback policy.
    pub fn get(&self, key: OplBankKey) -> Option<&OplBankEntry> {
        self.entries
            .binary_search_by_key(&key, |e| e.key)
            .ok()
            .map(|i| &self.entries[i])
    }

    /// Named entries in ascending MIDI address order.
    pub fn entries(&self) -> &[OplBankEntry] {
        &self.entries
    }

    pub(super) fn resolve(&self, key: OplBankKey) -> Option<OplInstrument> {
        self.get(key)
            .or_else(|| {
                self.get(OplBankKey {
                    msb: 0,
                    lsb: 0,
                    ..key
                })
            })
            .or_else(|| {
                self.get(OplBankKey {
                    msb: 0,
                    lsb: 0,
                    program: if key.percussion { 36 } else { 0 },
                    ..key
                })
            })
            .map(|e| e.instrument)
    }
}
