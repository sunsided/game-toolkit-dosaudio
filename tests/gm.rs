//! An original generated sine-loop SF2 exercises the optional General MIDI adapter.
#![cfg(feature = "gm")]

use game_toolkit_dosaudio::{Frame, GmSynth, MidiSynth};
use std::{f64::consts::TAU, sync::Arc};

fn chunk(id: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut out = id.to_vec();
    out.extend((data.len() as u32).to_le_bytes());
    out.extend(data);
    if !data.len().is_multiple_of(2) {
        out.push(0);
    }
    out
}

fn list(kind: &[u8; 4], children: &[Vec<u8>]) -> Vec<u8> {
    let mut data = kind.to_vec();
    for child in children {
        data.extend(child);
    }
    chunk(b"LIST", &data)
}

fn name(text: &[u8]) -> Vec<u8> {
    let mut out = vec![0; 20];
    out[..text.len()].copy_from_slice(text);
    out
}

fn preset(text: &[u8], program: u16, bag: u16) -> Vec<u8> {
    let mut out = name(text);
    for value in [program, 0, bag] {
        out.extend(value.to_le_bytes());
    }
    out.extend([0; 12]);
    out
}

fn sample_header(text: &[u8], terminal: bool) -> Vec<u8> {
    let mut out = name(text);
    for value in if terminal {
        [110_u32, 110, 110, 110, 32_000]
    } else {
        [0, 64, 0, 64, 32_000]
    } {
        out.extend(value.to_le_bytes());
    }
    out.extend([60, 0]); // Root key C4, zero pitch correction.
    out.extend(0_u16.to_le_bytes());
    out.extend(1_u16.to_le_bytes());
    out
}

fn font_bytes() -> Vec<u8> {
    // One original 64-sample sine period, then SF2's 46 guard samples. There
    // are two presets selecting the same tiny instrument to exercise programs.
    let mut pcm = Vec::new();
    for index in 0..110 {
        let sample = if index < 64 {
            (24_000.0 * (TAU * f64::from(index) / 64.0).sin()).round() as i16
        } else {
            0
        };
        pcm.extend(sample.to_le_bytes());
    }
    let mut phdr = preset(b"Original sine", 0, 0);
    phdr.extend(preset(b"Original sine 2", 1, 1));
    phdr.extend(preset(b"EOP", 0, 2));
    let mut inst = name(b"Original loop");
    inst.extend(0_u16.to_le_bytes());
    inst.extend(name(b"EOI"));
    inst.extend(1_u16.to_le_bytes());
    let generators = [(38_u16, (-6000_i16) as u16), (54, 1), (53, 0), (0, 0)];
    let igen: Vec<_> = generators
        .into_iter()
        .flat_map(|(kind, value)| kind.to_le_bytes().into_iter().chain(value.to_le_bytes()))
        .collect();
    let pgen: Vec<_> = [(41_u16, 0_u16), (41, 0), (0, 0)]
        .into_iter()
        .flat_map(|(kind, value)| kind.to_le_bytes().into_iter().chain(value.to_le_bytes()))
        .collect();
    let mut shdr = sample_header(b"Original sine", false);
    shdr.extend(sample_header(b"EOS", true));
    let pdta = list(
        b"pdta",
        &[
            chunk(b"phdr", &phdr),
            chunk(b"pbag", &[0, 0, 0, 0, 1, 0, 0, 0, 2, 0, 0, 0]),
            chunk(b"pmod", &[0; 10]),
            chunk(b"pgen", &pgen),
            chunk(b"inst", &inst),
            chunk(b"ibag", &[0, 0, 0, 0, 3, 0, 0, 0]),
            chunk(b"imod", &[0; 10]),
            chunk(b"igen", &igen),
            chunk(b"shdr", &shdr),
        ],
    );
    let info = list(
        b"INFO",
        &[
            chunk(b"ifil", &[2, 0, 1, 0]),
            chunk(b"isng", b"EMU8000\0"),
            chunk(b"INAM", b"Original sine fixture\0"),
        ],
    );
    let mut body = b"sfbk".to_vec();
    body.extend(info);
    body.extend(list(b"sdta", &[chunk(b"smpl", &pcm)]));
    body.extend(pdta);
    chunk(b"RIFF", &body)
}

fn font() -> Arc<rustysynth::SoundFont> {
    GmSynth::load_soundfont(&mut font_bytes().as_slice()).unwrap()
}
fn synth() -> GmSynth {
    let mut synth = GmSynth::with_soundfont(font(), 48_000).unwrap();
    // Isolate channel/envelope assertions from the default GM reverb tail.
    synth.message(&[0xb0, 91, 0]);
    synth
}
fn energy(frames: &[Frame], channel: usize) -> f64 {
    frames
        .iter()
        .map(|frame| f64::from(frame[channel]).powi(2))
        .sum::<f64>()
        / frames.len() as f64
}

#[test]
fn original_font_loads_from_read_only_stream_and_native_rates_are_bounded() {
    let font = font();
    assert_eq!(font.get_presets().len(), 2);
    assert!(GmSynth::with_soundfont(Arc::clone(&font), 0).is_err());
    for (requested, actual) in [
        (1, 16_000),
        (8_000, 16_000),
        (48_000, 48_000),
        (u32::MAX, 192_000),
    ] {
        assert_eq!(
            GmSynth::with_soundfont(Arc::clone(&font), requested)
                .unwrap()
                .sample_rate(),
            actual
        );
    }
    for bytes in [&[][..], &b"not a soundfont"[..], &b"RIFF\x04\0\0\0sfbk"[..]] {
        assert!(GmSynth::load_soundfont(&mut &*bytes).is_err());
    }
}

#[test]
fn notes_program_changes_and_pitch_bend_produce_finite_audible_output() {
    let mut plain = synth();
    let mut bent = synth();
    plain.message(&[0xc0, 1]);
    bent.message(&[0xc0, 1]);
    plain.message(&[0x90, 60, 110]);
    bent.message(&[0x90, 60, 110]);
    bent.message(&[0xe0, 0x7f, 0x7f]);
    let mut a = [[0.0; 2]; 4_800];
    let mut b = a;
    plain.render(&mut a);
    bent.render(&mut b);
    assert!(
        a.iter()
            .flatten()
            .chain(b.iter().flatten())
            .all(|value| value.is_finite())
    );
    assert!(energy(&a[480..], 0) > 0.0001);
    fn crossings(frames: &[Frame]) -> usize {
        frames
            .windows(2)
            .filter(|pair| pair[0][0] <= 0.0 && pair[1][0] > 0.0)
            .count()
    }
    assert!(crossings(&b[480..]) > crossings(&a[480..]) + 2);
}

#[test]
fn pan_and_channel_volume_change_the_expected_channel_energy() {
    let mut left = synth();
    let mut right = synth();
    left.message(&[0xb0, 10, 0]);
    right.message(&[0xb0, 10, 127]);
    left.message(&[0x90, 60, 110]);
    right.message(&[0x90, 60, 110]);
    let mut a = [[0.0; 2]; 2_400];
    let mut b = a;
    left.render(&mut a);
    right.render(&mut b);
    assert!(energy(&a, 0) > 10.0 * energy(&a, 1));
    assert!(energy(&b, 1) > 10.0 * energy(&b, 0));
    right.message(&[0xb0, 7, 0]);
    right.render(&mut b);
    assert!(energy(&b[64..], 1) < energy(&a[64..], 0) * 0.001);
}

#[test]
fn sustain_holds_a_released_note_until_the_pedal_is_released() {
    let mut held = synth();
    let mut released = synth();
    held.message(&[0xb0, 64, 127]);
    for synth in [&mut held, &mut released] {
        synth.message(&[0x90, 60, 110]);
        synth.render(&mut [[0.0; 2]; 480]);
        synth.message(&[0x80, 60, 0]);
    }
    let mut a = [[0.0; 2]; 4_800];
    let mut b = a;
    held.render(&mut a);
    released.render(&mut b);
    assert!(energy(&a[2_400..], 0) > 0.0001);
    assert!(energy(&b[2_400..], 0) < energy(&a[2_400..], 0) * 0.001);
    held.message(&[0xb0, 64, 0]);
    held.render(&mut a);
    assert!(energy(&a[2_400..], 0) < 0.00001);
}

#[test]
fn supported_resets_mute_voices_and_unknown_sysex_is_ignored() {
    for reset in [
        &[0xff][..],
        &[0xf0, 0x7e, 0x7f, 9, 1, 0xf7][..],
        &[0xf0, 0x7e, 0x10, 9, 3, 0xf7][..],
    ] {
        let mut synth = synth();
        synth.message(&[0x90, 60, 110]);
        let mut frames = [[0.0; 2]; 480];
        synth.render(&mut frames);
        assert!(energy(&frames, 0) > 0.0001);
        synth.message(&[0xf0, 0x41, 0x10, 0x16, 0x12, 0xf7]);
        synth.render(&mut frames);
        assert!(energy(&frames, 0) > 0.0001);
        synth.message(reset);
        synth.render(&mut frames);
        assert!(frames.iter().all(|frame| *frame == [0.0; 2]));
    }
}

#[test]
fn cached_native_blocks_are_chunk_independent_and_event_quantum_is_eight_frames() {
    let font = font();
    let mut whole = GmSynth::with_soundfont(Arc::clone(&font), 48_000).unwrap();
    let mut split = GmSynth::with_soundfont(font, 48_000).unwrap();
    for synth in [&mut whole, &mut split] {
        synth.message(&[0xb0, 91, 0]);
        synth.message(&[0x90, 60, 110]);
    }
    let mut a = [[0.0; 2]; 2_001];
    let mut b = a;
    whole.render(&mut a);
    for chunk in b.chunks_mut(7) {
        split.render(chunk);
    }
    assert_eq!(a, b);
    // Both have consumed one frame of the next cached eight-frame block.
    whole.message(&[0xb0, 120, 0]);
    split.message(&[0xb0, 120, 0]);
    let mut a = [[0.0; 2]; 24];
    let mut b = a;
    whole.render(&mut a);
    for chunk in b.chunks_mut(3) {
        split.render(chunk);
    }
    assert_eq!(a, b);
    assert!(a[..7].iter().any(|frame| frame[0] != 0.0));
    assert!(a[7..].iter().all(|frame| *frame == [0.0; 2]));
}

#[test]
fn messages_rendering_and_gm_reset_neither_allocate_nor_free_resources() {
    let mut synth = synth();
    let mut frames = [[0.0; 2]; 257];
    let measured = allocation_counter::measure(|| {
        synth.message(&[0xc0, 1]);
        synth.message(&[0xb0, 10, 32]);
        synth.message(&[0xe0, 0, 80]);
        synth.message(&[0x90, 60, 110]);
        synth.render(&mut frames);
        synth.message(&[0xb0, 64, 127]);
        synth.message(&[0x80, 60, 0]);
        synth.render(&mut frames);
        synth.message(&[0xf0, 0x7e, 0x7f, 9, 1, 0xf7]);
        synth.render(&mut frames);
        synth.message(&[0xff]);
        synth.render(&mut frames);
    });
    assert_eq!(measured.count_total, 0);
    assert_eq!(measured.count_current, 0);
    assert_eq!(measured.bytes_total, 0);
    assert_eq!(measured.bytes_current, 0);
}
