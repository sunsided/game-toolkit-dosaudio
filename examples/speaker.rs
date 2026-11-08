//! Original PLAY melody followed by a pulse-width-modulated tone.
use game_toolkit_dosaudio::{Melody, OfflineMachine, PlayOptions, SoundConfig, speaker_pwm};
use std::{f64::consts::TAU, time::Duration};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let path = args.next().unwrap_or_else(|| "speaker.wav".into());
    if args.next().is_some() {
        return Err("usage: speaker [out.wav]".into());
    }
    let mut machine = OfflineMachine::new(SoundConfig::none().speaker(true), 48_000)?;
    let melody = Melody::parse("T180 O3 L8 MN C E G > C < G E D4 P8 MS G A G E C4")?;
    let duration = melody.duration();
    let tune = machine.play_speaker(melody, PlayOptions::default())?;
    machine.advance_to(duration)?;
    machine.render_for(Duration::from_millis(80))?;
    assert!(tune.is_finished());
    let samples: Vec<u8> = (0..4_000)
        .map(|i| {
            let time = f64::from(i) / 8_000.0;
            let envelope = (f64::from(i.min(3_999 - i)) / 160.0).min(1.0);
            (128.0 + 100.0 * envelope * (TAU * (220.0 * time + 220.0 * time * time)).sin()).round()
                as u8
        })
        .collect();
    let pwm = machine.play_speaker(speaker_pwm(&samples, 8_000)?, PlayOptions::default())?;
    machine.render_for(Duration::from_millis(600))?;
    assert!(pwm.is_finished());
    std::fs::write(&path, machine.finish().to_bytes()?)?;
    println!("Wrote {path}: original QBasic PLAY phrase, then PWM chirp");
    Ok(())
}
