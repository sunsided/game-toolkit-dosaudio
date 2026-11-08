// Original register values authored for dosaudio, 2026-11-08. No extracted bank.
use super::*;

pub(super) fn bank() -> OplBank {
    let mut bank = OplBank::new();
    for (program, name, multiplier, level, attack, release, feedback) in [
        (0, "Clear bell", 2, 28, 0xf3, 0x65, 0),
        (24, "Plucked wire", 3, 20, 0xf5, 0x56, 2),
        (40, "Soft reed", 1, 26, 0x86, 0x67, 4),
        (64, "Bright brass", 1, 12, 0xa4, 0x56, 6),
        (80, "Hollow lead", 2, 16, 0xf2, 0x46, 8),
        (81, "Glass lead", 5, 24, 0xe4, 0x65, 2),
    ] {
        let patch = OplPatch {
            modulator: OplOperator {
                characteristics: 0x20 | multiplier,
                level,
                attack_decay: attack,
                sustain_release: release,
                waveform: 0,
            },
            carrier: OplOperator {
                characteristics: 0x21,
                level: 6,
                attack_decay: attack,
                sustain_release: release,
                waveform: 0,
            },
            feedback,
        };
        bank.insert(OplBankEntry {
            key: OplBankKey {
                msb: 0,
                lsb: 0,
                program,
                percussion: false,
            },
            name: name.into(),
            instrument: patch.into(),
        })
        .expect("valid starter patch");
    }
    for (program, name, note, multiplier, feedback) in [
        (36, "Low pulse", 36, 1, 0x0e),
        (38, "Dry rattle", 60, 7, 0x0e),
        (42, "Short tick", 96, 15, 0x0e),
        (45, "Wood pulse", 48, 2, 0x08),
        (49, "Metal splash", 84, 11, 0x0e),
    ] {
        let patch = OplPatch {
            modulator: OplOperator {
                characteristics: multiplier,
                level: 4,
                attack_decay: 0xf8,
                sustain_release: 0x08,
                waveform: 0,
            },
            carrier: OplOperator {
                characteristics: 1,
                level: 8,
                attack_decay: 0xf7,
                sustain_release: 0x08,
                waveform: 0,
            },
            feedback,
        };
        bank.insert(OplBankEntry {
            key: OplBankKey {
                msb: 0,
                lsb: 0,
                program,
                percussion: true,
            },
            name: name.into(),
            instrument: OplInstrument {
                fixed_note: Some(note),
                ..patch.into()
            },
        })
        .expect("valid starter percussion");
    }
    bank
}
