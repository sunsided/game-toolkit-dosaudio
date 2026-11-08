// Copyright (C) 2021-2026 Geoff Hill <geoff@geoffhill.org>
// Copyright (C) 2003-2026 Dean Beeler, Jerome Fisher, Sergey V. Mikayev
//
// This program is free software: you can redistribute it and/or modify it
// under the terms of the GNU Lesser General Public License as published by
// the Free Software Foundation, either version 2.1 of the License, or (at
// your option) any later version. Read COPYING.LESSER.txt for details.

use core::{error, fmt, mem};
use std::{boxed::Box, vec};

use crate::midi::mt32::cm32l::param::*;

pub const CONTROL_SIZE: usize = 0x10000;

pub const PCM_SIZE: usize = 0x100000;

pub(crate) const PCM_SAMPLES: usize = PCM_SIZE / 2;

const PCM_META_OFFSET: usize = 0x8100;
pub(crate) const PCM_META_COUNT: usize = 256;

const TIMBRE_A_MAP_OFFSET: usize = 0x8000;
const TIMBRE_A_COUNT: usize = 64;

const TIMBRE_B_MAP_OFFSET: usize = 0x8080;
const TIMBRE_B_COUNT: usize = 64;

const TIMBRE_ADDR_OFFSET: usize = 0x8000;

const RHYTHM_TIMBRE_MAP_OFFSET: usize = 0x8500;
pub(crate) const RHYTHM_TIMBRES_COUNT: usize = 64;

const RHYTHM_KEYS_OFFSET: usize = 0x8580;

const DEFAULT_PROGRAMS_OFFSET: usize = 0x4F9C;
const DEFAULT_PANPOTS_OFFSET: usize = 0x4FAE;

pub(crate) const MELODIC_TIMBRES_COUNT: usize = TIMBRE_A_COUNT + TIMBRE_B_COUNT;

const RHYTHM_MAX_TABLE_OFFSET: usize = 0x48CB;
const PATCH_MAX_TABLE_OFFSET: usize = 0x48CF;
const SYSTEM_MAX_TABLE_OFFSET: usize = 0x48E8;
const TIMBRE_MAX_TABLE_OFFSET: usize = 0x48FF;
const RESERVE_SETTINGS_OFFSET: usize = 0x4F93;

pub const TIMBRES_COUNT: usize = 256;

pub const PADDED_TIMBRE_SIZE: usize = 256;

pub type RawTimbreBank = [[u8; PADDED_TIMBRE_SIZE]; TIMBRES_COUNT];

pub type ControlArray = [u8; CONTROL_SIZE];

pub type PcmArray = [u8; PCM_SIZE];

#[derive(Debug, Copy, Clone)]
#[repr(C, packed)]
struct RawPcmMeta {
    pos: u8,
    flags: u8,
    pitch_lsb: u8,
    pitch_msb: u8,
}

#[derive(Debug, Default, Copy, Clone, PartialEq)]
pub struct PcmMeta {
    pub addr: usize,
    pub len: usize,
    pub loop_enabled: bool,
    pub unaffected_by_master_tune: bool,
    pub pitch: i32,
}

impl From<&RawPcmMeta> for PcmMeta {
    fn from(pcm: &RawPcmMeta) -> Self {
        let addr = (pcm.pos as usize) * 0x800;
        let len_exp = (pcm.flags & 0x70) >> 4;
        let len = 0x800 << len_exp;
        let loop_enabled = (pcm.flags & 0x80) != 0;
        let unaffected_by_master_tune = (pcm.flags & 0x01) != 0;
        let pitch = ((pcm.pitch_msb as i32) << 8) | (pcm.pitch_lsb as i32);

        PcmMeta {
            addr,
            len,
            loop_enabled,
            unaffected_by_master_tune,
            pitch,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Meta {
    pub pcm_metas: [PcmMeta; PCM_META_COUNT],
    pub rhythm_keys: [RhythmKey; RHYTHM_KEYS_COUNT],
    pub rhythm_timbres: [TimbreParam; RHYTHM_TIMBRES_COUNT],
    pub melodic_timbres: [TimbreParam; MELODIC_TIMBRES_COUNT],
    pub default_programs: [usize; MELODIC_PARTS_COUNT],
    pub default_panpots: [i32; 9],
    pub patch_max_table: [u8; 16],
    pub rhythm_max_table: [u8; 4],
    pub system_max_table: [u8; 23],
    pub timbre_max_table: [u8; PADDED_TIMBRE_SIZE],
    pub reserve_settings: [u8; 9],
}

#[non_exhaustive]
#[derive(Debug)]
pub struct Rom(RomRepr);

#[derive(Debug)]
enum RomRepr {
    #[cfg(feature = "bundle-rom")]
    Static {
        meta: &'static Meta,
        pcm: &'static [i16],
        raw_timbres: &'static RawTimbreBank,
    },
    Owned {
        meta: Box<Meta>,
        pcm: Box<[i16]>,
        control: Box<ControlArray>,
    },
}

impl Rom {
    #[cfg(feature = "bundle-rom")]
    pub(crate) const fn from_static(
        meta: &'static Meta,
        pcm: &'static [i16],
        raw_timbres: &'static RawTimbreBank,
    ) -> Rom {
        Rom(RomRepr::Static {
            meta,
            pcm,
            raw_timbres,
        })
    }

    pub(crate) fn meta(&self) -> &Meta {
        match &self.0 {
            #[cfg(feature = "bundle-rom")]
            RomRepr::Static { meta, .. } => meta,
            RomRepr::Owned { meta, .. } => meta,
        }
    }

    pub fn pcm(&self) -> &[i16] {
        match &self.0 {
            #[cfg(feature = "bundle-rom")]
            RomRepr::Static { pcm, .. } => pcm,
            RomRepr::Owned { pcm, .. } => pcm,
        }
    }

    pub(crate) fn control(&self) -> Option<&ControlArray> {
        match &self.0 {
            #[cfg(feature = "bundle-rom")]
            RomRepr::Static { .. } => None,
            RomRepr::Owned { control, .. } => Some(control),
        }
    }

    pub fn default_programs(&self) -> &[usize; MELODIC_PARTS_COUNT] {
        &self.meta().default_programs
    }

    pub fn default_panpots(&self) -> &[i32; 9] {
        &self.meta().default_panpots
    }

    pub fn reserve_settings(&self) -> &[u8; 9] {
        &self.meta().reserve_settings
    }

    pub fn fill_raw_timbres(&self, raw_timbres: &mut RawTimbreBank) {
        match &self.0 {
            #[cfg(feature = "bundle-rom")]
            RomRepr::Static {
                raw_timbres: src, ..
            } => {
                *raw_timbres = **src;
            }
            RomRepr::Owned { .. } => {
                if let Some(control) = self.control() {
                    Meta::fill_raw_timbres(control, raw_timbres);
                }
            }
        }
    }
}

impl PartialEq for Rom {
    fn eq(&self, other: &Self) -> bool {
        self.meta() == other.meta() && self.pcm() == other.pcm()
    }
}

#[non_exhaustive]
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum Error {
    WrongLength,
    CorruptedData,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Error::WrongLength => "ROM has wrong length",
            Error::CorruptedData => "ROM data corrupted",
        };
        f.write_str(s)
    }
}

impl error::Error for Error {}

impl Meta {
    pub fn init(&mut self, control: &[u8]) -> Result<(), Error> {
        self.parse_pcm_metas(control)?;
        self.parse_rhythm_keys(control)?;
        self.parse_rhythm_timbres(control)?;
        self.parse_melodic_timbres(control)?;
        self.parse_part_defaults(control)?;
        self.parse_max_tables(control)?;
        self.parse_reserve_settings(control)?;
        Ok(())
    }

    fn parse_part_defaults(&mut self, control: &[u8]) -> Result<(), Error> {
        let prog_end = DEFAULT_PROGRAMS_OFFSET + MELODIC_PARTS_COUNT;
        let pan_end = DEFAULT_PANPOTS_OFFSET + 9;

        if prog_end > control.len() || pan_end > control.len() {
            return Err(Error::CorruptedData);
        }

        for i in 0..MELODIC_PARTS_COUNT {
            self.default_programs[i] = control[DEFAULT_PROGRAMS_OFFSET + i] as usize;
        }
        for i in 0..9 {
            self.default_panpots[i] = control[DEFAULT_PANPOTS_OFFSET + i] as i32;
        }

        Ok(())
    }

    fn parse_pcm_metas(&mut self, control: &[u8]) -> Result<(), Error> {
        let start = PCM_META_OFFSET;
        let entry_size = mem::size_of::<RawPcmMeta>();
        let table_end = start + (PCM_META_COUNT * entry_size);

        if table_end > control.len() {
            return Err(Error::CorruptedData);
        }

        for i in 0..PCM_META_COUNT {
            let offset = start + (i * entry_size);
            let slice = &control[offset..offset + entry_size];
            let pcm_entry = &RawPcmMeta {
                pos: slice[0],
                flags: slice[1],
                pitch_lsb: slice[2],
                pitch_msb: slice[3],
            };

            let meta = PcmMeta::from(pcm_entry);
            if meta.addr + meta.len > PCM_SAMPLES {
                return Err(Error::CorruptedData);
            }

            self.pcm_metas[i] = meta;
        }

        Ok(())
    }

    fn parse_rhythm_keys(&mut self, control: &[u8]) -> Result<(), Error> {
        let start = RHYTHM_KEYS_OFFSET;
        let entry_size = mem::size_of::<RawRhythmKey>();
        let keys_end = start + (RHYTHM_KEYS_COUNT * entry_size);

        if keys_end > control.len() {
            return Err(Error::CorruptedData);
        }

        for i in 0..RHYTHM_KEYS_COUNT {
            let offset = start + (i * entry_size);
            let slice = &control[offset..offset + entry_size];
            let raw_rhythm_entry = &RawRhythmKey::decode(slice);

            let rhythm_key = RhythmKey::from(raw_rhythm_entry);
            self.rhythm_keys[i] = rhythm_key;
        }

        Ok(())
    }

    fn parse_rhythm_timbres(&mut self, control: &[u8]) -> Result<(), Error> {
        let map_start = RHYTHM_TIMBRE_MAP_OFFSET;
        let map_size = RHYTHM_TIMBRES_COUNT * 2;

        if map_start + map_size > control.len() {
            return Err(Error::CorruptedData);
        }

        for i in 0..RHYTHM_TIMBRES_COUNT {
            let map_offset = map_start + (i * 2);
            let addr_lo = control[map_offset] as usize;
            let addr_hi = control[map_offset + 1] as usize;
            let timbre_addr = (addr_hi << 8) | addr_lo;

            let timbre = Self::decompress_timbre(control, timbre_addr)?;
            self.rhythm_timbres[i] = timbre;
        }

        Ok(())
    }

    fn parse_melodic_timbres(&mut self, control: &[u8]) -> Result<(), Error> {
        let map_a_size = TIMBRE_A_COUNT * 2;
        if TIMBRE_A_MAP_OFFSET + map_a_size > control.len() {
            return Err(Error::CorruptedData);
        }

        for i in 0..TIMBRE_A_COUNT {
            let map_offset = TIMBRE_A_MAP_OFFSET + (i * 2);
            let addr_lo = control[map_offset] as usize;
            let addr_hi = control[map_offset + 1] as usize;
            let timbre_addr = ((addr_hi << 8) | addr_lo) + TIMBRE_ADDR_OFFSET;

            let timbre = Self::decompress_timbre(control, timbre_addr)?;
            self.melodic_timbres[i] = timbre;
        }

        let map_b_size = TIMBRE_B_COUNT * 2;
        if TIMBRE_B_MAP_OFFSET + map_b_size > control.len() {
            return Err(Error::CorruptedData);
        }

        for i in 0..TIMBRE_B_COUNT {
            let map_offset = TIMBRE_B_MAP_OFFSET + (i * 2);
            let addr_lo = control[map_offset] as usize;
            let addr_hi = control[map_offset + 1] as usize;
            let timbre_addr = ((addr_hi << 8) | addr_lo) + TIMBRE_ADDR_OFFSET;

            let timbre = Self::decompress_timbre(control, timbre_addr)?;
            self.melodic_timbres[TIMBRE_A_COUNT + i] = timbre;
        }

        Ok(())
    }

    fn decompress_timbre(control: &[u8], addr: usize) -> Result<TimbreParam, Error> {
        let common_size = 14;
        let partial_size = mem::size_of::<RawPartialParam>();

        if addr + common_size > control.len() {
            return Err(Error::CorruptedData);
        }

        let common_slice = &control[addr..addr + common_size];
        let idx12 = common_slice[10].min(12) as usize;
        let idx34 = common_slice[11].min(12) as usize;
        let (type0, type1, mode12) = PAIR_STRUCTURES[idx12];
        let (type2, type3, mode34) = PAIR_STRUCTURES[idx34];
        let partial_mute = common_slice[12];
        let no_sustain = common_slice[13] != 0;

        let mut src_pos = addr + common_size;
        let mut partials = [PartialParam::default(); 4];

        for t in 0..4 {
            if t != 0 && ((partial_mute >> t) & 0x1) == 0 {
                src_pos -= partial_size;
            } else if src_pos + partial_size > control.len() {
                return Err(Error::CorruptedData);
            }

            let partial_slice = &control[src_pos..src_pos + partial_size];
            let raw_partial = &RawPartialParam::decode(partial_slice);
            partials[t] = PartialParam::from(raw_partial);
            src_pos += partial_size;
        }

        Ok(TimbreParam {
            partial_types: [type0, type1, type2, type3],
            pair_modes: [mode12, mode34],
            partial_mute,
            no_sustain,
            partials,
        })
    }

    pub fn decompress_timbre_raw(
        control: &[u8],
        addr: usize,
    ) -> Result<[u8; TIMBRE_PARAM_SIZE], Error> {
        if addr + COMMON_PARAM_SIZE > control.len() {
            return Err(Error::CorruptedData);
        }

        let mut raw = [0u8; TIMBRE_PARAM_SIZE];
        raw[0..COMMON_PARAM_SIZE].copy_from_slice(&control[addr..addr + COMMON_PARAM_SIZE]);

        let partial_mute = control[addr + 12];
        let mut src_pos = addr + COMMON_PARAM_SIZE;
        let mut dst_pos = COMMON_PARAM_SIZE;

        for t in 0..4 {
            if t != 0 && ((partial_mute >> t) & 0x1) == 0 {
                src_pos -= PARTIAL_PARAM_SIZE;
            } else if src_pos + PARTIAL_PARAM_SIZE > control.len() {
                return Err(Error::CorruptedData);
            }

            raw[dst_pos..dst_pos + PARTIAL_PARAM_SIZE]
                .copy_from_slice(&control[src_pos..src_pos + PARTIAL_PARAM_SIZE]);
            src_pos += PARTIAL_PARAM_SIZE;
            dst_pos += PARTIAL_PARAM_SIZE;
        }

        Ok(raw)
    }

    pub fn fill_raw_timbres(control: &[u8], raw_timbres: &mut RawTimbreBank) {
        for i in 0..TIMBRE_A_COUNT {
            let map_offset = TIMBRE_A_MAP_OFFSET + (i * 2);
            let addr_lo = control[map_offset] as usize;
            let addr_hi = control[map_offset + 1] as usize;
            let addr = ((addr_hi << 8) | addr_lo) + TIMBRE_ADDR_OFFSET;
            if let Ok(raw) = Self::decompress_timbre_raw(control, addr) {
                raw_timbres[i][..TIMBRE_PARAM_SIZE].copy_from_slice(&raw);
            }
        }
        for i in 0..TIMBRE_B_COUNT {
            let map_offset = TIMBRE_B_MAP_OFFSET + (i * 2);
            let addr_lo = control[map_offset] as usize;
            let addr_hi = control[map_offset + 1] as usize;
            let addr = ((addr_hi << 8) | addr_lo) + TIMBRE_ADDR_OFFSET;
            if let Ok(raw) = Self::decompress_timbre_raw(control, addr) {
                raw_timbres[TIMBRE_A_COUNT + i][..TIMBRE_PARAM_SIZE].copy_from_slice(&raw);
            }
        }
        for i in 0..RHYTHM_TIMBRES_COUNT {
            let map_offset = RHYTHM_TIMBRE_MAP_OFFSET + (i * 2);
            let addr_lo = control[map_offset] as usize;
            let addr_hi = control[map_offset + 1] as usize;
            let addr = (addr_hi << 8) | addr_lo;
            if let Ok(raw) = Self::decompress_timbre_raw(control, addr) {
                raw_timbres[192 + i][..TIMBRE_PARAM_SIZE].copy_from_slice(&raw);
            }
        }
    }

    fn parse_max_tables(&mut self, control: &[u8]) -> Result<(), Error> {
        if PATCH_MAX_TABLE_OFFSET + 16 > control.len()
            || RHYTHM_MAX_TABLE_OFFSET + 4 > control.len()
            || SYSTEM_MAX_TABLE_OFFSET + 23 > control.len()
            || TIMBRE_MAX_TABLE_OFFSET + COMMON_PARAM_SIZE + PARTIAL_PARAM_SIZE > control.len()
        {
            return Err(Error::CorruptedData);
        }

        self.patch_max_table
            .copy_from_slice(&control[PATCH_MAX_TABLE_OFFSET..PATCH_MAX_TABLE_OFFSET + 16]);
        self.rhythm_max_table
            .copy_from_slice(&control[RHYTHM_MAX_TABLE_OFFSET..RHYTHM_MAX_TABLE_OFFSET + 4]);
        self.system_max_table
            .copy_from_slice(&control[SYSTEM_MAX_TABLE_OFFSET..SYSTEM_MAX_TABLE_OFFSET + 23]);

        // Build padded timbre max table: common(14) + partial(58)*4 + padding(10).
        let rom_timbre_max = &control[TIMBRE_MAX_TABLE_OFFSET..];
        self.timbre_max_table = [0u8; PADDED_TIMBRE_SIZE];
        self.timbre_max_table[0..COMMON_PARAM_SIZE]
            .copy_from_slice(&rom_timbre_max[..COMMON_PARAM_SIZE]);
        for i in 0..4 {
            let dst = COMMON_PARAM_SIZE + i * PARTIAL_PARAM_SIZE;
            self.timbre_max_table[dst..dst + PARTIAL_PARAM_SIZE].copy_from_slice(
                &rom_timbre_max[COMMON_PARAM_SIZE..COMMON_PARAM_SIZE + PARTIAL_PARAM_SIZE],
            );
        }

        Ok(())
    }

    fn parse_reserve_settings(&mut self, control: &[u8]) -> Result<(), Error> {
        if RESERVE_SETTINGS_OFFSET + 9 > control.len() {
            return Err(Error::CorruptedData);
        }

        self.reserve_settings
            .copy_from_slice(&control[RESERVE_SETTINGS_OFFSET..RESERVE_SETTINGS_OFFSET + 9]);

        Ok(())
    }
}

pub(crate) fn unscramble_pcm(x: u8, y: u8) -> i16 {
    const ORDER: [u32; 16] = [0, 9, 1, 2, 3, 4, 5, 6, 7, 10, 11, 12, 13, 14, 15, 8];

    let mut sample = 0i16;
    for u in 0..16 {
        let bit = if ORDER[u] < 8 {
            (x >> (7 - ORDER[u])) & 1
        } else {
            (y >> (7 - (ORDER[u] - 8))) & 1
        };
        sample |= (bit as i16) << (15 - u);
    }

    sample
}

impl Rom {
    pub fn new(control_rom: &[u8], pcm_rom: &[u8]) -> Result<Rom, Error> {
        if control_rom.len() != CONTROL_SIZE || pcm_rom.len() != PCM_SIZE {
            return Err(Error::WrongLength);
        }

        let mut pcm: Box<[i16]> = vec![0i16; PCM_SAMPLES].into_boxed_slice();
        let chunks = pcm_rom.chunks_exact(2);
        for (sample, chunk) in pcm.iter_mut().zip(chunks) {
            *sample = unscramble_pcm(chunk[0], chunk[1]);
        }

        let mut meta = Box::new(Meta {
            pcm_metas: [PcmMeta::default(); PCM_META_COUNT],
            rhythm_keys: [RhythmKey::default(); RHYTHM_KEYS_COUNT],
            rhythm_timbres: [TimbreParam::default(); RHYTHM_TIMBRES_COUNT],
            melodic_timbres: [TimbreParam::default(); MELODIC_TIMBRES_COUNT],
            default_programs: [0; MELODIC_PARTS_COUNT],
            default_panpots: [0; 9],
            patch_max_table: [0; 16],
            rhythm_max_table: [0; 4],
            system_max_table: [0; 23],
            timbre_max_table: [0; PADDED_TIMBRE_SIZE],
            reserve_settings: [0; 9],
        });
        meta.init(control_rom)?;

        let mut control = Box::new([0u8; CONTROL_SIZE]);
        control.copy_from_slice(control_rom);

        Ok(Rom(RomRepr::Owned { meta, pcm, control }))
    }
}
