// Copyright (C) 2021-2026 Geoff Hill <geoff@geoffhill.org>
// Copyright (C) 2003-2026 Dean Beeler, Jerome Fisher, Sergey V. Mikayev
//
// This program is free software: you can redistribute it and/or modify it
// under the terms of the GNU Lesser General Public License as published by
// the Free Software Foundation, either version 2.1 of the License, or (at
// your option) any later version. Read COPYING.LESSER.txt for details.

use crate::midi::mt32::cm32l::param::{
    PairMode, PartialParam, PartialType, RhythmKey, TimbreParam,
};
use crate::midi::mt32::cm32l::pcm::PcmWavegen;
use crate::midi::mt32::cm32l::rom::Rom;
use crate::midi::mt32::cm32l::synth::{OscillatorWavegen, WaveformType};
use crate::midi::mt32::cm32l::sysex::MemState;
use crate::midi::mt32::cm32l::tables;
use crate::midi::mt32::cm32l::tva::Tva;
use crate::midi::mt32::cm32l::tvf::Tvf;
use crate::midi::mt32::cm32l::tvp::Tvp;
use core::array;

const PAN_NUMERATOR_MASTER: [i32; 15] = [0, 0, 0, 0, 0, 0, 0, 0, 1, 2, 3, 4, 5, 6, 7];

const PAN_NUMERATOR_SLAVE: [i32; 15] = [0, 1, 2, 3, 4, 5, 6, 7, 7, 7, 7, 7, 7, 7, 7];

fn count_partials_needed(timbre: &TimbreParam) -> usize {
    (timbre.partial_mute & 0xF).count_ones() as usize
}

fn resolve_rhythm_timbre<'a>(
    note: u8,
    rhythm_temp: &'a [RhythmKey],
    timbres: &'a [TimbreParam],
) -> Option<(&'a RhythmKey, &'a TimbreParam, usize)> {
    if note < 24 || note > 108 {
        return None;
    }
    let rhythm_idx = (note - 24) as usize;
    let rhythm = &rhythm_temp[rhythm_idx];
    if rhythm.timbre < 64 {
        return None;
    }
    let abs_timbre = rhythm.timbre - 64 + 192;
    Some((rhythm, &timbres[abs_timbre], abs_timbre))
}

fn abort_first_poly(
    poly_indices: &[Option<usize>],
    state_filter: Option<PolyState>,
    partial_arena: &mut PartialArena,
    poly_arena: &mut PolyArena,
) -> bool {
    // Find the poly with the lowest creation_seq (oldest poly).
    let mut oldest_poly_idx = None;
    let mut oldest_seq = u32::MAX;

    for slot in poly_indices.iter() {
        let Some(poly_idx) = slot else { continue };
        let Some(poly) = &poly_arena.polys[*poly_idx] else {
            continue;
        };
        if state_filter.map_or(true, |s| poly.state == s) {
            if poly.creation_seq < oldest_seq {
                oldest_seq = poly.creation_seq;
                oldest_poly_idx = Some(*poly_idx);
            }
        }
    }

    if let Some(poly_idx) = oldest_poly_idx {
        if let Some(poly) = &mut poly_arena.polys[poly_idx] {
            return poly.start_abort(partial_arena);
        }
    }
    false
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum PolyState {
    Playing,
    Held,
    Releasing,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum PairRole {
    Independent,
    RingMaster { slave_idx: usize, ring_only: bool },
    RingSlave { ring_only: bool },
}

fn compute_pair_role(pair_mode: PairMode, position: usize, pair_idx: Option<usize>) -> PairRole {
    match pair_mode {
        PairMode::Mix | PairMode::Stereo => PairRole::Independent,
        PairMode::RingPlusTop | PairMode::RingOnly => {
            let ring_only = pair_mode == PairMode::RingOnly;
            if position == 0 {
                match pair_idx {
                    Some(slave_idx) => PairRole::RingMaster {
                        slave_idx,
                        ring_only,
                    },
                    None => PairRole::Independent,
                }
            } else {
                PairRole::RingSlave { ring_only }
            }
        }
    }
}

#[derive(Debug, Copy, Clone)]
pub struct PartialConfig {
    pub pair_role: PairRole,
    pub reverb: bool,
    pub rhythm_level: usize,
    pub panpot: i32,
    pub no_sustain: bool,
    pub part_idx: usize,
    pub owner_poly_idx: usize,
    pub timbre_slot: usize,
    pub timbre_lib_idx: Option<usize>,
}

pub fn lookup_partial_param<'a>(config: &PartialConfig, mem: &'a MemState) -> &'a PartialParam {
    let slot = config.timbre_slot;
    match config.timbre_lib_idx {
        None => &mem.timbre_temp[config.part_idx].partials[slot],
        Some(idx) => &mem.timbres[idx].partials[slot],
    }
}

pub fn lookup_timbre<'a>(config: &PartialConfig, mem: &'a MemState) -> &'a TimbreParam {
    match config.timbre_lib_idx {
        None => &mem.timbre_temp[config.part_idx],
        Some(idx) => &mem.timbres[idx],
    }
}

#[derive(Debug, Copy, Clone)]
pub struct AmpContext {
    pub master_volume: usize,
    pub part_volume: usize,
    pub expression: usize,
}

#[derive(Debug)]
pub enum Wavegen {
    Pcm(PcmWavegen),
    Oscillator(OscillatorWavegen),
}

#[derive(Debug)]
pub struct Part {
    pub poly_indices: [Option<usize>; 32],
    pub modulation: u8,
    pub pitch_bend: i32,
    pub program: usize,
    pub hold_pedal: bool,
    pub rpn: u16,
    pub nrpn: bool,
    pub amp_ctx: AmpContext,
}

#[derive(Debug)]
pub struct PartArena {
    pub parts: [Part; 9],
}

#[derive(Debug)]
pub struct Poly {
    pub partial_indices: [Option<usize>; 4],
    pub note: u8,
    pub poly_idx: usize,
    pub state: PolyState,
    pub part_idx: usize,
    pub creation_seq: u32,
    pub can_sustain: bool,
}

#[derive(Debug)]
pub struct PolyArena {
    pub polys: [Option<Poly>; 32],
    next_seq: u32,
}

#[derive(Debug)]
pub struct Partial {
    pub wavegen: Wavegen,
    pub tvp: Tvp,
    pub tvf: Tvf,
    pub tva: Tva,
    pub config: PartialConfig,
}

#[derive(Debug)]
pub struct PartialArena {
    pub partials: [Option<Partial>; 32],
    aborting_poly_idx: Option<usize>,
    reserve: [usize; 9],
    free_stack: [u8; 32],
    free_count: usize,
}

impl Part {
    pub fn new(idx: usize, master_volume: usize, rom: &Rom) -> Part {
        let program = if idx < 8 {
            rom.meta().default_programs[idx]
        } else {
            0
        };
        Part {
            poly_indices: [None; 32],
            modulation: 0,
            pitch_bend: 0,
            program,
            hold_pedal: false,
            rpn: 0xFFFF,
            nrpn: false,
            amp_ctx: AmpContext {
                master_volume,
                part_volume: 80,
                expression: 100,
            },
        }
    }

    pub fn note_on(
        &mut self,
        part_idx: usize,
        note: u8,
        velocity: u8,
        timbre: &TimbreParam,
        reverb: bool,
        panpot: i32,
        patch_fine_offset: i32,
        rom: &Rom,
        poly_arena: &mut PolyArena,
        partial_arena: &mut PartialArena,
    ) {
        let mut slot = None;
        for (i, poly_idx) in self.poly_indices.iter().enumerate() {
            if poly_idx.is_none() {
                slot = Some(i);
                break;
            }
        }

        let slot = match slot {
            Some(s) => s,
            None => return,
        };

        let poly_idx = match poly_arena.alloc() {
            Some(idx) => idx,
            None => return,
        };

        self.poly_indices[slot] = Some(poly_idx);
        let creation_seq = poly_arena.next_creation_seq();

        let poly = Poly::start_with_timbre(
            poly_idx,
            part_idx,
            timbre,
            note,
            velocity,
            rom,
            partial_arena,
            &self.amp_ctx,
            reverb,
            100,
            panpot,
            patch_fine_offset,
            creation_seq,
            None,
        );
        if let Some(poly) = poly {
            poly_arena.polys[poly_idx] = Some(poly);
        }
    }

    pub fn count_partials_needed_for(timbre: &TimbreParam) -> usize {
        count_partials_needed(timbre)
    }

    pub fn note_off(
        &mut self,
        note: u8,
        poly_arena: &mut PolyArena,
        partial_arena: &mut PartialArena,
        mem: &MemState,
    ) {
        for i in 0..self.poly_indices.len() {
            if let Some(poly_idx) = self.poly_indices[i] {
                if let Some(poly) = &mut poly_arena.polys[poly_idx] {
                    if poly.note != note {
                        continue;
                    }
                    if !poly.can_sustain {
                        continue;
                    }
                    if poly.state == PolyState::Releasing {
                        continue;
                    }
                    if self.hold_pedal {
                        poly.state = PolyState::Held;
                    } else {
                        poly.state = PolyState::Releasing;
                        poly.release(partial_arena, mem);
                    }
                    return;
                }
            }
        }
    }

    pub fn note_on_rhythm(
        &mut self,
        key: u8,
        velocity: u8,
        lookup_note: u8,
        rhythm_temp: &[RhythmKey],
        timbres: &[TimbreParam],
        rom: &Rom,
        poly_arena: &mut PolyArena,
        partial_arena: &mut PartialArena,
    ) {
        let mut slot = None;
        for (i, poly_idx) in self.poly_indices.iter().enumerate() {
            if poly_idx.is_none() {
                slot = Some(i);
                break;
            }
        }

        let slot = match slot {
            Some(s) => s,
            None => return,
        };

        let poly_idx = match poly_arena.alloc() {
            Some(idx) => idx,
            None => return,
        };

        self.poly_indices[slot] = Some(poly_idx);
        let creation_seq = poly_arena.next_creation_seq();

        let poly = Poly::start_rhythm(
            poly_idx,
            key,
            velocity,
            lookup_note,
            rhythm_temp,
            timbres,
            rom,
            partial_arena,
            &self.amp_ctx,
            creation_seq,
        );
        if let Some(poly) = poly {
            poly_arena.polys[poly_idx] = Some(poly);
        }
    }

    pub fn count_rhythm_partials_needed(
        note: u8,
        rhythm_temp: &[RhythmKey],
        timbres: &[TimbreParam],
    ) -> usize {
        match resolve_rhythm_timbre(note, rhythm_temp, timbres) {
            Some((_, timbre, _)) => count_partials_needed(timbre),
            None => 0,
        }
    }

    pub fn note_off_rhythm(
        &mut self,
        note: u8,
        poly_arena: &mut PolyArena,
        partial_arena: &mut PartialArena,
        mem: &MemState,
    ) {
        let pedal_held = self.hold_pedal && note != 0;
        for i in 0..self.poly_indices.len() {
            if let Some(poly_idx) = self.poly_indices[i] {
                if let Some(poly) = &mut poly_arena.polys[poly_idx] {
                    if poly.note != note {
                        continue;
                    }
                    if poly.can_sustain || note == 0 {
                        if poly.state == PolyState::Releasing {
                            continue;
                        }
                        if pedal_held {
                            if poly.state != PolyState::Held {
                                poly.state = PolyState::Held;
                            }
                        } else {
                            poly.state = PolyState::Releasing;
                            poly.release(partial_arena, mem);
                        }
                    }
                }
            }
        }
    }

    pub fn set_hold_pedal(
        &mut self,
        pressed: bool,
        poly_arena: &mut PolyArena,
        partial_arena: &mut PartialArena,
        mem: &MemState,
    ) {
        if self.hold_pedal && !pressed {
            self.hold_pedal = false;
            self.stop_pedal_hold(poly_arena, partial_arena, mem);
        } else {
            self.hold_pedal = pressed;
        }
    }

    fn stop_pedal_hold(
        &self,
        poly_arena: &mut PolyArena,
        partial_arena: &mut PartialArena,
        mem: &MemState,
    ) {
        for i in 0..self.poly_indices.len() {
            if let Some(poly_idx) = self.poly_indices[i] {
                if let Some(poly) = &mut poly_arena.polys[poly_idx] {
                    if poly.state == PolyState::Held {
                        poly.state = PolyState::Releasing;
                        poly.release(partial_arena, mem);
                    }
                }
            }
        }
    }

    pub fn all_notes_off(
        &self,
        poly_arena: &mut PolyArena,
        partial_arena: &mut PartialArena,
        mem: &MemState,
    ) {
        for i in 0..self.poly_indices.len() {
            if let Some(poly_idx) = self.poly_indices[i] {
                if let Some(poly) = &mut poly_arena.polys[poly_idx] {
                    if !poly.can_sustain {
                        continue;
                    }
                    if poly.state == PolyState::Releasing {
                        continue;
                    }
                    if self.hold_pedal {
                        poly.state = PolyState::Held;
                    } else {
                        poly.state = PolyState::Releasing;
                        poly.release(partial_arena, mem);
                    }
                }
            }
        }
    }

    pub fn all_sound_off(
        &self,
        poly_arena: &mut PolyArena,
        partial_arena: &mut PartialArena,
        mem: &MemState,
    ) {
        for i in 0..self.poly_indices.len() {
            if let Some(poly_idx) = self.poly_indices[i] {
                if let Some(poly) = &mut poly_arena.polys[poly_idx] {
                    if poly.state == PolyState::Releasing {
                        continue;
                    }
                    poly.state = PolyState::Releasing;
                    poly.release(partial_arena, mem);
                }
            }
        }
    }

    pub fn reset_all_controllers(
        &mut self,
        poly_arena: &mut PolyArena,
        partial_arena: &mut PartialArena,
        mem: &MemState,
    ) {
        self.modulation = 0;
        self.amp_ctx.expression = 100;
        self.pitch_bend = 0;
        self.set_hold_pedal(false, poly_arena, partial_arena, mem);
    }

    pub fn abort_first_poly_by_note(
        &self,
        note: u8,
        poly_arena: &mut PolyArena,
        partial_arena: &mut PartialArena,
    ) -> bool {
        for slot in self.poly_indices.iter() {
            let Some(poly_idx) = slot else { continue };
            let Some(poly) = &mut poly_arena.polys[*poly_idx] else {
                continue;
            };
            if poly.note == note {
                return poly.start_abort(partial_arena);
            }
        }
        false
    }
}

impl Poly {
    fn start_with_timbre(
        poly_idx: usize,
        part_idx: usize,
        timbre: &TimbreParam,
        note: u8,
        velocity: u8,
        rom: &Rom,
        partial_arena: &mut PartialArena,
        amp_ctx: &AmpContext,
        reverb: bool,
        level: usize,
        panpot: i32,
        patch_fine_offset: i32,
        creation_seq: u32,
        timbre_lib_idx: Option<usize>,
    ) -> Option<Poly> {
        let mut partial_count = 0;
        for i in 0..4 {
            if (timbre.partial_mute >> i) & 1 == 1 {
                partial_count += 1;
            }
        }

        if partial_count == 0 {
            return None;
        }

        let indices = partial_arena.alloc_multiple(partial_count)?;

        let mut partial_indices = [None; 4];
        let mut alloc_idx = 0;
        for i in 0..4 {
            if (timbre.partial_mute >> i) & 1 == 1 {
                let idx = indices[alloc_idx];
                partial_indices[i] = Some(idx);
                alloc_idx += 1;
            }
        }

        const PAIR_PARTIAL_NUMBERS: [usize; 4] = [1, 0, 3, 2];
        for (i, idx) in partial_indices.iter().copied().enumerate() {
            let Some(idx) = idx else { continue };

            let pair_num = if i < 2 { 0 } else { 1 };
            let structure_pos = i & 1;
            let pair_mode = timbre.pair_modes[pair_num];
            let partial_type = timbre.partial_types[i];
            let pair_idx = partial_indices[PAIR_PARTIAL_NUMBERS[i]];

            let final_panpot = if pair_mode == PairMode::Stereo {
                let panpot_idx = panpot as usize;
                if structure_pos == 0 {
                    PAN_NUMERATOR_MASTER[panpot_idx] << 1
                } else {
                    PAN_NUMERATOR_SLAVE[panpot_idx] << 1
                }
            } else {
                panpot
            };

            let pair_role = compute_pair_role(pair_mode, structure_pos, pair_idx);

            let config = PartialConfig {
                pair_role,
                reverb,
                rhythm_level: level,
                panpot: final_panpot,
                no_sustain: timbre.no_sustain,
                part_idx,
                owner_poly_idx: poly_idx,
                timbre_slot: i,
                timbre_lib_idx,
            };

            let partial = Partial::start(
                note,
                velocity,
                &timbre.partials[i],
                partial_type,
                rom,
                amp_ctx,
                &config,
                patch_fine_offset,
            );
            if let Some(partial) = partial {
                partial_arena.partials[idx] = Some(partial);
            }
        }

        Some(Poly {
            partial_indices,
            note,
            poly_idx,
            state: PolyState::Playing,
            part_idx,
            creation_seq,
            can_sustain: !timbre.no_sustain,
        })
    }

    pub fn start_rhythm(
        poly_idx: usize,
        key: u8,
        velocity: u8,
        lookup_note: u8,
        rhythm_temp: &[RhythmKey],
        timbres: &[TimbreParam],
        rom: &Rom,
        partial_arena: &mut PartialArena,
        amp_ctx: &AmpContext,
        creation_seq: u32,
    ) -> Option<Poly> {
        let (rhythm, timbre, abs_timbre) =
            resolve_rhythm_timbre(lookup_note, rhythm_temp, timbres)?;
        Self::start_with_timbre(
            poly_idx,
            8,
            timbre,
            key,
            velocity,
            rom,
            partial_arena,
            amp_ctx,
            rhythm.reverb,
            rhythm.level,
            rhythm.panpot,
            0,
            creation_seq,
            Some(abs_timbre),
        )
    }

    pub fn release(&self, partial_arena: &mut PartialArena, mem: &MemState) {
        for i in 0..4 {
            if let Some(partial_idx) = self.partial_indices[i] {
                if let Some(partial) = &mut partial_arena.partials[partial_idx] {
                    if partial.config.owner_poly_idx == self.poly_idx {
                        let pp = lookup_partial_param(&partial.config, mem);
                        partial.start_release(pp);
                    }
                }
            }
        }
    }

    fn start_abort(&mut self, partial_arena: &mut PartialArena) -> bool {
        if partial_arena.is_aborting_poly() {
            return false;
        }
        debug!(
            poly_idx = self.poly_idx,
            note = self.note,
            part_idx = self.part_idx,
            "poly abort"
        );
        for i in 0..4 {
            if let Some(partial_idx) = self.partial_indices[i] {
                if let Some(partial) = &mut partial_arena.partials[partial_idx] {
                    if partial.config.owner_poly_idx == self.poly_idx {
                        partial.start_abort();
                        partial_arena.aborting_poly_idx = Some(self.poly_idx);
                    }
                }
            }
        }
        partial_arena.aborting_poly_idx.is_some()
    }
}

impl Partial {
    pub fn start(
        note: u8,
        velocity: u8,
        partial_param: &PartialParam,
        partial_type: PartialType,
        rom: &Rom,
        amp_ctx: &AmpContext,
        config: &PartialConfig,
        patch_fine_offset: i32,
    ) -> Option<Partial> {
        let is_pcm = partial_type == PartialType::Pcm;
        let (wavegen, pitch, master_tune_enabled) = if is_pcm {
            let pcm = &rom.meta().pcm_metas[partial_param.wg_pcm_wave];
            let wg = Wavegen::Pcm(PcmWavegen::new(pcm.addr, pcm.len, pcm.loop_enabled));
            (wg, pcm.pitch, !pcm.unaffected_by_master_tune)
        } else {
            let waveform = match partial_param.wg_waveform & 1 {
                0 => WaveformType::Square,
                _ => WaveformType::Sawtooth,
            };

            let wg = Wavegen::Oscillator(OscillatorWavegen::new(
                waveform,
                partial_param.wg_pulse_width,
                partial_param.wg_pw_velo_sensitivity,
                velocity,
                partial_param.tvf.resonance + 1,
            ));

            let pitch = match waveform {
                WaveformType::Square => 37133,
                WaveformType::Sawtooth => 33037,
            };
            (wg, pitch, true)
        };

        let base_pitch = tables::calc_base_pitch(
            note,
            partial_param.wg_pitch_keyfollow,
            partial_param.wg_pitch_coarse_offset,
            partial_param.wg_pitch_fine_offset,
            patch_fine_offset,
            pitch,
        );

        let tvp = Tvp::new(
            &partial_param.tvp,
            partial_param.pitch_lfo_rate,
            partial_param.pitch_lfo_depth,
            partial_param.pitch_lfo_mod_sensitivity,
            note,
            velocity,
            base_pitch,
            partial_param.wg_pitch_bender_enabled,
            master_tune_enabled,
        );

        let tvf = Tvf::new(
            &partial_param.tvf,
            note,
            velocity,
            base_pitch,
            partial_param.wg_pitch_keyfollow,
            !config.no_sustain,
        );

        let ring_mod_no_mix = matches!(config.pair_role, PairRole::RingSlave { .. });

        let tva = Tva::new(
            &partial_param.tva,
            partial_param.tvf.resonance,
            note,
            velocity,
            config.rhythm_level,
            config.no_sustain,
            ring_mod_no_mix,
            amp_ctx,
        );

        info!(note, velocity, is_pcm, "partial start");

        Some(Partial {
            wavegen,
            tvp,
            tvf,
            tva,
            config: *config,
        })
    }

    pub fn start_release(&mut self, pp: &PartialParam) {
        self.tva.start_release(&pp.tva);
        self.tvf.start_release(&pp.tvf);
        self.tvp.start_release();
    }

    pub fn update_partial_param(&mut self, pp: &PartialParam) {
        self.tvp.update_partial_param(pp);
    }

    pub fn start_abort(&mut self) {
        self.tva.start_abort();
    }
}

impl PartArena {
    pub fn new(master_volume: usize, rom: &Rom) -> PartArena {
        PartArena {
            parts: array::from_fn(|i| Part::new(i, master_volume, rom)),
        }
    }
}

impl PolyArena {
    pub fn new() -> PolyArena {
        PolyArena {
            polys: array::from_fn(|_| None),
            next_seq: 0,
        }
    }

    pub fn alloc(&mut self) -> Option<usize> {
        for (idx, poly) in self.polys.iter().enumerate() {
            if poly.is_none() {
                info!(idx, "poly allocated");
                return Some(idx);
            }
        }
        info!("poly allocation failed - all 32 slots full");
        None
    }

    pub fn next_creation_seq(&mut self) -> u32 {
        let seq = self.next_seq;
        self.next_seq = self.next_seq.wrapping_add(1);
        seq
    }
}

impl PartialArena {
    pub fn new(reserve: &[u8; 9]) -> PartialArena {
        // Initialize stack so lowest are popped from tail first.
        let free_stack = array::from_fn(|i| (31 - i) as u8);
        PartialArena {
            partials: array::from_fn(|_| None),
            aborting_poly_idx: None,
            reserve: array::from_fn(|i| reserve[i] as usize),
            free_stack,
            free_count: 32,
        }
    }

    pub fn set_reserve(&mut self, reserve: &[u8]) {
        for (i, dst) in self.reserve.iter_mut().enumerate() {
            *dst = reserve.get(i).copied().unwrap_or_default() as usize;
        }
    }

    pub fn free_partial_count(&self) -> usize {
        self.free_count
    }

    fn count_active_for_part(&self, part_idx: usize) -> usize {
        let mut count = 0;
        for p in &self.partials {
            if let Some(partial) = p {
                if partial.config.part_idx == part_idx {
                    count += 1;
                }
            }
        }
        count
    }

    pub fn is_aborting_poly(&self) -> bool {
        self.aborting_poly_idx.is_some()
    }

    pub fn cleanup_finished_polys(&self, poly_arena: &mut PolyArena, parts: &mut PartArena) {
        for poly_idx in 0..32 {
            let Some(poly) = &poly_arena.polys[poly_idx] else {
                continue;
            };
            if self.aborting_poly_idx == Some(poly_idx) {
                continue;
            }
            let mut all_gone = true;
            for idx in &poly.partial_indices {
                if let Some(partial_idx) = idx {
                    if let Some(p) = &self.partials[*partial_idx] {
                        if p.config.owner_poly_idx == poly_idx {
                            all_gone = false;
                            break;
                        }
                    }
                }
            }
            if all_gone {
                let part_idx = poly.part_idx;
                for slot in parts.parts[part_idx].poly_indices.iter_mut() {
                    if *slot == Some(poly_idx) {
                        *slot = None;
                        break;
                    }
                }
                poly_arena.polys[poly_idx] = None;
            }
        }
    }

    pub fn check_aborting_poly_done(&mut self, poly_arena: &mut PolyArena, parts: &mut PartArena) {
        let Some(poly_idx) = self.aborting_poly_idx else {
            return;
        };

        let Some(poly) = &poly_arena.polys[poly_idx] else {
            self.aborting_poly_idx = None;
            return;
        };

        let mut all_gone = true;
        for idx in &poly.partial_indices {
            if let Some(partial_idx) = idx {
                if let Some(p) = &self.partials[*partial_idx] {
                    if p.config.owner_poly_idx == poly_idx {
                        all_gone = false;
                        break;
                    }
                }
            }
        }

        if all_gone {
            debug!(poly_idx, "aborting poly done");

            let part_idx = poly.part_idx;
            for slot in parts.parts[part_idx].poly_indices.iter_mut() {
                if *slot == Some(poly_idx) {
                    *slot = None;
                    break;
                }
            }

            self.aborting_poly_idx = None;
            poly_arena.polys[poly_idx] = None;
        }
    }

    pub fn free_partials(
        &mut self,
        needed: usize,
        part_idx: usize,
        parts: &mut PartArena,
        poly_arena: &mut PolyArena,
    ) -> bool {
        if needed == 0 {
            return true;
        }

        let free = self.free_partial_count();
        if free >= needed {
            return true;
        }

        {
            let mut active_per_part = [0usize; 9];
            for p in &self.partials {
                if let Some(partial) = p {
                    active_per_part[partial.config.part_idx] += 1;
                }
            }
            debug!(
                needed,
                free,
                part_idx,
                reserve = self.reserve[part_idx],
                ?active_per_part,
                "free_partials"
            );
        }

        loop {
            if !self.abort_first_releasing_poly_where_reserve_exceeded(0, parts, poly_arena) {
                break;
            }
            if self.is_aborting_poly() || self.free_partial_count() >= needed {
                return true;
            }
        }

        let active_non_releasing = self.count_active_non_releasing_for_part(part_idx, poly_arena);

        let exceeds_reserve = active_non_releasing + needed > self.reserve[part_idx];
        if exceeds_reserve {
            loop {
                if !self.abort_first_poly_prefer_held_where_reserve_exceeded(
                    part_idx as i32,
                    parts,
                    poly_arena,
                ) {
                    break;
                }
                if self.is_aborting_poly() || self.free_partial_count() >= needed {
                    return true;
                }
            }
            if needed > self.reserve[part_idx] {
                return false;
            }
        } else {
            loop {
                if !self.abort_first_poly_prefer_held_where_reserve_exceeded(-1, parts, poly_arena)
                {
                    break;
                }
                if self.is_aborting_poly() || self.free_partial_count() >= needed {
                    return true;
                }
            }
        }

        let poly_indices = &parts.parts[part_idx].poly_indices[..];
        loop {
            if !abort_first_poly(poly_indices, None, self, poly_arena) {
                break;
            }
            if self.is_aborting_poly() || self.free_partial_count() >= needed {
                return true;
            }
        }

        false
    }

    fn count_active_non_releasing_for_part(
        &self,
        part_idx: usize,
        poly_arena: &PolyArena,
    ) -> usize {
        let mut count = 0;
        for (poly_idx, poly) in poly_arena.polys.iter().enumerate() {
            let Some(poly) = poly else { continue };
            if poly.part_idx == part_idx && poly.state != PolyState::Releasing {
                for partial_idx in poly.partial_indices.iter().flatten() {
                    if let Some(p) = &self.partials[*partial_idx] {
                        if p.config.owner_poly_idx == poly_idx {
                            count += 1;
                        }
                    }
                }
            }
        }
        count
    }

    fn abort_first_releasing_poly_where_reserve_exceeded(
        &mut self,
        min_part: i32,
        parts: &mut PartArena,
        poly_arena: &mut PolyArena,
    ) -> bool {
        let actual_min = if min_part == 8 { -1 } else { min_part };
        for part_num in (actual_min..=7).rev() {
            let use_part = if part_num == -1 { 8 } else { part_num as usize };

            let active = self.count_active_for_part(use_part);
            if active > self.reserve[use_part] {
                debug!(
                    use_part,
                    active,
                    reserve = self.reserve[use_part],
                    "steal releasing poly"
                );
                let poly_indices = &parts.parts[use_part].poly_indices[..];
                let aborted =
                    abort_first_poly(poly_indices, Some(PolyState::Releasing), self, poly_arena);
                if aborted {
                    return true;
                }
            }
        }
        false
    }

    fn abort_first_poly_prefer_held_where_reserve_exceeded(
        &mut self,
        min_part: i32,
        parts: &mut PartArena,
        poly_arena: &mut PolyArena,
    ) -> bool {
        let actual_min = if min_part == 8 { -1 } else { min_part };
        for part_num in (actual_min..=7).rev() {
            let use_part = if part_num == -1 { 8 } else { part_num as usize };

            let active = self.count_active_for_part(use_part);
            if active > self.reserve[use_part] {
                debug!(
                    use_part,
                    active,
                    reserve = self.reserve[use_part],
                    "steal playing poly"
                );
                let poly_indices = &parts.parts[use_part].poly_indices[..];
                if abort_first_poly(poly_indices, Some(PolyState::Held), self, poly_arena) {
                    return true;
                }
                if abort_first_poly(poly_indices, None, self, poly_arena) {
                    return true;
                }
            }
        }
        false
    }

    pub fn deactivate(&mut self, idx: usize) {
        if self.partials[idx].is_none() {
            return;
        }
        self.partials[idx] = None;
        self.free_stack[self.free_count] = idx as u8;
        self.free_count += 1;
    }

    // LIFO allocation: most recently freed slots first.
    pub fn alloc_multiple(&mut self, count: usize) -> Option<[usize; 4]> {
        if self.free_count < count {
            info!(count, free = self.free_count, "partial allocation failed");
            return None;
        }

        let mut indices = [0usize; 4];
        for i in 0..count {
            self.free_count -= 1;
            indices[i] = self.free_stack[self.free_count] as usize;
        }

        info!(count, ?indices, "partials allocated");
        Some(indices)
    }

    pub fn update_tvp_params(&mut self, part_idx: usize, timbre: &TimbreParam) {
        for s in 0..32 {
            if let Some(partial) = &mut self.partials[s] {
                if partial.config.part_idx == part_idx {
                    let ts = partial.config.timbre_slot;
                    partial.update_partial_param(&timbre.partials[ts]);
                }
            }
        }
    }
}
