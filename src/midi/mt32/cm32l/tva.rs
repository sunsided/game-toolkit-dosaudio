// Copyright (C) 2021-2026 Geoff Hill <geoff@geoffhill.org>
// Copyright (C) 2003-2026 Dean Beeler, Jerome Fisher, Sergey V. Mikayev
//
// This program is free software: you can redistribute it and/or modify it
// under the terms of the GNU Lesser General Public License as published by
// the Free Software Foundation, either version 2.1 of the License, or (at
// your option) any later version. Read COPYING.LESSER.txt for details.

use crate::midi::mt32::cm32l::element::AmpContext;
use crate::midi::mt32::cm32l::param::TvaParam;
use crate::midi::mt32::cm32l::ramp::Ramp;
use crate::midi::mt32::cm32l::tables;

const BIAS_LEVEL_TO_AMP_SUBTRACTION_COEFF: [i32; 13] =
    [255, 187, 137, 100, 74, 54, 40, 29, 21, 15, 10, 5, 0];

#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum TvaPhase {
    Basic,
    Attack,
    Phase2,
    Phase3,
    Phase4,
    Sustain,
    Release,
    Dead,
}

fn calc_bias_amp_subtraction(bias_point: i32, bias_level: usize, key: u8) -> i32 {
    if let Some(bias) = tables::calc_bias_distance(bias_point, key) {
        (bias * BIAS_LEVEL_TO_AMP_SUBTRACTION_COEFF[bias_level]) >> 5
    } else {
        0
    }
}

fn calc_bias_amp_subtractions(tva_param: &TvaParam, key: u8) -> i32 {
    let x = calc_bias_amp_subtraction(tva_param.bias_point_1, tva_param.bias_level_1, key);
    if x > 255 {
        return 255;
    }

    let y = calc_bias_amp_subtraction(tva_param.bias_point_2, tva_param.bias_level_2, key);
    if y > 255 {
        return 255;
    }

    (x + y).min(255)
}

fn calc_velo_amp_subtraction(velo_sensitivity: i32, velocity: u8) -> i32 {
    let velocity_mult = velo_sensitivity - 50;
    let abs_velocity_mult = velocity_mult.abs();
    let velocity_mult = (velocity_mult * (i32::from(velocity) - 64)) << 2;
    abs_velocity_mult - (velocity_mult >> 8)
}

fn calc_sustain_increment(target_delta: i32) -> u8 {
    let abs_delta = target_delta.unsigned_abs() as usize;
    let descending = target_delta < 0;

    let log_time = tables::ENV_LOG_TIME[abs_delta] as i32;
    let mut inc = (log_time - 2).max(1) as u8;

    if descending {
        inc |= 0x80;
    }
    inc
}

#[derive(Debug)]
pub struct Tva {
    key_time_subtraction: i32,
    velocity: u8,
    no_sustain: bool,
    phase: TvaPhase,
    ramp: Ramp,
    target: u8,
    basic_amp: i32,
    static_amp: i32,
    ring_mod_no_mix: bool,
}

impl Tva {
    pub fn new(
        tva_param: &TvaParam,
        tvf_resonance: i32,
        key: u8,
        velocity: u8,
        rhythm_level: usize,
        no_sustain: bool,
        ring_mod_no_mix: bool,
        amp_ctx: &AmpContext,
    ) -> Self {
        let bias_amp_subtraction = calc_bias_amp_subtractions(tva_param, key);
        let velo_amp_subtraction = calc_velo_amp_subtraction(tva_param.velo_sensitivity, velocity);

        let base_offset = 155 - bias_amp_subtraction - velo_amp_subtraction - (tvf_resonance >> 1);

        let static_amp = if ring_mod_no_mix {
            base_offset
        } else {
            base_offset - tables::LEVEL_TO_AMP_SUBTRACTION[rhythm_level]
        };

        let key_time_subtraction = if tva_param.env_time_keyfollow != 0 {
            ((key as i32) - 60) >> (5 - tva_param.env_time_keyfollow)
        } else {
            0
        };

        let mut tva = Self {
            key_time_subtraction,
            velocity,
            no_sustain,
            phase: TvaPhase::Attack,
            ramp: Default::default(),
            target: 0,
            basic_amp: 0,
            static_amp,
            ring_mod_no_mix,
        };

        let basic_amp = tva.calc_basic_amp(amp_ctx, tva_param);
        tva.basic_amp = basic_amp;

        let env_level_0 = i32::from(tva_param.env_level[0]);
        let (target, phase) = if tva_param.env_time[0] == 0 {
            let t = (basic_amp + env_level_0).clamp(0, 255) as u8;
            (t, TvaPhase::Attack)
        } else {
            (basic_amp.clamp(0, 255) as u8, TvaPhase::Basic)
        };

        tva.phase = phase;
        tva.target = target;
        tva.ramp.start(target, 0x80 | 127);
        tva
    }

    pub fn next_amplitude(&mut self, amp_ctx: &AmpContext, tva: &TvaParam) -> u32 {
        if self.phase == TvaPhase::Dead {
            return 0;
        }

        let amp = self.ramp.next();

        trace!(amp, target = self.target, phase = ?self.phase, "tva amp");

        if self.ramp.at_target() {
            self.advance_phase(amp_ctx, tva);
        }
        amp
    }

    fn calc_basic_amp(&self, amp_ctx: &AmpContext, tva: &TvaParam) -> i32 {
        let amp = self.static_amp - tables::LEVEL_TO_AMP_SUBTRACTION[tva.level];
        if self.ring_mod_no_mix {
            return amp.max(0);
        }
        let amp = amp
            - tables::MASTER_VOL_TO_AMP_SUBTRACTION[amp_ctx.master_volume]
            - tables::LEVEL_TO_AMP_SUBTRACTION[amp_ctx.part_volume]
            - tables::LEVEL_TO_AMP_SUBTRACTION[amp_ctx.expression];
        amp.max(0)
    }

    fn advance_phase(&mut self, amp_ctx: &AmpContext, tva: &TvaParam) {
        let _old_phase = self.phase;

        let basic_amp = self.calc_basic_amp(amp_ctx, tva);
        self.basic_amp = basic_amp;

        let env_level_3 = i32::from(tva.env_level[3]);

        match self.phase {
            TvaPhase::Basic => {
                self.phase = TvaPhase::Attack;
                self.start_ramp_to_target(0, tva);
            }
            TvaPhase::Attack => {
                self.phase = TvaPhase::Phase2;
                self.start_ramp_to_target(1, tva);
            }
            TvaPhase::Phase2 => {
                self.phase = TvaPhase::Phase3;
                self.start_ramp_to_target(2, tva);
            }
            TvaPhase::Phase3 => {
                self.phase = TvaPhase::Phase4;
                self.start_ramp_to_target(3, tva);
            }
            TvaPhase::Phase4 => {
                if env_level_3 == 0 {
                    debug!("TVA -> Dead: env_level_3 is 0");
                    self.phase = TvaPhase::Dead;
                } else if self.no_sustain {
                    self.phase = TvaPhase::Release;
                    let inc = if tva.env_time[4] == 0 {
                        1
                    } else {
                        (-tva.env_time[4]) as u8
                    };
                    self.target = 0;
                    self.ramp.start(0, inc);
                } else {
                    self.phase = TvaPhase::Sustain;
                    let target = (self.basic_amp + env_level_3).clamp(0, 255) as u8;
                    self.target = target;
                    self.ramp.start(target, 0);
                }
            }
            TvaPhase::Sustain => {}
            TvaPhase::Release => {
                self.phase = TvaPhase::Dead;
            }
            TvaPhase::Dead => {}
        }

        if _old_phase != self.phase {
            debug!(
                ?_old_phase,
                new_phase = ?self.phase,
                target = self.target,
                "tva phase"
            );
        }
    }

    fn all_levels_zero(tva: &TvaParam, i: usize) -> bool {
        let lvl = tva.env_level;
        match i {
            0 => lvl[0] == 0 && lvl[1] == 0 && lvl[2] == 0 && lvl[3] == 0,
            1 => lvl[1] == 0 && lvl[2] == 0 && lvl[3] == 0,
            2 => lvl[2] == 0 && lvl[3] == 0,
            3 => lvl[3] == 0,
            _ => false,
        }
    }

    fn start_ramp_to_target(&mut self, i: usize, tva: &TvaParam) {
        let env_level_i = i32::from(tva.env_level[i]);
        let mut target = if Self::all_levels_zero(tva, i) {
            0
        } else {
            (self.basic_amp + env_level_i).clamp(0, 255) as u8
        };

        let adj_time = if i == 0 {
            self.calc_attack_time(tva)
        } else {
            tva.env_time[i] - self.key_time_subtraction
        };

        if adj_time > 0 && target == self.target {
            if target == 0 {
                // Firmware bug: target underflows to -1 (255).
                target = 1;
                let log_time = i32::from(tables::ENV_LOG_TIME[255]);
                let mut inc = (log_time - adj_time).max(1) as u8;
                inc |= 0x80;
                debug!(i, inc, "tva zero-target firmware bug path");
                self.target = target;
                self.ramp.start(target, inc);
                return;
            } else {
                target -= 1;
            }
        }

        let inc = self.calc_increment(target, adj_time);
        self.target = target;
        self.ramp.start(target, inc);
    }

    fn calc_attack_time(&self, tva: &TvaParam) -> i32 {
        let mut time = tva.env_time[0];
        let shift = 6 - tva.env_time_velo_sensitivity;
        let adjustment = (i32::from(self.velocity) - 64) >> shift;
        time -= adjustment;

        if time <= 0 && tva.env_time[0] != 0 {
            1
        } else {
            time.max(0)
        }
    }

    fn calc_increment(&self, target: u8, env_time: i32) -> u8 {
        if env_time <= 0 {
            return if target >= self.target {
                0x80 | 127
            } else {
                127
            };
        }

        let target_delta = i32::from(target) - i32::from(self.target);
        let abs_delta = target_delta.unsigned_abs() as usize;

        let log_time = i32::from(tables::ENV_LOG_TIME[abs_delta]);
        let mut inc = log_time - env_time;
        if inc <= 0 {
            inc = 1;
        }
        let mut inc = inc as u8;

        if target_delta < 0 {
            inc |= 0x80;
        }

        inc
    }

    pub fn is_dead(&self) -> bool {
        self.phase == TvaPhase::Dead
    }

    pub fn recalc_sustain(&mut self, amp_ctx: &AmpContext, tva: &TvaParam) {
        let env_level_3 = i32::from(tva.env_level[3]);
        debug!(phase = ?self.phase, env_level_3, "recalc_sustain called");

        if self.phase != TvaPhase::Sustain || env_level_3 == 0 {
            return;
        }

        let basic_amp = self.calc_basic_amp(amp_ctx, tva);

        let new_target = (basic_amp + env_level_3).clamp(0, 255) as u8;
        let target_delta = (new_target as i32) - (self.target as i32);

        let mut inc = calc_sustain_increment(target_delta);

        let descending = (inc & 0x80) != 0;
        if descending != self.ramp.is_below_current(new_target) {
            inc ^= 0x80;
        }

        debug!(
            new_target,
            target_delta, inc, basic_amp, "recalc_sustain -> Phase4"
        );

        self.phase = TvaPhase::Phase4;
        self.target = new_target;
        self.ramp.start(new_target, inc);
    }

    pub fn start_release(&mut self, tva: &TvaParam) {
        if self.phase >= TvaPhase::Release {
            return;
        }

        debug!("tva release");

        let inc = if tva.env_time[4] == 0 {
            1
        } else {
            (-tva.env_time[4]) as u8
        };
        self.phase = TvaPhase::Release;
        self.target = 0;
        self.ramp.start(0, inc);
    }

    pub fn start_abort(&mut self) {
        self.phase = TvaPhase::Release;
        self.target = 64;
        self.ramp.start(64, 0x80 | 127);
    }
}
