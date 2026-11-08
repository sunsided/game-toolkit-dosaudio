// Copyright (C) 2021-2026 Geoff Hill <geoff@geoffhill.org>
// Copyright (C) 2003-2026 Dean Beeler, Jerome Fisher, Sergey V. Mikayev
//
// This program is free software: you can redistribute it and/or modify it
// under the terms of the GNU Lesser General Public License as published by
// the Free Software Foundation, either version 2.1 of the License, or (at
// your option) any later version. Read COPYING.LESSER.txt for details.

use crate::midi::mt32::cm32l::CM32L;
use crate::midi::mt32::cm32l::element::{Part, PartArena, PartialArena, PolyArena};
use crate::midi::mt32::cm32l::midi::{Message, Msg};
use crate::midi::mt32::cm32l::param::{MELODIC_PARTS_COUNT, TIMBRE_PARAM_SIZE};
use crate::midi::mt32::cm32l::sysex::{MemState, SysexEffects, memaddr};

const CHAN_RHYTHM: u32 = memaddr(0x010000);
const CHAN_TIMBRE: u32 = memaddr(0x020000);
const CHAN_END: u32 = memaddr(0x030000);
const REMAP_PATCH: u32 = memaddr(0x030000);
const REMAP_RHYTHM: u32 = memaddr(0x030110) - memaddr(0x010000);
const REMAP_TIMBRE: u32 = memaddr(0x040000) - memaddr(0x020000);

enum Control {
    Modulation(u8),
    DataEntryMsb(u8),
    Volume(usize),
    Pan(u8),
    Expression(usize),
    HoldPedal(bool),
    Nrpn,
    RpnLsb(u8),
    RpnMsb(u8),
    ResetAllControllers,
    AllNotesOff,
    ChannelMode,
}

impl Control {
    fn from_cc(cc: u8, value: u8) -> Option<Control> {
        match cc {
            1 => Some(Control::Modulation(value)),
            6 => Some(Control::DataEntryMsb(value)),
            7 => Some(Control::Volume((value as usize) * 100 / 127)),
            10 => {
                let pan = (((value as u32) << 3) / 68).min(14) as u8;
                Some(Control::Pan(pan))
            }
            11 => Some(Control::Expression((value as usize) * 100 / 127)),
            64 => Some(Control::HoldPedal(value >= 64)),
            98 | 99 => Some(Control::Nrpn),
            100 => Some(Control::RpnLsb(value)),
            101 => Some(Control::RpnMsb(value)),
            121 => Some(Control::ResetAllControllers),
            123 => Some(Control::AllNotesOff),
            124..=127 => Some(Control::ChannelMode),
            _ => None,
        }
    }
}

fn midi_key_to_key(midi_key: u8, key_shift: i32) -> u8 {
    let mut key = midi_key as i32 + key_shift;
    if key < 36 {
        while key < 36 {
            key += 12;
        }
    } else if key > 132 {
        while key > 132 {
            key -= 12;
        }
    }
    (key - 24) as u8
}

pub fn rebuild_chantable(chantable: &mut [[u8; 9]; 16], raw_system: &[u8; 23]) {
    *chantable = [[0xFF; 9]; 16];
    for part in 0u8..9 {
        let chan = raw_system[13 + part as usize];
        if chan > 15 {
            continue;
        }
        for slot in &mut chantable[chan as usize] {
            if *slot > 8 {
                *slot = part;
                break;
            }
        }
    }
}

impl CM32L {
    pub(super) fn process_midi(&mut self, msg: Message) {
        match msg.0 {
            Msg::NoteOn {
                note,
                velocity,
                channel,
            } => {
                let parts = self.chantable[channel as usize];
                for i in self.aborting_part_ix..9 {
                    let p = parts[i];
                    if p > 8 {
                        break;
                    }
                    if p == 8 {
                        self.rhythm_note_on(note, velocity);
                    } else {
                        self.melodic_note_on(p as usize, note, velocity);
                    }
                    if self.free_partials.is_aborting_poly() {
                        self.aborting_part_ix = i;
                        return;
                    } else if self.aborting_part_ix > 0 {
                        self.aborting_part_ix = 0;
                    }
                }
            }
            Msg::NoteOff { note, channel } => {
                let parts = self.chantable[channel as usize];
                for i in 0..9 {
                    let p = parts[i];
                    if p > 8 {
                        break;
                    }
                    if p == 8 {
                        self.parts.parts[8].note_off_rhythm(
                            note,
                            &mut self.free_polys,
                            &mut self.free_partials,
                            &self.mem,
                        );
                    } else {
                        let idx = p as usize;
                        let key_shift = self.mem.patch_temp[idx].patch.key_shift;
                        let key = midi_key_to_key(note, key_shift);
                        self.parts.parts[idx].note_off(
                            key,
                            &mut self.free_polys,
                            &mut self.free_partials,
                            &self.mem,
                        );
                    }
                }
            }
            Msg::ControlChange { cc, value, channel } => {
                if let Some(control) = Control::from_cc(cc, value) {
                    self.execute_control(control, channel);
                }
            }
            Msg::ProgramChange { program, channel } => {
                let parts = self.chantable[channel as usize];
                for i in 0..9 {
                    let p = parts[i];
                    if p > 8 {
                        break;
                    }
                    if p < 8 {
                        let idx = p as usize;
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
                    }
                }
            }
            Msg::PitchBend { value, channel } => {
                let parts = self.chantable[channel as usize];
                for i in 0..9 {
                    let p = parts[i];
                    if p > 8 {
                        break;
                    }
                    let idx = p as usize;
                    let range = self.mem.patch_temp[idx].patch.bender_range;
                    let r = range as i32 * 683;
                    let bend = ((value as i32 - 8192) * r) >> 14;
                    self.parts.parts[idx].pitch_bend = bend;
                }
            }
            Msg::Sysex(data) => self.process_sysex(&data),
        }
    }

    fn execute_control(&mut self, control: Control, channel: u8) {
        let parts = self.chantable[channel as usize];
        match control {
            Control::Modulation(value) => {
                for i in 0..9 {
                    let p = parts[i];
                    if p > 8 {
                        break;
                    }
                    self.parts.parts[p as usize].modulation = value;
                }
            }
            Control::Volume(vol) => {
                for i in 0..9 {
                    let p = parts[i];
                    if p > 8 {
                        break;
                    }
                    let idx = p as usize;
                    self.mem.raw_patch_temp[idx][8] = vol as u8;
                    self.mem.patch_temp[idx].output_level = vol;
                    self.parts.parts[idx].amp_ctx.part_volume = vol;
                }
            }
            Control::Pan(pan) => {
                for i in 0..9 {
                    let p = parts[i];
                    if p > 8 {
                        break;
                    }
                    let idx = p as usize;
                    self.mem.raw_patch_temp[idx][9] = pan;
                    self.mem.patch_temp[idx].panpot = pan as i32;
                }
            }
            Control::Expression(exp) => {
                for i in 0..9 {
                    let p = parts[i];
                    if p > 8 {
                        break;
                    }
                    self.parts.parts[p as usize].amp_ctx.expression = exp;
                }
            }
            Control::HoldPedal(pressed) => {
                for i in 0..9 {
                    let p = parts[i];
                    if p > 8 {
                        break;
                    }
                    self.parts.parts[p as usize].set_hold_pedal(
                        pressed,
                        &mut self.free_polys,
                        &mut self.free_partials,
                        &self.mem,
                    );
                }
            }
            Control::ResetAllControllers => {
                for i in 0..9 {
                    let p = parts[i];
                    if p > 8 {
                        break;
                    }
                    self.parts.parts[p as usize].reset_all_controllers(
                        &mut self.free_polys,
                        &mut self.free_partials,
                        &self.mem,
                    );
                }
            }
            Control::AllNotesOff => {
                for i in 0..9 {
                    let p = parts[i];
                    if p > 8 {
                        break;
                    }
                    self.parts.parts[p as usize].all_notes_off(
                        &mut self.free_polys,
                        &mut self.free_partials,
                        &self.mem,
                    );
                }
            }
            Control::DataEntryMsb(v) => {
                for i in 0..9 {
                    let p = parts[i];
                    if p > 8 {
                        break;
                    }
                    let idx = p as usize;
                    let part = &mut self.parts.parts[idx];
                    if part.nrpn {
                        continue;
                    }
                    if part.rpn != 0 {
                        continue;
                    }
                    let range = v.min(24);
                    self.mem.patch_temp[idx].patch.bender_range = range;
                    self.mem.raw_patch_temp[idx][4] = range;
                }
            }
            Control::Nrpn => {
                for i in 0..9 {
                    let p = parts[i];
                    if p > 8 {
                        break;
                    }
                    self.parts.parts[p as usize].nrpn = true;
                }
            }
            Control::RpnLsb(v) => {
                for i in 0..9 {
                    let p = parts[i];
                    if p > 8 {
                        break;
                    }
                    let part = &mut self.parts.parts[p as usize];
                    part.nrpn = false;
                    part.rpn = (part.rpn & 0xFF00) | v as u16;
                }
            }
            Control::RpnMsb(v) => {
                for i in 0..9 {
                    let p = parts[i];
                    if p > 8 {
                        break;
                    }
                    let part = &mut self.parts.parts[p as usize];
                    part.nrpn = false;
                    part.rpn = (part.rpn & 0x00FF) | (v as u16) << 8;
                }
            }
            Control::ChannelMode => {
                for i in 0..9 {
                    let p = parts[i];
                    if p > 8 {
                        break;
                    }
                    self.parts.parts[p as usize].set_hold_pedal(
                        false,
                        &mut self.free_polys,
                        &mut self.free_partials,
                        &self.mem,
                    );
                    self.parts.parts[p as usize].all_notes_off(
                        &mut self.free_polys,
                        &mut self.free_partials,
                        &self.mem,
                    );
                }
            }
        }
    }

    fn process_sysex(&mut self, sysex: &[u8]) {
        if sysex.len() < 8 {
            return;
        }
        if sysex[0] != 0xF0 {
            return;
        }
        if sysex[sysex.len() - 1] != 0xF7 {
            return;
        }
        if sysex[1] != 0x41 {
            return;
        }
        let device = sysex[2];
        if device > 0x10 {
            return;
        }
        if sysex[3] != 0x16 {
            return;
        }
        let command = sysex[4];
        if command != 0x12 {
            return;
        }
        let provided_checksum = sysex[sysex.len() - 2];
        let mut sum = 0u8;
        for i in 5..sysex.len() - 2 {
            sum = sum.wrapping_add(sysex[i]);
        }
        let expected_checksum = (128 - (sum & 0x7F)) & 0x7F;
        if provided_checksum != expected_checksum {
            return;
        }
        if sysex[5] == 0x7F {
            self.reset();
            return;
        }
        if sysex.len() < 11 {
            return;
        }
        let addr = ((sysex[5] as u32) << 16) | ((sysex[6] as u32) << 8) | (sysex[7] as u32);
        let data = &sysex[8..sysex.len() - 2];
        let addr = memaddr(addr);
        if device < 0x10 {
            self.process_channel_sysex(device, addr, data);
        } else {
            self.apply_sysex(addr, data);
        }
    }

    fn process_channel_sysex(&mut self, channel: u8, addr: u32, data: &[u8]) {
        let parts = self.chantable[channel as usize];
        if addr < CHAN_RHYTHM {
            let base = addr + REMAP_PATCH;
            if parts[0] > 8 {
                self.apply_sysex(base, data);
                return;
            }
            for i in 0..9 {
                let p = parts[i];
                if p > 8 {
                    break;
                }
                let off = if p == 8 { 0 } else { p as u32 * 16 };
                self.apply_sysex(base + off, data);
            }
        } else if addr < CHAN_TIMBRE {
            let base = addr + REMAP_RHYTHM;
            self.apply_sysex(base, data);
        } else if addr < CHAN_END {
            let base = addr + REMAP_TIMBRE;
            if parts[0] > 8 {
                self.apply_sysex(base, data);
                return;
            }
            for i in 0..9 {
                let p = parts[i];
                if p > 8 {
                    break;
                }
                let off = if p == 8 {
                    0
                } else {
                    p as u32 * TIMBRE_PARAM_SIZE as u32
                };
                self.apply_sysex(base + off, data);
            }
        }
    }

    fn apply_sysex(&mut self, addr: u32, data: &[u8]) {
        let effects = self.mem.write_sysex(addr, data, self.rom.meta());
        self.apply_sysex_effects(effects);
    }

    fn apply_sysex_effects(&mut self, effects: SysexEffects) {
        if effects.refresh_part_volumes {
            for i in 0..MELODIC_PARTS_COUNT {
                self.parts.parts[i].amp_ctx.part_volume = self.mem.patch_temp[i].output_level;
            }
        }
        if effects.refresh_master_tune {
            let mt = self.mem.raw_system[0] as i32;
            self.master_tune_pitch_delta = ((mt - 64) * 171) >> 6;
        }
        if effects.refresh_reverb {
            let sys = &self.mem.raw_system;
            self.reverb.set_mode(sys[1]);
            self.reverb.set_time(sys[2]);
            self.reverb.set_level(sys[3]);
            self.reverb.mute();
        }
        if effects.refresh_reserve {
            self.free_partials.set_reserve(&self.mem.raw_system[4..13]);
        }
        if effects.refresh_master_volume {
            let vol = self.mem.master_volume;
            for part in &mut self.parts.parts {
                part.amp_ctx.master_volume = vol;
            }
        }
        if effects.timbre_changed_parts != 0 {
            for idx in 0..MELODIC_PARTS_COUNT {
                if effects.timbre_changed_parts & (1 << idx) != 0 {
                    let timbre = &self.mem.timbre_temp[idx];
                    self.free_partials.update_tvp_params(idx, timbre);
                }
            }
        }
        if let (Some(first), Some(last)) = (effects.chan_assign_first, effects.chan_assign_last) {
            for i in first..=last {
                if i <= 8 {
                    self.parts.parts[i].all_sound_off(
                        &mut self.free_polys,
                        &mut self.free_partials,
                        &self.mem,
                    );
                    self.parts.parts[i].reset_all_controllers(
                        &mut self.free_polys,
                        &mut self.free_partials,
                        &self.mem,
                    );
                }
            }
            rebuild_chantable(&mut self.chantable, &self.mem.raw_system);
        }
    }

    pub fn reset(&mut self) {
        self.free_polys = PolyArena::new();
        self.free_partials = PartialArena::new(&self.rom.meta().reserve_settings);
        self.mem.reset(&self.rom);
        let mv = self.mem.master_volume;
        self.parts = PartArena::new(mv, &self.rom);
        let sys = &self.mem.raw_system;
        self.reverb.set_mode(sys[1]);
        self.reverb.set_time(sys[2]);
        self.reverb.set_level(sys[3]);
        self.reverb.mute();
        rebuild_chantable(&mut self.chantable, &self.mem.raw_system);
        self.aborting_part_ix = 0;
        // Hardware bug: master tune delta is always zeroed on reset despite
        // the ROM storing 0x4A (74).
        self.master_tune_pitch_delta = 0;
    }

    fn melodic_note_on(&mut self, idx: usize, note: u8, velocity: u8) {
        let pt = &self.mem.patch_temp[idx];
        let key = midi_key_to_key(note, pt.patch.key_shift);

        let timbre = &self.mem.timbre_temp[idx];
        let needed = Part::count_partials_needed_for(timbre);
        if needed == 0 {
            return;
        }

        if pt.patch.assign_mode & 2 == 0 {
            self.parts.parts[idx].abort_first_poly_by_note(
                key,
                &mut self.free_polys,
                &mut self.free_partials,
            );
            if self.free_partials.is_aborting_poly() {
                return;
            }
        }

        if !self
            .free_partials
            .free_partials(needed, idx, &mut self.parts, &mut self.free_polys)
        {
            debug!(idx, note, needed, "melodic note_on failed to free partials");
            return;
        }
        if self.free_partials.is_aborting_poly() {
            debug!(idx, note, "melodic note_on deferred for abort");
            return;
        }

        let pt = &self.mem.patch_temp[idx];
        let reverb = pt.patch.reverb_switch;
        let panpot = pt.panpot;
        let fine_offset = (pt.patch.fine_tune - 50) * 4096 / 1200;
        self.parts.parts[idx].note_on(
            idx,
            key,
            velocity,
            timbre,
            reverb,
            panpot,
            fine_offset,
            &self.rom,
            &mut self.free_polys,
            &mut self.free_partials,
        );
    }

    fn rhythm_note_on(&mut self, note: u8, velocity: u8) {
        let needed =
            Part::count_rhythm_partials_needed(note, &self.mem.rhythm_temp, &self.mem.timbres);
        if needed == 0 {
            return;
        }

        // Special rhythm key remapping (CM-32L only).
        let mut key = note;
        if note >= 24 && note <= 108 {
            let rhythm_idx = (note - 24) as usize;
            if rhythm_idx < self.mem.rhythm_temp.len() {
                let timbre = self.mem.rhythm_temp[rhythm_idx].timbre;
                if timbre == 70 {
                    self.parts.parts[8].note_off_rhythm(
                        0,
                        &mut self.free_polys,
                        &mut self.free_partials,
                        &self.mem,
                    );
                    key = 1;
                } else if timbre == 71 {
                    self.parts.parts[8].note_off_rhythm(
                        0,
                        &mut self.free_polys,
                        &mut self.free_partials,
                        &self.mem,
                    );
                    key = 0;
                }
            }
        }

        let pt = &self.mem.patch_temp[8];
        if pt.patch.assign_mode & 2 == 0 {
            self.parts.parts[8].abort_first_poly_by_note(
                key,
                &mut self.free_polys,
                &mut self.free_partials,
            );
            if self.free_partials.is_aborting_poly() {
                return;
            }
        }

        if !self
            .free_partials
            .free_partials(needed, 8, &mut self.parts, &mut self.free_polys)
        {
            debug!(note, needed, "rhythm note_on failed to free partials");
            return;
        }
        if self.free_partials.is_aborting_poly() {
            debug!(note, "rhythm note_on deferred for abort");
            return;
        }

        self.parts.parts[8].note_on_rhythm(
            key,
            velocity,
            note,
            &self.mem.rhythm_temp,
            &self.mem.timbres,
            &self.rom,
            &mut self.free_polys,
            &mut self.free_partials,
        );
    }
}
