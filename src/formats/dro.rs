use super::{Builder, Reader};
use crate::{Error, OplChip, Result, Sequence};

/// DOSBox raw OPL captures, legacy v1 (also called 0.1) and uncompressed v2.
pub struct Dro;
impl Dro {
    /// Decode a capture, deriving duration from delays rather than header metadata.
    pub fn load(bytes: &[u8]) -> Result<Sequence> {
        let mut input = Reader::new(bytes);
        if input.take(8)? != b"DBRAWOPL" {
            return Err(Error::Format("invalid DRO signature"));
        }
        let version = (input.u16()?, input.u16()?);
        match version {
            (0, 1) | (1, 0) => Self::v1(input),
            (2, 0) => Self::v2(input),
            _ => Err(Error::Format("unsupported DRO version")),
        }
    }

    fn v1(mut input: Reader<'_>) -> Result<Sequence> {
        let _duration_ms = input.u32()?;
        let length = usize::try_from(input.u32()?).map_err(|_| Error::TooLarge)?;
        let chip = match input.u8()? {
            0 => OplChip::Opl2,
            1 => OplChip::Opl3,
            2 => OplChip::DualOpl2,
            _ => return Err(Error::Format("invalid DRO v1 hardware type")),
        };
        // v1 existed with both one-byte and four-byte hardware fields.
        if input.remaining() != length && input.take(3)? != [0, 0, 0] {
            return Err(Error::Format("invalid DRO v1 hardware padding"));
        }
        let data = input.take(length)?;
        if input.remaining() != 0 {
            return Err(Error::Format("trailing DRO v1 data"));
        }
        let mut data = Reader::new(data);
        let mut output = Builder::new(chip, 1_000)?;
        let mut bank = 0u16;
        while data.remaining() > 0 {
            match data.u8()? {
                0 => output.delay(u64::from(data.u8()?) + 1)?,
                1 => output.delay(u64::from(data.u16()?) + 1)?,
                2 => bank = 0,
                3 => bank = 0x100,
                4 => {
                    let reg = data.u8()?;
                    output.write(bank | u16::from(reg), data.u8()?);
                }
                reg => output.write(bank | u16::from(reg), data.u8()?),
            }
        }
        output.finish()
    }

    fn v2(mut input: Reader<'_>) -> Result<Sequence> {
        let pairs = usize::try_from(input.u32()?).map_err(|_| Error::TooLarge)?;
        let _duration_ms = input.u32()?;
        let chip = match input.u8()? {
            0 => OplChip::Opl2,
            1 => OplChip::DualOpl2,
            2 => OplChip::Opl3,
            _ => return Err(Error::Format("invalid DRO v2 hardware type")),
        };
        if input.u8()? != 0 || input.u8()? != 0 {
            return Err(Error::Format("unsupported DRO arrangement or compression"));
        }
        let short = input.u8()?;
        let long = input.u8()?;
        let count = usize::from(input.u8()?);
        if count > 128 || short == long {
            return Err(Error::Format("invalid DRO delay codes or register map"));
        }
        let map = input.take(count)?;
        let data = input.take(pairs.checked_mul(2).ok_or(Error::TooLarge)?)?;
        if input.remaining() != 0 {
            return Err(Error::Format("trailing DRO v2 data"));
        }
        let mut output = Builder::new(chip, 1_000)?;
        for pair in data.as_chunks::<2>().0 {
            let (command, value) = (pair[0], pair[1]);
            if command == short {
                output.delay(u64::from(value) + 1)?;
            } else if command == long {
                output.delay((u64::from(value) + 1) * 256)?;
            } else {
                let reg = *map
                    .get(usize::from(command & 0x7f))
                    .ok_or(Error::Format("DRO register code is outside the map"))?;
                output.write(u16::from(reg) | (u16::from(command & 0x80) << 1), value);
            }
        }
        output.finish()
    }
}
