use crate::{Error, OplChip, Result, Tick};

/// A successful game-thread port write and its absolute machine timestamp.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PortWrite {
    /// Timestamp supplied to the bus.
    pub at: Tick,
    /// I/O port number.
    pub port: u16,
    /// Value written.
    pub value: u8,
}

/// A game-thread register trace. Capturing allocates only on the control thread.
///
/// Music events scheduled directly inside `AudioEngine` are not port accesses
/// and are not included. A capture preserves writes, not a snapshot of a playing
/// chip: start it before programming instruments for a self-contained replay.
#[derive(Clone, Debug)]
pub struct Capture {
    pub(crate) start: Tick,
    pub(crate) end: Tick,
    pub(crate) chip: Option<OplChip>,
    pub(crate) address: [u8; 2],
    pub(crate) opl_alias: Option<u16>,
    pub(crate) writes: Vec<PortWrite>,
}
impl Capture {
    /// Recorded port writes, in dispatch order.
    pub fn writes(&self) -> &[PortWrite] {
        &self.writes
    }
    /// Capture duration through the time `finish_capture` was called.
    pub fn duration(&self) -> Tick {
        self.end - self.start
    }
    /// Export OPL accesses as a legacy v1 DRO capture with millisecond timing.
    ///
    /// Absolute relative timestamps are rounded to the nearest millisecond, preventing
    /// cumulative delay drift. Speaker and unrelated port writes are omitted.
    pub fn to_dro(&self) -> Result<Vec<u8>> {
        let chip = self
            .chip
            .ok_or(Error::Config("DRO export requires installed OPL hardware"))?;
        let milliseconds = |at: Tick| -> Result<u32> {
            let elapsed = at.checked_sub(self.start).ok_or(Error::BackwardsTime)?;
            u32::try_from(
                (u128::from(elapsed.0) * u128::from(Tick::HZ_DENOMINATOR) * 1_000
                    + u128::from(Tick::HZ_NUMERATOR) / 2)
                    / u128::from(Tick::HZ_NUMERATOR),
            )
            .map_err(|_| Error::TooLarge)
        };
        let mut data = Vec::new();
        let mut address = self.address;
        let mut bank = 0usize;
        let mut time = 0u32;
        for write in &self.writes {
            let port = if let Some(alias) = self.opl_alias
                && (alias..=alias + 1).contains(&write.port)
            {
                0x388 + write.port - alias
            } else {
                write.port
            };
            if !(0x388..=0x38b).contains(&port) {
                continue;
            }
            let target = usize::from(port >= 0x38a);
            if target == 1 && chip == OplChip::Opl2 {
                continue;
            }
            if port & 1 == 0 {
                address[target] = write.value;
                continue;
            }
            let next = milliseconds(write.at)?;
            delay(&mut data, next - time);
            time = next;
            if bank != target {
                data.push(if target == 0 { 2 } else { 3 });
                bank = target;
            }
            let register = address[target];
            if register <= 4 {
                data.push(4);
            }
            data.extend([register, write.value]);
        }
        let end = milliseconds(self.end)?;
        delay(&mut data, end - time);
        let size = u32::try_from(data.len()).map_err(|_| Error::TooLarge)?;
        let mut bytes = Vec::with_capacity(data.len().checked_add(24).ok_or(Error::TooLarge)?);
        bytes.extend(b"DBRAWOPL");
        bytes.extend(0u16.to_le_bytes());
        bytes.extend(1u16.to_le_bytes());
        bytes.extend(end.to_le_bytes());
        bytes.extend(size.to_le_bytes());
        bytes.extend(
            match chip {
                OplChip::Opl2 => 0u32,
                OplChip::Opl3 => 1,
                OplChip::DualOpl2 => 2,
            }
            .to_le_bytes(),
        );
        bytes.extend(data);
        Ok(bytes)
    }
}

fn delay(data: &mut Vec<u8>, mut milliseconds: u32) {
    while milliseconds > 0 {
        let amount = milliseconds.min(65_536);
        if amount <= 256 {
            data.extend([0, (amount - 1) as u8]);
        } else {
            data.push(1);
            data.extend(((amount - 1) as u16).to_le_bytes());
        }
        milliseconds -= amount;
    }
}
