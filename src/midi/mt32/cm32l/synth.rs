// Copyright (C) 2021-2026 Geoff Hill <geoff@geoffhill.org>
// Copyright (C) 2003-2026 Dean Beeler, Jerome Fisher, Sergey V. Mikayev
//
// This program is free software: you can redistribute it and/or modify it
// under the terms of the GNU Lesser General Public License as published by
// the Free Software Foundation, either version 2.1 of the License, or (at
// your option) any later version. Read COPYING.LESSER.txt for details.

use crate::midi::mt32::cm32l::tables;
use crate::midi::mt32::cm32l::tables::exp9_interpolate;

const SINE_SEGMENT_RELATIVE_LENGTH: u32 = 1 << 18;
const MIDDLE_CUTOFF_VALUE: u32 = 128 << 18;
const RESONANCE_DECAY_THRESHOLD_CUTOFF_VALUE: u32 = 144 << 18;
const MAX_CUTOFF_VALUE: u32 = 240 << 18;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Sign {
    Positive,
    Negative,
}

#[derive(Copy, Clone, Debug)]
pub struct LogSample {
    pub log_value: u16,
    pub sign: Sign,
}

const SILENCE: LogSample = LogSample {
    log_value: 65535,
    sign: Sign::Positive,
};

impl LogSample {
    fn add(&mut self, other: LogSample) {
        let log_value = (self.log_value as u32) + (other.log_value as u32);
        self.log_value = if log_value < 65536 {
            log_value as u16
        } else {
            65535
        };
        self.sign = match (self.sign, other.sign) {
            (Sign::Positive, Sign::Positive) | (Sign::Negative, Sign::Negative) => Sign::Positive,
            _ => Sign::Negative,
        };
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum WaveformType {
    Square,
    Sawtooth,
}

pub fn unlog(log_sample: LogSample) -> i16 {
    let int_log = (log_sample.log_value >> 12) as u32;
    let frac_log = (log_sample.log_value & 4095) as u32;
    let exp_val = exp9_interpolate(frac_log);
    let sample = (exp_val >> int_log) as i16;

    match log_sample.sign {
        Sign::Positive => sample,
        Sign::Negative => -sample,
    }
}

pub fn pcm_to_log(pcm_sample: i16, amp: u32) -> LogSample {
    let abs_sample = (pcm_sample & 0x7FFF) as u32;
    let mut log_value = (32787 - abs_sample) << 1;
    log_value += amp >> 10;

    let final_log = if log_value < 65536 {
        log_value as u16
    } else {
        65535
    };

    LogSample {
        log_value: final_log,
        sign: if pcm_sample < 0 {
            Sign::Negative
        } else {
            Sign::Positive
        },
    }
}

fn logsin9(idx: u32) -> u16 {
    tables::LOGSIN9_TABLE[idx as usize & 511]
}

#[derive(Copy, Clone, Debug)]
enum Phase {
    PositiveRising,
    PositiveLinear,
    PositiveFalling,
    NegativeFalling,
    NegativeLinear,
    NegativeRising,
}

#[derive(Copy, Clone, Debug)]
enum ResonancePhase {
    PositiveRising,
    PositiveFalling,
    NegativeFalling,
    NegativeRising,
}

#[derive(Debug)]
pub struct OscillatorWavegen {
    waveform: WaveformType,
    pulse_width: u8,

    wave_position: u32,
    square_wave_position: u32,
    phase: Phase,

    resonance_sine_position: u32,
    resonance_phase: ResonancePhase,
    resonance_amp_subtraction: u32,
    res_amp_decay_factor: u32,

    square_log_sample: LogSample,
    resonance_log_sample: LogSample,
}

impl OscillatorWavegen {
    pub fn new(
        waveform: WaveformType,
        pulse: usize,
        pw_velo_sensitivity: i32,
        velocity: u8,
        resonance: i32,
    ) -> Self {
        let base_pulse = tables::PULSE_WIDTH_100_TO_255[pulse];
        let velo_adjustment = (velocity as i32 - 64) * (pw_velo_sensitivity - 7);
        let pulse_width_val = (base_pulse as i32 + velo_adjustment).clamp(0, 255) as u8;

        let resonance_amp_subtraction = (32 - resonance as u32) << 10;
        let res_amp_decay_factor =
            (tables::RES_AMP_DECAY_FACTORS[(resonance >> 2) as usize] as u32) << 2;

        Self {
            waveform,
            pulse_width: pulse_width_val,
            wave_position: 0,
            square_wave_position: 0,
            phase: Phase::PositiveRising,
            resonance_sine_position: 0,
            resonance_phase: ResonancePhase::PositiveRising,
            resonance_amp_subtraction,
            res_amp_decay_factor,
            square_log_sample: SILENCE,
            resonance_log_sample: SILENCE,
        }
    }

    fn get_sample_step(&self, pitch: u32) -> u32 {
        let step = exp9_interpolate(!pitch & 4095);
        let step = step << (pitch >> 12);
        let step = step >> 8;
        step & !1
    }

    fn get_resonance_wave_length_factor(&self, eff_cutoff: u32) -> u32 {
        let factor = exp9_interpolate(!eff_cutoff & 4095);
        factor << (eff_cutoff >> 12)
    }

    fn get_high_linear_length(&self, eff_cutoff: u32) -> u32 {
        let eff_pulse_width = if self.pulse_width > 128 {
            ((self.pulse_width as u32) - 128) << 6
        } else {
            0
        };

        if eff_pulse_width < eff_cutoff {
            let exp_arg = eff_cutoff - eff_pulse_width;
            let mut len = exp9_interpolate(!exp_arg & 4095);
            len <<= 7 + (exp_arg >> 12);
            len.wrapping_sub(2 * SINE_SEGMENT_RELATIVE_LENGTH)
        } else {
            0
        }
    }

    fn compute_positions(&mut self, high_len: u32, low_len: u32, res_len_factor: u32) {
        self.square_wave_position = (self.wave_position >> 8) * (res_len_factor >> 4);
        self.resonance_sine_position = self.square_wave_position;

        if self.square_wave_position < SINE_SEGMENT_RELATIVE_LENGTH {
            self.phase = Phase::PositiveRising;
            return;
        }
        self.square_wave_position -= SINE_SEGMENT_RELATIVE_LENGTH;

        if self.square_wave_position < high_len {
            self.phase = Phase::PositiveLinear;
            return;
        }
        self.square_wave_position -= high_len;

        if self.square_wave_position < SINE_SEGMENT_RELATIVE_LENGTH {
            self.phase = Phase::PositiveFalling;
            return;
        }
        self.square_wave_position -= SINE_SEGMENT_RELATIVE_LENGTH;

        self.resonance_sine_position = self.square_wave_position;

        if self.square_wave_position < SINE_SEGMENT_RELATIVE_LENGTH {
            self.phase = Phase::NegativeFalling;
            return;
        }
        self.square_wave_position -= SINE_SEGMENT_RELATIVE_LENGTH;

        if self.square_wave_position < low_len {
            self.phase = Phase::NegativeLinear;
            return;
        }
        self.square_wave_position -= low_len;
        self.phase = Phase::NegativeRising;
    }

    fn advance_position(&mut self, pitch: u32, cutoff: u32) {
        self.wave_position =
            (self.wave_position + self.get_sample_step(pitch)) % (4 * SINE_SEGMENT_RELATIVE_LENGTH);

        let eff_cutoff = if cutoff > MIDDLE_CUTOFF_VALUE {
            (cutoff - MIDDLE_CUTOFF_VALUE) >> 10
        } else {
            0
        };

        let res_len_factor = self.get_resonance_wave_length_factor(eff_cutoff);
        let high_len = self.get_high_linear_length(eff_cutoff);
        let low_len = (res_len_factor << 8)
            .wrapping_sub(4 * SINE_SEGMENT_RELATIVE_LENGTH)
            .wrapping_sub(high_len);

        self.compute_positions(high_len, low_len, res_len_factor);

        let res_phase_offset = if matches!(
            self.phase,
            Phase::NegativeFalling | Phase::NegativeLinear | Phase::NegativeRising
        ) {
            2
        } else {
            0
        };

        let res_phase_val = ((self.resonance_sine_position >> 18) + res_phase_offset) & 3;
        self.resonance_phase = match res_phase_val {
            0 => ResonancePhase::PositiveRising,
            1 => ResonancePhase::PositiveFalling,
            2 => ResonancePhase::NegativeFalling,
            3 => ResonancePhase::NegativeRising,
            _ => ResonancePhase::PositiveRising,
        };
    }

    fn generate_square_wave(&mut self, amp: u32, cutoff: u32) {
        let log_value = match self.phase {
            Phase::PositiveRising | Phase::NegativeFalling => {
                logsin9(self.square_wave_position >> 9)
            }
            Phase::PositiveFalling | Phase::NegativeRising => {
                logsin9(!(self.square_wave_position >> 9))
            }
            Phase::PositiveLinear | Phase::NegativeLinear => 0,
        };

        let mut log_value = (log_value as u32) << 2;
        log_value += amp >> 10;

        if cutoff < MIDDLE_CUTOFF_VALUE {
            log_value += (MIDDLE_CUTOFF_VALUE - cutoff) >> 9;
        }

        self.square_log_sample.log_value = log_value.min(65535) as u16;
        self.square_log_sample.sign = match self.phase {
            Phase::PositiveRising | Phase::PositiveLinear | Phase::PositiveFalling => {
                Sign::Positive
            }
            Phase::NegativeFalling | Phase::NegativeLinear | Phase::NegativeRising => {
                Sign::Negative
            }
        };
    }

    fn generate_resonance_wave(&mut self, amp: u32, cutoff: u32) {
        let log_value = match self.resonance_phase {
            ResonancePhase::PositiveFalling | ResonancePhase::NegativeRising => {
                logsin9(!(self.resonance_sine_position >> 9))
            }
            ResonancePhase::PositiveRising | ResonancePhase::NegativeFalling => {
                logsin9(self.resonance_sine_position >> 9)
            }
        };

        let mut log_value = (log_value as u32) << 2;
        log_value = log_value.wrapping_add(amp >> 10);

        let decay_factor = match self.phase {
            Phase::PositiveRising | Phase::PositiveLinear | Phase::PositiveFalling => {
                self.res_amp_decay_factor
            }
            Phase::NegativeFalling | Phase::NegativeLinear | Phase::NegativeRising => {
                self.res_amp_decay_factor + 1
            }
        };

        let pre_decay = self.resonance_sine_position >> 4;
        let post_decay = pre_decay.wrapping_mul(decay_factor) >> 8;
        log_value = log_value.wrapping_add(self.resonance_amp_subtraction.wrapping_add(post_decay));

        match self.phase {
            Phase::PositiveRising | Phase::NegativeFalling => {
                log_value =
                    log_value.wrapping_add((logsin9(self.square_wave_position >> 9) as u32) << 2);
            }
            Phase::PositiveFalling | Phase::NegativeRising => {
                log_value = log_value
                    .wrapping_add((logsin9(!(self.square_wave_position >> 9)) as u32) << 3);
            }
            _ => (),
        }

        if cutoff < MIDDLE_CUTOFF_VALUE {
            log_value = log_value.wrapping_add(31743 + ((MIDDLE_CUTOFF_VALUE - cutoff) >> 9));
        } else if cutoff < RESONANCE_DECAY_THRESHOLD_CUTOFF_VALUE {
            log_value =
                log_value.wrapping_add((logsin9((cutoff - MIDDLE_CUTOFF_VALUE) >> 13) as u32) << 2);
        }

        log_value = log_value.saturating_sub(1 << 12);

        self.resonance_log_sample.log_value = log_value.min(65535) as u16;
        self.resonance_log_sample.sign = match self.resonance_phase {
            ResonancePhase::PositiveRising | ResonancePhase::PositiveFalling => Sign::Positive,
            ResonancePhase::NegativeFalling | ResonancePhase::NegativeRising => Sign::Negative,
        };
    }

    fn generate_sawtooth_cosine(&self) -> LogSample {
        let sawtooth_pos = self.wave_position + (1 << 18);
        let idx = if (sawtooth_pos & (1 << 18)) > 0 {
            !(sawtooth_pos >> 9)
        } else {
            sawtooth_pos >> 9
        };

        let log_value = logsin9(idx);
        let sign = if (sawtooth_pos & (1 << 19)) == 0 {
            Sign::Positive
        } else {
            Sign::Negative
        };

        LogSample {
            log_value: log_value << 2,
            sign,
        }
    }

    pub fn generate_samples(&mut self, amp: u32, pitch: u32, cutoff: u32) {
        let cutoff = cutoff.min(MAX_CUTOFF_VALUE);

        self.generate_square_wave(amp, cutoff);
        self.generate_resonance_wave(amp, cutoff);

        if self.waveform == WaveformType::Sawtooth {
            let cosine = self.generate_sawtooth_cosine();
            self.square_log_sample.add(cosine);
            self.resonance_log_sample.add(cosine);
        }

        self.advance_position(pitch, cutoff);
    }

    pub fn get_output_sample(&self) -> i16 {
        let first = unlog(self.square_log_sample);
        let second = unlog(self.resonance_log_sample);
        first.saturating_add(second)
    }
}
