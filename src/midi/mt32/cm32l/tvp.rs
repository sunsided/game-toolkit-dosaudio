// Copyright (C) 2021-2026 Geoff Hill <geoff@geoffhill.org>
// Copyright (C) 2003-2026 Dean Beeler, Jerome Fisher, Sergey V. Mikayev
//
// This program is free software: you can redistribute it and/or modify it
// under the terms of the GNU Lesser General Public License as published by
// the Free Software Foundation, either version 2.1 of the License, or (at
// your option) any later version. Read COPYING.LESSER.txt for details.

use crate::midi::mt32::cm32l::param::TvpParam;

const LOWER_DURATION_TO_DIVISOR: [u32; 8] =
    [34078, 37162, 40526, 44194, 48194, 52556, 57312, 62499];

const TIMER_COUNTER_RESET: u32 = 9;
const TIMER_INCREMENT_MULTIPLIER: u32 = 250;

#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum TvpPhase {
    Starting = 0,
    Attack = 1,
    Decay = 2,
    Sustain1 = 3,
    Sustain2 = 4,
    ReleaseTrigger = 5,
    Release = 6,
    Done = 7,
}

impl TvpPhase {
    fn next(self) -> Self {
        match self {
            TvpPhase::Starting => TvpPhase::Attack,
            TvpPhase::Attack => TvpPhase::Decay,
            TvpPhase::Decay => TvpPhase::Sustain1,
            TvpPhase::Sustain1 => TvpPhase::Sustain2,
            TvpPhase::Sustain2 => TvpPhase::Release,
            TvpPhase::ReleaseTrigger => TvpPhase::Release,
            TvpPhase::Release => TvpPhase::Done,
            TvpPhase::Done => TvpPhase::Done,
        }
    }

    fn env_index(self) -> i32 {
        match self {
            TvpPhase::Release => 4,
            _ => self as i32,
        }
    }
}

#[derive(Debug)]
struct PitchInterpolation {
    current_offset: i32,
    target_offset: i32,
    change_per_tock: i16,
    target_tock: u16,
    shifts: u32,
}

impl PitchInterpolation {
    fn new(initial_offset: i32) -> Self {
        Self {
            current_offset: initial_offset,
            target_offset: initial_offset,
            change_per_tock: 0,
            target_tock: 0,
            shifts: 0,
        }
    }

    fn setup(&mut self, target_offset: i32, change_duration: u8, current_tock: u16) {
        let negative_delta = target_offset < self.current_offset;
        let mut pitch_offset_delta = target_offset - self.current_offset;
        if pitch_offset_delta > 32767 || pitch_offset_delta < -32768 {
            pitch_offset_delta = 32767;
        }
        if negative_delta {
            pitch_offset_delta = -pitch_offset_delta;
        }

        let mut abs_pitch_offset_delta = ((pitch_offset_delta & 0xFFFF) as u32) << 16;
        let normalization_shifts = normalize(&mut abs_pitch_offset_delta);
        abs_pitch_offset_delta >>= 1;

        let change_duration = change_duration - 1;
        let upper_duration = (change_duration >> 3) as u32;
        self.shifts = normalization_shifts as u32 + upper_duration + 2;

        let divisor = LOWER_DURATION_TO_DIVISOR[(change_duration & 7) as usize];
        let mut change_per_tock = (((abs_pitch_offset_delta & 0xFFFF0000) / divisor) >> 1) as i16;
        if negative_delta {
            change_per_tock = -change_per_tock;
        }
        self.change_per_tock = change_per_tock;

        let current_tock = current_tock as i32;
        let mut duration_in_tocks = (divisor >> (12 - upper_duration)) as i32;
        if duration_in_tocks > 32767 {
            duration_in_tocks = 32767;
        }
        self.target_tock = current_tock.wrapping_add(duration_in_tocks) as u16;
    }

    fn update(&mut self, current_tock: u16, lfo_offset: i32) -> i32 {
        let negative_tocks_remaining = (current_tock as i16).wrapping_sub(self.target_tock as i16);

        if negative_tocks_remaining >= 0 {
            self.current_offset = self.target_offset + lfo_offset;
            return self.current_offset;
        }

        let mut right_shifts = self.shifts as i32;
        let mut neg_ticks = negative_tocks_remaining as i32;
        if right_shifts > 13 {
            right_shifts -= 13;
            neg_ticks >>= right_shifts & 0x1F;
            right_shifts = 13;
        }
        let new_result = (neg_ticks * self.change_per_tock as i32) >> (right_shifts & 0x1F);
        self.current_offset = new_result + self.target_offset + lfo_offset;
        self.current_offset
    }

    fn target_reached(&self, current_tock: u16) -> bool {
        let negative_tocks_remaining = (current_tock as i16).wrapping_sub(self.target_tock as i16);
        negative_tocks_remaining >= 0
    }

    fn target_offset(&self) -> i32 {
        self.target_offset
    }

    fn set_target_offset(&mut self, offset: i32) {
        self.target_offset = offset;
    }

    fn set_target_tock(&mut self, tock: u16) {
        self.target_tock = tock;
    }

    fn is_increasing(&self) -> bool {
        self.change_per_tock > 0
    }

    fn snap_to_target(&mut self, lfo_offset: i32) {
        self.current_offset = self.target_offset + lfo_offset;
    }
}

#[derive(Debug)]
struct LfoConfig {
    rate: i32,
    depth: i32,
    mod_sensitivity: i32,
}

impl LfoConfig {
    fn new(rate: u8, depth: u8, mod_sensitivity: u8) -> Self {
        Self {
            rate: rate as i32,
            depth: depth as i32,
            mod_sensitivity: mod_sensitivity as i32,
        }
    }

    fn calc_offset(&self, modulation: u8, invert: bool) -> i32 {
        let mut offset = ((modulation as i32) * self.mod_sensitivity) >> 7;
        offset = (offset + self.depth) << 1;
        if invert {
            offset = -offset;
        }
        offset
    }

    fn period(&self) -> u8 {
        (101 - self.rate) as u8
    }
}

#[derive(Debug, Default)]
struct ProcessTimer {
    counter: u32,
    elapsed: u32,
    increment: u32,
}

impl ProcessTimer {
    fn tick(&mut self) -> bool {
        let fired = self.counter == 0;
        if fired {
            self.elapsed = (self.elapsed + self.increment) & 0x00FFFFFF;
            self.counter = TIMER_COUNTER_RESET;
            self.increment = (TIMER_INCREMENT_MULTIPLIER * TIMER_COUNTER_RESET) >> 4;
        }
        self.counter -= 1;
        fired
    }

    fn current_tock(&self) -> u16 {
        (self.elapsed >> 8) as u16
    }
}

#[derive(Debug)]
struct EnvelopeConfig {
    time: [i32; 4],
    level_offsets: [i32; 5],
    time_keyfollow_subtraction: i32,
}

impl EnvelopeConfig {
    fn new(tvp_param: &TvpParam, key: u8, velocity: u8) -> Self {
        let velo_mult = calc_velo_mult(tvp_param.velo_sensitivity, velocity);

        let mut level_offsets = [0i32; 5];
        for i in 0..5 {
            let offset = (tvp_param.level[i] * velo_mult) >> (16 - tvp_param.depth);
            level_offsets[i] = offset;
        }

        let time_keyfollow_subtraction = if tvp_param.time_keyfollow != 0 {
            ((key as i32) - 60) >> (5 - tvp_param.time_keyfollow)
        } else {
            0
        };

        Self {
            time: tvp_param.time,
            level_offsets,
            time_keyfollow_subtraction,
        }
    }

    fn update(&mut self, tvp_param: &TvpParam, velocity: u8) {
        self.time = tvp_param.time;
        let velo_mult = calc_velo_mult(tvp_param.velo_sensitivity, velocity);
        for i in 0..5 {
            self.level_offsets[i] = (tvp_param.level[i] * velo_mult) >> (16 - tvp_param.depth);
        }
    }

    fn level_offset(&self, env_idx: i32) -> i32 {
        self.level_offsets[env_idx as usize]
    }

    fn adjusted_time(&self, env_idx: i32) -> i32 {
        self.time[(env_idx - 1) as usize] - self.time_keyfollow_subtraction
    }
}

#[derive(Debug)]
pub struct Tvp {
    base_pitch: u32,
    pitch_bender_enabled: bool,
    master_tune_enabled: bool,
    velocity: u8,
    lfo_pitch_offset: i32,
    phase: TvpPhase,
    envelope: EnvelopeConfig,
    lfo: LfoConfig,
    interpolation: PitchInterpolation,
    timer: ProcessTimer,
    cached_pitch: u32,
}

impl Tvp {
    pub fn new(
        tvp_param: &TvpParam,
        lfo_rate: u8,
        lfo_depth: u8,
        lfo_mod_sensitivity: u8,
        key: u8,
        velocity: u8,
        base_pitch: u32,
        pitch_bender_enabled: bool,
        master_tune_enabled: bool,
    ) -> Self {
        let envelope = EnvelopeConfig::new(tvp_param, key, velocity);
        let initial_offset = envelope.level_offset(0);

        Self {
            base_pitch,
            pitch_bender_enabled,
            master_tune_enabled,
            velocity,
            lfo_pitch_offset: 0,
            phase: TvpPhase::Starting,
            envelope,
            lfo: LfoConfig::new(lfo_rate, lfo_depth, lfo_mod_sensitivity),
            interpolation: PitchInterpolation::new(initial_offset),
            timer: ProcessTimer::default(),
            cached_pitch: base_pitch,
        }
    }

    pub fn next_pitch(&mut self, modulation: u8, pitch_bend: i32, master_tune: i32) -> (u32, bool) {
        let timer_fired = self.timer.tick();
        if timer_fired {
            self.process_envelope(modulation);
            let mut pitch = (self.base_pitch as i32) + self.interpolation.current_offset;
            if self.master_tune_enabled {
                pitch += master_tune;
            }
            if self.pitch_bender_enabled {
                pitch += pitch_bend;
            }
            self.cached_pitch = pitch.clamp(0, 59392) as u32;
        }
        (self.cached_pitch, timer_fired)
    }

    fn process_envelope(&mut self, modulation: u8) {
        if self.phase == TvpPhase::Starting {
            self.target_pitch_offset_reached(modulation);
            return;
        }
        if self.phase == TvpPhase::ReleaseTrigger {
            self.next_phase(modulation);
            return;
        }
        if self.phase >= TvpPhase::Done {
            return;
        }

        let current_tock = self.timer.current_tock();
        if self.interpolation.target_reached(current_tock) {
            self.target_pitch_offset_reached(modulation);
            return;
        }

        self.interpolation
            .update(current_tock, self.lfo_pitch_offset);
    }

    fn target_pitch_offset_reached(&mut self, modulation: u8) {
        self.interpolation.snap_to_target(self.lfo_pitch_offset);

        match self.phase {
            TvpPhase::Sustain1 | TvpPhase::Sustain2 => {
                let invert = self.interpolation.is_increasing();
                self.lfo_pitch_offset = self.lfo.calc_offset(modulation, invert);
                let target = self.interpolation.target_offset() + self.lfo_pitch_offset;
                let time = self.lfo.period();
                let current_tock = self.timer.current_tock();
                self.interpolation.setup(target, time, current_tock);
            }
            TvpPhase::Release => {}
            _ => {
                self.next_phase(modulation);
            }
        }
    }

    fn next_phase(&mut self, modulation: u8) {
        let _old_phase = self.phase;

        self.phase = self.phase.next();
        let env_idx = self.phase.env_index();

        debug!(
            ?_old_phase,
            new_phase = ?self.phase,
            "tvp phase"
        );

        let target_offset = self.envelope.level_offset(env_idx);
        self.interpolation.set_target_offset(target_offset);

        let change_duration = self.envelope.adjusted_time(env_idx);
        if change_duration > 0 {
            let current_tock = self.timer.current_tock();
            self.interpolation
                .setup(target_offset, change_duration as u8, current_tock);
        } else {
            self.target_pitch_offset_reached(modulation);
        }
    }

    pub fn update_partial_param(&mut self, pp: &crate::midi::mt32::cm32l::param::PartialParam) {
        self.pitch_bender_enabled = pp.wg_pitch_bender_enabled;
        self.lfo = LfoConfig::new(
            pp.pitch_lfo_rate,
            pp.pitch_lfo_depth,
            pp.pitch_lfo_mod_sensitivity,
        );
        self.envelope.update(&pp.tvp, self.velocity);
    }

    pub fn start_release(&mut self) {
        if self.phase >= TvpPhase::ReleaseTrigger {
            return;
        }

        debug!("tvp release");

        self.phase = TvpPhase::ReleaseTrigger;
        self.lfo_pitch_offset = 0;

        let current_tock = self.timer.current_tock();
        self.interpolation.set_target_tock(current_tock);
    }
}

fn normalize(val: &mut u32) -> u8 {
    let left_shifts = val.leading_zeros().min(31) as u8;
    *val <<= left_shifts;
    left_shifts
}

fn calc_velo_mult(velo_sensitivity: i32, velocity: u8) -> i32 {
    if velo_sensitivity == 0 {
        return 21845;
    }

    let reversed_velocity = 127 - velocity;
    let scaled_reversed_velocity = (reversed_velocity as u32) << (5 + velo_sensitivity);

    (((32768 - scaled_reversed_velocity) * 21845) >> 15) as i32
}
