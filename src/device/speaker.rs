use crate::Tick;

/// Square-wave frequency in hertz for a PIT channel 2 divisor.
pub(crate) fn pit_hz(divisor: u32) -> f64 {
    Tick::HZ_NUMERATOR as f64 / (Tick::HZ_DENOMINATOR as f64 * 12.0 * f64::from(divisor))
}

/// PIT channel 2 waveform state; passed by value to the synthesis thread.
#[derive(Clone, Copy, Debug)]
pub(crate) struct SpeakerState {
    pub divisor: u32,
    pub epoch: Tick,
    pub gate: bool,
    pub data: bool,
    pub mode: u8,
}

impl Default for SpeakerState {
    fn default() -> Self {
        Self {
            divisor: 65_536,
            epoch: Tick::ZERO,
            gate: false,
            data: false,
            mode: 3,
        }
    }
}

impl SpeakerState {
    fn high_ticks(self, elapsed: f64) -> f64 {
        let x = elapsed.max(0.0) / 12.0;
        let d = f64::from(self.divisor);
        let high = if self.mode == 2 {
            d - 1.0
        } else {
            (d / 2.0).ceil()
        };
        ((x / d).floor() * high + (x % d).min(high)) * 12.0
    }

    pub fn integral(self, start: f64, end: f64) -> f64 {
        if !self.data {
            return 0.0;
        }
        if !self.gate {
            return end - start;
        }
        self.high_ticks(end - self.epoch.0 as f64) - self.high_ticks(start - self.epoch.0 as f64)
    }

    pub fn output(self, at: Tick) -> bool {
        if !self.gate {
            return true;
        }
        let phase = at.0.saturating_sub(self.epoch.0) / 12 % u64::from(self.divisor);
        let high = if self.mode == 2 {
            self.divisor - 1
        } else {
            self.divisor.div_ceil(2)
        };
        phase < u64::from(high)
    }
}

#[derive(Clone, Copy)]
pub(crate) struct SpeakerControl {
    pub state: SpeakerState,
    port61: u8,
    access: u8,
    low: Option<u8>,
    read_high: bool,
    latched: Option<u16>,
}

impl Default for SpeakerControl {
    fn default() -> Self {
        Self {
            state: SpeakerState::default(),
            port61: 0,
            access: 3,
            low: None,
            read_high: false,
            latched: None,
        }
    }
}

impl SpeakerControl {
    pub fn write(&mut self, at: Tick, port: u16, value: u8) -> Option<SpeakerState> {
        match port {
            0x43 if value >> 6 == 2 => {
                let access = (value >> 4) & 3;
                if access == 0 {
                    if self.latched.is_none() {
                        self.latched = Some(self.counter(at));
                    }
                    return None;
                }
                let mode = (value >> 1) & 7;
                // This milestone supports the periodic binary modes used for speaker sound.
                self.state.mode = if mode == 2 || mode == 6 { 2 } else { 3 };
                self.access = access;
                self.low = None;
                self.read_high = false;
                self.latched = None;
                return Some(self.state);
            }
            0x42 => {
                let count = match self.access {
                    1 => u16::from(value),
                    2 => u16::from(value) << 8,
                    _ => match self.low.take() {
                        None => {
                            self.low = Some(value);
                            return None;
                        }
                        Some(low) => u16::from_le_bytes([low, value]),
                    },
                };
                self.state.divisor = if count == 0 { 65_536 } else { u32::from(count) };
                self.state.epoch = at;
            }
            0x61 => {
                if value & 1 != 0 && !self.state.gate {
                    self.state.epoch = at;
                }
                self.port61 = value & 0x0f;
                self.state.gate = value & 1 != 0;
                self.state.data = value & 2 != 0;
            }
            _ => return None,
        }
        Some(self.state)
    }

    fn counter(self, at: Tick) -> u16 {
        let elapsed = if self.state.gate {
            at.0.saturating_sub(self.state.epoch.0) / 12
        } else {
            0
        };
        let d = u64::from(self.state.divisor);
        let phase = elapsed % d;
        let count = if self.state.mode == 3 {
            let high = d.div_ceil(2);
            if phase == 0 {
                d
            } else if phase < high {
                d + d % 2 - 2 * phase
            } else {
                d - d % 2 - 2 * (phase - high)
            }
        } else {
            d - phase
        };
        count as u16
    }

    pub fn read(&mut self, at: Tick, port: u16) -> u8 {
        match port {
            0x61 => {
                // DRAM refresh toggles every 18 PIT input clocks.
                self.port61
                    | (((at.0 / 216) as u8 & 1) << 4)
                    | (u8::from(self.state.output(at)) << 5)
            }
            0x42 => {
                let count = self.latched.unwrap_or_else(|| self.counter(at));
                let value = if self.access == 2 || (self.access == 3 && self.read_high) {
                    (count >> 8) as u8
                } else {
                    count as u8
                };
                if self.access != 3 || self.read_high {
                    self.latched = None;
                }
                if self.access == 3 {
                    self.read_high = !self.read_high;
                }
                value
            }
            _ => 0xff,
        }
    }
}

pub(crate) struct SpeakerVoice {
    pub state: SpeakerState,
    hp_previous: f64,
    hp: f64,
    lp: f64,
    lp2: f64,
    hp_alpha: f64,
    lp_alpha: f64,
}

impl SpeakerVoice {
    pub fn new(rate: u32) -> Self {
        Self {
            state: SpeakerState::default(),
            hp_previous: 0.0,
            hp: 0.0,
            lp: 0.0,
            lp2: 0.0,
            hp_alpha: 1.0 / (1.0 + std::f64::consts::TAU * 20.0 / f64::from(rate)),
            lp_alpha: 1.0 - (-std::f64::consts::TAU * 7_000.0 / f64::from(rate)).exp(),
        }
    }

    pub fn filter(&mut self, mean: f64) -> f32 {
        self.hp = self.hp_alpha * (self.hp + mean - self.hp_previous);
        self.hp_previous = mean;
        self.lp += self.lp_alpha * (self.hp - self.lp);
        self.lp2 += self.lp_alpha * (self.lp - self.lp2);
        (self.lp2 * 0.5) as f32
    }
}
