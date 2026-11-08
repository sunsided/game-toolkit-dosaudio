use crate::{Error, Frame, MidiBackend, MidiSynth, Result, Tick, TickAccumulator};
const SYSEX_CAPACITY: usize = 65_536;

pub(crate) struct MidiVoice {
    pub core: Box<dyn MidiSynth>,
    pub held: Frame,
    pub next: Tick,
    clock: TickAccumulator,
    running: u8,
    short: [u8; 3],
    length: usize,
    needed: usize,
    sysex: Vec<u8>,
    in_sysex: bool,
    overflow: bool,
}
impl MidiVoice {
    pub fn new(backend: &MidiBackend, rate: u32) -> Result<Self> {
        let core = backend.build(rate)?;
        let native = core.sample_rate();
        if !(8_000..=192_000).contains(&native) {
            return Err(Error::Config("MIDI synth rate must be 8,000..=192,000 Hz"));
        }
        Ok(Self {
            core,
            held: [0.0; 2],
            next: Tick::ZERO,
            clock: TickAccumulator::new(u64::from(native)),
            running: 0,
            short: [0; 3],
            length: 0,
            needed: 0,
            sysex: Vec::with_capacity(SYSEX_CAPACITY),
            in_sysex: false,
            overflow: false,
        })
    }
    pub fn step(&mut self) {
        let mut out = [[0.0; 2]];
        self.core.render(&mut out);
        self.held = out[0];
        self.next += self.clock.advance(1);
    }
    pub fn message(&mut self, bytes: &[u8]) {
        self.core.message(bytes);
    }
    pub fn release(&mut self) {
        for channel in 0..16 {
            self.core.message(&[0xb0 | channel, 120, 0]);
        }
    }
    pub fn byte(&mut self, byte: u8) {
        if byte >= 0xf8 {
            if byte == 0xff {
                self.running = 0;
                self.needed = 0;
                self.length = 0;
                self.in_sysex = false;
                self.sysex.clear();
            }
            self.core.message(&[byte]);
            return;
        }
        if byte == 0xf0 {
            self.sysex.clear();
            self.sysex.push(byte);
            self.in_sysex = true;
            self.overflow = false;
            self.running = 0;
            self.needed = 0;
            return;
        }
        if self.in_sysex {
            if byte == 0xf7 {
                if !self.overflow {
                    self.sysex.push(byte);
                    self.core.message(&self.sysex);
                }
                self.sysex.clear();
                self.in_sysex = false;
                return;
            }
            if byte < 0x80 {
                if self.sysex.len() < SYSEX_CAPACITY - 1 {
                    self.sysex.push(byte);
                } else {
                    self.overflow = true;
                }
                return;
            }
            // A new status aborts malformed SysEx and begins a fresh message.
            self.sysex.clear();
            self.in_sysex = false;
        }
        if byte >= 0x80 {
            self.needed = match byte {
                0x80..=0xbf | 0xe0..=0xef | 0xf2 => 3,
                0xc0..=0xdf | 0xf1 | 0xf3 => 2,
                0xf6 => 1,
                _ => 0,
            };
            self.running = if byte < 0xf0 { byte } else { 0 };
            self.short[0] = byte;
            self.length = 1;
            if self.needed == 1 {
                self.core.message(&self.short[..1]);
                self.needed = 0;
            }
        } else if self.needed > 1 {
            self.short[self.length] = byte;
            self.length += 1;
            if self.length == self.needed {
                self.core.message(&self.short[..self.needed]);
                if self.running != 0 {
                    self.length = 1;
                    self.short[0] = self.running;
                } else {
                    self.length = 0;
                    self.needed = 0;
                }
            }
        }
    }
}
