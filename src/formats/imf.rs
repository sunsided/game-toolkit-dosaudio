use super::{Builder, Reader};
use crate::{Error, OplChip, Result, Sequence};

/// id Software IMF files. The caller supplies the game's tick rate (often 560 or 700 Hz).
pub struct Imf;
impl Imf {
    /// Load a headerless type-0 file or an exact-length type-1 file.
    ///
    /// IMF has no signature, so the formats are ambiguous. This selects type 1
    /// only if its nonempty declared data ends exactly at EOF; use the explicit
    /// loaders for type-1 files with trailing metadata or ambiguous type-0 data.
    pub fn load(bytes: &[u8], tick_hz: u32) -> Result<Sequence> {
        if bytes.len() >= 2 {
            let length = usize::from(u16::from_le_bytes([bytes[0], bytes[1]]));
            if length > 0 && length % 4 == 0 && length + 2 == bytes.len() {
                return Self::load_type1(bytes, tick_hz);
            }
        }
        Self::load_type0(bytes, tick_hz)
    }

    /// Load headerless register/value/u16-delay records.
    pub fn load_type0(bytes: &[u8], tick_hz: u32) -> Result<Sequence> {
        if !bytes.len().is_multiple_of(4) {
            return Err(Error::Format("IMF data length must be a multiple of four"));
        }
        let mut output = Builder::new(OplChip::Opl2, tick_hz)?;
        let mut input = Reader::new(bytes);
        while input.remaining() > 0 {
            let register = input.u8()?;
            let value = input.u8()?;
            output.write(u16::from(register), value);
            output.delay(u64::from(input.u16()?))?;
        }
        output.finish()
    }

    /// Load a u16 byte-length prefix and its records. Trailing metadata is ignored.
    pub fn load_type1(bytes: &[u8], tick_hz: u32) -> Result<Sequence> {
        let mut input = Reader::new(bytes);
        let length = usize::from(input.u16()?);
        Self::load_type0(input.take(length)?, tick_hz)
    }
}
