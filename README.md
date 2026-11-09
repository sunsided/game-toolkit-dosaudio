# game-toolkit-dosaudio

[![crates.io](https://img.shields.io/crates/v/game-toolkit-dosaudio.svg)](https://crates.io/crates/game-toolkit-dosaudio)
[![docs.rs](https://docs.rs/game-toolkit-dosaudio/badge.svg)](https://docs.rs/game-toolkit-dosaudio)
[![license](https://img.shields.io/badge/license-EUPL--1.2%20OR%20MIT%20OR%20Apache--2.0-blue.svg)](#license)

**The sound of a 1990s PC, for your Rust game.** Beep the PC speaker, play
QBasic `PLAY` strings, write AdLib registers, push samples through Sound Blaster
DMA or play MIDI on FM. All of it runs on one emulated hardware clock, so the
results match real hardware timing.

Use it as a hardware back end for an emulator. Or use it as a retro sound engine
in a modern game that never touches a port.

- 🔊 **PC speaker**: tones, QBasic `PLAY` melodies and PWM sample playback
- 🎹 **AdLib / OPL2 / dual OPL2 / OPL3**: raw registers, patch helpers, MIDI driver, instrument banks
- 💾 **Sound Blaster 1, 2, Pro, Pro 2 and 16**: DSP, 8237 DMA, IRQs, mixer, multi-voice SFX
- 🎼 **MIDI**: MPU-401 UART, General MIDI through SoundFonts, MT-32 with your own ROMs
- 📻 **Tandy / PCjr PSG**: three tones plus LFSR noise
- 📼 **File formats**: IMF, DRO, VGM and Standard MIDI files, plus DRO export of what you played
- 🎚️ **Game-friendly control**: looping, fades, live track replacement and per-device soundtracks
- 🧵 **Real-time safe**: rendering never allocates or takes a lock

## Quick start

```toml
[dependencies]
game-toolkit-dosaudio = "0.1"
```

No audio device is needed. `OfflineMachine` renders straight to WAV:

```rust,no_run
use game_toolkit_dosaudio::prelude::*;
use std::time::Duration;

let mut machine = OfflineMachine::new(SoundConfig::none().speaker(true), 48_000)?;
machine.sound(440.0)?;                          // like Turbo Pascal's Sound(440)
machine.render_for(Duration::from_millis(250))?;
machine.nosound()?;
std::fs::write("beep.wav", machine.finish().to_bytes()?)?;
# Ok::<(), Box<dyn std::error::Error>>(())
```

## Tour

### QBasic `PLAY` on the PC speaker

```rust
use game_toolkit_dosaudio::{Melody, prelude::*};

let mut machine = OfflineMachine::new(SoundConfig::none().speaker(true), 48_000)?;
let tune = Melody::parse("T180 O3 L8 MN C E G > C < G E D4 P8 MS G A G E C4")?;
let _handle = machine.play_speaker(tune, PlayOptions::default())?;
let wav = machine.render_to_end()?;
# Ok::<(), game_toolkit_dosaudio::Error>(())
```

The parser covers notes, accidentals, octaves, tempo, lengths, pauses, `N`
numbers, dots and `MN`/`ML`/`MS` articulation. `speaker_pwm(&samples, rate)`
plays 8-bit PCM through the speaker, the way old games produced speech.

### Talk to the AdLib like it's 1990

```rust
use game_toolkit_dosaudio::prelude::*;

let (mut ports, mut engine) = DosAudio::split(SoundConfig::adlib())?;

// Classic AdLib detection: start OPL timer 1, wait, check the status flags.
ports.outb(0x388, 0x02); ports.outb(0x389, 0xff);
ports.outb(0x388, 0x04); ports.outb(0x389, 0x21);
ports.wait_us(80);
assert_eq!(ports.inb(0x388) & 0xe0, 0xc0);

// Or skip the register maths and use the musical helpers.
let organ = OplPatch {
    modulator: OplOperator { characteristics: 0x21, level: 0x84, attack_decay: 0xf2, sustain_release: 0x73, waveform: 1 },
    carrier:   OplOperator { characteristics: 0x01, level: 0x45, attack_decay: 0xe3, sustain_release: 0x64, waveform: 2 },
    feedback: 0x0e,
};
let mut opl = ports.opl()?;
opl.set_patch(0, &organ)?;
opl.note_on(0, Note::A4, 100)?;

let mut frames = [[0.0f32; 2]; 1024];
engine.render(&mut frames); // runs on your audio thread
# Ok::<(), game_toolkit_dosaudio::Error>(())
```

`DosAudio` is the game side: ports, timers and register latches. `AudioEngine`
is the audio side: synthesis. A lock-free queue of timestamped writes connects
them.

### Play classic music files

```rust,no_run
use game_toolkit_dosaudio::prelude::*;

let song = Dro::load(&std::fs::read("song.dro")?)?;          // or Imf::load(bytes, 560)
let mut machine = OfflineMachine::new(SoundConfig::adlib(), 48_000)?;
let _music = machine.play(song)?;
std::fs::write("song.wav", machine.render_to_end()?.to_bytes()?)?;
# Ok::<(), Box<dyn std::error::Error>>(())
```

### MIDI on FM, with real instrument banks

```rust,no_run
use game_toolkit_dosaudio::prelude::*;
use std::sync::Arc;

let midi = Smf::load(&std::fs::read("song.mid")?)?;
let bank = OplBank::from_op2(&std::fs::read("GENMIDI.op2")?)?;   // or OplBank::starter()
let driver = OplMidiDriver::new(OplChip::Opl3, Arc::new(bank));

let mut machine = OfflineMachine::new(SoundConfig::sb16(), 48_000)?;
let _music = machine.play(driver.prepare(&midi)?)?;
std::fs::write("song.wav", machine.render_to_end()?.to_bytes()?)?;
# Ok::<(), Box<dyn std::error::Error>>(())
```

The driver loads SBI, OP2 (DMX), WOPL and AdLib BNK banks. It handles layered
voices, voice stealing, pitch bend, sustain, pan and percussion on channel 10.

### Sound Blaster sound effects

```rust
use game_toolkit_dosaudio::prelude::*;

let mut machine = OfflineMachine::new(SoundConfig::sb2(), 48_000)?;
let zap: Vec<u8> = (0..4_000).map(|i| if (i / (4 + i / 400)) % 2 == 0 { 64 } else { 192 }).collect();
let sfx = machine.play_pcm(zap, 8_000)?;           // unsigned 8-bit mono
let wav = machine.render_to_end()?;
assert!(sfx.is_finished());
# Ok::<(), game_toolkit_dosaudio::Error>(())
```

`play_pcm` uses the same DSP and DMA path a DOS game would program by hand. For
overlapping effects, `SfxMixer` mixes several voices into an auto-init DMA ring.
See [`examples/dma_refill.rs`](examples/dma_refill.rs).

### Live playback in a game

With the default `toolkit` feature, `AudioEngine` plugs into
[game-toolkit-audio](https://crates.io/crates/game-toolkit-audio) as a stereo
stream:

```rust,no_run
use game_toolkit_dosaudio::prelude::*;
use std::time::Duration;

let song = Imf::load(&std::fs::read("theme.imf")?, 700)?;
let (_ports, mut engine) = DosAudio::split(SoundConfig::adlib())?;
let music = engine.play(song, PlayOptions::looped())?;
let rate = engine.sample_rate();

let mut audio = game_toolkit_audio::Audio::new().map_err(|e| e.to_string())?;
let _stream = audio.play_stereo_stream(engine, rate)?;

// Later, from the game loop:
music.fade_volume(0.0, Duration::from_secs(2))?;
# Ok::<(), Box<dyn std::error::Error>>(())
```

Music handles can pause, resume, stop, fade and report position. Call
`engine.music_controller(capacity)` before streaming to swap tracks while audio
runs.

### One soundtrack, every sound card

A `Soundtrack` holds one arrangement per device, all with the same length. The
player picks the arrangement for the hardware it has. It can also switch device
during playback, without losing its place in the song:

```rust,no_run
# #[cfg(feature = "synthie")] fn main() -> Result<(), Box<dyn std::error::Error>> {
use game_toolkit_dosaudio::prelude::*;
use std::sync::Arc;
use synthie::{params::{ChannelNo, SynthParams}, pattern::{Song, Tempo}};

let song = Song::new(Tempo::bpm(137.0, 2))
    .patch(SynthParams::default())
    .lane(ChannelNo(0), "C4@0 . E4 G4 A4 . G4 -")?
    .lane(ChannelNo(3), "k h s h k h s h")?;
let soundtrack = Arc::new(Soundtrack::synthie(&song, OplChip::Opl2, 48_000)?);

let config = SoundConfig::adlib().speaker(true).native(true);
let (_ports, mut engine) = DosAudio::split(config)?;
let music = engine.play_soundtrack(soundtrack, MusicDevice::Opl2, PlayOptions::looped())?;

// Options menu: "Music device: PC Speaker"
music.switch_device(MusicDevice::Speaker)?;
# Ok(()) }
# #[cfg(not(feature = "synthie"))] fn main() {}
```

You can also build a soundtrack by hand with `Soundtrack::insert` from prepared
single-device music: DRO, IMF, VGM, MIDI or `PLAY` strings. `fallback` sets which
device to use when an arrangement or its hardware is missing, for example from
MT-32 to OPL2. See the [backend reference](tests/fixtures/music_backends.md) for
restoration limits and synthie mappings.

## Feature flags

| Feature | Default | Adds |
| --- | :---: | --- |
| `opl` | ✓ | MIT-licensed [`oplon`](https://docs.rs/oplon) FM core |
| `sb` | ✓ | Sound Blaster family, DMA, conventional memory, SFX mixer |
| `formats` | ✓ | IMF, DRO, VGM and SMF readers |
| `toolkit` | ✓ | `game-toolkit-audio` stream integration |
| `psg` | | Tandy SN76489 tone and noise |
| `mpu401` | | MPU-401 UART with custom `MidiSynth` back ends |
| `gm` | | General MIDI from a caller-supplied SF2 via RustySynth |
| `mt32` | | CM-32L/MT-32 synthesis from caller-supplied ROMs |
| `synthie` | | Compile [synthie](https://crates.io/crates/synthie) songs into native, FM and speaker arrangements |
| `nuked` | | Nuked OPL3 core (LGPL-2.1-or-later) |

The PC speaker, timeline, capture and WAV output need no features. The crate
uses `std`. Rust 1.95 or later is required.

## Examples and demos

```sh
task demo              # AdLib phrase, live
task demo:speaker      # PLAY melody and a PWM chirp
task demo:models       # the same sound on all five Sound Blaster models
task demo:dma          # four overlapping effects over memory DMA
task demo:music        # live track replacement and fades
task demo:opl-midi CHIP=opl3
task demo:gm SF2=/path/to/font.sf2
task demo:mt32 ROMS=/path/to/rom-directory
task demo:synthie      # native, OPL and speaker, played in turn
task demo:switch       # switch devices while the song continues
```

Bring your own files:

```sh
task demo:file FILE=song.dro
task demo:file FILE=song.imf ARG=700            # IMF needs the game's tick rate
task demo:file FILE=song.mid ARG=font.sf2       # MIDI through General MIDI
task demo:opl-midi CHIP=opl3 MIDI=song.mid BANK=GENMIDI.op2   # .op2, .wopl, .bnk or .sbi
task demo:mt32 ROMS=/path/to/rom-directory
```

The `render:*` variants, such as `render:file` and `render:opl-midi`, write the
WAV to `target/` without playing it. All demo music is original. WAV tasks play their output with `ffplay`, `afplay`,
`pw-play`, `paplay` or `aplay`, whichever it finds first. No ROMs, SoundFonts or
third-party music are included.

## How it works

- **One clock.** Time is counted in ticks of the PC crystal (exactly 315/22 MHz).
  Sample positions use exact rational arithmetic, so audio does not drift.
- **Two halves.** `DosAudio::split` returns a game side and an audio side. Port
  reads such as timer status come back from the game side immediately. Writes
  travel to the audio side through a bounded SPSC queue.
- **Fixed latency.** Live writes are delayed by a fixed 10 ms by default, which
  absorbs scheduling jitter. Offline rendering uses no delay.
- **Plugs into a bus.** `DosAudio` implements
  [`game_toolkit_dosbus::IoDevice`](https://crates.io/crates/game-toolkit-dosbus).
  An emulator can mount it next to its other devices. With the speaker
  installed, `DosAudio` owns the 8253 PIT on ports 40h–43h.
- **Bring your own core.** Implement `OplCore` and select it with
  `OplBackend::Custom` to replace the FM synthesizer.

## Hardware notes

Each section below gives the exact coverage and limits for one device.

<details>
<summary><b>Timing, queue and capture</b></summary>

Output intervals split at commands and at the OPL's 288-tick native boundaries.
The speaker integrates pulse area within each output sample, then applies a
coupling/cone filter. FM output uses held native frames with area resampling;
this renderer does not promise a band-limited resampler or bit-identical output
between chip cores.

`try_outb_at` reports a full queue without changing time or hardware state; the
infallible helpers panic on overflow. Late writes apply immediately and
increment `engine.stats().late_commands`. The host must pace its machine clock
against audio output; latency absorbs jitter but does not synchronize two
independent clocks.

The timer is the dosbus `Pit`: modes 0, 2 and 3 on all three channels, low/high
and paired counter writes, count latching, and IRQ 0 timing. Channel 2 drives
the speaker through the 61h gate/data bits and output readback; channel 1 drives
the DRAM-refresh bit. BCD, modes 1, 4 and 5 (run as mode 3) and the 8254
read-back command are not implemented. OPL timers are polling/status sources;
they do not drive an ISA IRQ.

`ports.start_capture()` and `ports.finish_capture()` record game-thread port
writes; `capture.to_dro()` exports FM accesses as v1 DRO with millisecond
timing. To include writes made by the audio-thread sequencer, call
`engine.start_capture(capacity)` before streaming, poll `try_finish()` on the
control thread, then `stop()` and keep polling until the renderer acknowledges
the next frame boundary. Overflow marks the capture incomplete. DRO carries OPL
writes only.

</details>

<details>
<summary><b>Music playback and runtime control</b></summary>

Sequencing and looping run inside the renderer, independent of game frame rate.
Music and manual port I/O share the same FM hardware; music volume/pause
affects that hardware's whole FM output, and stopping music releases its notes.
Dropping the last handle clone stops playback.

`controller.play(prepared, options)` replaces the active track; `play_at` and
`send_event_at` take absolute output timestamps. Requests keep FIFO order,
including equal timestamps. Full queues return `QueueFull`. Call
`controller.reclaim()` periodically to free retired players on the control
thread. `fade_volume` ramps across output frames, freezes while paused and
resumes from its current gain when interrupted.

`PreparedMusic` owns validated OPL, MIDI/SysEx, PSG, speaker or native PCM
events. `with_loop_start(at)` plays an intro once, then repeats `[at, duration)`.

Soundtrack arrangements must share duration and loop point. Switching restores
prepared register/controller/key checkpoints at an output-frame boundary;
sounding notes retrigger their envelopes. Limits: 4,096 checkpoints per
arrangement, 1,024 restore messages per checkpoint, one million overall; MIDI
restoration keeps up to 128 active notes and 16 SysEx messages (65,536 bytes)
since reset. MIDI state messages replay in source order; overlapping same-key
notes are rejected during preparation. A machine has one MIDI backend, so
GM and MT-32 arrangements require separate machines. Switching can select that
backend alongside installed FM, PSG, speaker and native output.

</details>

<details>
<summary><b>Sound Blaster</b></summary>

| Model | DSP | FM | DMA playback |
| --- | --- | --- | --- |
| SB1 | 1.05 | OPL2 | Unsigned 8-bit mono single-cycle |
| SB2 | 2.01 | OPL2 | Unsigned 8-bit mono, single/auto-init |
| Pro | 3.00 | Dual OPL2 | 8-bit mono/stereo, high-speed |
| Pro2 | 3.02 | OPL3 | 8-bit mono/stereo, high-speed |
| SB16 | 4.05 | OPL3 | 8/16-bit, signed/unsigned, mono/stereo |

Constructors are `sb1()`, `sb2()`, `sb_pro()`, `sb_pro2()` and `sb16()`;
`SoundConfig::from_blaster("A220 I5 D1 H5 T6")` parses a `BLASTER` string.
`sb2()` defaults to 220h, IRQ 5, DMA 1; use
`SoundConfig::none().sound_blaster(SoundBlasterConfig { base: 0x240, irq: 7, dma: 3 })`
for other resources. The 2x8h/2x9h FM ports share latches and timers with
388h/389h.

**Buffered PCM.** `play_pcm` accepts 1–65,536 bytes at 4,000–23,000 Hz;
`sound.sample_rate()` reports the quantized hardware rate. An `Arc<[u8]>`
shares the samples. One buffer plays at a time; a new buffer replaces the
previous transfer, and a raw write to the reset or command port hands the DSP
back to manual I/O. Stop takes effect at the current machine time on the next
advance and cannot retract queued samples. For live playback, call
`ports.try_advance_to(machine_time)` as the game advances; it returns
`QueueFull` without side effects if the advance does not fit.

**DSP.** Reset (hold 2x6h high ≥ 3 µs, read AAh from 2xAh), version and
complement queries, direct DAC, speaker enable/status and time-constant 8-bit
DMA. 14h single-cycle, 1Ch auto-init with 48h block size, D0h/D4h halt and
continue, DAh exit auto-init, D1h/D3h DAC gate, F2h software IRQ. Pro mixer 0Eh
enables alternating-byte stereo. SB16 41h sets the rate; Bxh/Cxh count words or
bytes with separate 2xEh/2xFh acknowledgement. CT1345/CT1745 master, PCM and FM
gains affect output. ADPCM, recording and DSP MIDI are unsupported.

**DMA.** Either service requests yourself (`next_dma_tick`, `try_dma_write_at`,
`IoDevice::dma_request`) or call `ports.attach_dma(Arc<ConventionalMemory>)` for
an emulated 8237 with 1 MiB of memory, programmed through normal ports. Then
`poll_dma_completion()` reports block/IRQ events for ring refills. Polling never
acknowledges an IRQ; reading 2xEh does.

```rust
use game_toolkit_dosaudio::prelude::*;

let (mut ports, mut engine) = DosAudio::split(SoundConfig::sb2())?;
// Enable output, select 8 kHz, and start a two-byte single-cycle block.
for byte in [0xd1, 0x40, 131, 0x14, 1, 0] { ports.outb(0x22c, byte); }
for sample in [32, 224] {
    let at = ports.next_dma_tick().unwrap();
    ports.try_dma_write_at(at, DmaChannel::new(1).unwrap(), sample)?;
}
assert!(ports.irq_pending(ports.now()));
ports.inb(0x22e); // Acknowledge the DSP interrupt.
engine.render(&mut [[0.0; 2]; 512]);
# Ok::<(), game_toolkit_dosaudio::Error>(())
```

SB2 uses an ideal DAC. Other models apply documented rate limits and
approximate analog response; these are not component-level simulations.

</details>

<details>
<summary><b>OPL MIDI driver and banks</b></summary>

`OplMidiDriver` drives OPL2's nine voices or dual OPL2/OPL3's eighteen, with
layered instruments and deterministic stealing of the oldest note. It handles
bank/program changes, volume, expression, sustain, pitch bend (RPN 0), pan and
all-notes/all-sound/reset-controller messages. Gain rounds to OPL's 0.75 dB
steps. Aftertouch, modulation and vendor SysEx are ignored; GM System On resets
the driver. `driver.message(message, emit)` is an allocation-free register
writer; `driver.write_message(&mut ports, message)` sends a live message through
the port queue (use at least 128 queue slots).

| Loader | Coverage and limits |
| --- | --- |
| `OplBank::from_sbi` | One SBI instrument as bank 0/program 0 |
| `OplBank::from_op2` | 175 DMX records; fixed pitch, transposition, layered voices and detune; delayed vibrato rejected |
| `OplBank::from_wopl` | v1–3, bank-select metadata, fixed pitch and pseudo-four-operator layers; true four-operator/rhythm timbres rejected |
| `OplBank::from_bnk` | AdLib v1.0 melodic timbres, mapped to programs by name-table order; HMI/BNK2 and rhythm timbres rejected |

`OplBank::starter()` has six original melodic and five percussion patches, not
a full GM set. Missing programs fall back to bank 0, then program 0 (drum 36).
No historical bank is included; provenance is in
`tests/fixtures/opl_banks.md`.

</details>

<details>
<summary><b>PSG, MPU-401, General MIDI and MT-32</b></summary>

**PSG** (`psg`): `SoundConfig::none().psg(true)` installs the Tandy/PCjr port
C0h; `ports.psg()` has tone, attenuation and noise helpers. Clock is the PC
crystal ÷ 4, a 15-bit LFSR with taps 0/4 and 2 dB steps. Sega/Game Gear variants
are unsupported.

**MPU-401** (`mpu401`): UART at 330h/331h. 3Fh enters UART mode, FFh resets.
Running status, interleaved real-time bytes and SysEx up to 65,536 bytes are
supported; intelligent mode is not. `MidiBackend::Custom` accepts any
allocation-free `MidiSynth`.

**General MIDI** (`gm`): load your SF2 with `GmSynth::load_soundfont(&mut reader)`
and pass it to `SoundConfig::gm(font)`. Rendering uses RustySynth.

**MT-32** (`mt32`): `Mt32Roms::from_rom_dir(path)` validates your CM-32L control
and PCM ROMs; `SoundConfig::mt32(roms)` installs them. Supported images are
CM-32L/LAPC-I control v1.00 or v1.02 plus the full 1 MiB CM-32L PCM ROM.
Original MT-32, CM-32LN and split ROM images are rejected. The private core is
adapted from moont 1.0.0 (LGPL-2.1-or-later). Native queues are preallocated;
`engine.midi_dropped_messages()` reports overflow. No ROMs are distributed.
Firmware hashes, queue limits and test coverage are in the
[backend reference](tests/fixtures/music_backends.md).

</details>

<details>
<summary><b>File formats</b></summary>

- **IMF**: `Imf::load(bytes, tick_hz)`. The game picks the tick rate, as the file
  does not store it. `load_type0`/`load_type1` resolve ambiguous headers.
- **DRO**: v1 and uncompressed interleaved v2, OPL2/dual OPL2/OPL3.
- **SMF**: type 0/1, PPQN and SMPTE divisions, tempo changes with exact phase,
  split SysEx. Type 2 and RIFF are errors.
- **VGM**: uncompressed OPL/PSG logs (YM3812, YM3526, YMF262, Tandy PSG) with
  intro/loop offsets. VGZ, other chips and data blocks are errors.

All parsing happens outside rendering.

</details>

## License

Dosaudio's own code is **EUPL-1.2 OR MIT OR Apache-2.0**, at your option. See
`LICENSE-EUPL`, `LICENSE-MIT` and `LICENSE-APACHE`.

Dependencies have their own terms: `game-toolkit-dosbus` and the optional toolkit
host are EUPL-1.2, `oplon` is MIT, `rtrb` is MIT OR Apache-2.0, and optional
`nuked-opl3` is LGPL-2.1-or-later. The optional synthie dependency is EUPL-1.2.
The included private CM-32L core is LGPL-2.1-or-later and retains its own
[notices and license texts](src/midi/mt32/cm32l/UPSTREAM.md). The package SPDX
expression includes that source license, even when `mt32` is disabled. The triple
license applies to this crate's own code; it does not relicense third-party code. See
[THIRD-PARTY.md](THIRD-PARTY.md).
