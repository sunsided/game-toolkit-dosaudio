use std::sync::{
    Arc,
    atomic::{AtomicU8, Ordering},
};

use crate::{DmaDirection, DmaRequest, Error, PortRange, Result};

const MEMORY_SIZE: usize = 1 << 20;
pub(crate) mod playback;
pub use playback::DmaCompletion;

/// Shared, zero-initialized conventional memory spanning physical addresses
/// 00000h through FFFFFh. Clones share the same bytes without locking.
///
/// Individual bytes are atomic; bulk reads and writes are not atomic snapshots.
/// All accesses validate their entire range before reading or writing any byte.
#[derive(Clone, Debug)]
pub struct ConventionalMemory {
    bytes: Arc<[AtomicU8]>,
}

impl Default for ConventionalMemory {
    fn default() -> Self {
        Self::new()
    }
}

impl ConventionalMemory {
    /// Allocate one megabyte of zeroed memory, shared by subsequent clones.
    pub fn new() -> Self {
        Self {
            bytes: (0..MEMORY_SIZE).map(|_| AtomicU8::new(0)).collect(),
        }
    }

    fn range(&self, address: u32, length: usize) -> Result<core::ops::Range<usize>> {
        let start = usize::try_from(address).map_err(|_| Error::TooLarge)?;
        let end = start
            .checked_add(length)
            .filter(|&end| end <= MEMORY_SIZE)
            .ok_or(Error::Config(
                "conventional memory access exceeds one megabyte",
            ))?;
        Ok(start..end)
    }

    /// Read one physical byte, or report an address outside conventional memory.
    pub fn read_byte(&self, address: u32) -> Result<u8> {
        let range = self.range(address, 1)?;
        Ok(self.bytes[range.start].load(Ordering::Acquire))
    }

    /// Write one physical byte, or report an address outside conventional memory.
    pub fn write_byte(&self, address: u32, value: u8) -> Result<()> {
        let range = self.range(address, 1)?;
        self.bytes[range.start].store(value, Ordering::Release);
        Ok(())
    }

    /// Write a fully validated physical range; errors leave memory unchanged.
    pub fn write(&self, address: u32, bytes: &[u8]) -> Result<()> {
        let range = self.range(address, bytes.len())?;
        for (destination, &value) in self.bytes[range].iter().zip(bytes) {
            destination.store(value, Ordering::Release);
        }
        Ok(())
    }

    /// Read a fully validated physical range; errors leave `out` unchanged.
    pub fn read(&self, address: u32, out: &mut [u8]) -> Result<()> {
        let range = self.range(address, out.len())?;
        for (source, destination) in self.bytes[range].iter().zip(out) {
            *destination = source.load(Ordering::Acquire);
        }
        Ok(())
    }
}

/// One bus-width unit delivered by the host DMA controller.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DmaUnit {
    /// One byte on DMA channels 0 through 3.
    Byte(u8),
    /// One little-endian word on DMA channels 5 through 7.
    Word(u16),
}

#[derive(Clone, Copy, Debug)]
struct Channel {
    address: u16,
    count: u16,
    base_address: u16,
    base_count: u16,
    page: u8,
    mode: u8,
    masked: bool,
    active: bool,
}

impl Channel {
    const RESET: Self = Self {
        address: 0,
        count: 0,
        base_address: 0,
        base_count: 0,
        page: 0,
        mode: 0,
        masked: true,
        active: false,
    };
}

/// The two PC/AT 8237 DMA controllers, serviced one unit per device request.
///
/// Models address/count registers, AT page registers, masks, transfer direction,
/// auto-initialization and terminal count. Address/count accesses share one
/// low/high flip-flop per controller. Channel 4 is the cascade and transfers no
/// device data. Only memory-to-device transfers are implemented.
///
/// This models register-visible transfer state, without bus arbitration, timing,
/// memory-to-memory transfers or peripheral-to-memory writes. Demand, single and
/// block modes all service one unit per call; cascade mode does not transfer.
#[derive(Clone, Copy, Debug)]
pub struct DmaController {
    channels: [Channel; 8],
    high_byte: [bool; 2],
    command: [u8; 2],
    terminal_count: [u8; 2],
    software_request: [u8; 2],
}

impl Default for DmaController {
    fn default() -> Self {
        Self::new()
    }
}

impl DmaController {
    /// Construct two controllers with all channels masked and inactive.
    pub const fn new() -> Self {
        Self {
            channels: [Channel::RESET; 8],
            high_byte: [false; 2],
            command: [0; 2],
            terminal_count: [0; 2],
            software_request: [0; 2],
        }
    }

    /// I/O ports decoded by the controllers and their seven AT page registers.
    pub fn ports(&self) -> &[PortRange] {
        const PORTS: &[PortRange] = &[
            PortRange::new(0x00, 0x0f),
            PortRange::new(0x81, 0x83),
            PortRange::single(0x87),
            PortRange::new(0x89, 0x8b),
            PortRange::single(0xc0),
            PortRange::single(0xc2),
            PortRange::single(0xc4),
            PortRange::single(0xc6),
            PortRange::single(0xc8),
            PortRange::single(0xca),
            PortRange::single(0xcc),
            PortRange::single(0xce),
            PortRange::single(0xd0),
            PortRange::single(0xd2),
            PortRange::single(0xd4),
            PortRange::single(0xd6),
            PortRange::single(0xd8),
            PortRange::single(0xda),
            PortRange::single(0xdc),
            PortRange::single(0xde),
        ];
        PORTS
    }

    fn page_channel(port: u16) -> Option<usize> {
        match port {
            0x87 => Some(0),
            0x83 => Some(1),
            0x81 => Some(2),
            0x82 => Some(3),
            0x8b => Some(5),
            0x89 => Some(6),
            0x8a => Some(7),
            _ => None,
        }
    }

    fn register(port: u16) -> Option<(usize, usize)> {
        match port {
            0x00..=0x0f => Some((0, usize::from(port))),
            0xc0..=0xde if port & 1 == 0 => Some((1, usize::from((port - 0xc0) / 2))),
            _ => None,
        }
    }

    fn set_half(register: &mut u16, value: u8, high: bool) {
        *register = if high {
            (*register & 0x00ff) | (u16::from(value) << 8)
        } else {
            (*register & 0xff00) | u16::from(value)
        };
    }

    /// Write an 8237 register or AT page register. Unknown ports ignore writes.
    pub fn outb(&mut self, port: u16, value: u8) {
        if let Some(channel) = Self::page_channel(port) {
            self.channels[channel].page = value;
            return;
        }
        let Some((chip, register)) = Self::register(port) else {
            return;
        };
        if register < 8 {
            let index = chip * 4 + register / 2;
            if index != 4 {
                let channel = &mut self.channels[index];
                if register & 1 == 0 {
                    Self::set_half(&mut channel.address, value, self.high_byte[chip]);
                    Self::set_half(&mut channel.base_address, value, self.high_byte[chip]);
                } else {
                    Self::set_half(&mut channel.count, value, self.high_byte[chip]);
                    Self::set_half(&mut channel.base_count, value, self.high_byte[chip]);
                }
                channel.active = true;
            }
            self.high_byte[chip] = !self.high_byte[chip];
            return;
        }
        let index = chip * 4 + usize::from(value & 3);
        match register {
            8 => self.command[chip] = value,
            9 => {
                let bit = 1 << (value & 3);
                if value & 4 == 0 {
                    self.software_request[chip] &= !bit;
                } else {
                    self.software_request[chip] |= bit;
                }
            }
            10 => self.channels[index].masked = value & 4 != 0,
            11 => {
                if index != 4 {
                    self.channels[index].mode = value;
                }
            }
            12 => self.high_byte[chip] = false,
            13 => {
                self.command[chip] = 0;
                self.terminal_count[chip] = 0;
                self.software_request[chip] = 0;
                self.high_byte[chip] = false;
                for channel in &mut self.channels[chip * 4..chip * 4 + 4] {
                    channel.masked = true;
                }
            }
            14 => {
                for channel in &mut self.channels[chip * 4..chip * 4 + 4] {
                    channel.masked = false;
                }
            }
            15 => {
                for (local, channel) in self.channels[chip * 4..chip * 4 + 4].iter_mut().enumerate()
                {
                    channel.masked = value & (1 << local) != 0;
                }
            }
            _ => {}
        }
    }

    /// Read a current address/count, page or status register. Status reads clear
    /// terminal-count flags; other unsupported reads return FFh.
    pub fn inb(&mut self, port: u16) -> u8 {
        if let Some(channel) = Self::page_channel(port) {
            return self.channels[channel].page;
        }
        let Some((chip, register)) = Self::register(port) else {
            return 0xff;
        };
        if register < 8 {
            let index = chip * 4 + register / 2;
            let value = if index == 4 {
                0xffff
            } else if register & 1 == 0 {
                self.channels[index].address
            } else {
                self.channels[index].count
            };
            let byte = if self.high_byte[chip] {
                (value >> 8) as u8
            } else {
                value as u8
            };
            self.high_byte[chip] = !self.high_byte[chip];
            byte
        } else if register == 8 {
            let value = self.terminal_count[chip] | (self.software_request[chip] << 4);
            self.terminal_count[chip] = 0;
            value
        } else {
            0xff
        }
    }

    /// Read one DMA unit and advance the channel only after memory access
    /// succeeds. Masked, disabled, exhausted or incompatible requests return
    /// `Ok(None)` without changing channel state.
    ///
    /// Byte channels wrap within a 64 KiB page. Word channels shift their
    /// 16-bit address by one, ignore page bit zero and wrap within 128 KiB.
    /// Count FFFFh means 65,536 units. Terminal count either reloads the base
    /// registers in auto-init mode or stops the channel until reprogrammed.
    pub fn transfer(
        &mut self,
        request: DmaRequest,
        memory: &ConventionalMemory,
    ) -> Result<Option<DmaUnit>> {
        let index = usize::from(request.channel.number());
        let chip = index / 4;
        let channel = self.channels[index];
        if request.direction != DmaDirection::MemoryToDevice
            || channel.masked
            || !channel.active
            || self.command[chip] & 4 != 0
            || channel.mode & 0x0c != 0x08
            || channel.mode & 0xc0 == 0xc0
        {
            return Ok(None);
        }
        let unit = if chip == 0 {
            let address = (u32::from(channel.page) << 16) | u32::from(channel.address);
            DmaUnit::Byte(memory.read_byte(address)?)
        } else {
            let address =
                (u32::from(channel.page & 0xfe) << 16) | (u32::from(channel.address) << 1);
            let mut bytes = [0; 2];
            memory.read(address, &mut bytes)?;
            DmaUnit::Word(u16::from_le_bytes(bytes))
        };
        let current = &mut self.channels[index];
        current.address = if channel.mode & 0x20 != 0 {
            channel.address.wrapping_sub(1)
        } else {
            channel.address.wrapping_add(1)
        };
        current.count = channel.count.wrapping_sub(1);
        if channel.count == 0 {
            self.terminal_count[chip] |= 1 << (index % 4);
            self.software_request[chip] &= !(1 << (index % 4));
            if channel.mode & 0x10 != 0 {
                current.address = current.base_address;
                current.count = current.base_count;
            } else {
                current.active = false;
            }
        }
        Ok(Some(unit))
    }
}
