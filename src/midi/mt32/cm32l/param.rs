// Copyright (C) 2021-2026 Geoff Hill <geoff@geoffhill.org>
// Copyright (C) 2003-2026 Dean Beeler, Jerome Fisher, Sergey V. Mikayev
//
// This program is free software: you can redistribute it and/or modify it
// under the terms of the GNU Lesser General Public License as published by
// the Free Software Foundation, either version 2.1 of the License, or (at
// your option) any later version. Read COPYING.LESSER.txt for details.

use core::fmt;

pub const TIMBRE_PARAM_SIZE: usize = 246;

pub const COMMON_PARAM_SIZE: usize = 14;

pub const PARTIAL_PARAM_SIZE: usize = 58;

pub const MELODIC_PARTS_COUNT: usize = 8;

pub const RHYTHM_KEYS_COUNT: usize = 85;

#[derive(Default, Copy, Clone, PartialEq, Eq)]
pub enum PartialType {
    #[default]
    Oscillator,
    Pcm,
}

// Match literal format for ROM literal serialization.
impl fmt::Debug for PartialType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PartialType::Oscillator => write!(f, "PartialType::Oscillator"),
            PartialType::Pcm => write!(f, "PartialType::Pcm"),
        }
    }
}

#[derive(Default, Copy, Clone, PartialEq, Eq)]
pub enum PairMode {
    #[default]
    Mix,
    RingPlusTop,
    RingOnly,
    Stereo,
}

// Match literal format for ROM literal serialization.
impl fmt::Debug for PairMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PairMode::Mix => write!(f, "PairMode::Mix"),
            PairMode::RingPlusTop => write!(f, "PairMode::RingPlusTop"),
            PairMode::RingOnly => write!(f, "PairMode::RingOnly"),
            PairMode::Stereo => write!(f, "PairMode::Stereo"),
        }
    }
}

use PairMode::{Mix, RingOnly, RingPlusTop, Stereo};
use PartialType::{Oscillator, Pcm};

pub const PAIR_STRUCTURES: [(PartialType, PartialType, PairMode); 13] = [
    (Oscillator, Oscillator, Mix),         // 0
    (Oscillator, Oscillator, RingPlusTop), // 1
    (Pcm, Oscillator, Mix),                // 2
    (Pcm, Oscillator, RingPlusTop),        // 3
    (Oscillator, Pcm, RingPlusTop),        // 4
    (Pcm, Pcm, Mix),                       // 5
    (Pcm, Pcm, RingPlusTop),               // 6
    (Oscillator, Oscillator, Stereo),      // 7
    (Pcm, Pcm, Stereo),                    // 8
    (Oscillator, Oscillator, RingOnly),    // 9
    (Pcm, Oscillator, RingOnly),           // 10
    (Oscillator, Pcm, RingOnly),           // 11
    (Pcm, Pcm, RingOnly),                  // 12
];

#[derive(Debug, Copy, Clone)]
#[repr(C, packed)]
pub(crate) struct RawTvpParam {
    pub depth: u8,
    pub velo_sensitivity: u8,
    pub time_keyfollow: u8,
    pub time: [u8; 4],
    pub level: [u8; 5],
}

#[derive(Debug, Copy, Clone, Default, PartialEq, Eq)]
pub struct TvpParam {
    pub depth: i32,
    pub velo_sensitivity: i32,
    pub time_keyfollow: i32,
    pub time: [i32; 4],
    pub level: [i32; 5],
}

impl From<&RawTvpParam> for TvpParam {
    fn from(raw: &RawTvpParam) -> Self {
        Self {
            depth: raw.depth.min(10) as i32,
            velo_sensitivity: raw.velo_sensitivity.min(3) as i32,
            time_keyfollow: raw.time_keyfollow.min(4) as i32,
            time: [
                raw.time[0].min(100) as i32,
                raw.time[1].min(100) as i32,
                raw.time[2].min(100) as i32,
                raw.time[3].min(100) as i32,
            ],
            level: [
                raw.level[0].min(100) as i32 - 50,
                raw.level[1].min(100) as i32 - 50,
                raw.level[2].min(100) as i32 - 50,
                raw.level[3].min(100) as i32 - 50,
                raw.level[4].min(100) as i32 - 50,
            ],
        }
    }
}

#[derive(Debug, Copy, Clone)]
#[repr(C, packed)]
pub(crate) struct RawTvfParam {
    pub cutoff: u8,
    pub resonance: u8,
    pub keyfollow: u8,
    pub bias_point: u8,
    pub bias_level: u8,
    pub env_depth: u8,
    pub env_velo_sensitivity: u8,
    pub env_depth_keyfollow: u8,
    pub env_time_keyfollow: u8,
    pub env_time: [u8; 5],
    pub env_level: [u8; 4],
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct TvfParam {
    pub cutoff: i32,
    pub resonance: i32,
    pub keyfollow: usize,
    pub bias_point: i32,
    pub bias_level: usize,
    pub env_depth: i32,
    pub env_velo_sensitivity: i32,
    pub env_depth_keyfollow: i32,
    pub env_time_keyfollow: i32,
    pub env_time: [i32; 5],
    pub env_level: [u8; 4],
}

impl Default for TvfParam {
    fn default() -> Self {
        Self {
            cutoff: 0,
            resonance: 0,
            keyfollow: 0,
            bias_point: 0,
            bias_level: 0,
            env_depth: 0,
            env_velo_sensitivity: 0,
            env_depth_keyfollow: 0,
            env_time_keyfollow: 0,
            env_time: [0; 5],
            env_level: [100; 4],
        }
    }
}

impl From<&RawTvfParam> for TvfParam {
    fn from(raw: &RawTvfParam) -> Self {
        Self {
            cutoff: raw.cutoff.min(100) as i32,
            resonance: raw.resonance.min(30) as i32,
            keyfollow: raw.keyfollow.min(16) as usize,
            bias_point: (raw.bias_point & 0x7F) as i32,
            bias_level: raw.bias_level.min(14) as usize,
            env_depth: raw.env_depth.min(100) as i32,
            env_velo_sensitivity: raw.env_velo_sensitivity.min(100) as i32,
            env_depth_keyfollow: raw.env_depth_keyfollow.min(4) as i32,
            env_time_keyfollow: raw.env_time_keyfollow.min(4) as i32,
            env_time: [
                raw.env_time[0].min(100) as i32,
                raw.env_time[1].min(100) as i32,
                raw.env_time[2].min(100) as i32,
                raw.env_time[3].min(100) as i32,
                raw.env_time[4].min(100) as i32,
            ],
            env_level: [
                raw.env_level[0].min(100),
                raw.env_level[1].min(100),
                raw.env_level[2].min(100),
                raw.env_level[3].min(100),
            ],
        }
    }
}

#[derive(Debug, Copy, Clone)]
#[repr(C, packed)]
pub(crate) struct RawTvaParam {
    pub level: u8,
    pub velo_sensitivity: u8,
    pub bias_point_1: u8,
    pub bias_level_1: u8,
    pub bias_point_2: u8,
    pub bias_level_2: u8,
    pub env_time_keyfollow: u8,
    pub env_time_velo_sensitivity: u8,
    pub env_time: [u8; 5],
    pub env_level: [u8; 4],
}

#[derive(Debug, Copy, Clone, Default, PartialEq, Eq)]
pub struct TvaParam {
    pub level: usize,
    pub velo_sensitivity: i32,
    pub bias_point_1: i32,
    pub bias_level_1: usize,
    pub bias_point_2: i32,
    pub bias_level_2: usize,
    pub env_time_keyfollow: i32,
    pub env_time_velo_sensitivity: i32,
    pub env_time: [i32; 5],
    pub env_level: [u8; 4],
}

impl From<&RawTvaParam> for TvaParam {
    fn from(raw: &RawTvaParam) -> Self {
        Self {
            level: raw.level.min(100) as usize,
            velo_sensitivity: raw.velo_sensitivity.min(100) as i32,
            bias_point_1: (raw.bias_point_1 & 0x7F) as i32,
            bias_level_1: raw.bias_level_1.min(12) as usize,
            bias_point_2: (raw.bias_point_2 & 0x7F) as i32,
            bias_level_2: raw.bias_level_2.min(12) as usize,
            env_time_keyfollow: raw.env_time_keyfollow.min(4) as i32,
            env_time_velo_sensitivity: raw.env_time_velo_sensitivity.min(4) as i32,
            env_time: [
                raw.env_time[0].min(100) as i32,
                raw.env_time[1].min(100) as i32,
                raw.env_time[2].min(100) as i32,
                raw.env_time[3].min(100) as i32,
                raw.env_time[4].min(100) as i32,
            ],
            env_level: [
                raw.env_level[0].min(100),
                raw.env_level[1].min(100),
                raw.env_level[2].min(100),
                raw.env_level[3].min(100),
            ],
        }
    }
}

#[derive(Debug, Copy, Clone)]
#[repr(C, packed)]
pub(crate) struct RawPartialParam {
    pub wg_pitch_coarse: u8,
    pub wg_pitch_fine: u8,
    pub wg_pitch_keyfollow: u8,
    pub wg_pitch_bender_enabled: u8,
    pub wg_waveform: u8,
    pub wg_pcm_wave: u8,
    pub wg_pulse_width: u8,
    pub wg_pw_velo_sensitivity: u8,
    pub tvp: RawTvpParam,
    pub pitch_lfo_rate: u8,
    pub pitch_lfo_depth: u8,
    pub pitch_lfo_mod_sensitivity: u8,
    pub tvf: RawTvfParam,
    pub tva: RawTvaParam,
}

#[derive(Debug, Default, Copy, Clone, PartialEq, Eq)]
pub struct PartialParam {
    pub wg_pitch_coarse_offset: i32,
    pub wg_pitch_fine_offset: i32,
    pub wg_pitch_keyfollow: usize,
    pub wg_pitch_bender_enabled: bool,
    pub wg_waveform: usize,
    pub wg_pcm_wave: usize,
    pub wg_pulse_width: usize,
    pub wg_pw_velo_sensitivity: i32,
    pub tvp: TvpParam,
    pub pitch_lfo_rate: u8,
    pub pitch_lfo_depth: u8,
    pub pitch_lfo_mod_sensitivity: u8,
    pub tvf: TvfParam,
    pub tva: TvaParam,
}

impl From<&RawPartialParam> for PartialParam {
    fn from(raw: &RawPartialParam) -> Self {
        let mut pcm_wave = (raw.wg_pcm_wave & 0x7F) as usize;
        let waveform = raw.wg_waveform.min(2) as usize;
        if waveform > 1 {
            pcm_wave += 128;
        }

        let coarse = raw.wg_pitch_coarse.min(108) as i32;
        let coarse_offset = (coarse - 36) * 4096 / 12;

        let fine = raw.wg_pitch_fine.min(100) as i32;
        let fine_offset = (fine - 50) * 4096 / 1200;

        let keyfollow = raw.wg_pitch_keyfollow.min(16) as usize;
        let pulse_width = raw.wg_pulse_width.min(100) as usize;
        let pw_velo_sensitivity = raw.wg_pw_velo_sensitivity.min(14) as i32;

        Self {
            wg_pitch_coarse_offset: coarse_offset,
            wg_pitch_fine_offset: fine_offset,
            wg_pitch_keyfollow: keyfollow,
            wg_pitch_bender_enabled: (raw.wg_pitch_bender_enabled & 1) != 0,
            wg_waveform: waveform,
            wg_pcm_wave: pcm_wave,
            wg_pulse_width: pulse_width,
            wg_pw_velo_sensitivity: pw_velo_sensitivity,
            tvp: TvpParam::from(&raw.tvp),
            pitch_lfo_rate: raw.pitch_lfo_rate.min(100),
            pitch_lfo_depth: raw.pitch_lfo_depth.min(100),
            pitch_lfo_mod_sensitivity: raw.pitch_lfo_mod_sensitivity.min(100),
            tvf: TvfParam::from(&raw.tvf),
            tva: TvaParam::from(&raw.tva),
        }
    }
}

#[derive(Debug, Copy, Clone)]
#[repr(C, packed)]
pub(crate) struct RawTimbreParam {
    pub name: [u8; 10],
    pub partial_structure_12: u8,
    pub partial_structure_34: u8,
    pub partial_mute: u8,
    pub no_sustain: u8,
    pub partials: [RawPartialParam; 4],
}

#[derive(Debug, Default, Copy, Clone, PartialEq, Eq)]
pub struct TimbreParam {
    pub partial_types: [PartialType; 4],
    pub pair_modes: [PairMode; 2],
    pub partial_mute: u8,
    pub no_sustain: bool,
    pub partials: [PartialParam; 4],
}

impl From<&RawTimbreParam> for TimbreParam {
    fn from(raw: &RawTimbreParam) -> Self {
        let idx12 = raw.partial_structure_12.min(12) as usize;
        let idx34 = raw.partial_structure_34.min(12) as usize;
        let (type0, type1, mode12) = PAIR_STRUCTURES[idx12];
        let (type2, type3, mode34) = PAIR_STRUCTURES[idx34];

        TimbreParam {
            partial_types: [type0, type1, type2, type3],
            pair_modes: [mode12, mode34],
            partial_mute: raw.partial_mute,
            no_sustain: raw.no_sustain != 0,
            partials: [
                PartialParam::from(&raw.partials[0]),
                PartialParam::from(&raw.partials[1]),
                PartialParam::from(&raw.partials[2]),
                PartialParam::from(&raw.partials[3]),
            ],
        }
    }
}

#[derive(Debug, Copy, Clone)]
#[repr(C, packed)]
pub(crate) struct RawRhythmKey {
    pub timbre: u8,
    pub level: u8,
    pub panpot: u8,
    pub reverb: u8,
}

#[derive(Debug, Default, Copy, Clone, PartialEq, Eq)]
pub struct RhythmKey {
    pub timbre: usize,
    pub level: usize,
    pub panpot: i32,
    pub reverb: bool,
}

impl From<&RawRhythmKey> for RhythmKey {
    fn from(raw: &RawRhythmKey) -> Self {
        let timbre = (raw.timbre & 0x7F) as usize;
        let level = raw.level.min(100) as usize;
        let panpot = raw.panpot.min(14) as i32;
        let reverb = (raw.reverb & 1) != 0;

        RhythmKey {
            timbre,
            level,
            panpot,
            reverb,
        }
    }
}

#[derive(Debug, Default, Copy, Clone, PartialEq, Eq)]
pub struct PatchParam {
    pub timbre_group: u8,
    pub timbre_num: u8,
    pub key_shift: i32,
    pub fine_tune: i32,
    pub bender_range: u8,
    pub assign_mode: u8,
    pub reverb_switch: bool,
}

#[derive(Debug, Default, Copy, Clone, PartialEq, Eq)]
pub struct PatchTemp {
    pub patch: PatchParam,
    pub output_level: usize,
    pub panpot: i32,
}

#[derive(Debug, Copy, Clone)]
#[repr(C, packed)]
pub(crate) struct RawPatchParam {
    pub timbre_group: u8,
    pub timbre_num: u8,
    pub key_shift: u8,
    pub fine_tune: u8,
    pub bender_range: u8,
    pub assign_mode: u8,
    pub reverb_switch: u8,
    pub dummy: u8,
}

impl From<&RawPatchParam> for PatchParam {
    fn from(raw: &RawPatchParam) -> Self {
        PatchParam {
            timbre_group: raw.timbre_group.min(3),
            timbre_num: raw.timbre_num.min(63),
            key_shift: raw.key_shift.min(48) as i32,
            fine_tune: raw.fine_tune.min(100) as i32,
            bender_range: raw.bender_range.min(24),
            assign_mode: raw.assign_mode.min(3),
            reverb_switch: raw.reverb_switch != 0,
        }
    }
}

#[derive(Debug, Copy, Clone)]
#[repr(C, packed)]
pub(crate) struct RawPatchTemp {
    pub patch: RawPatchParam,
    pub output_level: u8,
    pub panpot: u8,
    pub dummy: [u8; 6],
}

impl From<&RawPatchTemp> for PatchTemp {
    fn from(raw: &RawPatchTemp) -> Self {
        PatchTemp {
            patch: PatchParam::from(&raw.patch),
            output_level: raw.output_level.min(100) as usize,
            panpot: raw.panpot.min(14) as i32,
        }
    }
}

#[derive(Debug, Copy, Clone)]
#[repr(C, packed)]
pub(crate) struct RawRhythmTemp {
    pub timbre: u8,
    pub level: u8,
    pub panpot: u8,
    pub reverb_switch: u8,
}

impl From<&RawRhythmTemp> for RhythmKey {
    fn from(raw: &RawRhythmTemp) -> Self {
        RhythmKey {
            timbre: (raw.timbre & 0x7F) as usize,
            level: raw.level.min(100) as usize,
            panpot: raw.panpot.min(14) as i32,
            reverb: raw.reverb_switch != 0,
        }
    }
}

pub(crate) fn cast_raw_timbre(raw: &[u8; TIMBRE_PARAM_SIZE]) -> RawTimbreParam {
    RawTimbreParam::decode(raw)
}

impl RawTvpParam {
    pub(super) fn decode(bytes: &[u8]) -> Self {
        Self {
            depth: (&bytes[0..1])[0],
            velo_sensitivity: (&bytes[1..2])[0],
            time_keyfollow: (&bytes[2..3])[0],
            time: [
                (&(&bytes[3..7])[0..1])[0],
                (&(&bytes[3..7])[1..2])[0],
                (&(&bytes[3..7])[2..3])[0],
                (&(&bytes[3..7])[3..4])[0],
            ],
            level: [
                (&(&bytes[7..12])[0..1])[0],
                (&(&bytes[7..12])[1..2])[0],
                (&(&bytes[7..12])[2..3])[0],
                (&(&bytes[7..12])[3..4])[0],
                (&(&bytes[7..12])[4..5])[0],
            ],
        }
    }
}

impl RawTvfParam {
    pub(super) fn decode(bytes: &[u8]) -> Self {
        Self {
            cutoff: (&bytes[0..1])[0],
            resonance: (&bytes[1..2])[0],
            keyfollow: (&bytes[2..3])[0],
            bias_point: (&bytes[3..4])[0],
            bias_level: (&bytes[4..5])[0],
            env_depth: (&bytes[5..6])[0],
            env_velo_sensitivity: (&bytes[6..7])[0],
            env_depth_keyfollow: (&bytes[7..8])[0],
            env_time_keyfollow: (&bytes[8..9])[0],
            env_time: [
                (&(&bytes[9..14])[0..1])[0],
                (&(&bytes[9..14])[1..2])[0],
                (&(&bytes[9..14])[2..3])[0],
                (&(&bytes[9..14])[3..4])[0],
                (&(&bytes[9..14])[4..5])[0],
            ],
            env_level: [
                (&(&bytes[14..18])[0..1])[0],
                (&(&bytes[14..18])[1..2])[0],
                (&(&bytes[14..18])[2..3])[0],
                (&(&bytes[14..18])[3..4])[0],
            ],
        }
    }
}

impl RawTvaParam {
    pub(super) fn decode(bytes: &[u8]) -> Self {
        Self {
            level: (&bytes[0..1])[0],
            velo_sensitivity: (&bytes[1..2])[0],
            bias_point_1: (&bytes[2..3])[0],
            bias_level_1: (&bytes[3..4])[0],
            bias_point_2: (&bytes[4..5])[0],
            bias_level_2: (&bytes[5..6])[0],
            env_time_keyfollow: (&bytes[6..7])[0],
            env_time_velo_sensitivity: (&bytes[7..8])[0],
            env_time: [
                (&(&bytes[8..13])[0..1])[0],
                (&(&bytes[8..13])[1..2])[0],
                (&(&bytes[8..13])[2..3])[0],
                (&(&bytes[8..13])[3..4])[0],
                (&(&bytes[8..13])[4..5])[0],
            ],
            env_level: [
                (&(&bytes[13..17])[0..1])[0],
                (&(&bytes[13..17])[1..2])[0],
                (&(&bytes[13..17])[2..3])[0],
                (&(&bytes[13..17])[3..4])[0],
            ],
        }
    }
}

impl RawPartialParam {
    pub(super) fn decode(bytes: &[u8]) -> Self {
        Self {
            wg_pitch_coarse: (&bytes[0..1])[0],
            wg_pitch_fine: (&bytes[1..2])[0],
            wg_pitch_keyfollow: (&bytes[2..3])[0],
            wg_pitch_bender_enabled: (&bytes[3..4])[0],
            wg_waveform: (&bytes[4..5])[0],
            wg_pcm_wave: (&bytes[5..6])[0],
            wg_pulse_width: (&bytes[6..7])[0],
            wg_pw_velo_sensitivity: (&bytes[7..8])[0],
            tvp: RawTvpParam::decode(&bytes[8..20]),
            pitch_lfo_rate: (&bytes[20..21])[0],
            pitch_lfo_depth: (&bytes[21..22])[0],
            pitch_lfo_mod_sensitivity: (&bytes[22..23])[0],
            tvf: RawTvfParam::decode(&bytes[23..41]),
            tva: RawTvaParam::decode(&bytes[41..58]),
        }
    }
}

impl RawTimbreParam {
    pub(super) fn decode(bytes: &[u8]) -> Self {
        Self {
            name: [
                (&(&bytes[0..10])[0..1])[0],
                (&(&bytes[0..10])[1..2])[0],
                (&(&bytes[0..10])[2..3])[0],
                (&(&bytes[0..10])[3..4])[0],
                (&(&bytes[0..10])[4..5])[0],
                (&(&bytes[0..10])[5..6])[0],
                (&(&bytes[0..10])[6..7])[0],
                (&(&bytes[0..10])[7..8])[0],
                (&(&bytes[0..10])[8..9])[0],
                (&(&bytes[0..10])[9..10])[0],
            ],
            partial_structure_12: (&bytes[10..11])[0],
            partial_structure_34: (&bytes[11..12])[0],
            partial_mute: (&bytes[12..13])[0],
            no_sustain: (&bytes[13..14])[0],
            partials: [
                RawPartialParam::decode(&(&bytes[14..246])[0..58]),
                RawPartialParam::decode(&(&bytes[14..246])[58..116]),
                RawPartialParam::decode(&(&bytes[14..246])[116..174]),
                RawPartialParam::decode(&(&bytes[14..246])[174..232]),
            ],
        }
    }
}

impl RawRhythmKey {
    pub(super) fn decode(bytes: &[u8]) -> Self {
        Self {
            timbre: (&bytes[0..1])[0],
            level: (&bytes[1..2])[0],
            panpot: (&bytes[2..3])[0],
            reverb: (&bytes[3..4])[0],
        }
    }
}

impl RawPatchParam {
    pub(super) fn decode(bytes: &[u8]) -> Self {
        Self {
            timbre_group: (&bytes[0..1])[0],
            timbre_num: (&bytes[1..2])[0],
            key_shift: (&bytes[2..3])[0],
            fine_tune: (&bytes[3..4])[0],
            bender_range: (&bytes[4..5])[0],
            assign_mode: (&bytes[5..6])[0],
            reverb_switch: (&bytes[6..7])[0],
            dummy: (&bytes[7..8])[0],
        }
    }
}

impl RawPatchTemp {
    pub(super) fn decode(bytes: &[u8]) -> Self {
        Self {
            patch: RawPatchParam::decode(&bytes[0..8]),
            output_level: (&bytes[8..9])[0],
            panpot: (&bytes[9..10])[0],
            dummy: [
                (&(&bytes[10..16])[0..1])[0],
                (&(&bytes[10..16])[1..2])[0],
                (&(&bytes[10..16])[2..3])[0],
                (&(&bytes[10..16])[3..4])[0],
                (&(&bytes[10..16])[4..5])[0],
                (&(&bytes[10..16])[5..6])[0],
            ],
        }
    }
}

impl RawRhythmTemp {
    pub(super) fn decode(bytes: &[u8]) -> Self {
        Self {
            timbre: (&bytes[0..1])[0],
            level: (&bytes[1..2])[0],
            panpot: (&bytes[2..3])[0],
            reverb_switch: (&bytes[3..4])[0],
        }
    }
}
