// Copyright (C) 2021-2026 Geoff Hill <geoff@geoffhill.org>
// Copyright (C) 2003-2026 Dean Beeler, Jerome Fisher, Sergey V. Mikayev
//
// This program is free software: you can redistribute it and/or modify it
// under the terms of the GNU Lesser General Public License as published by
// the Free Software Foundation, either version 2.1 of the License, or (at
// your option) any later version. Read COPYING.LESSER.txt for details.

const COARSE_LPF_TAPS: [i32; 9] = [21965, -6608, 590, 1084, -1142, 812, -510, 314, -204];
const FRACTION_BITS: u32 = 14;
const RING_BUF_LEN: usize = 8;
const RING_BUF_MASK: usize = RING_BUF_LEN - 1;

#[derive(Debug)]
pub struct CoarseLpf {
    ring_buf: [i16; RING_BUF_LEN],
    pos: usize,
}

impl CoarseLpf {
    pub fn new() -> CoarseLpf {
        CoarseLpf {
            ring_buf: [0; RING_BUF_LEN],
            pos: 0,
        }
    }

    pub fn process(&mut self, in_sample: i16) -> i16 {
        let mut sample = COARSE_LPF_TAPS[RING_BUF_LEN] * (self.ring_buf[self.pos] as i32);

        self.ring_buf[self.pos] = in_sample;

        for i in 0..RING_BUF_LEN {
            let idx = (i + self.pos) & RING_BUF_MASK;
            sample += COARSE_LPF_TAPS[i] * (self.ring_buf[idx] as i32);
        }

        self.pos = (self.pos.wrapping_sub(1)) & RING_BUF_MASK;

        (sample >> FRACTION_BITS).clamp(i16::MIN as i32, i16::MAX as i32) as i16
    }
}
