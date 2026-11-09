# Music backend coverage and provenance

## MT-32 arrangements on CM-32L

The optional `mt32` feature uses a private adaptation of moont 1.0.0. Its
LGPL-2.1-or-later source, copyright notices, license texts, original archive hash
and adaptation notes are recorded in
[`UPSTREAM.md`](../../src/midi/mt32/cm32l/UPSTREAM.md). No proprietary firmware,
ROM-derived instrument tables or samples are included.

`Mt32Roms::from_bytes` checks the original lengths and SHA-1 identities before
parsing. `from_rom_dir` reads `CM32L_CONTROL.ROM` (or `cm32l_ctrl_1_02.rom`/`cm32l_ctrl_1_00.rom`) and `CM32L_PCM.ROM` (or `cm32l_pcm.rom`)
from the supplied directory. Supported full images:

| Image | Bytes | SHA-1 |
| --- | ---: | --- |
| CM-32L/LAPC-I control v1.00 | 65,536 | `73683d585cd6948cc19547942ca0e14a0319456d` |
| CM-32L/LAPC-I control v1.02 | 65,536 | `a439fbb390da38cada95a7cbb1d6ca199cd66ef8` |
| CM-32L PCM | 1,048,576 | `289cc298ad532b702461bfc738009d9ebe8025ea` |

These identities and revision-dependent table addresses come from Munt's
[ROMInfo.cpp](https://github.com/munt/munt/blob/master/mt32emu/src/ROMInfo.cpp)
and [ControlROMMaps in Synth.cpp](https://github.com/munt/munt/blob/master/mt32emu/src/Synth.cpp).
After identity validation, only v1.00's default/max tables are copied to the
v1.02 addresses expected by the core. Original MT-32, CM-32LN, split and
unrecognized images are rejected. SHA-1 identifies firmware; it is not used as
an authentication mechanism. This backend targets CM-32L compatibility and does
not promise original MT-32 revision quirks or bit-exact hardware output.

The synthesizer runs at 32 kHz. Dosaudio resamples its output through the existing
MIDI voice. UART delivery already accounts for serial timing; completed messages
enter the native core without a second wire delay. The native queue preserves
insertion order and applies one message per native frame, with the core's partial
abort delay. Its limits are 4,096 messages and sixteen retained SysEx payloads,
up to 65,536 bytes each. Overflow increments `Mt32Synth::dropped_messages` /
`AudioEngine::midi_dropped_messages`; it never grows the queues. Construction
preallocates every reverb network and SysEx slot. Reset reuses memory and clears
reverb; immediate FF reset cancels pending messages, while an ordered Roland
SysEx reset preserves following messages.

No real firmware was available for this implementation's verification. Unit tests
use original, structurally valid synthetic control tables and zero PCM outside
the public firmware hash gate. They exercise audible LA synthesis, first-use
allocation/deallocation checks, all four reverb modes, reset ordering, native
clock wrapping, queue overflow/reuse and callback-chunk independence. Public
integration tests cover missing/invalid ROMs. To run the explicit asset test:

```sh
DOSAUDIO_MT32_ROM_DIR=/path/to/roms cargo test --no-default-features \
  --features mt32 --test mt32 -- --ignored
```

`task demo:mt32 ROMS=/path/to/roms` renders and plays an original phrase. The
`mt32` example also accepts an optional SMF file after the output path.

## Soundtrack switching

`Soundtrack::insert` accepts one single-device `PreparedMusic` arrangement per
`MusicDevice`. Durations and loop-start ticks must match exactly; mismatches
fail transactionally. Mixed OPL+PSG VGM logs remain playable as ordinary music,
but cannot be a single-device soundtrack arrangement. `fallback(from, to)` sets
an explicit acyclic route for missing arrangements or unavailable hardware.
Selection distinguishes GM, MT-32 and custom MIDI backends. One machine installs
one MIDI backend; GM-to-MT-32 switching requires separate machines. A soundtrack
can switch that backend against available OPL, PSG, speaker and native output.

`MusicHandle::switch_device` requests the next output-frame boundary; the latest
pending request wins. `device()` reports the last applied selection. Switching
preserves elapsed position, loop position, pause and the current fade. It releases
the old arrangement, mutes its device family, and restores a prepared checkpoint
for the new one. Independent speaker handles and PCM SFX retain their existing
controls. Restored notes retrigger envelopes; oscillator phase, previous reverb
history and sample-perfect crossfades are not preserved.

Preparation bounds each arrangement to 4,096 event-time checkpoints, 1,024
restore messages per checkpoint and one million restore messages overall. MIDI
restoration supports 128 logical active notes and sixteen SysEx messages totaling
65,536 bytes per recognized reset epoch. It replays state messages in source
order, including program changes, controllers, RPN/NRPN selections and SysEx;
completed note events are omitted. GM and Roland resets clear the respective
backend's history. Overlapping same-channel/same-key notes (including notes held
by sustain) are rejected during soundtrack insertion. These restrictions do not
apply to ordinary prepared MIDI playback. Excessive history returns `TooLarge`
on the control thread. Arbitrary custom synths must implement MIDI System Reset
and the messages they receive consistently for restoration to work.

Checkpoints and all arrangements stay owned by the retained soundtrack during
playback. Rendering switches references without allocating, locking or freeing
resources. The runtime controller's existing `reclaim` path handles retired
players on the control thread. Tests cover held-note programs, ordering,
fallbacks, checkpoint limits, switching OPL3 compatibility mode, callback chunks,
pause/fade/loop and final-handle/resource ownership.

## synthie 0.8.0

The optional `synthie` feature pins synthie 0.8.0 (EUPL-1.2) without its audio
host. `Soundtrack::synthie(song, chip, sample_rate)` prepares three arrangements
on the control thread:

| Arrangement | Mapping |
| --- | --- |
| Native | Real synthie processor, source patches/effects, pre-rendered stereo PCM |
| OPL | Existing OPL MIDI driver; patch indices cycle through six original starter timbres |
| Speaker | Highest logically held melodic pitch; drums omitted |

Supported song cells include notes, holds, releases, chords of up to four notes,
patch changes and drums. Supported limits are four channels (0–3), 64 lanes,
65,536 rows, one native frame minimum per row, 8–192 kHz and sixteen million
native frames including the 250 ms release tail. Checkpoint limits may reject a
song earlier. Invalid channels/pitches/patch references, arpeggio cells and
enabled patch arpeggiators fail preparation. OPL instruments approximate the
source timbres. One finite pass is prepared regardless of `Song::looped`;
`PlayOptions` controls playback looping. Explicit stop/drop silences native
output, while natural completion renders the prepared tail.

Native output requires `SoundConfig::native(true)`. Native frames are indexed
using exact rational output position, with held-sample resampling when rates
differ. At the source rate, samples retain their original values. No synthie
processor or patch allocation runs in the audio callback.

Tests compare audible output and equal durations across all three arrangements,
exact native samples, callback-chunk independence, switch/pause/stop/drop lifetime
behavior and invalid inputs. `task demo:synthie` renders and plays all three;
`task demo:switch` plays an uninterrupted native → OPL → speaker sequence. The
four original demo WAVs were rendered and measured for nonzero signal; playback
command expansion was checked without opening an audio device.
