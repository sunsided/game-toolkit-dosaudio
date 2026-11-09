//! Public-interface invariants shared by the bounded fuzz targets.
use game_toolkit_dosaudio::{Frame, MidiBackend, MidiMessage, MidiSynth, prelude::*};
use std::sync::{
    Arc,
    atomic::{AtomicU64, AtomicUsize, Ordering},
};

pub fn check_music(music: &PreparedMusic) {
    assert!(
        music
            .events()
            .windows(2)
            .all(|pair| pair[0].at <= pair[1].at)
    );
    assert!(
        music
            .events()
            .iter()
            .all(|event| event.at <= music.duration())
    );
    assert!(music.loop_start() <= music.duration());
    PreparedMusic::new(
        music.target(),
        music.events().to_vec(),
        music.duration(),
        music.sysex(),
    )
    .expect("a successful reader must return valid events and SysEx storage");
}

pub type PlaybackTrace = (Vec<Frame>, Vec<(Tick, Option<MusicDevice>, bool, bool)>);

pub fn playback(program: &[u8], chunk: usize) -> PlaybackTrace {
    let duration = Tick::from_micros(2000);
    let mut events = Vec::new();
    for (register, value) in [
        (0x20, 1),
        (0x23, 1),
        (0x40, 63),
        (0x43, 0),
        (0x60, 0xf0),
        (0x63, 0xf0),
        (0x80, 0x77),
        (0x83, 0x77),
        (0xc0, 1),
        (0xa0, 0x98),
        (0xb0, 0x31),
    ] {
        events.push(TimedEvent {
            at: Tick::ZERO,
            event: MusicEvent::Opl { register, value },
        });
    }
    let mut soundtrack = Soundtrack::new();
    soundtrack
        .insert(
            MusicDevice::Opl2,
            PreparedMusic::new(MusicTarget::Opl(OplChip::Opl2), events, duration, []).unwrap(),
        )
        .unwrap();
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
    let mut controller = engine.music_controller(8).unwrap();
    let handle = controller
        .play_soundtrack(
            Arc::new(soundtrack),
            MusicDevice::Opl2,
            PlayOptions::looped(),
        )
        .unwrap();
    let mut frames = Vec::new();
    let mut states = Vec::new();
    for instruction in program.as_chunks::<3>().0 {
        match instruction[0] % 9 {
            0 => {
                let _ = handle.switch_device(MusicDevice::Opl2);
            }
            1 => {
                let _ = handle.switch_device(MusicDevice::Speaker);
            }
            2 => handle.pause(),
            3 => handle.resume(),
            4 => handle.set_volume(f32::from(instruction[1]) / 255.0),
            5 => handle
                .fade_volume(
                    f32::from(instruction[1]) / 255.0,
                    std::time::Duration::from_millis(1),
                )
                .unwrap(),
            6 => {
                drop(handle.clone());
            }
            7 => handle.stop(),
            _ => {
                let _ = controller.send_event(MusicEvent::Opl {
                    register: 0xa0,
                    value: instruction[1],
                });
            }
        }
        let start = frames.len();
        frames.resize(start + usize::from(instruction[2] % 65), [0.0; 2]);
        for block in frames[start..].chunks_mut(chunk) {
            engine.render(block);
        }
        assert!(
            frames[start..]
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
        controller.reclaim();
    }
    (frames, states)
}

#[derive(Default)]
struct Messages {
    hash: AtomicU64,
    count: AtomicUsize,
}
struct Probe {
    messages: Arc<Messages>,
    held: f32,
}
impl MidiSynth for Probe {
    fn sample_rate(&self) -> u32 {
        48_000
    }
    fn message(&mut self, bytes: &[u8]) {
        if bytes.first() == Some(&0xf0) {
            assert_eq!(bytes.last(), Some(&0xf7));
            assert!((2..=65_536).contains(&bytes.len()));
            assert!(bytes[1..bytes.len() - 1].iter().all(|byte| *byte < 128));
        } else {
            MidiMessage::new(bytes).expect("UART must frame complete short MIDI messages");
        }
        let mut hash = self.messages.hash.load(Ordering::Relaxed);
        for byte in bytes {
            hash = hash.wrapping_mul(0x100_0000_01b3) ^ u64::from(*byte);
        }
        self.messages
            .hash
            .store(hash ^ bytes.len() as u64, Ordering::Relaxed);
        self.messages.count.fetch_add(1, Ordering::Relaxed);
        self.held = f32::from(bytes.last().copied().unwrap_or(0) & 127) / 512.0;
    }
    fn render(&mut self, frames: &mut [Frame]) {
        frames.fill([self.held; 2]);
    }
}

pub fn midi_uart(bytes: &[u8], chunk: usize) -> (Vec<Frame>, u64, usize) {
    let messages = Arc::new(Messages::default());
    let sink = messages.clone();
    let backend = MidiBackend::Custom(Arc::new(move |_| {
        Ok(Box::new(Probe {
            messages: sink.clone(),
            held: 0.0,
        }))
    }));
    let (mut ports, mut engine) = DosAudio::split(
        SoundConfig::none()
            .mpu401(backend)
            .latency(Tick::ZERO)
            .queue_capacity(1024),
    )
    .unwrap();
    ports.outb(0x331, 0x3f);
    assert_eq!(ports.inb(0x330), 0xfe);
    for (index, byte) in bytes.iter().enumerate() {
        ports
            .try_outb_at(Tick::from_micros(index as u64 * 80), 0x330, *byte)
            .unwrap();
    }
    let mut frames = vec![[0.0; 2]; 2304];
    for block in frames.chunks_mut(chunk) {
        engine.render(block);
    }
    assert!(
        frames
            .iter()
            .flatten()
            .all(|s| s.is_finite() && (-1.0..=1.0).contains(s))
    );
    (
        frames,
        messages.hash.load(Ordering::Relaxed),
        messages.count.load(Ordering::Relaxed),
    )
}
