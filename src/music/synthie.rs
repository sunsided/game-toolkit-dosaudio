//! synthie 0.8.0 songs compiled outside the callback into native/OPL/speaker arrangements.
use crate::{
    Error, MidiMessage, MusicDevice, MusicEvent, MusicTarget, OplChip, OplMidiDriver,
    PreparedMusic, Result, Soundtrack, Tick, TimedEvent,
};
use synthie::{
    audio::processor::SynthProcessor,
    params::{AudioEvent, DrumHit, MidiNote},
    pattern::{Action, Song},
};

fn tick(row: usize, song: &Song) -> Result<Tick> {
    let ticks = row as f64 * 60.0 * Tick::HZ_NUMERATOR as f64
        / (f64::from(song.tempo.as_rows_per_minute()) * Tick::HZ_DENOMINATOR as f64);
    if !ticks.is_finite() || ticks > u64::MAX as f64 {
        return Err(Error::TooLarge);
    }
    Ok(Tick(ticks.floor() as u64))
}
fn drum(hit: DrumHit) -> Result<u8> {
    Ok(match hit {
        DrumHit::Kick => 36,
        DrumHit::Snare => 38,
        DrumHit::HiHatClosed => 42,
        DrumHit::HiHatOpen => 46,
        DrumHit::Crash => 49,
        DrumHit::Clap => 39,
        DrumHit::TomLow => 45,
        DrumHit::TomMid => 47,
        DrumHit::TomHigh => 50,
        _ => return Err(Error::Format("unsupported synthie drum")),
    })
}

/// Compile the same song to native synthie, OPL and PC speaker arrangements.
///
/// Supports four synthie channels, up to four chord notes per lane, patch cells,
/// holds, releases and drums. Native output uses the song's real parameter
/// snapshots; OPL maps patch indices cyclically to the six original starter
/// timbres. Speaker plays the highest held melodic note and omits drums.
/// Arpeggio actions/enabled patch arpeggiators are rejected rather than silently
/// changing their timing. Empty songs, invalid channels/patches and songs longer
/// than 16 million native frames fail before rendering. `PlayOptions` owns looping;
/// one finite pass is prepared regardless of the song's `looped` flag.
///
/// Native PCM is rendered once on this calling thread, preserving synthie's
/// instrument/FX implementation while reusing dosaudio's controls and lifetime
/// management. No upstream sequencer rewrite or extra audio host is needed.
pub fn soundtrack(song: &Song, chip: OplChip, sample_rate: u32) -> Result<Soundtrack> {
    if song.is_empty() || song.lanes.len() > 64 || !(8_000..=192_000).contains(&sample_rate) {
        return Err(Error::Config(
            "synthie requires a nonempty song, at most 64 lanes and an 8–192 kHz rate",
        ));
    }
    if song.patches.iter().any(|p| p.arp.enabled) {
        return Err(Error::Format(
            "synthie arpeggiators are not supported by chip arrangement compilation",
        ));
    }
    for lane in &song.lanes {
        if lane.channel.0 >= 4 {
            return Err(Error::Format(
                "synthie arrangements support channels 0 through 3",
            ));
        }
        for cell in &lane.rows {
            if cell
                .patch
                .is_some_and(|p| usize::from(p) >= song.patches.len())
            {
                return Err(Error::Format(
                    "synthie patch index is outside the song bank",
                ));
            }
            if matches!(cell.action, Action::Arp { .. }) {
                return Err(Error::Format("synthie chip arpeggio cells are unsupported"));
            }
            if let Action::Chord { count, .. } = cell.action
                && !(2..=4).contains(&count)
            {
                return Err(Error::Format("invalid synthie chord count"));
            }
        }
    }
    if song.tempo.frames_per_row(sample_rate as f32) < 1.0 {
        return Err(Error::Format(
            "synthie row duration must be at least one native frame",
        ));
    }
    let rows = song.len();
    let duration = tick(rows, song)?;
    let frames = (rows as f64 * song.tempo.frames_per_row(sample_rate as f32)).floor();
    if !frames.is_finite() || frames + f64::from(sample_rate) * 0.25 > 16_000_000.0 || rows > 65_536
    {
        return Err(Error::TooLarge);
    }
    let frames = frames as usize;
    let mut processor = SynthProcessor::<4>::new(sample_rate as f32);
    let tail = sample_rate as usize / 4;
    let mut pcm = vec![[0.0; 2]; frames + tail];
    let mut interleaved = Vec::new();
    let mut native_events = Vec::new();
    let mut midi = Vec::new();
    let mut speaker = Vec::new();
    let mut held: Vec<Vec<MidiNote>> = vec![vec![]; song.lanes.len()];
    let mut speaker_note = None;
    let add = |out: &mut Vec<TimedEvent>, at, bytes: &[u8]| -> Result<()> {
        out.push(TimedEvent {
            at,
            event: MusicEvent::Midi(MidiMessage::new(bytes)?),
        });
        Ok(())
    };
    for row in 0..rows {
        let at = tick(row, song)?;
        native_events.clear();
        for (i, lane) in song.lanes.iter().enumerate() {
            let Some(cell) = lane.rows.get(row) else {
                continue;
            };
            let ch = lane.channel;
            if let Some(patch) = cell.patch {
                native_events.push(AudioEvent::LoadPatchChannel(
                    ch,
                    Box::new(song.patches[usize::from(patch)].clone()),
                ));
                add(
                    &mut midi,
                    at,
                    &[0xc0 | ch.0, [0, 24, 40, 64, 80, 81][usize::from(patch) % 6]],
                )?;
            }
            if !matches!(cell.action, Action::Hold | Action::Drums(_)) {
                for note in held[i].drain(..) {
                    native_events.push(AudioEvent::NoteOffChannel(ch, note));
                    add(&mut midi, at, &[0x80 | ch.0, note.0, 0])?;
                }
            }
            let notes: &[MidiNote] = match &cell.action {
                Action::Note(note) => std::slice::from_ref(note),
                Action::Chord { notes, count } => &notes[..usize::from(*count)],
                _ => &[],
            };
            for &note in notes {
                if note.0 > 127 {
                    return Err(Error::Format("invalid synthie MIDI pitch"));
                }
                held[i].push(note);
                native_events.push(AudioEvent::NoteOnChannel(ch, note));
                add(&mut midi, at, &[0x90 | ch.0, note.0, 110])?;
            }
            if let Action::Drums(hits) = cell.action {
                for hit in hits.iter() {
                    native_events.push(AudioEvent::Drum(hit));
                    let note = drum(hit)?;
                    add(&mut midi, at, &[0x99, note, 110])?;
                    let off = (at + Tick::from_micros(40_000)).min(tick(row + 1, song)?);
                    add(&mut midi, off, &[0x89, note, 0])?;
                }
            }
        }
        let highest = held.iter().flatten().map(|n| n.0).max();
        if highest != speaker_note {
            speaker_note = highest;
            let divisor = highest
                .map(|note| {
                    (315_000_000.0
                        / (22.0 * 12.0)
                        / (440.0 * 2f64.powf((f64::from(note) - 69.0) / 12.0)))
                    .round()
                    .clamp(1.0, 65536.0) as u32 as u16
                })
                .unwrap_or(0);
            speaker.push(TimedEvent {
                at,
                event: MusicEvent::Speaker {
                    divisor,
                    enabled: highest.is_some(),
                },
            });
        }
        let begin = (row as f64 * song.tempo.frames_per_row(sample_rate as f32)).floor() as usize;
        let end = (((row + 1) as f64 * song.tempo.frames_per_row(sample_rate as f32)).floor()
            as usize)
            .min(frames);
        interleaved.resize((end - begin) * 2, 0.0);
        processor.process_block(&native_events, &mut interleaved, 2);
        for (dst, src) in pcm[begin..end]
            .iter_mut()
            .zip(interleaved.as_chunks::<2>().0)
        {
            *dst = *src;
        }
    }
    native_events.clear();
    for (lane, notes) in song.lanes.iter().zip(held) {
        for note in notes {
            add(&mut midi, duration, &[0x80 | lane.channel.0, note.0, 0])?;
            native_events.push(AudioEvent::NoteOffChannel(lane.channel, note));
        }
    }
    interleaved.resize(tail * 2, 0.0);
    processor.process_block(&native_events, &mut interleaved, 2);
    for (dst, src) in pcm[frames..].iter_mut().zip(interleaved.as_chunks::<2>().0) {
        *dst = *src;
    }
    speaker.push(TimedEvent {
        at: duration,
        event: MusicEvent::Speaker {
            divisor: 0,
            enabled: false,
        },
    });
    midi.sort_by_key(|e| e.at);
    let midi = PreparedMusic::new(MusicTarget::Midi, midi, duration, [])?;
    let mut native = PreparedMusic::from_pcm(pcm, sample_rate)?;
    // All three arrangements share the source's crystal-tick endpoint. Native
    // frame quantization can leave a sub-frame silent gap, never tempo drift.
    native.duration = duration;
    let mut result = Soundtrack::new();
    result.insert(MusicDevice::Native, native)?;
    result.insert(
        match chip {
            OplChip::Opl2 => MusicDevice::Opl2,
            OplChip::DualOpl2 => MusicDevice::DualOpl2,
            OplChip::Opl3 => MusicDevice::Opl3,
        },
        OplMidiDriver::with_starter_bank(chip).prepare(&midi)?,
    )?;
    result.insert(
        MusicDevice::Speaker,
        PreparedMusic::new(MusicTarget::Speaker, speaker, duration, [])?,
    )?;
    Ok(result)
}
