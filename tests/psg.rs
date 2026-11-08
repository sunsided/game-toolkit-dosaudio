//! Tandy PSG behavior through public port I/O, helpers and the shared renderer.

#[cfg(not(feature = "psg"))]
#[test]
fn psg_configuration_requires_its_feature() {
    use game_toolkit_dosaudio::{DosAudio, SoundConfig};
    assert!(DosAudio::split(SoundConfig::none().psg(true)).is_err());
}

#[cfg(feature = "psg")]
mod enabled {
    use game_toolkit_dosaudio::{Error, prelude::*};

    fn config() -> SoundConfig {
        SoundConfig::none()
            .psg(true)
            .sample_rate(48_000)
            .latency(Tick::ZERO)
    }

    fn write(ports: &mut DosAudio, bytes: &[u8]) {
        for &byte in bytes {
            ports.outb(0xc0, byte);
        }
    }

    fn render(bytes: &[u8], frames: usize, chunks: &[usize]) -> Vec<Frame> {
        let (mut ports, mut engine) = DosAudio::split(config()).unwrap();
        write(&mut ports, bytes);
        let mut result = vec![[0.0; 2]; frames];
        let mut offset = 0;
        for &chunk in chunks.iter().cycle() {
            let end = (offset + chunk).min(result.len());
            engine.render(&mut result[offset..end]);
            offset = end;
            if offset == result.len() {
                break;
            }
        }
        result
    }

    fn rising_edges(frames: &[Frame]) -> usize {
        frames
            .windows(2)
            .filter(|pair| pair[0][0] <= 0.0 && pair[1][0] > 0.0)
            .count()
    }

    #[test]
    fn power_on_is_muted_and_the_data_port_reads_as_open_bus() {
        let (mut ports, mut engine) = DosAudio::split(config()).unwrap();
        assert_eq!(ports.inb(0xc0), 0xff);
        let mut frames = [[1.0; 2]; 512];
        engine.render(&mut frames);
        assert!(frames.iter().all(|frame| *frame == [0.0; 2]));
        let (mut absent, _) = DosAudio::split(SoundConfig::none()).unwrap();
        assert!(absent.psg().is_err());
    }

    #[test]
    fn tone_helper_has_the_expected_pitch_and_each_channel_can_be_muted() {
        for channel in 0..3 {
            let (mut ports, mut engine) = DosAudio::split(config()).unwrap();
            ports.psg().unwrap().tone(channel, 440.0, 0).unwrap();
            let mut frames = [[0.0; 2]; 4_800];
            engine.render(&mut frames);
            assert!((43..=45).contains(&rising_edges(&frames)));
            assert!(frames.iter().all(|frame| frame[0] == frame[1]));
            assert!(frames.iter().any(|frame| frame[0] > 0.2));
            assert!(frames.iter().any(|frame| frame[0] < -0.2));
            ports.advance_to(engine.now());
            ports.psg().unwrap().off(channel).unwrap();
            engine.render(&mut frames[..32]);
            assert!(frames[..32].iter().all(|frame| *frame == [0.0; 2]));
        }
    }

    #[test]
    fn attenuation_uses_two_db_steps_and_fifteen_is_silent() {
        let loud = render(&[0x8e, 0x0f, 0x90], 512, &[512]);
        let quiet = render(&[0x8e, 0x0f, 0x91], 512, &[512]);
        let ratio = 10.0f32.powf(-0.1);
        for (a, b) in loud.iter().zip(&quiet) {
            assert!((b[0] - a[0] * ratio).abs() < 1e-6);
        }
        assert!(
            render(&[0x8e, 0x0f, 0x9f], 512, &[512])
                .iter()
                .all(|f| *f == [0.0; 2])
        );
    }

    #[test]
    fn latch_and_continuation_bytes_update_the_selected_register() {
        assert_eq!(
            render(&[0x80, 0x12, 0x85, 0x90], 1_024, &[1_024]),
            render(&[0x85, 0x12, 0x90], 1_024, &[1_024])
        );
        assert_eq!(
            render(&[0x8e, 0x0f, 0x9f, 0x02], 1_024, &[1_024]),
            render(&[0x8e, 0x0f, 0x92], 1_024, &[1_024])
        );
        assert_eq!(
            render(&[0xe5, 0x04, 0xf0], 1_024, &[1_024]),
            render(&[0xe4, 0xf0], 1_024, &[1_024])
        );
    }

    #[test]
    fn zero_tone_divisor_uses_the_documented_1024_period() {
        let frames = render(&[0x80, 0x00, 0x90], 4_800, &[4_800]);
        assert!((10..=11).contains(&rising_edges(&frames)));
    }

    #[test]
    fn periodic_noise_has_fifteen_bit_period_and_tone_two_can_clock_it() {
        let fixed = render(&[0xe0, 0xf0], 4_800, &[4_800]);
        // Input clock / 16 prescaler / 32 noise ticks / 15 shift-register bits.
        assert!((45..=47).contains(&rising_edges(&fixed)));
        let slower = render(&[0xe1, 0xf0], 4_800, &[4_800]);
        assert!((22..=24).contains(&rising_edges(&slower)));
        let slowest = render(&[0xe2, 0xf0], 4_800, &[4_800]);
        assert!((10..=12).contains(&rising_edges(&slowest)));
        // Muted tone 2 with divisor 32 supplies one falling edge every 64 native ticks.
        let tone_clock = render(&[0xc0, 0x02, 0xdf, 0xe3, 0xf0], 4_800, &[4_800]);
        assert!((22..=24).contains(&rising_edges(&tone_clock)));
    }

    #[test]
    fn white_noise_is_repeatable_distinct_and_restarts_from_a_fixed_seed() {
        let white = render(&[0xe4, 0xf0], 4_800, &[4_800]);
        assert_eq!(white, render(&[0xe4, 0xf0], 4_800, &[4_800]));
        assert_ne!(white, render(&[0xe0, 0xf0], 4_800, &[4_800]));
        assert!(white.iter().any(|frame| frame[0] > 0.0));
        assert!(white.iter().any(|frame| frame[0] < 0.0));

        // At 175 kHz, 176 output frames span exactly 225 native PSG ticks.
        // Reset at a shared boundary so both captures have the same resampling phase.
        let (mut ports, mut engine) = DosAudio::split(config().sample_rate(175_000)).unwrap();
        ports.psg().unwrap().noise(4, 0).unwrap();
        let mut first = [[0.0; 2]; 1_056];
        engine.render(&mut first);
        assert!(first.iter().any(|frame| frame[0] > 0.0));
        ports.advance_to(engine.now());
        ports.psg().unwrap().noise(4, 0).unwrap();
        let mut restarted = first;
        engine.render(&mut restarted);
        assert_eq!(first, restarted);
    }

    #[test]
    fn tone_and_noise_are_independent_of_callback_chunk_sizes() {
        let bytes = [
            0x8e, 0x0f, 0x92, 0xa3, 0x10, 0xb4, 0xc0, 0x02, 0xd5, 0xe7, 0xf3,
        ];
        assert_eq!(
            render(&bytes, 4_800, &[4_800]),
            render(&bytes, 4_800, &[1, 7, 83, 511])
        );
    }

    #[test]
    fn helper_validation_and_queue_overflow_leave_the_whole_batch_unchanged() {
        let (mut ports, mut engine) = DosAudio::split(config().queue_capacity(2)).unwrap();
        ports.start_capture().unwrap();
        for (channel, hz, level) in [
            (3, 440.0, 0),
            (0, 0.0, 0),
            (0, -1.0, 0),
            (0, f64::NAN, 0),
            (0, f64::INFINITY, 0),
            (0, 440.0, 16),
        ] {
            assert!(ports.psg().unwrap().tone(channel, hz, level).is_err());
        }
        assert!(ports.psg().unwrap().noise(8, 0).is_err());
        assert!(ports.psg().unwrap().noise(0, 16).is_err());
        assert!(ports.psg().unwrap().off(4).is_err());
        assert_eq!(
            ports.psg().unwrap().tone(0, 440.0, 0),
            Err(Error::QueueFull)
        );
        assert!(ports.finish_capture().unwrap().writes().is_empty());
        let mut frames = [[1.0; 2]; 32];
        engine.render(&mut frames);
        assert!(frames.iter().all(|frame| *frame == [0.0; 2]));
        ports.advance_to(engine.now());
        ports.psg().unwrap().noise(0, 0).unwrap();
        assert_eq!(ports.psg().unwrap().off(3), Err(Error::QueueFull));
        engine.render(&mut frames);
        assert!(frames.iter().any(|frame| frame[0] != 0.0));
    }

    #[test]
    fn first_render_and_live_noise_changes_allocate_and_deallocate_nothing() {
        let (mut ports, mut engine) = DosAudio::split(config()).unwrap();
        ports.psg().unwrap().tone(0, 440.0, 2).unwrap();
        ports.psg().unwrap().noise(4, 3).unwrap();
        let mut frames = [[0.0; 2]; 1_024];
        let allocations = allocation_counter::measure(|| {
            engine.render(&mut frames);
            ports.advance_to(engine.now());
            ports.psg().unwrap().noise(3, 4).unwrap();
            engine.render(&mut frames);
            ports.advance_to(engine.now());
            ports.psg().unwrap().off(0).unwrap();
            ports.psg().unwrap().off(3).unwrap();
            engine.render(&mut frames);
        });
        assert_eq!(allocations.count_total, 0);
        assert_eq!(allocations.count_current, 0);
    }
}
