//! Public port protocols and audio timing, without a host audio device.
use game_toolkit_dosaudio::Error;
use game_toolkit_dosaudio::prelude::*;

fn speaker() -> (DosAudio, AudioEngine) {
    DosAudio::split(SoundConfig::none().speaker(true).latency(Tick::ZERO)).unwrap()
}

#[test]
fn absent_hardware_and_unhandled_pit_channels_are_open_bus() {
    let (mut audio, mut engine) = DosAudio::split(SoundConfig::none()).unwrap();
    for port in [0x42, 0x61, 0x220, 0x388, 0x389] {
        assert_eq!(audio.inb(port), 0xff);
    }
    audio.outb(0x388, 0x20);
    let mut samples = [[1.0; 2]; 500];
    engine.render(&mut samples);
    assert_eq!(samples, [[0.0; 2]; 500]);
    let (mut audio, _) = speaker();
    audio.outb(0x43, 0x36); // Channel zero belongs to the host, not speaker control.
    assert_eq!(audio.inb(0x61) & 3, 0);
}

#[cfg(not(feature = "sb"))]
#[test]
fn sound_blaster_configuration_requires_its_feature() {
    assert!(matches!(
        DosAudio::split(SoundConfig::sb2()),
        Err(Error::Config(_))
    ));
}

#[test]
fn speaker_gate_counter_and_refresh_bits_follow_the_machine_clock() {
    let (mut audio, _) = speaker();
    audio.outb(0x43, 0xb6);
    audio.outb(0x42, 5);
    audio.outb(0x42, 0);
    audio.outb(0x61, 3);
    assert_eq!(audio.inb(0x61) & 0x20, 0x20);
    audio.advance(Tick(36)); // Odd divisor: three PIT clocks high, two low.
    assert_eq!(audio.inb(0x61) & 0x20, 0);
    audio.advance(Tick(24));
    assert_eq!(audio.inb(0x61) & 0x20, 0x20);
    audio.advance_to(Tick(216));
    assert_eq!(audio.inb(0x61) & 0x10, 0x10);
    audio.outb(0x61, 2); // Gate low holds PIT output high.
    assert_eq!(audio.inb(0x61) & 0x20, 0x20);
}

#[test]
fn speaker_latch_survives_time_between_counter_byte_reads() {
    let (mut audio, _) = speaker();
    audio.outb(0x43, 0xb6);
    audio.outb(0x42, 0);
    audio.outb(0x42, 0); // Zero encodes 65,536.
    audio.outb(0x61, 3);
    audio.outb(0x43, 0x80); // Latch channel two.
    assert_eq!(audio.inb(0x42), 0);
    audio.advance(Tick(12));
    assert_eq!(audio.inb(0x42), 0);
}

#[test]
fn a_pulse_inside_one_sample_is_integrated_instead_of_disappearing() {
    let (mut full, mut full_engine) = speaker();
    full.outb(0x61, 2);
    let mut reference = [[0.0; 2]];
    full_engine.render(&mut reference);
    let (mut pulse, mut pulse_engine) = speaker();
    pulse.try_outb_at(Tick(100), 0x61, 2).unwrap();
    pulse.try_outb_at(Tick(200), 0x61, 0).unwrap();
    let mut actual = [[0.0; 2]];
    pulse_engine.render(&mut actual);
    let expected = 100.0 / (315_000_000.0 / (48_000.0 * 22.0));
    assert!((f64::from(actual[0][0] / reference[0][0]) - expected).abs() < 1e-6);
    assert_eq!(actual[0][0], actual[0][1]);
}

#[test]
fn full_queue_rejects_writes_without_changing_hardware_or_time() {
    let (mut audio, mut engine) = DosAudio::split(
        SoundConfig::none()
            .speaker(true)
            .latency(Tick::ZERO)
            .queue_capacity(1),
    )
    .unwrap();
    audio.outb(0x61, 2);
    assert_eq!(audio.try_outb_at(Tick(10), 0x61, 0), Err(Error::QueueFull));
    assert_eq!(audio.now(), Tick::ZERO);
    assert_eq!(audio.inb(0x61) & 2, 2);
    engine.render(&mut [[0.0; 2]]);
    audio.try_outb_at(Tick(10), 0x61, 0).unwrap();
    engine.render(&mut [[0.0; 2]]);
    assert_eq!(engine.stats().late_commands, 1);
    assert_eq!(
        audio.try_outb_at(Tick(9), 0x61, 2),
        Err(Error::BackwardsTime)
    );
}

#[test]
fn tone_helpers_validate_inputs_and_are_transactional() {
    let (mut audio, _) = speaker();
    assert!(audio.sound(f64::NAN).is_err());
    assert!(audio.sound(0.0).is_err());
    audio.sound(440.0).unwrap();
    assert_eq!(audio.inb(0x61) & 3, 3);
    audio.nosound().unwrap();
    assert_eq!(audio.inb(0x61) & 3, 0);
}

fn render_speaker(chunks: &[usize]) -> Vec<Frame> {
    let (mut audio, mut engine) = speaker();
    audio.sound(440.0).unwrap();
    audio
        .try_outb_at(Tick::from_micros(5_123), 0x61, 0)
        .unwrap();
    let mut output = vec![[0.0; 2]; 2_000];
    let mut cursor = 0;
    for chunk in chunks.iter().cycle() {
        let end = (cursor + chunk).min(output.len());
        engine.render(&mut output[cursor..end]);
        cursor = end;
        if cursor == output.len() {
            break;
        }
    }
    output
}

#[test]
fn speaker_output_is_identical_for_any_callback_chunk_size() {
    let whole = render_speaker(&[2_000]);
    assert!(whole.iter().any(|frame| frame[0].abs() > 0.01));
    assert_eq!(whole, render_speaker(&[1, 7, 31, 128, 513]));
}

#[test]
fn fixed_latency_delays_the_first_port_event() {
    let (mut audio, mut engine) = DosAudio::split(
        SoundConfig::none()
            .speaker(true)
            .latency(Tick::from_micros(10_000)),
    )
    .unwrap();
    audio.outb(0x61, 2);
    let mut early = [[1.0; 2]; 479];
    engine.render(&mut early);
    assert!(early.iter().all(|&frame| frame == [0.0; 2]));
    let mut later = [[0.0; 2]; 10];
    engine.render(&mut later);
    assert!(later.iter().any(|frame| frame[0] > 0.0));
    assert_eq!(engine.stats().late_commands, 0);
}

#[test]
fn control_half_attaches_through_the_published_bus_trait() {
    let (audio, _) = speaker();
    let mut device: Box<dyn IoDevice> = Box::new(audio.bus_device());
    assert!(device.ports().iter().any(|range| range.contains(0x61)));
    device.outb(Tick(100), 0x61, 2);
    assert_eq!(device.inb(Tick(100), 0x61) & 2, 2);
    device.advance_to(Tick(1_000));
    assert!(!device.irq_pending(Tick(1_000)));
}

#[cfg(feature = "opl")]
mod opl {
    use super::*;
    fn write(audio: &mut DosAudio, register: u8, value: u8) {
        audio.outb(0x388, register);
        audio.outb(0x389, value);
    }

    #[test]
    fn classic_adlib_detection_succeeds_without_rendering_audio() {
        for config in [SoundConfig::adlib(), SoundConfig::opl3()] {
            let (mut audio, _) = DosAudio::split(config).unwrap();
            write(&mut audio, 4, 0x60);
            write(&mut audio, 4, 0x80);
            assert_eq!(audio.inb(0x388) & 0xe0, 0);
            write(&mut audio, 2, 0xff);
            write(&mut audio, 4, 0x21);
            audio.wait_us(79);
            assert_eq!(audio.inb(0x388) & 0xe0, 0);
            audio.wait_us(2);
            assert_eq!(audio.inb(0x388) & 0xe0, 0xc0);
            write(&mut audio, 4, 0x80);
            assert_eq!(audio.inb(0x388) & 0xe0, 0);
        }
    }

    #[test]
    fn timer_two_masking_and_stop_preserve_expected_status() {
        let (mut audio, _) = DosAudio::split(SoundConfig::adlib()).unwrap();
        write(&mut audio, 3, 0xff);
        write(&mut audio, 4, 2);
        audio.wait_us(320);
        assert_eq!(audio.inb(0x388) & 0xe0, 0xa0);
        write(&mut audio, 4, 0); // Stop without acknowledging overflow.
        assert_eq!(audio.inb(0x388) & 0xe0, 0xa0);
        write(&mut audio, 4, 0x20); // Mask and clear timer two.
        assert_eq!(audio.inb(0x388) & 0xe0, 0);
    }

    #[test]
    fn opl2_and_opl3_expose_different_port_maps_and_status() {
        let (mut opl2, _) = DosAudio::split(SoundConfig::adlib()).unwrap();
        let (mut opl3, _) = DosAudio::split(SoundConfig::opl3()).unwrap();
        assert_eq!(opl2.inb(0x388), 6);
        assert_eq!(opl3.inb(0x388), 0);
        assert!(
            !IoDevice::ports(&opl2)
                .iter()
                .any(|range| range.contains(0x38b))
        );
        assert!(
            IoDevice::ports(&opl3)
                .iter()
                .any(|range| range.contains(0x38b))
        );
        assert_eq!(opl2.inb(0x38a), 0xff);
        assert_eq!(opl3.inb(0x389), 0xff);
    }

    #[test]
    fn resetting_timer_flags_preserves_the_next_overflow_phase() {
        let (mut audio, _) = DosAudio::split(SoundConfig::adlib()).unwrap();
        write(&mut audio, 2, 0xff);
        write(&mut audio, 4, 1);
        audio.wait_us(100);
        assert_eq!(audio.inb(0x388) & 0xe0, 0xc0);
        write(&mut audio, 4, 0x80);
        assert_eq!(audio.inb(0x388) & 0xe0, 0);
        audio.wait_us(61); // The next overflow is at 160 us, not 180 us.
        assert_eq!(audio.inb(0x388) & 0xe0, 0xc0);
    }
}
