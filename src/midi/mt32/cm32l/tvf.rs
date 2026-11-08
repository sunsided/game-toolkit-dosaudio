// Copyright (C) 2021-2026 Geoff Hill <geoff@geoffhill.org>
// Copyright (C) 2003-2026 Dean Beeler, Jerome Fisher, Sergey V. Mikayev
//
// This program is free software: you can redistribute it and/or modify it
// under the terms of the GNU Lesser General Public License as published by
// the Free Software Foundation, either version 2.1 of the License, or (at
// your option) any later version. Read COPYING.LESSER.txt for details.

use crate::midi::mt32::cm32l::param::TvfParam;
use crate::midi::mt32::cm32l::ramp::Ramp;
use crate::midi::mt32::cm32l::tables;

#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum TvfPhase {
    Attack,
    Phase2,
    Phase3,
    Phase4,
    Sustain,
    Release,
    Done,
}

#[derive(Debug)]
pub struct Tvf {
    phase: TvfPhase,
    ramp: Ramp,
    level_mult: u8,
    key_time_subtraction: i32,
    base_cutoff: u8,
    target: u8,
    can_sustain: bool,
}

impl Tvf {
    pub fn new(
        tvf_param: &TvfParam,
        key: u8,
        velocity: u8,
        base_pitch: u32,
        pitch_keyfollow: usize,
        can_sustain: bool,
    ) -> Self {
        let base_cutoff = calc_base_cutoff(tvf_param, key, base_pitch, pitch_keyfollow);

        let mut level_mult = (velocity as i32 * tvf_param.env_velo_sensitivity) >> 6;
        level_mult += 109 - tvf_param.env_velo_sensitivity;
        level_mult += ((key as i32) - 60) >> (4 - tvf_param.env_depth_keyfollow);
        if level_mult < 0 {
            level_mult = 0;
        }
        level_mult = (level_mult * tvf_param.env_depth) >> 6;
        if level_mult > 255 {
            level_mult = 255;
        }

        let key_time_subtraction = if tvf_param.env_time_keyfollow != 0 {
            ((key as i32) - 60) >> (5 - tvf_param.env_time_keyfollow)
        } else {
            0
        };

        let target = ((level_mult as u32 * tvf_param.env_level[0] as u32) >> 8) as u8;
        let env_time = tvf_param.env_time[0] - key_time_subtraction;
        let inc = if env_time <= 0 {
            0xFF
        } else {
            let log_time = tables::ENV_LOG_TIME[target as usize] as i32;
            let mut inc = log_time - env_time;
            if inc <= 0 {
                inc = 1;
            }
            inc as u8
        };

        let mut tvf = Self {
            phase: TvfPhase::Attack,
            ramp: Default::default(),
            level_mult: level_mult as u8,
            key_time_subtraction,
            base_cutoff,
            target,
            can_sustain,
        };

        tvf.ramp.start(target, inc);
        tvf
    }

    pub fn next_cutoff(&mut self, tvf: &TvfParam) -> u32 {
        if self.phase == TvfPhase::Done {
            return (self.base_cutoff as u32) << 18;
        }

        self.ramp.next();
        if self.ramp.at_target() {
            self.advance_phase(tvf);
        }
        let cutoff_modifier_val = self.ramp.current_value();
        ((self.base_cutoff as u32) << 18) + cutoff_modifier_val
    }

    fn advance_phase(&mut self, tvf: &TvfParam) {
        let _old_phase = self.phase;

        match self.phase {
            TvfPhase::Attack => {
                self.phase = TvfPhase::Phase2;
                self.start_phase(1, tvf);
            }
            TvfPhase::Phase2 => {
                self.phase = TvfPhase::Phase3;
                self.start_phase(2, tvf);
            }
            TvfPhase::Phase3 => {
                self.phase = TvfPhase::Phase4;
                self.start_phase(3, tvf);
            }
            TvfPhase::Phase4 => {
                self.phase = TvfPhase::Sustain;
                if !self.can_sustain {
                    self.start_release(tvf);
                    return;
                }
                let target = ((self.level_mult as u32 * tvf.env_level[3] as u32) >> 8) as u8;
                self.target = target;
                self.ramp.start(target, 0);
            }
            TvfPhase::Sustain => {
                self.phase = TvfPhase::Release;
                let env_time = tvf.env_time[4];
                let inc = if env_time == 0 {
                    1
                } else {
                    0x80 | env_time as u8
                };
                self.target = 0;
                self.ramp.start(0, inc);
            }
            TvfPhase::Release => {
                self.phase = TvfPhase::Done;
                self.target = 0;
                self.ramp.start(0, 0);
            }
            TvfPhase::Done => {}
        }

        if _old_phase != self.phase {
            debug!(
                ?_old_phase,
                new_phase = ?self.phase,
                target = self.target,
                "tvf phase"
            );
        }
    }

    fn start_phase(&mut self, phase_idx: usize, tvf: &TvfParam) {
        let mut new_target =
            ((self.level_mult as u32 * tvf.env_level[phase_idx] as u32) >> 8) as u8;
        let mut target_delta = (new_target as i16) - (self.target as i16);

        let env_time = tvf.env_time[phase_idx] - self.key_time_subtraction;

        let inc = if env_time > 0 {
            if target_delta == 0 {
                if new_target == 0 {
                    target_delta = 1;
                    new_target = 1;
                } else {
                    target_delta = -1;
                    new_target -= 1;
                }
            }

            let delta = target_delta.unsigned_abs() as usize;
            let log_time = tables::ENV_LOG_TIME[delta] as i32;
            let mut inc = log_time - env_time;
            if inc <= 0 {
                inc = 1;
            }
            let mut inc = inc as u8;
            if target_delta < 0 {
                inc |= 0x80;
            }
            inc
        } else if new_target >= self.target {
            0xFF
        } else {
            0x7F
        };

        self.target = new_target;
        self.ramp.start(new_target, inc);
    }

    pub fn start_release(&mut self, tvf: &TvfParam) {
        if self.phase >= TvfPhase::Release {
            return;
        }

        debug!("tvf release");

        self.phase = TvfPhase::Release;
        let inc = if tvf.env_time[4] == 0 {
            1
        } else {
            (-tvf.env_time[4]) as u8
        };
        self.ramp.start(0, inc);
    }
}

fn calc_base_cutoff(tvf_param: &TvfParam, key: u8, base_pitch: u32, pitch_keyfollow: usize) -> u8 {
    const BIAS_LEVEL_TO_BIAS_MULT: [i8; 15] =
        [85, 42, 21, 16, 10, 5, 2, 0, -2, -5, -10, -16, -21, -74, -85];
    const KEYFOLLOW_MULT_21: [i8; 17] = [
        -21, -10, -5, 0, 2, 5, 8, 10, 13, 16, 18, 21, 26, 32, 42, 21, 21,
    ];

    let tvf_keyfollow = KEYFOLLOW_MULT_21[tvf_param.keyfollow] as i32;
    let pitch_kf = KEYFOLLOW_MULT_21[pitch_keyfollow] as i32;
    let mut base_cutoff = (tvf_keyfollow - pitch_kf) * ((key as i32) - 60);

    if let Some(bias) = tables::calc_bias_distance(tvf_param.bias_point, key) {
        let mult = BIAS_LEVEL_TO_BIAS_MULT[tvf_param.bias_level] as i32;
        base_cutoff += -bias * mult;
    }

    base_cutoff += (tvf_param.cutoff << 4) - 800;

    if base_cutoff >= 0 {
        let pitch_delta = ((base_pitch >> 4) as i32) + base_cutoff - 3584;
        if pitch_delta > 0 {
            base_cutoff -= pitch_delta;
        }
    } else if base_cutoff < -2048 {
        base_cutoff = -2048;
    }

    base_cutoff += 2056;
    base_cutoff >>= 4;

    base_cutoff = base_cutoff.clamp(0, 255);

    base_cutoff as u8
}
