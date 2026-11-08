// Copyright (C) 2021-2026 Geoff Hill <geoff@geoffhill.org>
// Copyright (C) 2003-2026 Dean Beeler, Jerome Fisher, Sergey V. Mikayev
//
// This program is free software: you can redistribute it and/or modify it
// under the terms of the GNU Lesser General Public License as published by
// the Free Software Foundation, either version 2.1 of the License, or (at
// your option) any later version. Read COPYING.LESSER.txt for details.

use crate::midi::mt32::cm32l::param::{
    MELODIC_PARTS_COUNT, PatchParam, PatchTemp, RHYTHM_KEYS_COUNT, RawPatchParam, RawPatchTemp,
    RawRhythmTemp, RhythmKey, TIMBRE_PARAM_SIZE, TimbreParam, cast_raw_timbre,
};
use crate::midi::mt32::cm32l::rom::{self, Meta, Rom};
use core::fmt;
use std::boxed::Box;

fn cast_patch(raw: &[u8; 8]) -> PatchParam {
    PatchParam::from(&RawPatchParam::decode(raw))
}

fn cast_patch_temp(raw: &[u8; 16]) -> PatchTemp {
    PatchTemp::from(&RawPatchTemp::decode(raw))
}

fn cast_rhythm_temp(raw: &[u8; 4]) -> RhythmKey {
    RhythmKey::from(&RawRhythmTemp::decode(raw))
}

fn cast_timbre(raw: &[u8; TIMBRE_PARAM_SIZE]) -> TimbreParam {
    TimbreParam::from(&cast_raw_timbre(raw))
}

pub const fn memaddr(x: u32) -> u32 {
    ((x & 0x7f0000) >> 2) | ((x & 0x7f00) >> 1) | (x & 0x7f)
}

// Memory region base addresses (post-memaddr conversion).
const MEM_PATCH_TEMP: u32 = memaddr(0x030000);
const MEM_RHYTHM_TEMP: u32 = memaddr(0x030110);
const MEM_TIMBRE_TEMP: u32 = memaddr(0x040000);
const MEM_PATCHES: u32 = memaddr(0x050000);
const MEM_TIMBRES: u32 = memaddr(0x080000);
const MEM_SYSTEM: u32 = memaddr(0x100000);

pub struct SysexEffects {
    pub refresh_part_volumes: bool,
    pub refresh_reverb: bool,
    pub refresh_master_tune: bool,
    pub refresh_master_volume: bool,
    pub refresh_reserve: bool,
    pub chan_assign_first: Option<usize>,
    pub chan_assign_last: Option<usize>,
    pub timbre_changed_parts: u16,
}

impl SysexEffects {
    fn none() -> Self {
        SysexEffects {
            refresh_part_volumes: false,
            refresh_reverb: false,
            refresh_master_tune: false,
            refresh_master_volume: false,
            refresh_reserve: false,
            chan_assign_first: None,
            chan_assign_last: None,
            timbre_changed_parts: 0,
        }
    }

    fn merge(&mut self, other: SysexEffects) {
        self.refresh_part_volumes |= other.refresh_part_volumes;
        self.refresh_reverb |= other.refresh_reverb;
        self.refresh_master_tune |= other.refresh_master_tune;
        self.refresh_master_volume |= other.refresh_master_volume;
        self.refresh_reserve |= other.refresh_reserve;
        self.timbre_changed_parts |= other.timbre_changed_parts;
        if other.chan_assign_first.is_some() {
            self.chan_assign_first = other.chan_assign_first;
            self.chan_assign_last = other.chan_assign_last;
        }
    }
}

pub struct MemState {
    pub raw_patch_temp: [[u8; 16]; 9],
    pub raw_rhythm_temp: [[u8; 4]; RHYTHM_KEYS_COUNT],
    pub raw_timbre_temp: [[u8; TIMBRE_PARAM_SIZE]; MELODIC_PARTS_COUNT],
    pub raw_patches: [[u8; 8]; 128],
    pub raw_timbres: rom::RawTimbreBank,
    pub raw_system: [u8; 23],

    pub patch_temp: [PatchTemp; 9],
    pub rhythm_temp: [RhythmKey; RHYTHM_KEYS_COUNT],
    pub timbre_temp: [TimbreParam; MELODIC_PARTS_COUNT],
    pub patches: [PatchParam; 128],
    pub timbres: [TimbreParam; rom::TIMBRES_COUNT],
    pub master_volume: usize,
}

impl fmt::Debug for MemState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MemState")
            .field("master_volume", &self.master_volume)
            .finish_non_exhaustive()
    }
}

impl MemState {
    pub fn new(rom: &Rom) -> Box<MemState> {
        let meta = rom.meta();

        let mut mem = Box::new(MemState {
            raw_patch_temp: [[0u8; 16]; 9],
            raw_rhythm_temp: [[0u8; 4]; RHYTHM_KEYS_COUNT],
            raw_timbre_temp: [[0u8; TIMBRE_PARAM_SIZE]; MELODIC_PARTS_COUNT],
            raw_patches: [[0u8; 8]; 128],
            raw_timbres: [[0u8; 256]; 256],
            raw_system: [0u8; 23],
            patch_temp: [PatchTemp::default(); 9],
            rhythm_temp: [RhythmKey::default(); RHYTHM_KEYS_COUNT],
            timbre_temp: [TimbreParam::default(); MELODIC_PARTS_COUNT],
            patches: [PatchParam::default(); 128],
            timbres: [TimbreParam::default(); 256],
            master_volume: 100,
        });

        mem.reset(rom);
        mem
    }

    pub fn reset(&mut self, rom: &Rom) {
        let meta = rom.meta();
        self.raw_patch_temp.fill([0; 16]);
        self.raw_rhythm_temp.fill([0; 4]);
        self.raw_timbre_temp.fill([0; TIMBRE_PARAM_SIZE]);
        self.raw_patches.fill([0; 8]);
        self.raw_timbres.fill([0; 256]);
        self.raw_system.fill(0);
        self.patch_temp.fill(PatchTemp::default());
        self.rhythm_temp.fill(RhythmKey::default());
        self.timbre_temp.fill(TimbreParam::default());
        self.patches.fill(PatchParam::default());
        self.timbres.fill(TimbreParam::default());
        self.master_volume = 100;
        // Init patches: program N maps to timbre group N/64, num N%64.
        for i in 0..128 {
            let raw = &mut self.raw_patches[i];
            raw[0] = (i / 64) as u8; // timbreGroup
            raw[1] = (i % 64) as u8; // timbreNum
            raw[2] = 24; // keyShift (default = no shift)
            raw[3] = 50; // fineTune (default = no detune)
            raw[4] = 12; // benderRange
            raw[5] = 0; // assignMode
            raw[6] = 1; // reverbSwitch
            raw[7] = 0; // dummy
            self.patches[i] = cast_patch(raw);
        }

        // Init system.
        self.raw_system[0] = 0x4A; // masterTune
        self.raw_system[1] = 0; // reverbMode
        self.raw_system[2] = 5; // reverbTime
        self.raw_system[3] = 3; // reverbLevel
        self.raw_system[4..13].copy_from_slice(&meta.reserve_settings);
        self.raw_system[13] = 1; // chanAssign part 1
        self.raw_system[14] = 2;
        self.raw_system[15] = 3;
        self.raw_system[16] = 4;
        self.raw_system[17] = 5;
        self.raw_system[18] = 6;
        self.raw_system[19] = 7;
        self.raw_system[20] = 8;
        self.raw_system[21] = 9; // chanAssign part 9 (rhythm)
        self.raw_system[22] = 100; // masterVol
        self.master_volume = 100;

        // Init rhythm_temp from ROM rhythm_keys.
        for i in 0..RHYTHM_KEYS_COUNT {
            let rk = &meta.rhythm_keys[i];
            let raw = &mut self.raw_rhythm_temp[i];
            raw[0] = rk.timbre as u8;
            raw[1] = rk.level as u8;
            raw[2] = rk.panpot as u8;
            raw[3] = if rk.reverb { 1 } else { 0 };
            self.rhythm_temp[i] = *rk;
        }

        // Init patch_temp for 9 parts.
        for i in 0..9 {
            let raw = &mut self.raw_patch_temp[i];
            // PatchParam defaults.
            raw[0] = 0; // timbreGroup
            raw[1] = 0; // timbreNum
            raw[2] = 24; // keyShift
            raw[3] = 50; // fineTune
            raw[4] = 12; // benderRange
            raw[5] = 0; // assignMode
            raw[6] = 1; // reverbSwitch
            raw[7] = 0; // dummy
            raw[8] = 80; // outputLevel
            raw[9] = meta.default_panpots[i] as u8;
            raw[10] = 0; // dummyv[0]
            raw[11] = 127; // dummyv[1]
            // Rest is 0.
            self.patch_temp[i] = cast_patch_temp(raw);
        }

        // For melodic parts, apply default programs (like setProgram).
        for i in 0..MELODIC_PARTS_COUNT {
            let prog = meta.default_programs[i];
            let patch = &self.patches[prog];
            // Copy patch into patch_temp.
            self.raw_patch_temp[i][0] = patch.timbre_group;
            self.raw_patch_temp[i][1] = patch.timbre_num;
            self.raw_patch_temp[i][2] = patch.key_shift as u8;
            self.raw_patch_temp[i][3] = patch.fine_tune as u8;
            self.raw_patch_temp[i][4] = patch.bender_range;
            self.raw_patch_temp[i][5] = patch.assign_mode;
            self.raw_patch_temp[i][6] = if patch.reverb_switch { 1 } else { 0 };
            self.raw_patch_temp[i][7] = 0;
            self.patch_temp[i] = cast_patch_temp(&self.raw_patch_temp[i]);
        }

        // Init raw_timbres + timbres from ROM.
        rom.fill_raw_timbres(&mut self.raw_timbres);
        for i in 0..rom::MELODIC_TIMBRES_COUNT {
            self.timbres[i] = meta.melodic_timbres[i];
        }
        for i in 0..rom::RHYTHM_TIMBRES_COUNT {
            self.timbres[192 + i] = meta.rhythm_timbres[i];
        }

        // Init timbre_temp + raw_timbre_temp: for each melodic part,
        // copy the timbre referenced by its default program.
        for i in 0..MELODIC_PARTS_COUNT {
            let pt = &self.patch_temp[i];
            let abs = pt.patch.timbre_group as usize * 64 + pt.patch.timbre_num as usize;
            self.timbre_temp[i] = self.timbres[abs];
            self.raw_timbre_temp[i].copy_from_slice(&self.raw_timbres[abs][..TIMBRE_PARAM_SIZE]);
        }
    }

    pub fn set_program(&mut self, part: usize, program: usize) {
        let patch = self.patches[program];
        self.raw_patch_temp[part][0] = patch.timbre_group;
        self.raw_patch_temp[part][1] = patch.timbre_num;
        self.raw_patch_temp[part][2] = patch.key_shift as u8;
        self.raw_patch_temp[part][3] = patch.fine_tune as u8;
        self.raw_patch_temp[part][4] = patch.bender_range;
        self.raw_patch_temp[part][5] = patch.assign_mode;
        self.raw_patch_temp[part][6] = if patch.reverb_switch { 1 } else { 0 };
        self.raw_patch_temp[part][7] = 0;
        self.patch_temp[part] = cast_patch_temp(&self.raw_patch_temp[part]);

        let abs = patch.timbre_group as usize * 64 + patch.timbre_num as usize;
        self.timbre_temp[part] = self.timbres[abs];
        self.raw_timbre_temp[part].copy_from_slice(&self.raw_timbres[abs][..TIMBRE_PARAM_SIZE]);
    }

    pub fn write_sysex(&mut self, mut addr: u32, mut data: &[u8], meta: &Meta) -> SysexEffects {
        let mut effects = SysexEffects::none();
        while !data.is_empty() {
            if addr >= MEM_PATCH_TEMP && addr < MEM_PATCH_TEMP + 16 * 9 {
                let end = MEM_PATCH_TEMP + 16 * 9;
                let n = data.len().min((end - addr) as usize);
                effects.merge(self.write_patch_temp(addr, &data[..n], meta));
                addr += n as u32;
                data = &data[n..];
            } else if addr >= MEM_RHYTHM_TEMP
                && addr < MEM_RHYTHM_TEMP + 4 * RHYTHM_KEYS_COUNT as u32
            {
                let end = MEM_RHYTHM_TEMP + 4 * RHYTHM_KEYS_COUNT as u32;
                let n = data.len().min((end - addr) as usize);
                self.write_rhythm_temp(addr, &data[..n], meta);
                addr += n as u32;
                data = &data[n..];
            } else if addr >= MEM_TIMBRE_TEMP
                && addr < MEM_TIMBRE_TEMP + TIMBRE_PARAM_SIZE as u32 * MELODIC_PARTS_COUNT as u32
            {
                let end = MEM_TIMBRE_TEMP + TIMBRE_PARAM_SIZE as u32 * MELODIC_PARTS_COUNT as u32;
                let n = data.len().min((end - addr) as usize);
                effects.merge(self.write_timbre_temp(addr, &data[..n], meta));
                addr += n as u32;
                data = &data[n..];
            } else if addr >= MEM_PATCHES && addr < MEM_PATCHES + 8 * 128 {
                let end = MEM_PATCHES + 8 * 128;
                let n = data.len().min((end - addr) as usize);
                self.write_patches(addr, &data[..n], meta);
                addr += n as u32;
                data = &data[n..];
            } else if addr >= MEM_TIMBRES && addr < MEM_TIMBRES + 128 * 256 {
                let end = MEM_TIMBRES + 128 * 256;
                let n = data.len().min((end - addr) as usize);
                effects.merge(self.write_timbres(addr, &data[..n], meta));
                addr += n as u32;
                data = &data[n..];
            } else if addr >= MEM_SYSTEM && addr < MEM_SYSTEM + 23 {
                let end = MEM_SYSTEM + 23;
                let n = data.len().min((end - addr) as usize);
                effects.merge(self.write_system(addr, &data[..n], meta));
                addr += n as u32;
                data = &data[n..];
            } else {
                break;
            }
        }
        effects
    }

    fn write_patch_temp(&mut self, addr: u32, data: &[u8], meta: &Meta) -> SysexEffects {
        let base = (addr - MEM_PATCH_TEMP) as usize;
        let first = base / 16;
        let off = base % 16;
        let last = (base + data.len() - 1) / 16;
        let max = &meta.patch_max_table;

        for (i, &byte) in data.iter().enumerate() {
            let pos = off + i;
            let entry = first + pos / 16;
            let slot = pos % 16;
            let clamped = byte.min(max[slot]);
            self.raw_patch_temp[entry][slot] = clamped;
        }

        let mut effects = SysexEffects::none();
        for entry in first..=last.min(8) {
            self.patch_temp[entry] = cast_patch_temp(&self.raw_patch_temp[entry]);

            if entry < MELODIC_PARTS_COUNT {
                let should_reset = if entry == first { off <= 2 } else { true };
                if should_reset {
                    let pt = &self.patch_temp[entry];
                    let abs = pt.patch.timbre_group as usize * 64 + pt.patch.timbre_num as usize;
                    self.timbre_temp[entry] = self.timbres[abs];
                    self.raw_timbre_temp[entry]
                        .copy_from_slice(&self.raw_timbres[abs][..TIMBRE_PARAM_SIZE]);
                    effects.timbre_changed_parts |= 1 << entry;
                }
                effects.refresh_part_volumes = true;
            }
        }
        effects
    }

    fn write_rhythm_temp(&mut self, addr: u32, data: &[u8], meta: &Meta) {
        let base = (addr - MEM_RHYTHM_TEMP) as usize;
        let first = base / 4;
        let off = base % 4;
        let last = (base + data.len() - 1) / 4;
        let max = &meta.rhythm_max_table;

        for (i, &byte) in data.iter().enumerate() {
            let pos = off + i;
            let entry = first + pos / 4;
            let slot = pos % 4;
            let clamped = byte.min(max[slot]);
            self.raw_rhythm_temp[entry][slot] = clamped;
        }

        for entry in first..=last.min(RHYTHM_KEYS_COUNT - 1) {
            self.rhythm_temp[entry] = cast_rhythm_temp(&self.raw_rhythm_temp[entry]);
        }
    }

    fn write_timbre_temp(&mut self, addr: u32, data: &[u8], meta: &Meta) -> SysexEffects {
        let base = (addr - MEM_TIMBRE_TEMP) as usize;
        let first = base / TIMBRE_PARAM_SIZE;
        let off = base % TIMBRE_PARAM_SIZE;
        let last = (base + data.len() - 1) / TIMBRE_PARAM_SIZE;
        let max = &meta.timbre_max_table;

        for (i, &byte) in data.iter().enumerate() {
            let pos = off + i;
            let entry = first + pos / TIMBRE_PARAM_SIZE;
            let slot = pos % TIMBRE_PARAM_SIZE;
            let max_val = max[slot];
            let clamped = byte.min(max_val);
            self.raw_timbre_temp[entry][slot] = clamped;
        }

        let mut effects = SysexEffects::none();
        for entry in first..=last.min(MELODIC_PARTS_COUNT - 1) {
            self.timbre_temp[entry] = cast_timbre(&self.raw_timbre_temp[entry]);
            effects.timbre_changed_parts |= 1 << entry;
        }
        effects
    }

    fn write_patches(&mut self, addr: u32, data: &[u8], meta: &Meta) {
        let base = (addr - MEM_PATCHES) as usize;
        let first = base / 8;
        let off = base % 8;
        let last = (base + data.len() - 1) / 8;
        let max = &meta.patch_max_table;

        for (i, &byte) in data.iter().enumerate() {
            let pos = off + i;
            let entry = first + pos / 8;
            let slot = pos % 8;
            let clamped = byte.min(max[slot]);
            self.raw_patches[entry][slot] = clamped;
        }

        for entry in first..=last.min(127) {
            self.patches[entry] = cast_patch(&self.raw_patches[entry]);
        }
    }

    fn write_timbres(&mut self, addr: u32, data: &[u8], meta: &Meta) -> SysexEffects {
        let base = (addr - MEM_TIMBRES) as usize;
        let first_raw = base / 256;
        let off = base % 256;
        let last_raw = (base + data.len() - 1) / 256;
        let max = &meta.timbre_max_table;

        // Timbres region covers all 256 entries in raw memory,
        // but SysEx address 0x080000 maps to index 128.
        let first = first_raw + 128;
        let last = last_raw + 128;

        for (i, &byte) in data.iter().enumerate() {
            let pos = off + i;
            let entry = first_raw + pos / 256;
            let slot = pos % 256;
            let max_val = if slot < TIMBRE_PARAM_SIZE {
                max[slot]
            } else {
                0
            };
            let clamped = byte.min(max_val);
            self.raw_timbres[entry + 128][slot] = clamped;
        }

        for entry in first..=last.min(255) {
            let raw = &self.raw_timbres[entry];
            let mut raw_timbre = [0u8; TIMBRE_PARAM_SIZE];
            raw_timbre.copy_from_slice(&raw[..TIMBRE_PARAM_SIZE]);
            self.timbres[entry] = cast_timbre(&raw_timbre);
        }

        let mut effects = SysexEffects::none();
        for entry in first..=last.min(255) {
            for part in 0..MELODIC_PARTS_COUNT {
                let pt = &self.patch_temp[part];
                let abs_timbre = pt.patch.timbre_group as usize * 64 + pt.patch.timbre_num as usize;
                if abs_timbre == entry {
                    self.timbre_temp[part] = self.timbres[entry];
                    effects.timbre_changed_parts |= 1 << part;
                }
            }
        }
        effects
    }

    fn write_system(&mut self, addr: u32, data: &[u8], meta: &Meta) -> SysexEffects {
        let off = (addr - MEM_SYSTEM) as usize;
        let max = &meta.system_max_table;

        for (i, &byte) in data.iter().enumerate() {
            let pos = off + i;
            if pos < 23 {
                let clamped = byte.min(max[pos]);
                self.raw_system[pos] = clamped;
            }
        }

        let end = off + data.len();
        let mut effects = SysexEffects::none();

        // Master tune (offset 0).
        if off == 0 {
            effects.refresh_master_tune = true;
        }

        // Reverb parameters (offsets 1-3).
        if off <= 3 && end > 1 {
            effects.refresh_reverb = true;
        }

        // Reserve settings (offsets 4-12).
        if off <= 12 && end > 4 {
            effects.refresh_reserve = true;
        }

        // Channel assignment (offsets 13-21).
        if off <= 21 && end > 13 {
            let first = off.saturating_sub(13);
            let last = (end - 13).min(8);
            effects.chan_assign_first = Some(first);
            effects.chan_assign_last = Some(last);
        }

        // Master volume (offset 22).
        if off <= 22 && end > 22 {
            self.master_volume = self.raw_system[22] as usize;
            effects.refresh_master_volume = true;
        }

        effects
    }
}
