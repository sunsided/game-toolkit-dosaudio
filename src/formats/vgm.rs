use crate::{
    Error, MusicEvent, MusicTarget, OplChip, PreparedMusic, Result, Tick, TickAccumulator,
    TimedEvent,
};

const MAX_BYTES: usize = 64 * 1024 * 1024;
const MAX_COMMANDS: usize = 1_000_000;
const DUAL: u32 = 0x4000_0000;
const HARD_PAN: u32 = 0x8000_0000;

/// Uncompressed VGM register logs for YM3812, YM3526, YMF262 and Tandy PSG.
///
/// Header/command definitions: [VGM specification 1.71](https://github.com/vgmrips/vgmplay-legacy/blob/master/VGMPlay/vgmspec171.txt).
/// Waits retain their exact cumulative 44,100 Hz phase; durations and loop starts
/// come from parsed commands rather than sample-count metadata.
///
/// Fixed chip clocks must round to 3,579,545 Hz (OPL2/PSG) or 14,318,180 Hz
/// (OPL3), within two Hz. PSG requires explicit 15-bit noise width and feedback
/// mask 0011h; Sega's default 16-bit/0009h noise is unsupported. In v1.51+ the
/// PSG flags must select divisor-zero-as-1024, with ordinary polarity, divider
/// and XOR feedback. PSG output is mono; Game Gear stereo commands are rejected.
/// Dual YM3812 requires both dual-chip and Sound Blaster Pro hard-pan flags;
/// mono dual OPL2, dual YM3526 and dual OPL3 cannot match the installed routing.
///
/// Gzip/VGZ, other chips, data blocks, extra headers, volume modifiers, undefined
/// commands (including the unpublished 64h wait-override proposal) are errors.
/// Missing chip declarations may be inferred from supported commands at the fixed
/// clock. The EOF field must match the supplied input length exactly.
/// Input is bounded to 64 MiB and one million commands. GD3 tags are bounds-
/// checked but their descriptive text and playback loop-count modifiers are ignored.
pub struct Vgm;

impl Vgm {
    /// Decode a complete VGM file into owned, target-validated chip events.
    pub fn load(bytes: &[u8]) -> Result<PreparedMusic> {
        if bytes.starts_with(&[0x1f, 0x8b]) {
            return Err(Error::Format("compressed VGM/VGZ is unsupported"));
        }
        if bytes.len() > MAX_BYTES {
            return Err(Error::TooLarge);
        }
        if bytes.len() < 0x40 || &bytes[..4] != b"Vgm " {
            return Err(Error::Format("invalid VGM header"));
        }
        let eof = relative(bytes, 4)?.ok_or(Error::Format("VGM requires an EOF offset"))?;
        if eof != bytes.len() {
            return Err(Error::Format("VGM EOF offset does not match input length"));
        }
        let version = le32(bytes, 8)?;
        if !matches!(
            version,
            0x100 | 0x101 | 0x110 | 0x150 | 0x151 | 0x160 | 0x161 | 0x170 | 0x171
        ) {
            return Err(Error::Format("unsupported VGM specification version"));
        }
        let data = if version >= 0x150 {
            relative(bytes, 0x34)?.unwrap_or(0x40)
        } else {
            0x40
        };
        if !(0x40..eof).contains(&data) {
            return Err(Error::Format("VGM data offset is outside its file"));
        }
        let header = Header {
            bytes: &bytes[..data],
            version,
        };
        if data > 0x100 && header.bytes[0x100..].iter().any(|&byte| byte != 0) {
            return Err(Error::Format("unsupported nonzero VGM header extension"));
        }
        let gd3 = relative(bytes, 0x14)?;
        if let Some(at) = gd3 {
            validate_gd3(bytes, at, data)?;
        }
        let limit = gd3.unwrap_or(eof);
        if limit <= data {
            return Err(Error::Format("VGM GD3 overlaps command data"));
        }
        let loop_offset = relative(bytes, 0x1c)?;
        if loop_offset.is_some_and(|at| at < data || at >= limit) {
            return Err(Error::Format("VGM loop offset is outside command data"));
        }
        let mut chips = Chips::new(&header)?;
        let mut position = data;
        let mut clock = TickAccumulator::new(44_100);
        let mut now = Tick::ZERO;
        let mut loop_start = None;
        let mut events = Vec::new();
        for _ in 0..MAX_COMMANDS {
            let command_at = position;
            let command = take(bytes, &mut position, 1, limit)?[0];
            if loop_offset == Some(command_at) {
                if command == 0x66 {
                    return Err(Error::Format("VGM loop points at EndOfData"));
                }
                loop_start = Some((now, events.len()));
            }
            let wait = match command {
                0x00 => 0,
                0x50 => {
                    let value = take(bytes, &mut position, 1, limit)?[0];
                    chips.psg = true;
                    events.push(TimedEvent {
                        at: now,
                        event: MusicEvent::Psg(value),
                    });
                    0
                }
                0x5a | 0x5b | 0x5e | 0x5f | 0xaa | 0xab => {
                    if version < 0x151 {
                        return Err(Error::Format("VGM FM command requires version 1.51"));
                    }
                    let data = take(bytes, &mut position, 2, limit)?;
                    let (family, bank, second) = match command {
                        0x5a => (Family::Ym3812, 0, false),
                        0x5b => (Family::Ym3526, 0, false),
                        0x5e => (Family::Ymf262, 0, false),
                        0x5f => (Family::Ymf262, 0x100, false),
                        0xaa => (Family::Ym3812, 0x100, true),
                        0xab => (Family::Ym3526, 0x100, true),
                        _ => unreachable!("matched FM command"),
                    };
                    chips.observe(family, second)?;
                    events.push(TimedEvent {
                        at: now,
                        event: MusicEvent::Opl {
                            register: bank | u16::from(data[0]),
                            value: data[1],
                        },
                    });
                    0
                }
                0x61 => u64::from(u16::from_le_bytes(
                    take(bytes, &mut position, 2, limit)?
                        .try_into()
                        .expect("two bytes"),
                )),
                0x62 => 735,
                0x63 => 882,
                0x70..=0x7f => u64::from((command & 15) + 1),
                0x66 => {
                    if loop_offset.is_some() && loop_start.is_none() {
                        return Err(Error::Format("VGM loop offset is not a command boundary"));
                    }
                    if bytes[position..limit].iter().any(|&byte| byte != 0) {
                        return Err(Error::Format("unexpected VGM data after EndOfData"));
                    }
                    let target = chips.target(&header)?;
                    let music = PreparedMusic::new(target, events, now, Vec::<u8>::new())?;
                    return if let Some((at, index)) = loop_start {
                        music.with_loop_boundary(at, index)
                    } else {
                        Ok(music)
                    };
                }
                0x30 | 0x3f => return Err(Error::Format("dual PSG is unsupported")),
                0x4f => {
                    return Err(Error::Format(
                        "Game Gear stereo configuration is unsupported",
                    ));
                }
                0x5c | 0xac => return Err(Error::Format("Y8950 ADPCM is unsupported")),
                0xae | 0xaf => return Err(Error::Format("dual OPL3 is unsupported")),
                0x64 => {
                    return Err(Error::Format(
                        "VGM 64h wait override is not in the supported published specification",
                    ));
                }
                0x67 | 0x68 | 0x80..=0x95 | 0xe0 => {
                    return Err(Error::Format("VGM PCM/data commands are unsupported"));
                }
                _ => return Err(Error::Format("unsupported VGM chip or undefined command")),
            };
            let delta = clock.checked_advance(wait).ok_or(Error::TooLarge)?;
            now = now.checked_add(delta).ok_or(Error::TooLarge)?;
        }
        Err(Error::TooLarge)
    }
}

fn le32(bytes: &[u8], at: usize) -> Result<u32> {
    let end = at.checked_add(4).ok_or(Error::TooLarge)?;
    Ok(u32::from_le_bytes(
        bytes
            .get(at..end)
            .ok_or(Error::Format("truncated VGM header"))?
            .try_into()
            .expect("four bytes"),
    ))
}
fn relative(bytes: &[u8], at: usize) -> Result<Option<usize>> {
    let offset = le32(bytes, at)?;
    if offset == 0 {
        return Ok(None);
    }
    at.checked_add(usize::try_from(offset).map_err(|_| Error::TooLarge)?)
        .map(Some)
        .ok_or(Error::TooLarge)
}
fn take<'a>(bytes: &'a [u8], position: &mut usize, count: usize, limit: usize) -> Result<&'a [u8]> {
    let end = position.checked_add(count).ok_or(Error::TooLarge)?;
    if end > limit {
        return Err(Error::Format("truncated VGM command or missing EndOfData"));
    }
    let data = bytes
        .get(*position..end)
        .ok_or(Error::Format("truncated VGM data"))?;
    *position = end;
    Ok(data)
}
fn validate_gd3(bytes: &[u8], at: usize, data: usize) -> Result<()> {
    let header_end = at.checked_add(12).ok_or(Error::TooLarge)?;
    if at < data || bytes.get(at..header_end).is_none() || bytes.get(at..at + 4) != Some(b"Gd3 ") {
        return Err(Error::Format("invalid VGM GD3 offset or signature"));
    }
    if le32(bytes, at + 4)? != 0x100 {
        return Err(Error::Format("unsupported GD3 version"));
    }
    let length = usize::try_from(le32(bytes, at + 8)?).map_err(|_| Error::TooLarge)?;
    let end = header_end.checked_add(length).ok_or(Error::TooLarge)?;
    if length % 2 != 0 || end != bytes.len() {
        return Err(Error::Format("invalid GD3 payload length"));
    }
    Ok(())
}

struct Header<'a> {
    bytes: &'a [u8],
    version: u32,
}
impl Header<'_> {
    fn field(&self, at: usize, since: u32) -> u32 {
        if self.version < since {
            return 0;
        }
        let mut raw = [0; 4];
        for (index, byte) in raw.iter_mut().enumerate() {
            *byte = self.bytes.get(at + index).copied().unwrap_or(0);
        }
        u32::from_le_bytes(raw)
    }
    fn byte(&self, at: usize, since: u32) -> u8 {
        self.field(at, since) as u8
    }
}
#[derive(Clone, Copy, Eq, PartialEq)]
enum Family {
    Ym3812,
    Ym3526,
    Ymf262,
}
struct Chips {
    fm: Option<Family>,
    dual: bool,
    psg: bool,
}
impl Chips {
    fn new(header: &Header<'_>) -> Result<Self> {
        // Clock fields only exist before the declared data offset and at their
        // introducing version; command bytes must never masquerade as clocks.
        for (since, fields) in [
            (0x100, &[0x10][..]),
            (0x110, &[0x2c, 0x30][..]),
            (
                0x151,
                &[
                    0x38, 0x40, 0x44, 0x48, 0x4c, 0x58, 0x60, 0x64, 0x68, 0x6c, 0x70, 0x74,
                ][..],
            ),
            (
                0x161,
                &[
                    0x80, 0x84, 0x88, 0x8c, 0x90, 0x98, 0x9c, 0xa0, 0xa4, 0xa8, 0xac, 0xb0, 0xb4,
                ][..],
            ),
            (
                0x171,
                &[
                    0xb8, 0xc0, 0xc4, 0xc8, 0xcc, 0xd0, 0xd8, 0xdc, 0xe0, 0xe4, 0xe8, 0xec, 0xf0,
                    0xf4, 0xf8, 0xfc,
                ][..],
            ),
        ] {
            if fields.iter().any(|&at| header.field(at, since) != 0) {
                return Err(Error::Format("VGM declares an unsupported sound chip"));
            }
        }
        if header.field(0xbc, 0x170) != 0 {
            return Err(Error::Format(
                "VGM extra clock/volume headers are unsupported",
            ));
        }
        if header.byte(0x7c, 0x150) != 0 {
            return Err(Error::Format("VGM volume modifiers are unsupported"));
        }
        let mut chips = Self {
            fm: None,
            dual: false,
            psg: false,
        };
        for (at, family) in [
            (0x50, Family::Ym3812),
            (0x54, Family::Ym3526),
            (0x5c, Family::Ymf262),
        ] {
            let raw = header.field(at, 0x151);
            if raw == 0 {
                continue;
            }
            if chips.fm.is_some() {
                return Err(Error::Format("multiple VGM FM families are unsupported"));
            }
            let flags = raw & 0xc000_0000;
            chips.dual = flags == (DUAL | HARD_PAN) && family == Family::Ym3812;
            if flags != 0 && !chips.dual {
                return Err(Error::Format(
                    "unsupported VGM dual/mono FM routing or clock flag",
                ));
            }
            check_clock(
                raw & 0x3fff_ffff,
                if family == Family::Ymf262 {
                    14_318_180
                } else {
                    3_579_545
                },
            )?;
            chips.fm = Some(family);
        }
        let psg = header.field(0x0c, 0x100);
        if psg != 0 {
            if psg & 0xc000_0000 != 0 {
                return Err(Error::Format("dual PSG/T6W28 is unsupported"));
            }
            check_clock(psg, 3_579_545)?;
            chips.psg = true;
        }
        Ok(chips)
    }
    fn observe(&mut self, family: Family, second: bool) -> Result<()> {
        if second && !(self.dual && family == Family::Ym3812) {
            return Err(Error::Format(
                "second OPL2 requires declared hard-panned dual YM3812",
            ));
        }
        if self.fm.is_some_and(|declared| declared != family) {
            return Err(Error::Format(
                "VGM command conflicts with declared FM hardware",
            ));
        }
        self.fm = Some(family);
        Ok(())
    }
    fn target(&self, header: &Header<'_>) -> Result<MusicTarget> {
        if self.psg {
            let feedback = header.field(0x28, 0x110) & 0xffff;
            let width = header.byte(0x2a, 0x110);
            let flags = header.byte(0x2b, 0x151);
            if feedback != 0x11
                || width != 15
                || flags & !5 != 0
                || header.version >= 0x151 && flags & 1 == 0
            {
                return Err(Error::Format(
                    "VGM PSG requires Tandy 15-bit/0011h noise and divisor-zero-as-1024 flags",
                ));
            }
        }
        let opl = self.fm.map(|family| {
            if self.dual {
                OplChip::DualOpl2
            } else if family == Family::Ymf262 {
                OplChip::Opl3
            } else {
                OplChip::Opl2
            }
        });
        match (opl, self.psg) {
            (Some(chip), true) => Ok(MusicTarget::OplPsg(chip)),
            (Some(chip), false) => Ok(MusicTarget::Opl(chip)),
            (None, true) => Ok(MusicTarget::Psg),
            (None, false) => Err(Error::Format(
                "VGM has no supported declared or observed chip",
            )),
        }
    }
}
fn check_clock(value: u32, expected: u32) -> Result<()> {
    if value.abs_diff(expected) > 2 {
        return Err(Error::Format(
            "VGM chip clock differs from supported fixed hardware clock",
        ));
    }
    Ok(())
}
