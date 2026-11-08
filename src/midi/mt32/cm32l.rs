// Copyright (C) 2021-2026 Geoff Hill <geoff@geoffhill.org>
// Copyright (C) 2003-2026 Dean Beeler, Jerome Fisher, Sergey V. Mikayev
//
// This program is free software: you can redistribute it and/or modify it
// under the terms of the GNU Lesser General Public License as published by
// the Free Software Foundation, either version 2.1 of the License, or (at
// your option) any later version. Read COPYING.LESSER.txt for details.

// Modified for dosaudio: safe decoding, bounded SysEx, in-place reset and pooled reverb.
macro_rules! trace {
    ($($arg:tt)*) => {};
}
macro_rules! debug {
    ($($arg:tt)*) => {};
}
macro_rules! warn {
    ($($arg:tt)*) => {};
}
macro_rules! info {
    ($($arg:tt)*) => {};
}
macro_rules! error {
    ($($arg:tt)*) => {};
}
macro_rules! debug_span {
    ($($arg:tt)*) => {
        ()
    };
}
use core::{error, fmt};
pub use midi::{Error as MidiError, Message};
pub(super) use rom::Rom;
use std::boxed::Box;

mod dispatch;
mod element;

mod lpf;
mod midi;
mod midiqueue;
pub mod param;
mod pcm;
mod ramp;
mod render;
mod reverb;
mod rom;

mod synth;
mod sysex;
mod tables;
mod tva;
mod tvf;
mod tvp;

pub mod cm32l {
    pub use crate::midi::mt32::cm32l::CM32L as Device;

    pub use crate::midi::mt32::cm32l::midi::{MelodicInstrument, RhythmInstrument};
    pub use crate::midi::mt32::cm32l::rom::{
        CONTROL_SIZE, ControlArray, Error as RomError, PADDED_TIMBRE_SIZE, PCM_SIZE, PcmArray,
        PcmMeta, RawTimbreBank, Rom, TIMBRES_COUNT,
    };
}

use element::{PartArena, PartialArena, PolyArena};
use lpf::CoarseLpf;
use midiqueue::MidiQueue;
use reverb::Reverb;
use sysex::MemState;

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
#[repr(C)]
pub struct Frame(pub i16, pub i16);

#[non_exhaustive]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ControlCommand {
    SetPartProgram { part: u8, program: u8 },
    SetPartVolume { part: u8, volume: u8 },
    SetMasterVolume { volume: u8 },
    SetReverbMode { mode: u8 },
    SetReverbTime { time: u8 },
    SetReverbLevel { level: u8 },
    Reset,
}

#[non_exhaustive]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ControlError {
    PartOutOfRange(u8),
    MelodicPartOutOfRange(u8),
    ProgramOutOfRange(u8),
    VolumeOutOfRange(u8),
    ReverbModeOutOfRange(u8),
    ReverbTimeOutOfRange(u8),
    ReverbLevelOutOfRange(u8),
}

impl fmt::Display for ControlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            ControlError::PartOutOfRange(part) => {
                write!(f, "part out of range (expected 0-8): {part}")
            }
            ControlError::MelodicPartOutOfRange(part) => {
                write!(f, "melodic part out of range (expected 0-7): {part}")
            }
            ControlError::ProgramOutOfRange(program) => {
                write!(f, "program out of range (expected 0-127): {program}")
            }
            ControlError::VolumeOutOfRange(volume) => {
                write!(f, "volume out of range (expected 0-100): {volume}")
            }
            ControlError::ReverbModeOutOfRange(mode) => {
                write!(f, "reverb mode out of range: {mode}")
            }
            ControlError::ReverbTimeOutOfRange(time) => {
                write!(f, "reverb time out of range: {time}")
            }
            ControlError::ReverbLevelOutOfRange(level) => {
                write!(f, "reverb level out of range: {level}")
            }
        }
    }
}

impl error::Error for ControlError {}

pub trait Synth {
    fn current_time(&self) -> u32;

    fn play_msg_at(&mut self, msg: u32, time: u32) -> bool;

    fn play_msg(&mut self, msg: u32) -> bool {
        self.play_msg_at(msg, self.current_time())
    }

    fn play_sysex_at(&mut self, sysex: &[u8], time: u32) -> bool;

    fn play_sysex(&mut self, sysex: &[u8]) -> bool {
        self.play_sysex_at(sysex, self.current_time())
    }

    fn render(&mut self, out: &mut [Frame]);

    fn apply_command(&mut self, command: ControlCommand) -> Result<(), ControlError>;
}

#[non_exhaustive]
#[derive(Debug)]
pub struct CM32L {
    time: u32,
    rom: Rom,
    mem: Box<MemState>,
    midi_queue: MidiQueue,
    sysex_pool: [std::sync::Arc<[u8]>; 16],
    parts: PartArena,
    free_polys: PolyArena,
    free_partials: PartialArena,
    reverb: Reverb,
    lpf_left: CoarseLpf,
    lpf_right: CoarseLpf,
    last_midi_timestamp: u32,
    chantable: [[u8; 9]; 16],
    aborting_part_ix: usize,
    master_tune_pitch_delta: i32,
}

impl CM32L {
    pub fn new(rom: Rom) -> CM32L {
        let reverb = Reverb::new();
        let mem = MemState::new(&rom);
        let master_volume = mem.master_volume;
        let parts = PartArena::new(master_volume, &rom);
        let free_partials = PartialArena::new(&rom.meta().reserve_settings);
        let mut chantable = [[0xFF; 9]; 16];
        dispatch::rebuild_chantable(&mut chantable, &mem.raw_system);
        CM32L {
            time: 0,
            rom,
            mem,
            midi_queue: MidiQueue::new(),
            sysex_pool: std::array::from_fn(|_| std::sync::Arc::from(vec![0u8; 65_536])),
            parts,
            free_polys: PolyArena::new(),
            free_partials,
            reverb,
            lpf_left: CoarseLpf::new(),
            lpf_right: CoarseLpf::new(),
            last_midi_timestamp: 0,
            chantable,
            aborting_part_ix: 0,
            master_tune_pitch_delta: 0,
        }
    }

    fn apply_command_inner(&mut self, command: ControlCommand) -> Result<(), ControlError> {
        match command {
            ControlCommand::SetPartProgram { part, program } => {
                if part > 7 {
                    return Err(ControlError::MelodicPartOutOfRange(part));
                }
                if program > 127 {
                    return Err(ControlError::ProgramOutOfRange(program));
                }
                let idx = part as usize;
                self.parts.parts[idx].hold_pedal = false;
                self.parts.parts[idx].all_sound_off(
                    &mut self.free_polys,
                    &mut self.free_partials,
                    &self.mem,
                );
                self.parts.parts[idx].program = program as usize;
                self.mem.set_program(idx, program as usize);
                let timbre = &self.mem.timbre_temp[idx];
                self.free_partials.update_tvp_params(idx, timbre);
                Ok(())
            }
            ControlCommand::SetPartVolume { part, volume } => {
                if part > 8 {
                    return Err(ControlError::PartOutOfRange(part));
                }
                if volume > 100 {
                    return Err(ControlError::VolumeOutOfRange(volume));
                }
                let idx = part as usize;
                self.mem.raw_patch_temp[idx][8] = volume;
                self.mem.patch_temp[idx].output_level = volume as usize;
                self.parts.parts[idx].amp_ctx.part_volume = volume as usize;
                Ok(())
            }
            ControlCommand::SetMasterVolume { volume } => {
                if volume > 100 {
                    return Err(ControlError::VolumeOutOfRange(volume));
                }
                self.mem.raw_system[22] = volume;
                self.mem.master_volume = volume as usize;
                for part in &mut self.parts.parts {
                    part.amp_ctx.master_volume = volume as usize;
                }
                Ok(())
            }
            ControlCommand::SetReverbMode { mode } => {
                if mode > 3 {
                    return Err(ControlError::ReverbModeOutOfRange(mode));
                }
                self.mem.raw_system[1] = mode;
                self.reverb.set_mode(mode);
                Ok(())
            }
            ControlCommand::SetReverbTime { time } => {
                if time > 7 {
                    return Err(ControlError::ReverbTimeOutOfRange(time));
                }
                self.mem.raw_system[2] = time;
                self.reverb.set_time(time);
                Ok(())
            }
            ControlCommand::SetReverbLevel { level } => {
                if level > 7 {
                    return Err(ControlError::ReverbLevelOutOfRange(level));
                }
                self.mem.raw_system[3] = level;
                self.reverb.set_level(level);
                Ok(())
            }
            ControlCommand::Reset => {
                self.reset();
                Ok(())
            }
        }
    }
}

impl CM32L {
    pub(super) fn clear_pending(&mut self) {
        while self.midi_queue.pop().is_some() {}
        self.last_midi_timestamp = self.time;
    }
}

#[cfg(test)]
impl CM32L {
    pub(super) fn set_test_clock(&mut self, time: u32) {
        self.time = time;
    }
}
