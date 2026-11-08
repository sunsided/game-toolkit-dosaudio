// Copyright (C) 2021-2026 Geoff Hill <geoff@geoffhill.org>
// Copyright (C) 2003-2026 Dean Beeler, Jerome Fisher, Sergey V. Mikayev
//
// This program is free software: you can redistribute it and/or modify it
// under the terms of the GNU Lesser General Public License as published by
// the Free Software Foundation, either version 2.1 of the License, or (at
// your option) any later version. Read COPYING.LESSER.txt for details.

use crate::midi::mt32::cm32l::synth;
use crate::midi::mt32::cm32l::tables;

#[derive(Debug)]
pub struct PcmWavegen {
    position: u32,
    addr: usize,
    len: usize,
    loop_enabled: bool,
    exhausted: bool,
}

impl PcmWavegen {
    pub fn new(addr: usize, len: usize, loop_enabled: bool) -> Self {
        Self {
            position: 0,
            addr,
            len,
            loop_enabled,
            exhausted: false,
        }
    }

    pub fn is_exhausted(&self) -> bool {
        self.exhausted
    }

    fn calculate_step(&self, pitch: u32) -> u32 {
        let int_part = pitch >> 12;
        let frac_part = pitch & 0xFFF;

        let step = tables::exp9_interpolate(!frac_part & 0xFFF);
        let step = step << int_part;
        step >> 9
    }

    pub fn get_interpolated_sample(&self, pcm_rom: &[i16], amp: u32) -> i16 {
        let sample_index = (self.position >> 8) as usize;

        if sample_index >= self.len {
            return 0;
        }

        let first_pcm = pcm_rom[self.addr + sample_index];
        let first_log = synth::pcm_to_log(first_pcm, amp);
        let first_linear = synth::unlog(first_log);

        let next_index = sample_index + 1;
        let second_pcm = if next_index < self.len {
            pcm_rom[self.addr + next_index]
        } else if self.loop_enabled {
            pcm_rom[self.addr]
        } else {
            0
        };

        let second_log = synth::pcm_to_log(second_pcm, amp);
        let second_linear = synth::unlog(second_log);

        let factor = ((self.position & 0xFF) >> 1) as i32;
        let first = first_linear as i32;
        let second = second_linear as i32;
        let delta = second - first;

        (first + ((delta * factor) >> 7)) as i16
    }

    pub fn get_first_sample(&self, pcm_rom: &[i16], amp: u32) -> i16 {
        let sample_index = (self.position >> 8) as usize;

        if sample_index >= self.len {
            return 0;
        }

        let first_pcm = pcm_rom[self.addr + sample_index];
        let first_log = synth::pcm_to_log(first_pcm, amp);
        synth::unlog(first_log)
    }

    pub fn advance(&mut self, pitch: u32) {
        let step = self.calculate_step(pitch);
        self.position += step;

        let position_limit = (self.len << 8) as u32;
        if self.position >= position_limit {
            if self.loop_enabled {
                self.position -= position_limit;
            } else {
                self.exhausted = true;
            }
        }
    }
}
