//! Music files decoded into immutable, timestamped sequences before playback.
mod dro;
mod imf;
pub use dro::Dro;
pub use imf::Imf;

use crate::{Error, MusicEvent, OplChip, Result, Sequence, Tick, TickAccumulator, TimedEvent};

struct Builder {
    events: Vec<TimedEvent>,
    now: Tick,
    clock: TickAccumulator,
    chip: OplChip,
}
impl Builder {
    fn new(chip: OplChip, rate: u32) -> Result<Self> {
        if rate == 0 {
            return Err(Error::Format("music tick rate must be nonzero"));
        }
        Ok(Self {
            events: Vec::new(),
            now: Tick::ZERO,
            clock: TickAccumulator::new(u64::from(rate)),
            chip,
        })
    }
    fn write(&mut self, register: u16, value: u8) {
        // DOSBox includes high-bank initialization in some OPL2 captures.
        if self.chip == OplChip::Opl2 && register > 0xff {
            return;
        }
        self.events.push(TimedEvent {
            at: self.now,
            event: MusicEvent::Opl { register, value },
        });
    }
    fn delay(&mut self, ticks: u64) -> Result<()> {
        let delta = self.clock.checked_advance(ticks).ok_or(Error::TooLarge)?;
        self.now = self.now.checked_add(delta).ok_or(Error::TooLarge)?;
        Ok(())
    }
    fn finish(self) -> Result<Sequence> {
        Sequence::new(self.chip, self.events, self.now)
    }
}

struct Reader<'a> {
    bytes: &'a [u8],
    position: usize,
}
impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, position: 0 }
    }
    fn take(&mut self, length: usize) -> Result<&'a [u8]> {
        let end = self.position.checked_add(length).ok_or(Error::TooLarge)?;
        let data = self
            .bytes
            .get(self.position..end)
            .ok_or(Error::Format("truncated music file"))?;
        self.position = end;
        Ok(data)
    }
    fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_le_bytes(
            self.take(2)?.try_into().expect("two bytes"),
        ))
    }
    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(
            self.take(4)?.try_into().expect("four bytes"),
        ))
    }
    fn remaining(&self) -> usize {
        self.bytes.len() - self.position
    }
}
