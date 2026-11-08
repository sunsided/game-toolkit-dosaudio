use std::{io::Read, sync::Arc};

use rustysynth::{SoundFont, Synthesizer, SynthesizerSettings};

use crate::{Error, Frame, MidiSynth, Result};

const BLOCK_SIZE: usize = 8;

/// SoundFont synthesis using a caller-supplied SF2 and RustySynth.
///
/// Load fonts and construct the adapter outside the audio callback. Rendering,
/// channel messages and supported resets retain all allocations. The adapter
/// owns a shared reference to its font until it is dropped outside rendering.
///
/// RustySynth caches eight native frames: a channel event received within a
/// cached block takes effect at the next block, up to seven native frames later.
/// MPU UART timestamps still describe when complete messages reach this adapter;
/// they do not remove the synth's internal event quantum. Native rates are
/// clamped to 16000 through 192000 Hz; lower engine rates use device resampling.
///
/// Channel handling follows RustySynth, including program, controllers, pitch
/// bend and sustain. Aftertouch is ignored by that backend. Universal GM1/GM2
/// System On messages reset synthesis state; this does not implement additional
/// GM2 synthesis features. Other SysEx, including vendor-specific messages, is
/// ignored. A one-byte FFh System Reset also resets the synthesizer.
pub struct GmSynth {
    synth: Synthesizer,
    rate: u32,
    left: [f32; BLOCK_SIZE],
    right: [f32; BLOCK_SIZE],
}

impl GmSynth {
    /// Parse a user-supplied SoundFont on the calling thread. Only `Read` is
    /// required; a byte slice, file or other sequential reader can supply it.
    pub fn load_soundfont(reader: &mut impl Read) -> Result<Arc<SoundFont>> {
        SoundFont::new(reader)
            .map(Arc::new)
            .map_err(|_| Error::Format("invalid or unsupported SoundFont"))
    }

    /// Allocate a synthesizer sharing the supplied font. Zero is an invalid
    /// requested rate; other rates clamp to the backend's native range.
    pub fn with_soundfont(font: Arc<SoundFont>, sample_rate: u32) -> Result<Self> {
        if sample_rate == 0 {
            return Err(Error::Config("GM sample rate must not be zero"));
        }
        let rate = sample_rate.clamp(16_000, 192_000);
        let mut settings = SynthesizerSettings::new(rate as i32);
        settings.block_size = BLOCK_SIZE;
        let synth = Synthesizer::new(&font, &settings)
            .map_err(|_| Error::Config("invalid GM synthesizer settings"))?;
        Ok(Self {
            synth,
            rate,
            left: [0.0; BLOCK_SIZE],
            right: [0.0; BLOCK_SIZE],
        })
    }

    fn is_gm_reset(bytes: &[u8]) -> bool {
        matches!(bytes, [0xf0, 0x7e, device, 0x09, 0x01 | 0x03, 0xf7] if *device < 0x80)
    }
}

impl MidiSynth for GmSynth {
    fn sample_rate(&self) -> u32 {
        self.rate
    }

    fn message(&mut self, bytes: &[u8]) {
        if bytes == [0xff] || Self::is_gm_reset(bytes) {
            // reset() clears voice activity and existing effect buffers in place.
            self.synth.reset();
            return;
        }
        let Some(&status @ 0x80..=0xef) = bytes.first() else {
            return;
        };
        let length = if matches!(status & 0xf0, 0xc0 | 0xd0) {
            2
        } else {
            3
        };
        if bytes.len() != length || bytes[1..].iter().any(|&value| value >= 0x80) {
            return;
        }
        self.synth.process_midi_message(
            i32::from(status & 15),
            i32::from(status & 0xf0),
            i32::from(bytes[1]),
            i32::from(*bytes.get(2).unwrap_or(&0)),
        );
    }

    fn render(&mut self, out: &mut [Frame]) {
        for chunk in out.chunks_mut(BLOCK_SIZE) {
            let count = chunk.len();
            self.synth
                .render(&mut self.left[..count], &mut self.right[..count]);
            for (index, frame) in chunk.iter_mut().enumerate() {
                *frame = [self.left[index], self.right[index]];
            }
        }
    }
}
