//! Optional CM-32L synthesis for MT-32 arrangements, using caller-supplied ROMs.
use crate::{Error, Frame, MidiSynth, Result};
use sha1::{Digest, Sha1};
use std::{io::Read, path::Path, sync::Arc};

#[allow(
    clippy::all,
    dead_code,
    missing_docs,
    unused_imports,
    unused_variables,
    unused_macros,
    unexpected_cfgs
)]
mod cm32l;
use cm32l::Synth;

/// Validated CM-32L firmware identity.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum Mt32Model {
    /// CM-32L/LAPC-I control v1.00, with full 1 MiB CM-32L PCM.
    Cm32l100,
    /// CM-32L/LAPC-I control v1.02, with full 1 MiB CM-32L PCM.
    Cm32l102,
}

/// Owned, hash-validated user ROMs; no proprietary bytes are distributed.
///
/// Only full CM-32L v1.00/v1.02 control and PCM images are supported. MT-32,
/// split ROMs and CM-32LN images are rejected. SHA-1 identities come from Munt's
/// ROMInfo table; SHA-1 is used for firmware identification, not authentication.
#[derive(Clone)]
pub struct Mt32Roms {
    control: Arc<[u8]>,
    pcm: Arc<[u8]>,
    model: Mt32Model,
}
impl Mt32Roms {
    /// Validate lengths, known firmware hashes and parsed ROM tables.
    pub fn from_bytes(control: &[u8], pcm: &[u8]) -> Result<Arc<Self>> {
        if control.len() != 65_536 || pcm.len() != 1_048_576 {
            return Err(Error::Format(
                "MT-32 backend requires a 64 KiB CM-32L control ROM and full 1 MiB PCM ROM",
            ));
        }
        let control_hash = format!("{:x}", Sha1::digest(control));
        let model = match control_hash.as_str() {
            "73683d585cd6948cc19547942ca0e14a0319456d" => Mt32Model::Cm32l100,
            "a439fbb390da38cada95a7cbb1d6ca199cd66ef8" => Mt32Model::Cm32l102,
            _ => {
                return Err(Error::Format(
                    "unrecognized CM-32L control ROM SHA-1 (requires v1.00 or v1.02)",
                ));
            }
        };
        if format!("{:x}", Sha1::digest(pcm)) != "289cc298ad532b702461bfc738009d9ebe8025ea" {
            return Err(Error::Format(
                "unrecognized CM-32L PCM ROM SHA-1; provide the full 1 MiB image",
            ));
        }
        decode_rom(control, pcm, model).map_err(|_| Error::Format("invalid CM-32L ROM tables"))?;
        Ok(Arc::new(Self {
            control: control.into(),
            pcm: pcm.into(),
            model,
        }))
    }
    /// Read the control and PCM images from this exact directory, accepting
    /// Munt's names (`CM32L_CONTROL.ROM`, `CM32L_PCM.ROM`) and MAME's
    /// versioned names (`cm32l_ctrl_1_02.rom`, `cm32l_ctrl_1_00.rom`,
    /// `cm32l_pcm.rom`). The first existing name wins; hashes then identify
    /// the firmware. No tilde expansion, ROM downloads or embedded assets.
    pub fn from_rom_dir(path: impl AsRef<Path>) -> Result<Arc<Self>> {
        fn read(dir: &Path, names: &[&str], limit: u64) -> Result<Vec<u8>> {
            let file = names
                .iter()
                .find_map(|name| std::fs::File::open(dir.join(name)).ok())
                .ok_or(Error::Config(
                    "no CM-32L control or PCM ROM in the ROM directory (expected CM32L_CONTROL.ROM \
                     or cm32l_ctrl_1_0x.rom, and CM32L_PCM.ROM or cm32l_pcm.rom)",
                ))?;
            let mut bytes = Vec::new();
            file.take(limit + 1)
                .read_to_end(&mut bytes)
                .map_err(|_| Error::Config("unable to read CM-32L ROM file"))?;
            Ok(bytes)
        }
        let dir = path.as_ref();
        let control = read(
            dir,
            &[
                "CM32L_CONTROL.ROM",
                "cm32l_ctrl_1_02.rom",
                "cm32l_ctrl_1_00.rom",
            ],
            65_536,
        )?;
        let pcm = read(dir, &["CM32L_PCM.ROM", "cm32l_pcm.rom"], 1_048_576)?;
        Self::from_bytes(&control, &pcm)
    }
    /// Validated hardware/firmware model.
    pub fn model(&self) -> Mt32Model {
        self.model
    }
}

// Both supported revisions share timbre/PCM maps. Their default/max tables
// have different offsets (Munt ControlROMMaps). Normalize only those data tables
// on the control thread, after validating the original image's identity.
fn normalized_control(control: &[u8], model: Mt32Model) -> std::borrow::Cow<'_, [u8]> {
    if model == Mt32Model::Cm32l102 {
        return std::borrow::Cow::Borrowed(control);
    }
    let mut normalized = control.to_vec();
    for (from, to, len) in [
        (0x4f65, 0x4f93, 9),
        (0x4f80, 0x4fae, 9),
        (0x4f6e, 0x4f9c, 8),
        (0x48a1, 0x48cb, 4),
        (0x48a5, 0x48cf, 16),
        (0x48be, 0x48e8, 23),
        (0x48d5, 0x48ff, 72),
    ] {
        normalized[to..to + len].copy_from_slice(&control[from..from + len]);
    }
    std::borrow::Cow::Owned(normalized)
}
fn decode_rom(
    control: &[u8],
    pcm: &[u8],
    model: Mt32Model,
) -> std::result::Result<cm32l::Rom, cm32l::cm32l::RomError> {
    cm32l::Rom::new(&normalized_control(control, model), pcm)
}

/// 32 kHz LA synthesis with preallocated MIDI/SysEx, reset and reverb state.
///
/// The private core is adapted from moont 1.0.0 under LGPL-2.1-or-later; see its
/// source notices. A completed message enters an insertion-ordered native queue;
/// one message is applied per native frame, with bounded voice-abort delay.
/// No extra serial delay is added after dosaudio completes a UART message.
/// At most 4,096 messages and 16 complete SysEx payloads (65,536 bytes each) may
/// wait at once. Overflow increments an observable counter instead of allocating.
/// Construction and final destruction must occur outside the callback.
pub struct Mt32Synth {
    device: cm32l::CM32L,
    dropped: u64,
}
impl Mt32Synth {
    /// Decode caller-owned ROMs and preallocate all synthesis resources.
    pub fn new(roms: Arc<Mt32Roms>) -> Result<Self> {
        let rom = decode_rom(&roms.control, &roms.pcm, roms.model)
            .map_err(|_| Error::Format("invalid CM-32L ROM tables"))?;
        Ok(Self {
            device: cm32l::CM32L::new(rom),
            dropped: 0,
        })
    }
    /// Messages rejected by the bounded native queues.
    pub fn dropped_messages(&self) -> u64 {
        self.dropped
    }
}
impl MidiSynth for Mt32Synth {
    fn sample_rate(&self) -> u32 {
        32_000
    }
    fn message(&mut self, bytes: &[u8]) {
        let accepted = if bytes == [0xff] {
            self.device.clear_pending();
            self.device
                .apply_command(cm32l::ControlCommand::Reset)
                .expect("valid reset");
            true
        } else if bytes.first() == Some(&0xf0) {
            if bytes.last() != Some(&0xf7)
                || bytes.len() > 65_536
                || bytes[1..bytes.len() - 1].iter().any(|b| *b >= 128)
            {
                return;
            }
            self.device.play_sysex(bytes)
        } else {
            let Ok(message) = crate::MidiMessage::new(bytes) else {
                return;
            };
            if bytes[0] >= 0xf0 {
                return;
            }
            let mut packed = 0;
            for (i, byte) in message.bytes().iter().enumerate() {
                packed |= u32::from(*byte) << (i * 8);
            }
            self.device.play_msg(packed)
        };
        if !accepted {
            self.dropped = self.dropped.saturating_add(1);
        }
    }
    fn render(&mut self, out: &mut [Frame]) {
        for frame in out {
            let mut native = [cm32l::Frame(0, 0)];
            self.device.render(&mut native);
            *frame = [
                f32::from(native[0].0) / 32768.0,
                f32::from(native[0].1) / 32768.0,
            ];
        }
    }
    fn dropped_messages(&self) -> u64 {
        self.dropped
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn synthetic() -> Mt32Synth {
        // Original structurally valid control tables, not Roland firmware.
        let mut control = vec![0; 65_536];
        for base in [0x8000, 0x8080, 0x8500] {
            for i in 0..64 {
                control[base + i * 2..base + i * 2 + 2].copy_from_slice(&0x1000u16.to_le_bytes());
            }
        }
        control[0x4fae..0x4fae + 9].fill(7);
        control[0x4f93..0x4f93 + 8].fill(4);
        control[0x48cb..0x48cb + 4].fill(127);
        control[0x48cf..0x48cf + 16].fill(127);
        control[0x48e8..0x48e8 + 23].fill(127);
        control[0x48ff..0x48ff + 72].fill(127);
        control[0x900c] = 1;
        control[0x900e..0x900e + 58].fill(64);
        control[0x900e] = 36;
        control[0x900e + 1] = 50;
        control[0x900e + 4] = 0;
        let rom = cm32l::Rom::new(&control, &vec![0; 1_048_576]).unwrap();
        Mt32Synth {
            device: cm32l::CM32L::new(rom),
            dropped: 0,
        }
    }
    fn sysex(address: [u8; 3], data: &[u8]) -> Vec<u8> {
        let mut bytes = vec![0xf0, 0x41, 0x10, 0x16, 0x12];
        bytes.extend(address);
        bytes.extend(data);
        let sum = bytes[5..].iter().fold(0u8, |n, b| n.wrapping_add(*b));
        bytes.push((128 - (sum & 127)) & 127);
        bytes.push(0xf7);
        bytes
    }
    #[test]
    fn revision_100_normalizes_only_the_documented_default_and_maximum_tables() {
        let mut original = vec![0u8; 65536];
        for (i, b) in original.iter_mut().enumerate() {
            *b = (i % 251) as u8;
        }
        let normalized = normalized_control(&original, Mt32Model::Cm32l100);
        assert_eq!(
            &normalized[0x4f93..0x4f93 + 9],
            &original[0x4f65..0x4f65 + 9]
        );
        assert_eq!(
            &normalized[0x48ff..0x48ff + 72],
            &original[0x48d5..0x48d5 + 72]
        );
        assert_eq!(&normalized[0x8000..], &original[0x8000..]);
        assert!(matches!(
            normalized_control(&original, Mt32Model::Cm32l102),
            std::borrow::Cow::Borrowed(_)
        ));
    }
    #[test]
    fn construction_fits_an_ordinary_control_thread_stack() {
        let mut synth = synthetic();
        synth.render(&mut [[0.0; 2]; 1]);
    }
    #[test]
    fn native_first_use_sysex_all_reverb_modes_and_resets_allocate_and_free_nothing() {
        std::thread::Builder::new()
            .stack_size(8 * 1024 * 1024)
            .spawn(|| {
                let mut synth = synthetic();
                let mut out = vec![[0.0; 2]; 4096];
                let modes: Vec<_> = (0..4)
                    .map(|mode| sysex([0x10, 0, 1], &[mode, 5, 3]))
                    .collect();
                let reset = sysex([0x7f, 0, 0], &[]);
                let patch = sysex([3, 0, 0], &[0, 0, 24, 50, 12, 0, 1, 0]);
                let mut heard = false;
                let allocations = allocation_counter::measure(|| {
                    synth.message(&patch);
                    synth.message(&[0x91, 60, 100]);
                    synth.render(&mut out);
                    heard = out.iter().flatten().any(|v| v.abs() > 0.00001);
                    for _ in 0..3 {
                        for message in &modes {
                            synth.message(message);
                            synth.render(&mut out);
                        }
                    }
                    synth.message(&reset);
                    synth.render(&mut out);
                    synth.message(&[0xff]);
                    synth.render(&mut out);
                });
                assert!(
                    heard,
                    "original synthetic timbre must exercise audible synthesis"
                );
                assert_eq!(allocations.count_total, 0);
                assert_eq!(allocations.count_current, 0);
                assert!(out.iter().flatten().all(|v| v.is_finite()));
            })
            .unwrap()
            .join()
            .unwrap();
    }
    #[test]
    fn queued_roland_reset_preserves_messages_received_after_it() {
        let mut synth = synthetic();
        let reset = sysex([0x7f, 0, 0], &[]);
        let mut out = vec![[0.0; 2]; 4096];
        let usage = allocation_counter::measure(|| {
            synth.message(&reset);
            synth.message(&[0x91, 60, 100]);
            synth.render(&mut out);
        });
        assert_eq!(usage.count_total, 0);
        assert_eq!(usage.count_current, 0);
        assert!(out.iter().flatten().any(|v| v.abs() > 0.00001));
    }
    #[test]
    fn native_queue_drains_across_the_u32_frame_clock_wrap() {
        let mut synth = synthetic();
        synth.device.set_test_clock(u32::MAX - 4);
        let packet = sysex([0x10, 0, 22], &[80]);
        for _ in 0..16 {
            synth.message(&packet);
        }
        synth.render(&mut [[0.0; 2]; 32]);
        for _ in 0..16 {
            synth.message(&packet);
        }
        assert_eq!(synth.dropped_messages(), 0);
    }
    #[test]
    fn sysex_pool_overflow_is_observable_reusable_and_chunk_independent() {
        std::thread::Builder::new()
            .stack_size(8 * 1024 * 1024)
            .spawn(|| {
                let mut a = synthetic();
                let mut b = synthetic();
                let message = sysex([0x10, 0, 22], &[80]);
                for _ in 0..17 {
                    a.message(&message);
                    b.message(&message);
                }
                assert_eq!(a.dropped_messages(), 1);
                let mut whole = vec![[0.0; 2]; 1024];
                let mut parts = whole.clone();
                a.render(&mut whole);
                for c in parts.chunks_mut(37) {
                    b.render(c);
                }
                assert_eq!(whole, parts);
                a.message(&message);
                a.render(&mut whole);
                assert_eq!(a.dropped_messages(), 1);
            })
            .unwrap()
            .join()
            .unwrap();
    }
}
