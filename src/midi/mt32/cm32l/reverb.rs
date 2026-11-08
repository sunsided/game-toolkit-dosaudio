// Copyright (C) 2021-2026 Geoff Hill <geoff@geoffhill.org>
// Copyright (C) 2003-2026 Dean Beeler, Jerome Fisher, Sergey V. Mikayev
//
// This program is free software: you can redistribute it and/or modify it
// under the terms of the GNU Lesser General Public License as published by
// the Free Software Foundation, either version 2.1 of the License, or (at
// your option) any later version. Read COPYING.LESSER.txt for details.

use crate::midi::mt32::cm32l::Frame;
use std::{boxed::Box, vec};

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
enum ReverbMode {
    Room,
    Hall,
    Plate,
    TapDelay,
}

#[derive(Debug, Copy, Clone)]
enum ReverbOutput {
    Off,
    On { dry_amp: i32, wet_level: i32 },
}

impl TryFrom<u8> for ReverbMode {
    type Error = ();

    fn try_from(val: u8) -> Result<Self, Self::Error> {
        match val {
            0 => Ok(Self::Room),
            1 => Ok(Self::Hall),
            2 => Ok(Self::Plate),
            3 => Ok(Self::TapDelay),
            _ => Err(()),
        }
    }
}

const PROCESS_DELAY: usize = 1;

#[derive(Debug)]
struct StandardParams {
    allpass_sizes: [usize; 3],
    comb_sizes: [usize; 4],
    lpf_amp: u8,
    out_l: [usize; 3],
    out_r: [usize; 3],
    comb_factors: [u8; 4],
    comb_feedback: [u8; 32],
    dry_amp: [u8; 8],
    wet_amp: [u8; 8],
}

#[derive(Debug)]
struct TapDelayParams {
    delay_size: usize,
    out_l: [usize; 8],
    out_r: [usize; 8],
    comb_feedback: [u8; 2],
    comb_factor: u8,
    dry_amp: [u8; 16],
    wet_amp: [u8; 8],
}

const MODE_0_PARAMS: StandardParams = StandardParams {
    allpass_sizes: [994, 729, 78],
    comb_sizes: [705 + PROCESS_DELAY, 2349, 2839, 3632],
    lpf_amp: 0x60,
    out_l: [2349, 141, 1960],
    out_r: [1174, 1570, 145],
    comb_factors: [0xA0, 0x60, 0x60, 0x60],
    comb_feedback: [
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x28, 0x48, 0x60, 0x78, 0x80, 0x88, 0x90,
        0x98, 0x28, 0x48, 0x60, 0x78, 0x80, 0x88, 0x90, 0x98, 0x28, 0x48, 0x60, 0x78, 0x80, 0x88,
        0x90, 0x98,
    ],
    dry_amp: [0xA0, 0xA0, 0xA0, 0xA0, 0xB0, 0xB0, 0xB0, 0xD0],
    wet_amp: [0x10, 0x30, 0x50, 0x70, 0x90, 0xC0, 0xF0, 0xF0],
};

const MODE_1_PARAMS: StandardParams = StandardParams {
    allpass_sizes: [1324, 809, 176],
    comb_sizes: [961 + PROCESS_DELAY, 2619, 3545, 4519],
    lpf_amp: 0x60,
    out_l: [2618, 1760, 4518],
    out_r: [1300, 3532, 2274],
    comb_factors: [0x80, 0x60, 0x60, 0x60],
    comb_feedback: [
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x28, 0x48, 0x60, 0x70, 0x78, 0x80, 0x90,
        0x98, 0x28, 0x48, 0x60, 0x78, 0x80, 0x88, 0x90, 0x98, 0x28, 0x48, 0x60, 0x78, 0x80, 0x88,
        0x90, 0x98,
    ],
    dry_amp: [0xA0, 0xA0, 0xB0, 0xB0, 0xB0, 0xB0, 0xB0, 0xE0],
    wet_amp: [0x10, 0x30, 0x50, 0x70, 0x90, 0xC0, 0xF0, 0xF0],
};

const MODE_2_PARAMS: StandardParams = StandardParams {
    allpass_sizes: [969, 644, 157],
    comb_sizes: [116 + PROCESS_DELAY, 2259, 2839, 3539],
    lpf_amp: 0x80,
    out_l: [2259, 718, 1769],
    out_r: [1136, 2128, 1],
    comb_factors: [0, 0x20, 0x20, 0x20],
    comb_feedback: [
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x30, 0x58, 0x78, 0x88, 0xA0, 0xB8, 0xC0,
        0xD0, 0x30, 0x58, 0x78, 0x88, 0xA0, 0xB8, 0xC0, 0xD0, 0x30, 0x58, 0x78, 0x88, 0xA0, 0xB8,
        0xC0, 0xD0,
    ],
    dry_amp: [0xA0, 0xA0, 0xB0, 0xB0, 0xB0, 0xB0, 0xC0, 0xE0],
    wet_amp: [0x10, 0x30, 0x50, 0x70, 0x90, 0xC0, 0xF0, 0xF0],
};

const MODE_3_PARAMS: TapDelayParams = TapDelayParams {
    delay_size: 16003,
    out_l: [400, 624, 960, 1488, 2256, 3472, 5280, 8000],
    out_r: [800, 1248, 1920, 2976, 4512, 6944, 10560, 16000],
    comb_feedback: [0x68, 0x60],
    comb_factor: 0x68,
    dry_amp: [
        0x20, 0x50, 0x50, 0x50, 0x50, 0x50, 0x50, 0x50, 0x20, 0x50, 0x50, 0x50, 0x50, 0x50, 0x50,
        0x50,
    ],
    wet_amp: [0x18, 0x18, 0x28, 0x40, 0x60, 0x80, 0xA8, 0xF8],
};

const MODE_3_ADDITIONAL_DELAY: usize = 1;
const MODE_3_FEEDBACK_DELAY: usize = 1;

#[derive(Debug)]
struct RingBuffer {
    buffer: Box<[i16]>,
    index: usize,
}

impl RingBuffer {
    fn new(size: usize) -> Self {
        Self {
            buffer: vec![0; size].into_boxed_slice(),
            index: 0,
        }
    }

    fn next(&mut self) -> i16 {
        self.index += 1;
        if self.index >= self.buffer.len() {
            self.index = 0;
        }
        self.buffer[self.index]
    }

    fn mute(&mut self) {
        self.buffer.fill(0);
        self.index = 0;
    }
}

#[derive(Debug)]
struct AllpassFilter {
    ring: RingBuffer,
}

impl AllpassFilter {
    fn new(size: usize) -> Self {
        Self {
            ring: RingBuffer::new(size),
        }
    }

    fn process(&mut self, input: i16) -> i16 {
        let buffer_out = self.ring.next();
        self.ring.buffer[self.ring.index] = input - (buffer_out >> 1);
        buffer_out + (self.ring.buffer[self.ring.index] >> 1)
    }

    fn mute(&mut self) {
        self.ring.mute();
    }
}

#[derive(Debug)]
struct CombFilter {
    ring: RingBuffer,
    filter_factor: u8,
    feedback_factor: u8,
}

impl CombFilter {
    fn new(size: usize, filter_factor: u8) -> Self {
        Self {
            ring: RingBuffer::new(size),
            filter_factor,
            feedback_factor: 0,
        }
    }

    fn process(&mut self, input: i16) {
        let last = self.ring.buffer[self.ring.index];
        let next = self.ring.next();
        let filter_in = input + ((next as i32 * self.feedback_factor as i32) >> 8) as i16;
        self.ring.buffer[self.ring.index] =
            ((last as i32 * self.filter_factor as i32) >> 8) as i16 - filter_in;
    }

    fn get_output_at(&self, out_index: usize) -> i16 {
        let pos = (self.ring.buffer.len() + self.ring.index - out_index) % self.ring.buffer.len();
        self.ring.buffer[pos]
    }

    fn set_feedback_factor(&mut self, factor: u8) {
        self.feedback_factor = factor;
    }

    fn mute(&mut self) {
        self.ring.mute();
    }
}

#[derive(Debug)]
struct DelayWithLpf {
    ring: RingBuffer,
    filter_factor: u8,
    amp: u8,
}

impl DelayWithLpf {
    fn new(size: usize, filter_factor: u8, amp: u8) -> Self {
        Self {
            ring: RingBuffer::new(size),
            filter_factor,
            amp,
        }
    }

    fn process(&mut self, input: i16) {
        let last = self.ring.buffer[self.ring.index];
        self.ring.next();
        let lpf_out = ((last as i32 * self.filter_factor as i32) >> 8) as i16 + input;
        self.ring.buffer[self.ring.index] = ((lpf_out as i32 * self.amp as i32) >> 8) as i16;
    }

    fn get_output_at(&self, out_index: usize) -> i16 {
        let pos = (self.ring.buffer.len() + self.ring.index - out_index) % self.ring.buffer.len();
        self.ring.buffer[pos]
    }

    fn mute(&mut self) {
        self.ring.mute();
    }
}

#[derive(Debug)]
struct TapDelayCombFilter {
    ring: RingBuffer,
    filter_factor: u8,
    feedback_factor: u8,
    out_l: usize,
    out_r: usize,
}

impl TapDelayCombFilter {
    fn new(size: usize, filter_factor: u8) -> Self {
        Self {
            ring: RingBuffer::new(size),
            filter_factor,
            feedback_factor: 0,
            out_l: 0,
            out_r: 0,
        }
    }

    fn process(&mut self, input: i16) {
        let last = self.ring.buffer[self.ring.index];
        self.ring.next();
        let feedback_pos = self.out_r + MODE_3_FEEDBACK_DELAY;
        let feedback_sample = self.get_output_at(feedback_pos);
        let filter_in =
            input + ((feedback_sample as i32 * self.feedback_factor as i32) >> 8) as i16;
        self.ring.buffer[self.ring.index] =
            ((last as i32 * self.filter_factor as i32) >> 8) as i16 - filter_in;
    }

    fn get_output_at(&self, out_index: usize) -> i16 {
        let pos = (self.ring.buffer.len() + self.ring.index - out_index) % self.ring.buffer.len();
        self.ring.buffer[pos]
    }

    fn get_left_output(&self) -> i32 {
        self.get_output_at(self.out_l + PROCESS_DELAY + MODE_3_ADDITIONAL_DELAY) as i32
    }

    fn get_right_output(&self) -> i32 {
        self.get_output_at(self.out_r + PROCESS_DELAY + MODE_3_ADDITIONAL_DELAY) as i32
    }

    fn set_output_positions(&mut self, out_l: usize, out_r: usize) {
        self.out_l = out_l;
        self.out_r = out_r;
    }

    fn set_feedback_factor(&mut self, factor: u8) {
        self.feedback_factor = factor;
    }

    fn mute(&mut self) {
        self.ring.mute();
    }
}

pub const MAX_BUFFER_LEN: usize = 4096;

#[derive(Debug)]
struct StereoBuffer {
    left: [i16; MAX_BUFFER_LEN],
    right: [i16; MAX_BUFFER_LEN],
}

impl Default for StereoBuffer {
    fn default() -> Self {
        Self {
            left: [0; MAX_BUFFER_LEN],
            right: [0; MAX_BUFFER_LEN],
        }
    }
}

impl StereoBuffer {
    fn clear(&mut self, len: usize) {
        self.left[..len].fill(0);
        self.right[..len].fill(0);
    }

    fn as_mut(&mut self, len: usize) -> (&mut [i16], &mut [i16]) {
        (&mut self.left[..len], &mut self.right[..len])
    }
}

#[derive(Debug, Default)]
struct Buffers {
    clean: StereoBuffer,
    dry: StereoBuffer,
    wet: StereoBuffer,
    active_len: usize,
}

impl Buffers {
    fn new() -> Self {
        Default::default()
    }

    fn prepare(&mut self, len: usize) {
        debug_assert!(len <= MAX_BUFFER_LEN);
        self.active_len = len;
        self.clean.clear(len);
        self.dry.clear(len);
        self.wet.clear(len);
    }
}

#[derive(Debug)]
enum ReverbFilters {
    Standard {
        allpasses: [AllpassFilter; 3],
        combs: [CombFilter; 3],
        entrance_delay: DelayWithLpf,
        params: &'static StandardParams,
    },
    TapDelay {
        tap_delay: TapDelayCombFilter,
        params: &'static TapDelayParams,
    },
}

#[derive(Debug)]
pub struct Reverb {
    filters: ReverbFilters,
    spares: [Option<ReverbFilters>; 4],
    mode: ReverbMode,
    time: u8,
    level: u8,
    output: ReverbOutput,
    buffers: Buffers,
}

impl Reverb {
    fn create_standard_filters(params: &'static StandardParams) -> ReverbFilters {
        ReverbFilters::Standard {
            allpasses: [
                AllpassFilter::new(params.allpass_sizes[0]),
                AllpassFilter::new(params.allpass_sizes[1]),
                AllpassFilter::new(params.allpass_sizes[2]),
            ],
            combs: [
                CombFilter::new(params.comb_sizes[1], params.comb_factors[1]),
                CombFilter::new(params.comb_sizes[2], params.comb_factors[2]),
                CombFilter::new(params.comb_sizes[3], params.comb_factors[3]),
            ],
            entrance_delay: DelayWithLpf::new(
                params.comb_sizes[0],
                params.comb_factors[0],
                params.lpf_amp,
            ),
            params,
        }
    }

    fn create_filters(mode: ReverbMode) -> ReverbFilters {
        match mode {
            ReverbMode::Room => Self::create_standard_filters(&MODE_0_PARAMS),
            ReverbMode::Hall => Self::create_standard_filters(&MODE_1_PARAMS),
            ReverbMode::Plate => Self::create_standard_filters(&MODE_2_PARAMS),
            ReverbMode::TapDelay => ReverbFilters::TapDelay {
                tap_delay: TapDelayCombFilter::new(
                    MODE_3_PARAMS.delay_size,
                    MODE_3_PARAMS.comb_factor,
                ),
                params: &MODE_3_PARAMS,
            },
        }
    }

    pub fn new() -> Self {
        let mode = ReverbMode::Room;
        let time = 5;
        let level = 3;
        let mut reverb = Self {
            filters: Self::create_filters(mode),
            spares: std::array::from_fn(|i| {
                if i == 0 {
                    None
                } else {
                    Some(Self::create_filters(ReverbMode::try_from(i as u8).unwrap()))
                }
            }),
            mode,
            time,
            level,
            output: ReverbOutput::Off,
            buffers: Buffers::new(),
        };

        reverb.apply_parameters();
        reverb
    }

    pub fn prepare(&mut self, len: usize) {
        self.buffers.prepare(len);
    }

    pub fn clean_bufs(&mut self) -> (&mut [i16], &mut [i16]) {
        self.buffers.clean.as_mut(self.buffers.active_len)
    }

    pub fn dry_bufs(&mut self) -> (&mut [i16], &mut [i16]) {
        self.buffers.dry.as_mut(self.buffers.active_len)
    }

    pub fn apply_la32_output(&mut self) {
        let len = self.buffers.active_len;
        for sample in &mut self.buffers.clean.left[..len] {
            *sample = sample.saturating_mul(2);
        }
        for sample in &mut self.buffers.clean.right[..len] {
            *sample = sample.saturating_mul(2);
        }
        for sample in &mut self.buffers.dry.left[..len] {
            *sample = sample.saturating_mul(2);
        }
        for sample in &mut self.buffers.dry.right[..len] {
            *sample = sample.saturating_mul(2);
        }
    }

    pub fn mute(&mut self) {
        match &mut self.filters {
            ReverbFilters::Standard {
                allpasses,
                combs,
                entrance_delay,
                ..
            } => {
                for ap in allpasses {
                    ap.mute();
                }
                for comb in combs {
                    comb.mute();
                }
                entrance_delay.mute();
            }
            ReverbFilters::TapDelay { tap_delay, .. } => {
                tap_delay.mute();
            }
        }
    }

    pub fn set_time(&mut self, time: u8) {
        self.time = time & 7;
        self.apply_parameters();
    }

    pub fn set_level(&mut self, level: u8) {
        self.level = level & 7;
        self.apply_parameters();
    }

    fn apply_parameters(&mut self) {
        let was_off = matches!(self.output, ReverbOutput::Off);
        let level = self.level as usize;
        let time = self.time as usize;

        match &mut self.filters {
            ReverbFilters::Standard { combs, params, .. } => {
                for (i, comb) in combs.iter_mut().enumerate() {
                    let feedback_idx = ((i + 1) << 3) + time;
                    comb.set_feedback_factor(params.comb_feedback[feedback_idx]);
                }

                if time == 0 && level == 0 {
                    self.output = ReverbOutput::Off;
                } else {
                    let dry_amp = params.dry_amp[level] as i32;
                    let wet_level = params.wet_amp[level] as i32;
                    self.output = ReverbOutput::On { dry_amp, wet_level };
                }
            }
            ReverbFilters::TapDelay { tap_delay, params } => {
                tap_delay.set_output_positions(params.out_l[time], params.out_r[time]);
                let feedback_idx = if (level < 3) || (time < 6) { 0 } else { 1 };
                tap_delay.set_feedback_factor(params.comb_feedback[feedback_idx]);

                if time == 0 && level == 0 {
                    self.output = ReverbOutput::Off;
                } else {
                    let dry_amp = if (time == 0) || (time == 1 && level == 1) {
                        params.dry_amp[level + 8] as i32
                    } else {
                        params.dry_amp[level] as i32
                    };
                    let wet_level = params.wet_amp[level] as i32;
                    self.output = ReverbOutput::On { dry_amp, wet_level };
                }
            }
        }

        if was_off && matches!(self.output, ReverbOutput::On { .. }) {
            self.mute();
        }
    }

    pub fn set_mode(&mut self, new_mode: u8) {
        debug!(new_mode, current_mode = ?self.mode, "reverb set_mode");

        let Ok(new_mode) = ReverbMode::try_from(new_mode) else {
            return;
        };
        if new_mode != self.mode {
            debug!(?new_mode, "reverb set_mode: creating new filters");

            let next = self.spares[new_mode as usize]
                .take()
                .expect("preallocated reverb mode");
            self.spares[self.mode as usize] = Some(std::mem::replace(&mut self.filters, next));
            self.mode = new_mode;
            self.mute();
        }
        self.apply_parameters();
    }

    pub fn process(&mut self) {
        let ReverbOutput::On { dry_amp, wet_level } = self.output else {
            return;
        };
        let len = self.buffers.active_len;
        match &mut self.filters {
            ReverbFilters::Standard {
                allpasses,
                combs,
                entrance_delay,
                params,
            } => {
                for i in 0..len {
                    let dry = (self.buffers.dry.left[i] >> 2) + (self.buffers.dry.right[i] >> 2);
                    let dry = ((dry as i32 * dry_amp) >> 8) as i16;

                    let link = entrance_delay.get_output_at(params.comb_sizes[0] - 1);
                    entrance_delay.process(dry);

                    let link = allpasses[0].process(link);
                    let link = allpasses[1].process(link);
                    let link = allpasses[2].process(link);

                    let out_l1 = combs[0].get_output_at(params.out_l[0] - 1);

                    combs[0].process(link);
                    combs[1].process(link);
                    combs[2].process(link);

                    let out_l2 = combs[1].get_output_at(params.out_l[1]);
                    let out_l3 = combs[2].get_output_at(params.out_l[2]);
                    let out_l = out_l1 as i32
                        + (out_l1 as i32 >> 1)
                        + out_l2 as i32
                        + (out_l2 as i32 >> 1)
                        + out_l3 as i32;
                    let out_l = out_l.clamp(i16::MIN as i32, i16::MAX as i32) as i16;
                    self.buffers.wet.left[i] = ((out_l as i32 * wet_level) >> 8) as i16;

                    let out_r1 = combs[0].get_output_at(params.out_r[0]);
                    let out_r2 = combs[1].get_output_at(params.out_r[1]);
                    let out_r3 = combs[2].get_output_at(params.out_r[2]);
                    let out_r = out_r1 as i32
                        + (out_r1 as i32 >> 1)
                        + out_r2 as i32
                        + (out_r2 as i32 >> 1)
                        + out_r3 as i32;
                    let out_r = out_r.clamp(i16::MIN as i32, i16::MAX as i32) as i16;
                    self.buffers.wet.right[i] = ((out_r as i32 * wet_level) >> 8) as i16;
                }
            }
            ReverbFilters::TapDelay { tap_delay, .. } => {
                for i in 0..len {
                    let dry = (self.buffers.dry.left[i] >> 1) + (self.buffers.dry.right[i] >> 1);
                    let dry = ((dry as i32 * dry_amp) >> 8) as i16;

                    tap_delay.process(dry);

                    self.buffers.wet.left[i] =
                        ((tap_delay.get_left_output() * wet_level) >> 8) as i16;
                    self.buffers.wet.right[i] =
                        ((tap_delay.get_right_output() * wet_level) >> 8) as i16;
                }
            }
        }
    }

    pub fn mix_output(&self, out: &mut [Frame]) {
        let bufs = &self.buffers;

        for (i, item) in out.iter_mut().enumerate() {
            let clean_left = bufs.clean.left[i] as i32;
            let clean_right = bufs.clean.right[i] as i32;
            let dry_left = bufs.dry.left[i] as i32;
            let dry_right = bufs.dry.right[i] as i32;

            let wet_left = bufs.wet.left[i] as i32 * 174;
            let wet_right = bufs.wet.right[i] as i32 * 174;

            let combined_left = (clean_left + dry_left) * 256 + wet_left;
            let combined_right = (clean_right + dry_right) * 256 + wet_right;

            let left = (combined_left >> 8).clamp(i16::MIN as i32, i16::MAX as i32) as i16;
            let right = (combined_right >> 8).clamp(i16::MIN as i32, i16::MAX as i32) as i16;

            *item = Frame(left, right);
        }
    }
}
