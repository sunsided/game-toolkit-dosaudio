//! UART acknowledgments, MIDI framing and timestamped synthesis use public APIs.
#![cfg(feature = "mpu401")]

use game_toolkit_dosaudio::{
    AudioEngine, DosAudio, Error, Frame, IoDevice, MidiBackend, MidiSynth, SoundConfig, Tick,
};
use std::sync::{
    Arc,
    atomic::{AtomicU64, AtomicUsize, Ordering},
};

struct Records {
    messages: [AtomicU64; 64],
    count: AtomicUsize,
}

impl Records {
    fn new() -> Self {
        Self {
            messages: core::array::from_fn(|_| AtomicU64::new(0)),
            count: AtomicUsize::new(0),
        }
    }
    fn read(&self) -> Vec<Vec<u8>> {
        (0..self.count.load(Ordering::Acquire).min(64))
            .map(|index| {
                let encoded = self.messages[index].load(Ordering::Acquire);
                (0..(encoded >> 56) as usize)
                    .map(|byte| (encoded >> (byte * 8)) as u8)
                    .collect()
            })
            .collect()
    }
}

struct ProbeSynth {
    records: Arc<Records>,
    rate: u32,
    held: f32,
}

impl MidiSynth for ProbeSynth {
    fn sample_rate(&self) -> u32 {
        self.rate
    }
    fn message(&mut self, bytes: &[u8]) {
        let index = self.records.count.fetch_add(1, Ordering::AcqRel);
        if index < 64 {
            let mut encoded = (bytes.len().min(7) as u64) << 56;
            for (index, &byte) in bytes.iter().take(7).enumerate() {
                encoded |= u64::from(byte) << (index * 8);
            }
            self.records.messages[index].store(encoded, Ordering::Release);
        }
        match bytes {
            [0xff] => self.held = 0.0,
            [status, _, velocity] if status & 0xf0 == 0x90 => {
                self.held = f32::from(*velocity) / 127.0
            }
            [status, _, _] if status & 0xf0 == 0x80 => self.held = 0.0,
            _ => {}
        }
    }
    fn render(&mut self, out: &mut [Frame]) {
        out.fill([self.held; 2]);
    }
}

fn machine(capacity: usize, port: u16, native_rate: u32) -> (DosAudio, AudioEngine, Arc<Records>) {
    let records = Arc::new(Records::new());
    let sink = Arc::clone(&records);
    let backend = MidiBackend::Custom(Arc::new(move |_| {
        Ok(Box::new(ProbeSynth {
            records: Arc::clone(&sink),
            rate: native_rate,
            held: 0.0,
        }) as Box<dyn MidiSynth>)
    }));
    let (audio, engine) = DosAudio::split(
        SoundConfig::none()
            .mpu401(backend)
            .midi_port(port)
            .sample_rate(48_000)
            .latency(Tick::ZERO)
            .queue_capacity(capacity),
    )
    .unwrap();
    (audio, engine, records)
}

fn uart(audio: &mut DosAudio, base: u16) {
    audio.outb(base + 1, 0x3f);
    assert_eq!(audio.inb(base), 0xfe);
}

#[test]
fn reset_uart_ack_and_status_follow_the_two_port_protocol() {
    let (mut audio, mut engine, records) = machine(32, 0x330, 48_000);
    assert_eq!(audio.inb(0x331), 0x80);
    assert_eq!(audio.inb(0x330), 0xff);
    for value in [0x90, 60, 127] {
        audio.outb(0x330, value);
    }
    audio.outb(0x331, 0x42); // No intelligent-mode command implementation.
    assert_eq!(audio.inb(0x331), 0x80);
    engine.render(&mut [[0.0; 2]; 8]);
    assert!(records.read().is_empty());

    audio.outb(0x331, 0xff);
    assert_eq!(audio.inb(0x331), 0);
    assert_eq!(audio.inb(0x330), 0xfe);
    assert_eq!(audio.inb(0x331), 0x80);
    uart(&mut audio, 0x330);
    for value in [0x90, 60, 127] {
        audio.outb(0x330, value);
    }
    engine.render(&mut [[0.0; 2]; 8]);
    assert_eq!(records.read(), vec![vec![0xff], vec![0x90, 60, 127]]);
    audio.outb(0x331, 0xff);
    for value in [0x90, 61, 127] {
        audio.outb(0x330, value);
    }
    engine.render(&mut [[0.0; 2]; 8]);
    assert_eq!(records.read().last().unwrap(), &[0xff]);
    assert_eq!(audio.inb(0x330), 0xfe);
    assert_eq!(audio.inb(0x330), 0xff);
}

#[test]
fn acknowledgment_fifo_is_bounded_and_reset_replaces_pending_acks() {
    let (mut audio, _engine, _) = machine(32, 0x330, 48_000);
    for _ in 0..20 {
        audio.outb(0x331, 0x3f);
    }
    for _ in 0..16 {
        assert_eq!(audio.inb(0x330), 0xfe);
    }
    assert_eq!(audio.inb(0x330), 0xff);
    for _ in 0..4 {
        audio.outb(0x331, 0x3f);
    }
    audio.outb(0x331, 0xff);
    assert_eq!(audio.inb(0x330), 0xfe);
    assert_eq!(audio.inb(0x330), 0xff);
}

#[test]
fn parser_preserves_running_status_and_realtime_inside_short_messages_and_sysex() {
    let (mut audio, mut engine, records) = machine(64, 0x330, 48_000);
    uart(&mut audio, 0x330);
    for byte in [
        60, 0x90, 60, 0xf8, 100, 61, 0, 0xf0, 0x41, 0x10, 0xf8, 0x16, 0x12, 0xf7, 0xc2, 9, 10,
        0xf6, 11,
    ] {
        audio.outb(0x330, byte);
    }
    engine.render(&mut [[0.0; 2]; 8]);
    assert_eq!(
        records.read(),
        vec![
            vec![0xf8],
            vec![0x90, 60, 100],
            vec![0x90, 61, 0],
            vec![0xf8],
            vec![0xf0, 0x41, 0x10, 0x16, 0x12, 0xf7],
            vec![0xc2, 9],
            vec![0xc2, 10],
            vec![0xf6],
        ]
    );
}

#[test]
fn incomplete_sysex_survives_render_chunks_and_a_new_status_resynchronizes() {
    let (mut audio, mut engine, records) = machine(32, 0x330, 48_000);
    uart(&mut audio, 0x330);
    for byte in [0xf0, 0x41, 0x10] {
        audio.outb(0x330, byte);
    }
    engine.render(&mut [[0.0; 2]; 8]);
    assert!(records.read().is_empty());
    for byte in [0x16, 0xf7] {
        audio.outb(0x330, byte);
    }
    engine.render(&mut [[0.0; 2]; 8]);
    for byte in [0xf0, 0x41, 0x90, 63, 99] {
        audio.outb(0x330, byte);
    }
    engine.render(&mut [[0.0; 2]; 8]);
    assert_eq!(
        records.read(),
        vec![vec![0xf0, 0x41, 0x10, 0x16, 0xf7], vec![0x90, 63, 99]]
    );
}

#[test]
fn relocated_uart_ports_are_advertised_and_old_ports_stay_open_bus() {
    let (mut audio, mut engine, records) = machine(32, 0x300, 48_000);
    assert!(
        audio
            .ports()
            .iter()
            .any(|range| range.contains(0x300) && range.contains(0x301))
    );
    assert!(!audio.ports().iter().any(|range| range.contains(0x330)));
    assert_eq!(audio.inb(0x330), 0xff);
    audio.outb(0x331, 0x3f);
    for byte in [0x90, 60, 127] {
        audio.outb(0x330, byte);
    }
    uart(&mut audio, 0x300);
    for byte in [0x90, 61, 100] {
        audio.outb(0x300, byte);
    }
    engine.render(&mut [[0.0; 2]; 8]);
    assert_eq!(records.read(), vec![vec![0x90, 61, 100]]);
}

#[test]
fn queue_overflow_changes_neither_reset_ack_nor_uart_mode() {
    let (mut audio, mut engine, records) = machine(1, 0x330, 48_000);
    uart(&mut audio, 0x330);
    audio.outb(0x330, 0x90); // Fill the sole audio-command slot.
    assert_eq!(
        audio.try_outb_at(Tick::ZERO, 0x331, 0xff),
        Err(Error::QueueFull)
    );
    assert_eq!(audio.inb(0x331), 0x80);
    assert_eq!(audio.inb(0x330), 0xff);
    assert_eq!(audio.now(), Tick::ZERO);
    engine.render(&mut [[0.0; 2]; 8]);
    audio.outb(0x330, 60);
    engine.render(&mut [[0.0; 2]; 8]);
    audio.outb(0x330, 127);
    engine.render(&mut [[0.0; 2]; 8]);
    assert_eq!(records.read(), vec![vec![0x90, 60, 127]]);
    audio.outb(0x331, 0xff);
    assert_eq!(audio.inb(0x330), 0xfe);
    engine.render(&mut [[0.0; 2]; 8]);
    assert_eq!(records.read().last().unwrap(), &[0xff]);
}

fn timed(chunks: &[usize]) -> (Vec<Frame>, Vec<Vec<u8>>) {
    let (mut audio, mut engine, records) = machine(32, 0x330, 8_000);
    uart(&mut audio, 0x330);
    for byte in [0x90, 60, 127] {
        audio
            .try_outb_at(Tick::from_micros(130), 0x330, byte)
            .unwrap();
    }
    for byte in [0x80, 60, 0] {
        audio
            .try_outb_at(Tick::from_micros(380), 0x330, byte)
            .unwrap();
    }
    let mut output = Vec::new();
    for &length in chunks {
        let mut frames = vec![[0.0; 2]; length];
        engine.render(&mut frames);
        output.extend(frames);
    }
    (output, records.read())
}

#[test]
fn native_rate_boundaries_and_chunk_sizes_preserve_timestamped_output() {
    let (whole, messages) = timed(&[36]);
    let (split, split_messages) = timed(&[1, 5, 7, 3, 11, 9]);
    assert_eq!(whole, split);
    assert_eq!(messages, split_messages);
    assert_eq!(messages, vec![vec![0x90, 60, 127], vec![0x80, 60, 0]]);
    // Commands at 130/380 us reach an 8 kHz synth at native 250/500 us.
    // A rounded crystal timestamp can put a tiny sliver in the preceding frame.
    assert!(whole[..11].iter().all(|frame| *frame == [0.0; 2]));
    assert!(
        whole[12..23]
            .iter()
            .all(|frame| frame[0] > 0.99 && frame[1] > 0.99)
    );
    assert!(whole[25..].iter().all(|frame| *frame == [0.0; 2]));
}

#[test]
fn receiving_messages_and_sysex_neither_allocates_nor_frees_in_render() {
    let (mut audio, mut engine, records) = machine(32, 0x330, 48_000);
    uart(&mut audio, 0x330);
    for byte in [0x90, 60, 100, 0xf0, 0x41, 0x10, 0x16, 0xf7, 0xff] {
        audio.outb(0x330, byte);
    }
    let mut output = [[0.0; 2]; 32];
    let measured = allocation_counter::measure(|| engine.render(&mut output));
    assert_eq!(measured.count_total, 0);
    assert_eq!(measured.count_current, 0);
    assert_eq!(
        records.read(),
        vec![
            vec![0x90, 60, 100],
            vec![0xf0, 0x41, 0x10, 0x16, 0xf7],
            vec![0xff]
        ]
    );
}

#[test]
fn absent_uart_ports_are_open_bus() {
    let (mut audio, mut engine) = DosAudio::split(SoundConfig::none()).unwrap();
    for (port, value) in [(0x331, 0xff), (0x331, 0x3f), (0x330, 0x90)] {
        audio.outb(port, value);
    }
    assert_eq!(audio.inb(0x330), 0xff);
    assert_eq!(audio.inb(0x331), 0xff);
    let mut out = [[1.0; 2]; 8];
    engine.render(&mut out);
    assert_eq!(out, [[0.0; 2]; 8]);
}
