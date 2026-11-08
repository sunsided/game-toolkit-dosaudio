//! Musical helpers are checked through the public custom-core and port interfaces.
#![cfg(feature = "opl")]

use game_toolkit_dosaudio::{Drum, Error, Note, OplCore, OplOperator, OplPatch, prelude::*};
use std::sync::{
    Arc,
    atomic::{AtomicU16, AtomicUsize, Ordering},
};

#[derive(Clone)]
struct Registers(Arc<[AtomicU16; 512]>);

impl Registers {
    fn get(&self, register: u16) -> Option<u8> {
        let value = self.0[usize::from(register)].load(Ordering::Relaxed);
        (value & 0x100 != 0).then_some(value as u8)
    }
}

struct Probe {
    registers: Registers,
    bank: u16,
}

impl OplCore for Probe {
    fn write(&mut self, register: u16, value: u8) {
        self.registers.0[usize::from(register | self.bank)]
            .store(0x100 | u16::from(value), Ordering::Relaxed);
    }

    fn generate(&mut self) -> Frame {
        [0.0; 2]
    }
}

fn machine(config: SoundConfig) -> (DosAudio, AudioEngine, Registers) {
    let registers = Registers(Arc::new(std::array::from_fn(|_| AtomicU16::new(0))));
    let shared = registers.clone();
    let count = AtomicUsize::new(0);
    let backend = OplBackend::Custom(Arc::new(move || {
        Box::new(Probe {
            registers: shared.clone(),
            bank: (count.fetch_add(1, Ordering::Relaxed) as u16) << 8,
        })
    }));
    let (ports, engine) = DosAudio::split(config.latency(Tick::ZERO).opl_backend(backend)).unwrap();
    (ports, engine, registers)
}

fn patch() -> OplPatch {
    OplPatch {
        modulator: OplOperator {
            characteristics: 0x21,
            level: 0x84,
            attack_decay: 0xf2,
            sustain_release: 0x73,
            waveform: 1,
        },
        carrier: OplOperator {
            characteristics: 0x01,
            level: 0x45,
            attack_decay: 0xe3,
            sustain_release: 0x64,
            waveform: 2,
        },
        feedback: 0x0e,
    }
}

fn raw_write(ports: &mut DosAudio, register: u16, value: u8) {
    let address = 0x388 + (register >> 8) * 2;
    ports.outb(address, register as u8);
    ports.outb(address + 1, value);
}

fn flush(engine: &mut AudioEngine) {
    engine.render(&mut [[0.0; 2]; 1]);
}

#[test]
fn sbi_maps_interleaved_operators_and_accepts_trailing_metadata() {
    let mut bytes = [0; 52].to_vec();
    bytes[..4].copy_from_slice(b"SBI\x1a");
    bytes[36..47].copy_from_slice(&[0x21, 1, 0x84, 0x45, 0xf2, 0xe3, 0x73, 0x64, 1, 2, 0x0e]);
    bytes[47..].fill(0xff); // Reserved bytes are not operator data.
    assert_eq!(OplPatch::from_sbi(&bytes).unwrap(), patch());
    bytes.extend(b"instrument metadata");
    assert_eq!(OplPatch::from_sbi(&bytes).unwrap(), patch());
}

#[test]
fn sbi_rejects_every_truncated_prefix_and_bad_signatures() {
    let mut bytes = [0; 52];
    bytes[..4].copy_from_slice(b"SBI\x1a");
    for end in 0..52 {
        assert!(OplPatch::from_sbi(&bytes[..end]).is_err(), "prefix {end}");
    }
    bytes[3] = 0;
    assert!(OplPatch::from_sbi(&bytes).is_err());
}

#[test]
fn notes_use_concert_tuning_and_reject_invalid_midi_numbers() {
    assert_eq!(Note::from_midi(60).unwrap(), Note::C4);
    assert_eq!(Note::from_midi(69).unwrap(), Note::A4);
    assert_eq!(Note::A4.frequency(), 440.0);
    assert!((Note::C4.frequency() - 261.625_565).abs() < 1e-6);
    assert!(Note::from_midi(0).is_ok());
    assert!(Note::from_midi(127).is_ok());
    assert!(Note::from_midi(128).is_err());
    assert!(Note::from_midi(255).is_err());
}

#[test]
fn channel_limits_match_each_chip_and_absent_hardware_fails() {
    for (config, limit) in [
        (SoundConfig::adlib(), 9),
        (SoundConfig::dual_opl2(), 18),
        (SoundConfig::opl3(), 18),
    ] {
        let (mut ports, _, _) = machine(config);
        let mut opl = ports.opl().unwrap();
        opl.set_patch(limit - 1, &patch()).unwrap();
        opl.note_on(limit - 1, Note::A4, 127).unwrap();
        opl.note_off(limit - 1).unwrap();
        assert!(opl.set_patch(limit, &patch()).is_err());
        assert!(opl.note_on(limit, Note::A4, 127).is_err());
        assert!(opl.note_off(limit).is_err());
    }
    let (mut absent, _) = DosAudio::split(SoundConfig::none()).unwrap();
    assert!(absent.opl().is_err());
}

#[test]
fn patch_operator_offsets_and_opl3_routing_preserve_both_banks() {
    let (mut ports, mut engine, registers) = machine(SoundConfig::opl3());
    raw_write(&mut ports, 0x1c8, 0x10);
    ports.opl().unwrap().set_patch(8, &patch()).unwrap();
    ports.opl().unwrap().set_patch(17, &patch()).unwrap();
    flush(&mut engine);
    assert_eq!(registers.get(0x105), Some(1));
    assert_eq!(registers.get(0x32), Some(0x21));
    assert_eq!(registers.get(0x35), Some(1));
    assert_eq!(registers.get(0x132), Some(0x21));
    assert_eq!(registers.get(0x135), Some(1));
    assert_eq!(registers.get(0xc8), Some(0x3e));
    assert_eq!(registers.get(0x1c8), Some(0x1e));
}

#[test]
fn dual_opl2_channels_reach_the_selected_physical_chip() {
    let (mut ports, mut engine, registers) = machine(SoundConfig::dual_opl2());
    ports.opl().unwrap().set_patch(9, &patch()).unwrap();
    ports.opl().unwrap().note_on(9, Note::A4, 127).unwrap();
    flush(&mut engine);
    assert_eq!(registers.get(0x120), Some(0x21));
    assert_eq!(registers.get(0x123), Some(1));
    assert_eq!(registers.get(0x20), None);
    assert_eq!(registers.get(0x1b0).unwrap() & 0x20, 0x20);
    assert_eq!(registers.get(0xb0), None);
}

#[cfg(feature = "sb")]
#[test]
fn sb_pro_helpers_use_the_selected_chip_without_broadcasting() {
    let (mut ports, mut engine, registers) = machine(SoundConfig::sb_pro());
    ports.start_capture().unwrap();
    ports.opl().unwrap().set_patch(9, &patch()).unwrap();
    ports.opl().unwrap().note_on(9, Note::A4, 127).unwrap();
    let capture = ports.finish_capture().unwrap();
    assert!(
        capture
            .writes()
            .iter()
            .all(|write| (0x222..=0x223).contains(&write.port))
    );
    flush(&mut engine);
    assert_eq!(registers.get(0x120), Some(0x21));
    assert_eq!(registers.get(0x20), None);
    assert_eq!(registers.get(0x1b0).unwrap() & 0x20, 0x20);
    assert_eq!(registers.get(0xb0), None);
}

#[test]
fn note_pitch_uses_nearest_f_number_and_note_off_preserves_tuning() {
    let (mut ports, mut engine, registers) = machine(SoundConfig::adlib());
    ports.opl().unwrap().set_patch(0, &patch()).unwrap();
    ports.opl().unwrap().note_on(0, Note::A4, 127).unwrap();
    flush(&mut engine);
    let low = registers.get(0xa0).unwrap();
    let high = registers.get(0xb0).unwrap();
    let number = u16::from(low) | (u16::from(high & 3) << 8);
    let block = (high >> 2) & 7;
    let native_rate = 315_000_000.0 / (22.0 * 288.0);
    let frequency = f64::from(number) * native_rate * 2.0f64.powi(i32::from(block)) / 1_048_576.0;
    assert!((frequency - 440.0).abs() < 0.4);
    assert!(block == 0 || number * 2 > 1023);
    ports.opl().unwrap().note_off(0).unwrap();
    flush(&mut engine);
    assert_eq!(registers.get(0xa0), Some(low));
    assert_eq!(registers.get(0xb0), Some(high & !0x20));
}

#[test]
fn velocity_restores_original_levels_and_respects_connection_mode() {
    let (mut ports, mut engine, registers) = machine(SoundConfig::adlib());
    ports.opl().unwrap().set_patch(0, &patch()).unwrap();
    ports.opl().unwrap().note_on(0, Note::C4, 1).unwrap();
    flush(&mut engine);
    assert_eq!(registers.get(0x40), Some(0x84));
    assert_eq!(registers.get(0x43), Some(0x7f));
    ports.opl().unwrap().note_on(0, Note::C4, 127).unwrap();
    flush(&mut engine);
    assert_eq!(registers.get(0x43), Some(0x45));

    let mut additive = patch();
    additive.feedback |= 1;
    ports.opl().unwrap().set_patch(0, &additive).unwrap();
    ports.opl().unwrap().note_on(0, Note::C4, 1).unwrap();
    flush(&mut engine);
    assert_eq!(registers.get(0x40), Some(0xbf));
    ports.opl().unwrap().note_on(0, Note::C4, 127).unwrap();
    flush(&mut engine);
    assert_eq!(registers.get(0x40), Some(0x84));
    assert_eq!(registers.get(0x43), Some(0x45));
    ports.opl().unwrap().note_on(0, Note::C4, 0).unwrap();
    flush(&mut engine);
    assert_eq!(registers.get(0xb0).unwrap() & 0x20, 0);
}

#[test]
fn rhythm_keys_preserve_depth_bits_and_other_drums() {
    let (mut ports, mut engine, registers) = machine(SoundConfig::opl3());
    raw_write(&mut ports, 0xbd, 0xc0);
    ports.opl().unwrap().rhythm(true).unwrap();
    for drum in [
        Drum::BassDrum,
        Drum::Snare,
        Drum::Tom,
        Drum::Cymbal,
        Drum::HiHat,
    ] {
        ports.opl().unwrap().hit(drum).unwrap();
    }
    flush(&mut engine);
    assert_eq!(registers.get(0xbd), Some(0xff));
    ports.opl().unwrap().release(Drum::Snare).unwrap();
    flush(&mut engine);
    assert_eq!(registers.get(0xbd), Some(0xf7));
    ports.opl().unwrap().rhythm(false).unwrap();
    flush(&mut engine);
    assert_eq!(registers.get(0xbd), Some(0xc0));
    assert_eq!(registers.get(0x1bd), None);
}

#[test]
fn invalid_notes_and_full_queue_leave_registers_and_patch_levels_unchanged() {
    let (mut ports, mut engine, registers) = machine(SoundConfig::adlib().queue_capacity(32));
    ports.opl().unwrap().set_patch(0, &patch()).unwrap();
    flush(&mut engine);
    ports.start_capture().unwrap();
    assert!(ports.opl().unwrap().note_on(0, Note::A4, 128).is_err());
    assert!(
        ports
            .opl()
            .unwrap()
            .note_on(0, Note::from_midi(127).unwrap(), 127)
            .is_err()
    );
    assert!(ports.finish_capture().unwrap().writes().is_empty());
    for _ in 0..32 {
        raw_write(&mut ports, 1, 0x20);
    }
    let mut replacement = patch();
    replacement.modulator.level = 0x01;
    replacement.carrier.level = 0x02;
    ports.start_capture().unwrap();
    assert_eq!(
        ports.opl().unwrap().set_patch(0, &replacement),
        Err(Error::QueueFull)
    );
    assert_eq!(
        ports.opl().unwrap().note_on(0, Note::A4, 127),
        Err(Error::QueueFull)
    );
    assert!(ports.finish_capture().unwrap().writes().is_empty());
    flush(&mut engine);
    assert_eq!(registers.get(0x40), Some(0x84));
    assert_eq!(registers.get(0x43), Some(0x45));
    ports.opl().unwrap().note_on(0, Note::A4, 127).unwrap();
    flush(&mut engine);
    assert_eq!(registers.get(0x40), Some(0x84));
    assert_eq!(registers.get(0x43), Some(0x45));
}

#[test]
fn musical_helper_batches_do_not_allocate() {
    let (mut ports, _, _) = machine(SoundConfig::opl3());
    let patch = patch();
    let allocations = allocation_counter::measure(|| {
        let mut opl = ports.opl().unwrap();
        opl.set_patch(0, &patch).unwrap();
        opl.note_on(0, Note::A4, 100).unwrap();
        opl.note_off(0).unwrap();
        opl.rhythm(true).unwrap();
        opl.hit(Drum::BassDrum).unwrap();
        opl.release(Drum::BassDrum).unwrap();
    });
    assert_eq!(allocations.count_total, 0);
    assert_eq!(allocations.count_current, 0);
}
