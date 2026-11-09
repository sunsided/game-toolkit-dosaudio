#![no_main]
use game_toolkit_dosaudio::{
    Melody, MidiMessage, MusicEvent, MusicTarget, PreparedMusic, SoundConfig, Tick, TimedEvent,
    formats::{Dro, Imf, Smf, Vgm},
};
use game_toolkit_dosaudio_fuzz::check_music;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if data.len() > 65_536 {
        return;
    }
    let Some((&format, bytes)) = data.split_first() else {
        return;
    };
    let parsed = match format % 8 {
        0 => Imf::load_type0(bytes, 700).map(PreparedMusic::from),
        1 => Imf::load_type1(bytes, 560).map(PreparedMusic::from),
        2 => Dro::load(bytes).map(PreparedMusic::from),
        3 => Vgm::load(bytes),
        4 => Smf::load(bytes),
        5 => match std::str::from_utf8(bytes) {
            Ok(source) => Melody::parse(source).map(PreparedMusic::from),
            Err(_) => return,
        },
        6 => {
            if let Ok(source) = std::str::from_utf8(bytes) {
                let _ = SoundConfig::from_blaster(source);
            }
            if let Ok(message) = MidiMessage::new(bytes) {
                assert_eq!(message.bytes(), bytes);
            }
            return;
        }
        _ => {
            if bytes.len() < 24 {
                return;
            }
            let offset = u32::from_le_bytes(bytes[..4].try_into().unwrap());
            let length = u32::from_le_bytes(bytes[4..8].try_into().unwrap());
            let at = Tick(u64::from_le_bytes(bytes[8..16].try_into().unwrap()));
            let duration = Tick(u64::from_le_bytes(bytes[16..24].try_into().unwrap()));
            PreparedMusic::new(
                MusicTarget::Midi,
                vec![TimedEvent {
                    at,
                    event: MusicEvent::SysEx { offset, length },
                }],
                duration,
                &bytes[24..],
            )
        }
    };
    if let Ok(music) = parsed {
        check_music(&music);
    }
});
