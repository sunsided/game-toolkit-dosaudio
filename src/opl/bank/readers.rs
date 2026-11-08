use super::*;

fn name(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).into_owned()
}
fn range(bytes: &[u8], offset: usize, count: usize) -> Result<&[u8]> {
    bytes
        .get(offset..offset.checked_add(count).ok_or(Error::TooLarge)?)
        .ok_or(Error::Format("truncated OPL bank or invalid offset"))
}
fn le16(bytes: &[u8]) -> u16 {
    u16::from_le_bytes([bytes[0], bytes[1]])
}
fn be16(bytes: &[u8]) -> u16 {
    u16::from_be_bytes([bytes[0], bytes[1]])
}

impl OplBank {
    /// Load one SBI instrument as melodic bank 0, program 0.
    pub fn from_sbi(bytes: &[u8]) -> Result<Self> {
        let patch = OplPatch::from_sbi(bytes)?;
        let mut bank = Self::new();
        bank.insert(OplBankEntry {
            key: OplBankKey {
                msb: 0,
                lsb: 0,
                program: 0,
                percussion: false,
            },
            name: name(&bytes[4..36]),
            instrument: patch.into(),
        })?;
        Ok(bank)
    }

    /// Load DMX OP2 (`#OPL_II#`), 128 melodic and 47 percussion entries.
    /// Requires the 175 fixed 36-byte records, with either no name table or a
    /// complete 175-by-32-byte name table. Fixed notes, transposition and layered
    /// two-voice instruments are retained. Delayed-vibrato flag 02h is rejected.
    /// Format reference: <https://github.com/Wohlstand/OPL3BankEditor/blob/master/Specifications/DMX%20OP2%20Bank%20Format.txt>.
    pub fn from_op2(bytes: &[u8]) -> Result<Self> {
        const RECORD_END: usize = 8 + 175 * 36;
        if !(bytes.len() == RECORD_END || bytes.len() == RECORD_END + 175 * 32)
            || !bytes.starts_with(b"#OPL_II#")
        {
            return Err(Error::Format(
                "invalid OP2 signature or record/name-table length",
            ));
        }
        let mut bank = Self::new();
        for index in 0..175 {
            let d = &bytes[8 + index * 36..8 + (index + 1) * 36];
            let flags = le16(d);
            if flags & !5 != 0 {
                return Err(Error::Format("unsupported OP2 instrument flags"));
            }
            let decode = |voice: &[u8]| {
                let operator = |o: &[u8]| OplOperator {
                    characteristics: o[0],
                    attack_decay: o[1],
                    sustain_release: o[2],
                    waveform: o[3],
                    level: (o[4] & 0xc0) | (o[5] & 63),
                };
                OplPatch {
                    modulator: operator(voice),
                    carrier: operator(&voice[7..]),
                    feedback: voice[6] & 15,
                }
            };
            let fixed_note = if flags & 1 != 0 { Some(d[3]) } else { None };
            let instrument = OplInstrument {
                patch: decode(&d[4..20]),
                second: (flags & 4 != 0).then(|| decode(&d[20..36])),
                note_offsets: if fixed_note.is_some() {
                    [0, 0]
                } else {
                    [le16(&d[18..20]) as i16, le16(&d[34..36]) as i16]
                },
                detune: (i16::from(d[2]) - 128) as i8,
                fixed_note,
                velocity_offset: 0,
            };
            let key = OplBankKey {
                msb: 0,
                lsb: 0,
                program: if index < 128 {
                    index as u8
                } else {
                    (index - 128 + 35) as u8
                },
                percussion: index >= 128,
            };
            let title = if bytes.len() > RECORD_END {
                name(&bytes[RECORD_END + index * 32..RECORD_END + (index + 1) * 32])
            } else {
                String::new()
            };
            bank.insert(OplBankEntry {
                key,
                name: title,
                instrument,
            })?;
        }
        Ok(bank)
    }

    /// Load WOPL versions 1–3, supporting two-operator and pseudo-four-operator
    /// instruments, bank-select metadata, fixed notes and depth flags.
    /// True four-operator/rhythm instruments, MT-32 defaults and nonzero ADLMIDI
    /// volume-model emulations are errors. Blank entries are omitted; v3 busy-time
    /// hints are ignored because this driver steals by note age. Bank counts are
    /// limited to 128 per family. Version-1 banks use file order as MSB, LSB zero.
    /// Extra/truncated data and duplicate banks fail.
    /// Format: <https://github.com/Wohlstand/OPL3BankEditor/blob/master/Specifications/WOPL-and-OPLI-Specification.txt>.
    pub fn from_wopl(bytes: &[u8]) -> Result<Self> {
        let header = range(bytes, 0, 19)?;
        if &header[..11] != b"WOPL3-BANK\0" {
            return Err(Error::Format("invalid WOPL signature"));
        }
        let version = le16(&header[11..]);
        if !(1..=3).contains(&version) {
            return Err(Error::Format("unsupported WOPL version"));
        }
        let melodic = usize::from(be16(&header[13..]));
        let percussion = usize::from(be16(&header[15..]));
        if melodic + percussion == 0 || melodic > 128 || percussion > 128 {
            return Err(Error::Format("unsupported WOPL bank count"));
        }
        if header[17] & !3 != 0 || header[18] != 0 {
            return Err(Error::Format(
                "unsupported WOPL global flags or volume model",
            ));
        }
        let count = melodic + percussion;
        let size = if version == 3 { 66 } else { 62 };
        let metadata = if version >= 2 { count * 34 } else { 0 };
        if bytes.len() != 19 + metadata + count * 128 * size {
            return Err(Error::Format("invalid WOPL record length"));
        }
        let mut bank = Self::new();
        bank.depth = ((header[17] & 1) << 7) | ((header[17] & 2) << 5);
        let mut addresses = Vec::with_capacity(count);
        for b in 0..count {
            let (lsb, msb) = if version >= 2 {
                (bytes[19 + b * 34 + 32], bytes[19 + b * 34 + 33])
            } else {
                (
                    0,
                    if b < melodic {
                        b as u8
                    } else {
                        (b - melodic) as u8
                    },
                )
            };
            let address = (b >= melodic, msb, lsb);
            if msb > 127 || lsb > 127 || addresses.contains(&address) {
                return Err(Error::Format("invalid or duplicate WOPL bank address"));
            }
            addresses.push(address);
            for program in 0..128 {
                let offset = 19 + metadata + (b * 128 + program) * size;
                let d = &bytes[offset..offset + size];
                if d[39] & 4 != 0 {
                    continue;
                }
                if d[39] & !0x42 != 0 {
                    return Err(Error::Format(
                        "unsupported WOPL four-operator, rhythm or reserved instrument flags",
                    ));
                }
                let operator = |o: &[u8]| OplOperator {
                    characteristics: o[0],
                    level: o[1],
                    attack_decay: o[2],
                    sustain_release: o[3],
                    waveform: o[4],
                };
                let patch = |i: usize| OplPatch {
                    modulator: operator(&d[47 + i * 10..]),
                    carrier: operator(&d[42 + i * 10..]),
                    feedback: d[40 + i] & 15,
                };
                bank.insert(OplBankEntry {
                    key: OplBankKey {
                        msb,
                        lsb,
                        program: program as u8,
                        percussion: b >= melodic,
                    },
                    name: name(&d[..32]),
                    instrument: OplInstrument {
                        patch: patch(0),
                        second: (d[39] & 2 != 0).then(|| patch(1)),
                        note_offsets: [be16(&d[32..]) as i16, be16(&d[34..]) as i16],
                        detune: d[37] as i8,
                        fixed_note: if b >= melodic || d[39] & 0x40 != 0 {
                            Some(d[38])
                        } else {
                            None
                        },
                        velocity_offset: d[36] as i8,
                    },
                })?;
            }
        }
        Ok(bank)
    }

    /// Load AdLib BNK version 1.0 (28-byte header, 12-byte names, 30-byte timbres).
    /// Melodic entries become bank-0 programs in used-name-table order; names and
    /// referenced timbres are resolved through the name-table indices. At most 128 used
    /// instruments/4,096 table slots are accepted. Rhythm-mode timbres and other
    /// BNK families (HMI/BNK2) are unsupported. BNK itself defines no GM mapping.
    /// Layout reference: <https://github.com/adplug/adplug/blob/master/src/composer.cpp>.
    pub fn from_bnk(bytes: &[u8]) -> Result<Self> {
        let h = range(bytes, 0, 28)?;
        if h[..2] != [1, 0] || &h[2..8] != b"ADLIB-" {
            return Err(Error::Format("unsupported BNK version or signature"));
        }
        let used = usize::from(le16(&h[8..]));
        let total = usize::from(le16(&h[10..]));
        if used > 128 || used > total || total > 4096 {
            return Err(Error::Format("unsupported BNK instrument count"));
        }
        let offset = |d: &[u8]| u32::from_le_bytes([d[0], d[1], d[2], d[3]]) as usize;
        let names_at = offset(&h[12..]);
        let data_at = offset(&h[16..]);
        let names = range(bytes, names_at, total * 12)?;
        let data = range(bytes, data_at, total * 30)?;
        if names_at < 28
            || data_at < 28
            || names_at < data_at + data.len() && data_at < names_at + names.len()
        {
            return Err(Error::Format("overlapping BNK regions"));
        }
        let mut bank = Self::new();
        let mut count = 0;
        let mut seen = vec![false; total];
        for entry in names.as_chunks::<12>().0 {
            if entry[2] > 1 {
                return Err(Error::Format("invalid BNK used flag"));
            }
            if entry[2] == 0 {
                continue;
            }
            let index = usize::from(le16(entry));
            if index >= total || seen[index] || count >= used {
                return Err(Error::Format("invalid or duplicate BNK data index/count"));
            }
            seen[index] = true;
            let d = &data[index * 30..(index + 1) * 30];
            if d[0] != 0 {
                return Err(Error::Format("BNK rhythm-mode instruments are unsupported"));
            }
            let operator = |o: &[u8], waveform: u8| -> Result<OplOperator> {
                let limits = [3, 15, 7, 15, 15, 1, 15, 15, 63, 1, 1, 1, 1];
                if o.iter().zip(limits).any(|(&v, limit)| v > limit) || waveform > 3 {
                    return Err(Error::Format("invalid BNK operator bit field"));
                }
                Ok(OplOperator {
                    characteristics: (o[9] << 7) | (o[10] << 6) | (o[5] << 5) | (o[11] << 4) | o[1],
                    level: (o[0] << 6) | o[8],
                    attack_decay: (o[3] << 4) | o[6],
                    sustain_release: (o[4] << 4) | o[7],
                    waveform,
                })
            };
            let patch = OplPatch {
                modulator: operator(&d[2..15], d[28])?,
                carrier: operator(&d[15..28], d[29])?,
                feedback: (d[4] << 1) | (d[14] ^ 1),
            };
            bank.insert(OplBankEntry {
                key: OplBankKey {
                    msb: 0,
                    lsb: 0,
                    program: count as u8,
                    percussion: false,
                },
                name: name(&entry[3..12]),
                instrument: patch.into(),
            })?;
            count += 1;
        }
        if count != used {
            return Err(Error::Format(
                "BNK used-entry count does not match its table",
            ));
        }
        Ok(bank)
    }
}
