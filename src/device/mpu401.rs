//! The MPU-401's two-port UART subset, without intelligent-mode sequencing.

/// Control-side UART state. The bus previews a copy before reserving any MIDI
/// command, so failed queue writes cannot change mode or acknowledgment state.
#[derive(Clone, Copy)]
pub(crate) struct MpuControl {
    pub base: u16,
    uart: bool,
    replies: [u8; 16],
    head: usize,
    length: usize,
}

impl MpuControl {
    pub(crate) fn new(base: u16) -> Self {
        Self {
            base,
            uart: false,
            replies: [0; 16],
            head: 0,
            length: 0,
        }
    }

    fn acknowledge(&mut self) {
        // Only command acknowledgments enter this FIFO. No external MIDI input
        // is simulated. Repeated unread UART acknowledgments saturate at 16.
        if self.length < self.replies.len() {
            self.replies[(self.head + self.length) % self.replies.len()] = 0xfe;
            self.length += 1;
        }
    }

    /// Return a byte to pass to the synth, including FFh for command-port reset.
    pub(crate) fn write(&mut self, port: u16, value: u8) -> Option<u8> {
        match port.checked_sub(self.base) {
            Some(0) if self.uart => Some(value),
            Some(1) => {
                match value {
                    0xff => {
                        self.uart = false;
                        self.head = 0;
                        self.length = 0;
                        self.acknowledge();
                        return Some(0xff);
                    }
                    0x3f => {
                        self.uart = true;
                        self.acknowledge();
                    }
                    _ => {} // Intelligent-mode commands are not implemented.
                }
                None
            }
            _ => None,
        }
    }

    pub(crate) fn read(&mut self, port: u16) -> u8 {
        match port.checked_sub(self.base) {
            Some(0) if self.length != 0 => {
                let value = self.replies[self.head];
                self.head = (self.head + 1) % self.replies.len();
                self.length -= 1;
                value
            }
            Some(1) => {
                if self.length == 0 {
                    0x80
                } else {
                    0
                }
            }
            _ => 0xff,
        }
    }
}
