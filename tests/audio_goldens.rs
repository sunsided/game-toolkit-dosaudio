//! Original, headless synthesis baselines; provenance and review policy: fixtures/audio.md.
#![cfg(feature = "opl")]

use game_toolkit_dosaudio::Wav;
use game_toolkit_dosaudio::prelude::*;
use std::time::Duration;

const RATE: u32 = 48_000;
const DURATION_MS: u64 = 750;

// A quiet modulator and sustained sine carrier: F-number 580, block 4 (~440 Hz).
fn instrument(bank: u16, pan: u8, low_octave: bool) -> Vec<(u16, u8)> {
    vec![
        (bank | 0x20, 0x21),
        (bank | 0x23, 0x21),
        (bank | 0x40, 0x3f),
        (bank | 0x43, 0x08),
        (bank | 0x60, 0xf0),
        (bank | 0x63, 0xf0),
        (bank | 0x80, 0x08),
        (bank | 0x83, 0x08),
        (bank | 0xc0, pan),
        (bank | 0xa0, if low_octave { 0x22 } else { 0x44 }),
        (bank | 0xb0, if low_octave { 0x31 } else { 0x32 }),
    ]
}

fn event(ms: u64, register: u16, value: u8) -> TimedEvent {
    TimedEvent {
        at: Tick::from_micros(ms * 1_000),
        event: MusicEvent::Opl { register, value },
    }
}

fn dual_trace() -> Sequence {
    let mut events: Vec<_> = instrument(0, 0, false)
        .into_iter()
        .map(|(r, v)| event(0, r, v))
        .collect();
    events.extend(
        instrument(0x100, 0, true)
            .into_iter()
            .map(|(r, v)| event(100, r, v)),
    );
    events.extend([event(400, 0xb0, 0x12), event(500, 0x1b0, 0x11)]);
    Sequence::new(OplChip::DualOpl2, events, Tick::from_micros(750_000)).unwrap()
}

#[cfg(feature = "formats")]
fn imf_tone(low_octave: bool) -> Sequence {
    let mut bytes = Vec::new();
    for (register, value) in instrument(0, 0, low_octave) {
        bytes.extend([register as u8, value]);
        // IMF delay follows the write. At 1 kHz the last setup write holds 400 ms.
        bytes.extend((if register == 0xb0 { 400u16 } else { 0 }).to_le_bytes());
    }
    bytes.extend([0xb0, if low_octave { 0x11 } else { 0x12 }]);
    bytes.extend(350u16.to_le_bytes());
    Imf::load_type0(&bytes, 1_000).unwrap()
}

#[cfg(feature = "formats")]
fn dro_pan(swapped: bool) -> Sequence {
    let left = if swapped { 0x20 } else { 0x10 };
    let right = if swapped { 0x10 } else { 0x20 };
    let mut writes = vec![(0x105, 1)];
    writes.extend(instrument(0, left, false));
    let mut map = Vec::new();
    let mut data = Vec::new();
    {
        let mut write = |register: u16, value: u8| {
            let low = register as u8;
            let index = map.iter().position(|&r| r == low).unwrap_or_else(|| {
                map.push(low);
                map.len() - 1
            });
            data.extend([
                index as u8 | if register & 0x100 != 0 { 0x80 } else { 0 },
                value,
            ]);
        };
        for (register, value) in writes {
            write(register, value);
        }
        // 200 ms left, then 200 ms right, then key-off and a 350 ms release tail.
        // Delay code 0xfe is short (value+1 ms); 0xff is long ((value+1)*256 ms).
    }
    data.extend([0xfe, 199]);
    let pan_index = map.iter().position(|&r| r == 0xc0).unwrap() as u8;
    let key_index = map.iter().position(|&r| r == 0xb0).unwrap() as u8;
    data.extend([
        pan_index, right, 0xfe, 199, key_index, 0x12, 0xff, 0, 0xfe, 93,
    ]);
    let mut bytes = b"DBRAWOPL".to_vec();
    bytes.extend(2u16.to_le_bytes());
    bytes.extend(0u16.to_le_bytes());
    bytes.extend(((data.len() / 2) as u32).to_le_bytes());
    bytes.extend(750u32.to_le_bytes());
    bytes.extend([2, 0, 0, 0xfe, 0xff, map.len() as u8]);
    bytes.extend(map);
    bytes.extend(data);
    Dro::load(&bytes).unwrap()
}

fn capture(sequence: Sequence, backend: OplBackend) -> Wav {
    let config = match sequence.chip() {
        OplChip::Opl2 => SoundConfig::adlib(),
        OplChip::Opl3 => SoundConfig::opl3(),
        OplChip::DualOpl2 => SoundConfig::dual_opl2(),
    };
    let mut machine = OfflineMachine::new(config.opl_backend(backend), RATE).unwrap();
    let _music = machine.play(sequence).unwrap();
    machine
        .render_for(Duration::from_millis(DURATION_MS))
        .unwrap();
    let wav = machine.finish();
    assert_eq!(wav.sample_rate(), RATE);
    assert_eq!(wav.frames().len(), 35_999);
    assert!(
        wav.frames()
            .iter()
            .flatten()
            .all(|v| v.is_finite() && v.abs() < 1.0)
    );
    wav
}

// FNV-1a-64 over interleaved stereo PCM16 LE data only, excluding the WAV header.
fn pcm_hash(wav: &Wav) -> u64 {
    wav.to_bytes().unwrap()[44..]
        .iter()
        .fold(0xcbf29ce484222325, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
        })
}

#[test]
fn oplon_dual_register_trace_pcm_golden() {
    assert_eq!(
        pcm_hash(&capture(dual_trace(), OplBackend::Oplon)),
        18_132_943_180_728_515_164
    );
}

#[cfg(feature = "nuked")]
#[test]
fn nuked_dual_register_trace_pcm_golden() {
    assert_eq!(
        pcm_hash(&capture(dual_trace(), OplBackend::Nuked)),
        16_305_653_591_510_393_370
    );
}

#[cfg(feature = "formats")]
#[test]
fn oplon_imf_and_dro_pcm_goldens() {
    assert_eq!(
        (
            pcm_hash(&capture(imf_tone(false), OplBackend::Oplon)),
            pcm_hash(&capture(dro_pan(false), OplBackend::Oplon))
        ),
        (8_251_495_682_276_887_477, 11_970_232_720_812_347_942)
    );
}

#[cfg(all(feature = "formats", feature = "nuked"))]
#[test]
fn nuked_imf_and_dro_pcm_goldens() {
    assert_eq!(
        (
            pcm_hash(&capture(imf_tone(false), OplBackend::Nuked)),
            pcm_hash(&capture(dro_pan(false), OplBackend::Nuked))
        ),
        (7_503_558_725_369_065_500, 832_608_049_917_537_739)
    );
}

#[cfg(all(feature = "formats", feature = "nuked"))]
mod comparison {
    use super::*;

    fn channel(wav: &Wav, start_ms: usize, end_ms: usize, channel: usize) -> Vec<f64> {
        wav.frames()[start_ms * RATE as usize / 1_000..end_ms * RATE as usize / 1_000]
            .iter()
            .map(|frame| f64::from(frame[channel]))
            .collect()
    }

    fn rms(samples: &[f64]) -> f64 {
        (samples.iter().map(|v| v * v).sum::<f64>() / samples.len() as f64).sqrt()
    }

    // Rectangular-window DFT: the 200 ms window includes exactly 88 cycles at 440 Hz.
    fn spectral_fraction(samples: &[f64], hz: f64) -> f64 {
        let step = std::f64::consts::TAU * hz / f64::from(RATE);
        let (mut re, mut im) = (0.0, 0.0);
        for (i, sample) in samples.iter().enumerate() {
            let (sin, cos) = (step * i as f64).sin_cos();
            re += sample * cos;
            im += sample * sin;
        }
        2.0 * (re * re + im * im)
            / samples.len() as f64
            / samples.iter().map(|v| v * v).sum::<f64>()
    }

    #[derive(Debug)]
    struct Tone {
        pitch: f64,
        fundamental: f64,
        harmonics: f64,
        level: f64,
        attack: f64,
        release: f64,
        release_middle: f64,
    }

    fn tone(wav: &Wav) -> Tone {
        let steady = channel(wav, 100, 300, 0);
        let level = rms(&steady);
        let pitch = (200..=500)
            .step_by(2)
            .map(f64::from)
            .max_by(|a, b| {
                spectral_fraction(&steady, *a).total_cmp(&spectral_fraction(&steady, *b))
            })
            .unwrap();
        Tone {
            pitch,
            fundamental: spectral_fraction(&steady, 440.0),
            harmonics: spectral_fraction(&steady, 880.0) + spectral_fraction(&steady, 1_320.0),
            level,
            attack: rms(&channel(wav, 0, 10, 0)) / level,
            release: rms(&channel(wav, 650, 700, 0)) / level,
            release_middle: rms(&channel(wav, 425, 450, 0)) / level,
        }
    }

    fn compare_tone(reference: &Tone, candidate: &Tone) -> Result<(), &'static str> {
        if (reference.pitch - 440.0).abs() > 4.0 || (candidate.pitch - reference.pitch).abs() > 4.0
        {
            return Err("pitch differs by more than 4 Hz");
        }
        if reference.fundamental < 0.95
            || candidate.fundamental < 0.95
            || (reference.fundamental - candidate.fundamental).abs() > 0.02
            || reference.harmonics > 0.02
            || candidate.harmonics > 0.02
        {
            return Err("sine spectrum changed");
        }
        if reference.level < 0.005 || !(0.9..=1.1).contains(&(candidate.level / reference.level)) {
            return Err("sustained level changed");
        }
        if !(0.8..=1.2).contains(&reference.attack)
            || !(0.8..=1.2).contains(&candidate.attack)
            || (reference.attack - candidate.attack).abs() > 0.05
            || !(0.05..=0.4).contains(&reference.release_middle)
            || !(0.05..=0.4).contains(&candidate.release_middle)
            || (reference.release_middle - candidate.release_middle).abs() > 0.05
            || reference.release > 0.02
            || candidate.release > 0.02
        {
            return Err("attack or release envelope changed");
        }
        Ok(())
    }

    fn pan_ratios(wav: &Wav) -> [f64; 2] {
        [
            rms(&channel(wav, 50, 150, 0)) / rms(&channel(wav, 50, 150, 1)).max(1e-12),
            rms(&channel(wav, 250, 350, 1)) / rms(&channel(wav, 250, 350, 0)).max(1e-12),
        ]
    }

    fn compare_pan(reference: [f64; 2], candidate: [f64; 2]) -> Result<(), &'static str> {
        if reference
            .into_iter()
            .chain(candidate)
            .any(|ratio| ratio < 100.0)
        {
            return Err("OPL3 pan isolation is less than 40 dB");
        }
        Ok(())
    }

    #[test]
    fn cores_agree_on_pitch_spectrum_envelope_and_stereo() {
        let oplon = tone(&capture(imf_tone(false), OplBackend::Oplon));
        let nuked = tone(&capture(imf_tone(false), OplBackend::Nuked));
        assert!(
            compare_tone(&oplon, &nuked).is_ok(),
            "oplon: {oplon:?}; nuked: {nuked:?}"
        );
        let oplon = pan_ratios(&capture(dro_pan(false), OplBackend::Oplon));
        let nuked = pan_ratios(&capture(dro_pan(false), OplBackend::Nuked));
        assert!(
            compare_pan(oplon, nuked).is_ok(),
            "oplon: {oplon:?}; nuked: {nuked:?}"
        );
    }

    #[test]
    fn comparison_rejects_wrong_pitch_and_swapped_pan() {
        let reference = tone(&capture(imf_tone(false), OplBackend::Oplon));
        let wrong = tone(&capture(imf_tone(true), OplBackend::Nuked));
        assert_eq!(
            compare_tone(&reference, &wrong),
            Err("pitch differs by more than 4 Hz")
        );
        let reference = pan_ratios(&capture(dro_pan(false), OplBackend::Oplon));
        let wrong = pan_ratios(&capture(dro_pan(true), OplBackend::Nuked));
        assert_eq!(
            compare_pan(reference, wrong),
            Err("OPL3 pan isolation is less than 40 dB")
        );
    }
}
