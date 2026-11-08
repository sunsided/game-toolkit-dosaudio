// Copyright (C) 2021-2026 Geoff Hill <geoff@geoffhill.org>
// Copyright (C) 2003-2026 Dean Beeler, Jerome Fisher, Sergey V. Mikayev
//
// This program is free software: you can redistribute it and/or modify it
// under the terms of the GNU Lesser General Public License as published by
// the Free Software Foundation, either version 2.1 of the License, or (at
// your option) any later version. Read COPYING.LESSER.txt for details.

use crate::midi::mt32::cm32l::tables;

const TARGET_SHIFTS: u32 = 18;
const MAX_CURRENT: u32 = 0xFF << TARGET_SHIFTS;
const INTERRUPT_TIME: i32 = 7;

#[derive(Debug, Default)]
pub struct Ramp {
    current: u32,
    target: u32,
    increment: u32,
    descending: bool,
    interrupt_countdown: i32,
    interrupt_raised: bool,
}

impl Ramp {
    pub fn start(&mut self, target: u8, inc: u8) {
        self.target = u32::from(target) << TARGET_SHIFTS;
        self.interrupt_countdown = 0;
        self.interrupt_raised = false;

        if inc == 0 {
            self.increment = 0;
            return;
        }

        let inc_bits = inc & 0x7F;
        let exp_arg = (inc_bits as usize) << 6;
        let mut large_inc = 8191u32 - u32::from(tables::EXP9_TABLE[(!exp_arg) & 511]);
        large_inc <<= inc_bits >> 3;
        large_inc += 64;
        large_inc >>= 9;

        self.descending = (inc & 0x80) != 0;
        if self.descending {
            large_inc += 1;
        }
        self.increment = large_inc;

        trace!(target, increment = self.increment, "ramp start");
    }

    pub fn next(&mut self) -> u32 {
        if self.interrupt_countdown > 0 {
            self.interrupt_countdown -= 1;
            if self.interrupt_countdown == 0 {
                self.interrupt_raised = true;
            }
        } else if self.increment != 0 {
            if self.descending {
                if self.increment > self.current {
                    self.current = self.target;
                    self.interrupt_countdown = INTERRUPT_TIME;
                } else {
                    self.current -= self.increment;
                    if self.current <= self.target {
                        self.current = self.target;
                        self.interrupt_countdown = INTERRUPT_TIME;
                    }
                }
            } else if MAX_CURRENT - self.current < self.increment {
                self.current = self.target;
                self.interrupt_countdown = INTERRUPT_TIME;
            } else {
                self.current += self.increment;
                if self.current >= self.target {
                    self.current = self.target;
                    self.interrupt_countdown = INTERRUPT_TIME;
                }
            }
        }

        self.current
    }

    pub fn at_target(&mut self) -> bool {
        let raised = self.interrupt_raised;
        self.interrupt_raised = false;
        raised
    }

    pub fn current_value(&self) -> u32 {
        self.current
    }

    pub fn is_below_current(&self, target: u8) -> bool {
        u32::from(target) << TARGET_SHIFTS < self.current
    }
}
