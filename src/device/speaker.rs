use crate::Tick;
use game_toolkit_dosbus::{Channel, Mode, Pit};

/// Square-wave frequency in hertz for a PIT channel 2 divisor.
pub(crate) fn pit_hz(divisor: u32) -> f64 {
    Tick::HZ_NUMERATOR as f64 / (Tick::HZ_DENOMINATOR as f64 * 12.0 * f64::from(divisor))
}

/// PIT channel 2 waveform state; passed by value to the synthesis thread.
/// `mode` is 0, 2 or 3, matching the dosbus `Pit` channel 2 mode.
#[derive(Clone, Copy, Debug, PartialEq)]
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
        if self.mode == 0 {
            // One-shot: low for `divisor` clocks after the epoch, then high; low while gated off.
            if !self.gate {
                return 0.0;
            }
            let rise = self.epoch.0 as f64 + 12.0 * f64::from(self.divisor);
            return (end - start.max(rise)).max(0.0);
        }
        if !self.gate {
            return end - start;
        }
        self.high_ticks(end - self.epoch.0 as f64) - self.high_ticks(start - self.epoch.0 as f64)
    }
}

/// Port-side speaker: the machine's single 8253 plus port 61h.
#[derive(Clone)]
pub(crate) struct SpeakerControl {
    pub state: SpeakerState,
    pub pit: Pit,
    port61: u8,
}

impl Default for SpeakerControl {
    fn default() -> Self {
        let mut control = Self {
            state: SpeakerState::default(),
            pit: Pit::new(),
            port61: 0,
        };
        control.sync();
        control
    }
}

impl SpeakerControl {
    /// Apply a 40h-43h or 61h write; returns channel 2's new state if it changed.
    pub fn write(&mut self, at: Tick, port: u16, value: u8) -> Option<SpeakerState> {
        match port {
            0x40..=0x43 => self.pit.write(at, port, value),
            0x61 => {
                self.port61 = value & 0x0f;
                self.pit.set_gate(Channel::Ch2, at, value & 1 != 0);
            }
            _ => return None,
        }
        let previous = self.state;
        self.sync();
        if port == 0x61 {
            self.state.data = value & 2 != 0;
        }
        (self.state != previous).then_some(self.state)
    }

    fn sync(&mut self) {
        let channel = self.pit.channel(Channel::Ch2);
        self.state.divisor = channel.divisor();
        self.state.epoch = channel.epoch();
        self.state.gate = channel.gate();
        self.state.mode = match channel.mode() {
            Mode::TerminalCount => 0,
            Mode::RateGenerator => 2,
            _ => 3,
        };
    }

    pub fn read(&mut self, at: Tick, port: u16) -> u8 {
        match port {
            0x61 => {
                let refresh = self.pit.channel(Channel::Ch1).rising_edges(Tick::ZERO, at) & 1;
                self.port61
                    | ((refresh as u8) << 4)
                    | (u8::from(self.pit.output(Channel::Ch2, at)) << 5)
            }
            _ => self.pit.read(at, port),
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
