use crate::{Frame, OplBackend, OplChip, Tick};

/// A synthesis core running at one frame per 288 master-crystal ticks.
///
/// Timers, register latches and status remain in the control half. Implementations
/// must not allocate or block in either method and must return finite samples.
pub trait OplCore: Send {
    /// Write a register in 000h through 1FFh.
    fn write(&mut self, register: u16, value: u8);
    /// Generate one native stereo frame.
    fn generate(&mut self) -> Frame;
}

struct OplonCore(oplon::Opl2);
impl OplCore for OplonCore {
    fn write(&mut self, register: u16, value: u8) {
        if register < 0x100 {
            self.0.write_reg(register as u8, value);
        } else {
            self.0.write_reg_high(register as u8, value);
        }
    }
    fn generate(&mut self) -> Frame {
        let (left, right) = self.0.render_frame();
        [left as f32 / 32768.0, right as f32 / 32768.0]
    }
}

#[cfg(feature = "nuked")]
struct NukedCore(nuked_opl3::Opl3Chip);
#[cfg(feature = "nuked")]
impl OplCore for NukedCore {
    fn write(&mut self, register: u16, value: u8) {
        self.0.write_register(register, value);
    }
    fn generate(&mut self) -> Frame {
        let mut frame = [0i16; 2];
        self.0.generate(&mut frame).expect("fixed stereo frame");
        [f32::from(frame[0]) / 32768.0, f32::from(frame[1]) / 32768.0]
    }
}

pub(crate) fn core(backend: OplBackend) -> Box<dyn OplCore> {
    match backend {
        // Equal native and output rates bypass the core's rate conversion.
        OplBackend::Oplon => Box::new(OplonCore(oplon::Opl2::with_native_rate(49_716, 49_716))),
        #[cfg(feature = "nuked")]
        OplBackend::Nuked => Box::new(NukedCore(nuked_opl3::Opl3Chip::new(49_716))),
        OplBackend::Custom(factory) => factory(),
    }
}

#[derive(Clone, Copy, Default)]
struct Timer {
    value: u8,
    deadline: Option<u128>,
    overflow: bool,
    masked: bool,
}

impl Timer {
    const DENOMINATOR: u128 = 22_000_000;
    fn period(self, micros: u64) -> u128 {
        (256 - u128::from(self.value)) * u128::from(micros) * u128::from(Tick::HZ_NUMERATOR)
    }
    fn sync(&mut self, at: Tick, micros: u64) {
        if let Some(deadline) = self.deadline {
            // Deadlines are rational crystal ticks, observed at their floor.
            let now = u128::from(at.0) * Self::DENOMINATOR + Self::DENOMINATOR - 1;
            if now >= deadline {
                self.overflow |= !self.masked;
                let period = self.period(micros);
                self.deadline = Some(deadline + ((now - deadline) / period + 1) * period);
            }
        }
    }

    fn running(&mut self, at: Tick, enabled: bool, micros: u64) {
        if !enabled {
            self.deadline = None;
        } else if self.deadline.is_none() {
            self.deadline = Some(u128::from(at.0) * Self::DENOMINATOR + self.period(micros));
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) struct OplControl {
    pub chip: OplChip,
    address: [u8; 2],
    timers: [[Timer; 2]; 2],
    registers: [u8; 512],
    patch_levels: [[u8; 2]; 18],
}

impl OplControl {
    pub fn addresses(&self) -> [u8; 2] {
        self.address
    }
    pub fn new(chip: OplChip) -> Self {
        Self {
            chip,
            address: [0; 2],
            timers: [[Timer::default(); 2]; 2],
            registers: [0; 512],
            patch_levels: [[0; 2]; 18],
        }
    }

    pub fn sync(&mut self, at: Tick) {
        for timers in &mut self.timers {
            timers[0].sync(at, 80);
            timers[1].sync(at, 320);
        }
    }

    pub fn write(&mut self, at: Tick, port: u16, value: u8) -> Option<(u16, u8)> {
        self.sync(at);
        let bank = usize::from(port >= 0x38a);
        if bank != 0 && self.chip == OplChip::Opl2 {
            return None;
        }
        if port & 1 == 0 {
            self.address[bank] = value;
            return None;
        }
        let register = self.address[bank];
        self.registers[usize::from(register) + bank * 256] = value;
        if bank == 0 || self.chip == OplChip::DualOpl2 {
            match register {
                2 | 3 => self.timers[bank][usize::from(register - 2)].value = value,
                4 => {
                    if value & 0x80 != 0 {
                        for timer in &mut self.timers[bank] {
                            timer.overflow = false;
                        }
                    } else {
                        for (index, timer) in self.timers[bank].iter_mut().enumerate() {
                            timer.masked = value & (0x40 >> index) != 0;
                            if timer.masked {
                                timer.overflow = false;
                            }
                            timer.running(
                                at,
                                value & (1 << index) != 0,
                                if index == 0 { 80 } else { 320 },
                            );
                        }
                    }
                }
                _ => return Some((register as u16 | ((bank as u16) << 8), value)),
            }
            return None;
        }
        Some((register as u16 | 0x100, value))
    }

    pub fn read(&mut self, at: Tick, port: u16) -> u8 {
        self.sync(at);
        if port & 1 != 0 {
            return 0xff;
        }
        let bank = usize::from(port >= 0x38a);
        if bank != 0 && self.chip != OplChip::DualOpl2 {
            return 0xff;
        }
        let timers = self.timers[bank];
        let flags = (u8::from(timers[0].overflow && !timers[0].masked) << 6)
            | (u8::from(timers[1].overflow && !timers[1].masked) << 5);
        flags | if flags != 0 { 0x80 } else { 0 } | if self.chip == OplChip::Opl3 { 0 } else { 6 }
    }

    pub fn register(&self, register: u16) -> u8 {
        self.registers[usize::from(register)]
    }
    pub fn patch_levels(&self, channel: usize) -> [u8; 2] {
        self.patch_levels[channel]
    }
    pub fn set_patch_levels(&mut self, channel: usize, levels: [u8; 2]) {
        self.patch_levels[channel] = levels;
    }
}

pub(crate) struct OplVoice {
    pub chip: OplChip,
    first: Box<dyn OplCore>,
    second: Option<Box<dyn OplCore>>,
    pub held: Frame,
    pub next: Tick,
    registers: [u8; 512],
}

impl OplVoice {
    pub fn new(chip: OplChip, backend: OplBackend) -> Self {
        Self {
            chip,
            first: core(backend.clone()),
            second: (chip == OplChip::DualOpl2).then(|| core(backend)),
            held: [0.0; 2],
            next: Tick::ZERO,
            registers: [0; 512],
        }
    }

    pub fn write(&mut self, register: u16, value: u8) {
        self.registers[usize::from(register)] = value;
        if let Some(second) = &mut self.second {
            if register & 0x100 != 0 {
                second.write(register & 0xff, value);
            } else {
                self.first.write(register, value);
            }
        } else if self.chip == OplChip::Opl3 || register < 0x100 {
            self.first.write(register, value);
        }
    }

    pub fn step(&mut self) {
        let first = self.first.generate();
        self.held = if let Some(second) = &mut self.second {
            [first[0], second.generate()[0]]
        } else {
            first
        };
        self.next += Tick(288);
    }

    pub fn release_notes(&mut self, mut record: impl FnMut(u16, u8)) {
        for bank in [0, 0x100] {
            for register in 0xb0..=0xb8 {
                let register = register | bank;
                let value = self.registers[usize::from(register)] & !0x20;
                if self.chip != OplChip::Opl2 || register < 0x100 {
                    record(register, value);
                }
                self.write(register, value);
            }
            let rhythm = 0xbd | bank;
            let value = self.registers[usize::from(rhythm)] & !0x1f;
            if self.chip != OplChip::Opl2 || rhythm < 0x100 {
                record(rhythm, value);
            }
            self.write(rhythm, value);
        }
    }
}
