//! Generated valid inputs and state-machine oracles through the public API.
use game_toolkit_dosaudio::{Melody, MidiMessage, prelude::*};
use proptest::prelude::*;
use std::sync::Arc;
#[cfg(feature = "opl")]
mod common;

const DEVICES: [MusicDevice; 9] = [
    MusicDevice::Opl2,
    MusicDevice::DualOpl2,
    MusicDevice::Opl3,
    MusicDevice::GeneralMidi,
    MusicDevice::Mt32,
    MusicDevice::CustomMidi,
    MusicDevice::Psg,
    MusicDevice::Speaker,
    MusicDevice::Native,
];

proptest! {
    #[test]
    fn valid_midi_messages_retain_every_byte(
        channel in 0u8..16, kind in 0u8..7, a in 0u8..128, b in 0u8..128,
    ) {
        let status = 0x80 + kind * 16 + channel;
        let mut bytes = vec![status, a];
        if kind != 4 && kind != 5 { bytes.push(b); }
        let message = MidiMessage::new(&bytes).unwrap();
        prop_assert_eq!(message.bytes(), bytes.as_slice());
        bytes[1] |= 128;
        prop_assert!(MidiMessage::new(&bytes).is_err());
    }

    #[test]
    fn sysex_payload_ranges_reject_overflow_and_invalid_framing(
        prefix in prop::collection::vec(any::<u8>(), 0..16),
        payload in prop::collection::vec(0u8..128, 0..128),
        suffix in prop::collection::vec(any::<u8>(), 0..16),
        corruption in 0u8..4,
    ) {
        let mut storage = prefix.clone();
        storage.push(0xf0);
        storage.extend_from_slice(&payload);
        storage.push(0xf7);
        storage.extend(suffix);
        let mut offset = prefix.len() as u32;
        let mut length = payload.len() as u32 + 2;
        match corruption {
            1 => offset = u32::MAX,
            2 => length = u32::MAX,
            3 => storage[prefix.len()] = 0xff,
            _ => {}
        }
        let result = PreparedMusic::new(MusicTarget::Midi,
            vec![TimedEvent { at: Tick::ZERO, event: MusicEvent::SysEx { offset, length } }],
            Tick(1), storage.clone());
        prop_assert_eq!(result.is_ok(), corruption == 0);
        if let Ok(music) = result { prop_assert_eq!(music.sysex(), storage.as_slice()); }
    }

    #[test]
    fn fallback_graph_matches_an_independent_reachability_model(
        edges in prop::collection::vec((0usize..9, 0usize..9), 0..40),
    ) {
        let mut soundtrack = Soundtrack::new();
        soundtrack.insert(MusicDevice::Speaker, Melody::parse("T120 C").unwrap()).unwrap();
        let mut graph = [None; 9];
        for (from, to) in edges {
            let mut cursor = Some(to);
            let mut cyclic = false;
            while let Some(node) = cursor {
                if node == from { cyclic = true; break; }
                cursor = graph[node];
            }
            prop_assert_eq!(soundtrack.fallback(DEVICES[from], DEVICES[to]).is_ok(), !cyclic);
            if !cyclic { graph[from] = Some(to); }
        }
        let soundtrack = Arc::new(soundtrack);
        let (_, mut engine) = DosAudio::split(SoundConfig::none().speaker(true)).unwrap();
        for (requested, device) in DEVICES.into_iter().enumerate() {
            let mut cursor = Some(requested);
            let mut reaches_speaker = false;
            while let Some(node) = cursor {
                if node == 7 { reaches_speaker = true; break; }
                cursor = graph[node];
            }
            let result = engine.play_soundtrack(soundtrack.clone(), device, PlayOptions::default());
            prop_assert_eq!(result.is_ok(), reaches_speaker);
            if let Ok(handle) = result { prop_assert_eq!(handle.device(), Some(MusicDevice::Speaker)); }
        }
    }
}

#[cfg(feature = "formats")]
mod formats {
    use super::*;
    use game_toolkit_dosaudio::formats::Imf;
    proptest! {
        #[test]
        fn imf_variants_agree_and_delays_keep_exact_rational_phase(
            records in prop::collection::vec((any::<u8>(), any::<u8>(), any::<u16>()), 0..128),
            hz in 1u32..1_000_001,
        ) {
            let mut raw = Vec::new();
            for &(reg, value, delay) in &records {
                raw.extend([reg, value]);
                raw.extend(delay.to_le_bytes());
            }
            let mut prefixed = (raw.len() as u16).to_le_bytes().to_vec();
            prefixed.extend_from_slice(&raw);
            let zero = Imf::load_type0(&raw, hz).unwrap();
            let one = Imf::load_type1(&prefixed, hz).unwrap();
            prop_assert_eq!(zero.events(), one.events());
            prop_assert_eq!(zero.duration(), one.duration());
            let ticks = |units| Tick((u128::from(units) * u128::from(Tick::HZ_NUMERATOR)
                / (u128::from(hz) * u128::from(Tick::HZ_DENOMINATOR))) as u64);
            let mut elapsed = 0u64;
            for (event, &(register, value, delay)) in zero.events().iter().zip(&records) {
                prop_assert_eq!(event.at, ticks(elapsed));
                prop_assert_eq!(event.event, MusicEvent::Opl { register: u16::from(register), value });
                elapsed += u64::from(delay);
            }
            prop_assert_eq!(zero.duration(), ticks(elapsed));
        }
    }
}

#[cfg(all(feature = "opl", feature = "formats"))]
mod capture {
    use super::*;
    use game_toolkit_dosaudio::formats::Dro;
    proptest! {
        #[test]
        fn port_capture_dro_roundtrip_preserves_banks_escaped_registers_and_final_delay(
            writes in prop::collection::vec((0u16..512, any::<u8>(), 0u32..100), 0..64),
            final_ms in 0u32..100,
        ) {
            let (mut ports, _engine) = DosAudio::split(SoundConfig::opl3().queue_capacity(256)).unwrap();
            ports.start_capture().unwrap();
            let mut milliseconds = 0u64;
            let mut expected = Vec::new();
            for (register, value, delay) in writes {
                milliseconds += u64::from(delay);
                let at = Tick::from_micros(milliseconds * 1000);
                let port = 0x388 + (register >> 8) * 2;
                ports.try_outb_at(at, port, register as u8).unwrap();
                ports.try_outb_at(at, port + 1, value).unwrap();
                expected.push(TimedEvent { at, event: MusicEvent::Opl { register, value } });
            }
            milliseconds += u64::from(final_ms);
            ports.try_advance_to(Tick::from_micros(milliseconds * 1000)).unwrap();
            let sequence = Dro::load(&ports.finish_capture().unwrap().to_dro().unwrap()).unwrap();
            prop_assert_eq!(sequence.events(), expected.as_slice());
            prop_assert_eq!(sequence.duration(), Tick::from_micros(milliseconds * 1000));
        }
    }
}

#[cfg(feature = "opl")]
mod switching {
    use super::common;
    use super::*;

    type Trace = (Vec<Frame>, Vec<(Tick, Option<MusicDevice>, bool, bool)>);

    fn run(operations: &[(u8, u8, u16)], chunk: usize) -> Trace {
        let fm = common::tune(OplChip::Opl2, 0, 0x30);
        let duration = fm.duration();
        let mut soundtrack = Soundtrack::new();
        soundtrack.insert(MusicDevice::Opl2, fm).unwrap();
        soundtrack
            .insert(
                MusicDevice::Speaker,
                PreparedMusic::new(
                    MusicTarget::Speaker,
                    vec![TimedEvent {
                        at: Tick::ZERO,
                        event: MusicEvent::Speaker {
                            divisor: 2712,
                            enabled: true,
                        },
                    }],
                    duration,
                    [],
                )
                .unwrap(),
            )
            .unwrap();
        let (_, mut engine) =
            DosAudio::split(SoundConfig::adlib().speaker(true).latency(Tick::ZERO)).unwrap();
        let handle = engine
            .play_soundtrack(
                Arc::new(soundtrack),
                MusicDevice::Opl2,
                PlayOptions::looped(),
            )
            .unwrap();
        let mut output = Vec::new();
        let mut states = Vec::new();
        for &(op, value, frames) in operations {
            match op {
                0 => {
                    let _ = handle.switch_device(MusicDevice::Opl2);
                }
                1 => {
                    let _ = handle.switch_device(MusicDevice::Speaker);
                }
                2 => handle.pause(),
                3 => handle.resume(),
                4 => handle.set_volume(f32::from(value) / 255.0),
                5 => {
                    handle
                        .fade_volume(
                            f32::from(value) / 255.0,
                            std::time::Duration::from_millis(5),
                        )
                        .unwrap();
                }
                6 => {
                    drop(handle.clone());
                }
                _ => handle.stop(),
            }
            let start = output.len();
            output.resize(start + usize::from(frames), [0.0; 2]);
            for block in output[start..].chunks_mut(chunk) {
                engine.render(block);
            }
            assert!(
                output[start..]
                    .iter()
                    .flatten()
                    .all(|s| s.is_finite() && (-1.0..=1.0).contains(s))
            );
            states.push((
                handle.position(),
                handle.device(),
                handle.is_paused(),
                handle.is_finished(),
            ));
        }
        (output, states)
    }

    proptest! {
        #[test]
        fn switch_pause_fade_loop_stop_traces_are_callback_chunk_independent(
            operations in prop::collection::vec((0u8..8, any::<u8>(), 0u16..513), 1..40),
            chunk in 1usize..257,
        ) {
            prop_assert_eq!(run(&operations, 513), run(&operations, chunk));
        }
    }
}
