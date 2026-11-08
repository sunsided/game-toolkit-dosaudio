# game-toolkit-dosaudio

DOS-era PC sound devices for ordinary Rust games: PC speaker, Sound Blaster, AdLib,
Tandy PSG and MPU-401 MIDI, sharing the clock and I/O interfaces of
[game-toolkit-dosbus](https://crates.io/crates/game-toolkit-dosbus).

It covers SB1/2/Pro/Pro2/16 with conventional-memory DMA, a fixed-voice SFX
mixer, runtime music replacement/fades, QBasic `PLAY` and PWM, PSG, MIDI
UART/General MIDI, SMF/VGM readers and sequenced OPL capture. MT-32, instrument
bank readers/OPL MIDI driving, arrangement selection/switching and synthie
integration are not implemented yet.

## Render without an audio device

```rust
use game_toolkit_dosaudio::prelude::*;
use std::time::Duration;

let mut machine = OfflineMachine::new(SoundConfig::none().speaker(true), 48_000)?;
machine.sound(440.0)?;
machine.render_for(Duration::from_millis(100))?;
machine.nosound()?;
machine.render_for(Duration::from_millis(20))?;
std::fs::write("beep.wav", machine.finish().to_bytes()?)?;
# Ok::<(), Box<dyn std::error::Error>>(())
```

For FM music, load `Imf::load(bytes, tick_hz)` or `Dro::load(bytes)`, keep the
handle returned by `machine.play(sequence)` alive, then call
`machine.render_to_end()?.to_bytes()?`. IMF's tick rate belongs to the game;
the file does not store it. Explicit `load_type0` and `load_type1` loaders resolve
IMF's ambiguous header detection. DRO supports legacy v1 and uncompressed,
interleaved v2, with OPL2/dual OPL2/OPL3 hardware selection.

## One clock, two halves

```rust
use game_toolkit_dosaudio::prelude::*;

let (mut ports, mut engine) = DosAudio::split(
    SoundConfig::adlib().speaker(true).latency(Tick::ZERO)
)?;
ports.outb(0x388, 2);      // OPL timer 1 count register
ports.outb(0x389, 0xff);
ports.outb(0x388, 4);      // Start timer 1; mask timer 2
ports.outb(0x389, 0x21);
ports.wait_us(80);
assert_eq!(ports.inb(0x388) & 0xe0, 0xc0);

let mut frames = [[0.0; 2]; 512];
engine.render(&mut frames);
# Ok::<(), game_toolkit_dosaudio::Error>(())
```

The game thread owns timers, register latches and immediate port reads. The audio
thread owns synthesis. A bounded SPSC queue carries timestamped changes; rendering
allocates no memory and acquires no locks. `try_outb_at` reports a full queue
without changing time or hardware state; the infallible helpers panic on overflow.

Ticks count the PC crystal at exactly 315/22 MHz. Sample positions use integer
rational arithmetic. Output intervals split at commands and the OPL's 288-tick
native boundaries. The speaker integrates pulse area within each output sample,
then applies a coupling/cone filter. FM output uses held native frames with area
resampling; this initial renderer does not promise a band-limited resampler or
bit-identical output between different chip cores.

Live port writes have a configurable fixed latency (10 ms by default); offline
rendering uses zero latency. The host must pace its machine clock against audio
output. Late writes apply immediately and increment `engine.stats().late_commands`.
The latency absorbs jitter; it does not synchronize two independently running clocks.

`DosAudio` implements `game_toolkit_dosbus::IoDevice` and can be moved into a
host's bus directly or through `ports.bus_device()`. The host charges I/O costs,
advances attached devices and arbitrates the shared PIT control port 43h.
This crate does not depend on dosvideo; dosvideo integration is a separate change.

## Playback and controls

Prepare music with `engine.play(sequence, PlayOptions::looped())` before moving
the engine into an audio stream. The cloneable handle supports pause, resume,
volume, stop and position queries; dropping the last clone stops playback.
Sequencing and looping run inside the renderer, independent of game frame rate.
Music and manual port I/O use the same FM hardware; music volume/pause affects
that hardware's complete FM output, and stopping music releases its FM notes.
For runtime loading, create `engine.music_controller(capacity)` before moving
it into the stream. `controller.play(prepared, options)` replaces the active
track; `play_at` and `send_event_at` use absolute output timestamps, without
adding port latency. Requests preserve FIFO order, including equal timestamps;
an immediate request queued behind a future request waits for it. Full queues
return `QueueFull`. Call `controller.reclaim()` periodically to release retired
players on the control thread; a full retirement queue delays replacement.
`handle.fade_volume(target, duration)` ramps across output frames, freezes while
paused and resumes from its current gain when interrupted.

`PreparedMusic` owns validated OPL, MIDI/SysEx, PSG or speaker events. Set
`with_loop_start(at)` to play an introduction once and repeat `[at, duration)`;
whole-track looping still starts at zero. Shared event/payload allocations remain
owned until reclaimed or the engine is destroyed outside its callback.

Third-party FM cores implement `OplCore` and are selected with
`OplBackend::Custom(Arc::new(|| Box::new(MyCore::new())))`. The factory runs at
machine construction, once per physical chip. Timers, status, register latches,
native timing and output mixing continue to use this crate's device layer.

With `toolkit`, `AudioEngine` implements `game_toolkit_audio::StereoStream`:
pass it and `engine.sample_rate()` to `Audio::play_stereo_stream`. Keep the music
handle alive outside the stream. The host's stream handle controls overall gain
and playback; web gesture/deferred behavior belongs to the toolkit host.

`ports.start_capture()` and `ports.finish_capture()` record successful game-thread
port writes. `capture.to_dro()` exports FM accesses as v1 DRO, rounding absolute
relative timestamps to milliseconds. It includes the capture's final delay and
retains starting address latches. Start capture before programming instruments
for self-contained replay. Music emitted directly by the audio-thread sequencer
is not included in port captures.

To capture synthesized OPL writes too, call `engine.start_capture(capacity)`
before streaming and retain the returned `MusicCapture` on the control thread.
Its bounded queue records actual application times after port latency and late
write adjustment, including sequence/live events, loops, replacements and note
releases. Poll `try_finish()` while recording to drain it; call `stop()` and keep polling after
the renderer acknowledges the next frame boundary. Polling early returns no
capture yet. Overflow is explicit and marks the capture incomplete. Start before
instrument setup for self-contained replay. DRO carries OPL writes only; PCM,
MIDI, PSG and speaker audio are excluded.

The speaker currently models binary PIT periodic modes 2 and 3, low/high and
paired counter writes, count latching, gate/data bits, output readback, and the
DRAM-refresh bit. BCD, other PIT modes and the 8254 read-back command are not
implemented. OPL timers are polling/status sources; they do not drive an ISA IRQ.

## Sound Blaster

For a sound effect, pass an unsigned 8-bit mono buffer to `play_pcm`:

```rust
use game_toolkit_dosaudio::prelude::*;

let mut machine = OfflineMachine::new(SoundConfig::sb2(), 48_000)?;
let sound = machine.play_pcm(vec![32, 224, 32, 224], 8_000)?;
let wav = machine.render_to_end()?;
assert!(sound.is_finished());
# Ok::<(), game_toolkit_dosaudio::Error>(())
```

`DosAudio::play_pcm` uses the same DSP/DMA path as manual port I/O. Advancing
the machine clock supplies buffer bytes at exact DSP timestamps, raises the
normal block-completion IRQ and mutes the DAC after a full interval for the last
sample. Buffer playback accepts 1 through 65,536 bytes and a requested rate of
4,000 through 23,000 Hz; `sound.sample_rate()` reports the quantized hardware rate.
Passing an `Arc<[u8]>` shares the samples; slices and vectors are also accepted.

Keep the cloneable `SfxHandle` alive while playing. It reports `samples_played`
and `is_finished`, and supports `stop`; dropping the last clone requests a stop.
These controls follow machine time, ahead of output by the fixed command latency.
Stop takes effect at the current machine timestamp on the next successful time
advance or port access. It cannot retract already queued samples. Buffer data
is released on the control thread; rendering never owns or frees it.

The card plays one buffer at a time. A new buffer replaces the previous DSP
transfer; a raw write to its reset or command port releases buffer ownership
to manual I/O. FM and speaker I/O can continue alongside PCM. While a buffer owns
DMA, external DRQ is hidden and manual DMA byte delivery is rejected.

For live playback, call `ports.try_advance_to(machine_time)` as the game advances.
It returns `QueueFull` without changing time, DSP, playback position or IRQ if
the whole advance cannot fit. Render queued audio or advance in smaller steps
before retrying; infallible time/bus helpers panic on overflow. The queue must
hold all events within one output sample. `OfflineMachine` interleaves DMA and
rendering automatically, and `render_to_end` waits for both finite music and PCM.
Starting buffers allocates; the renderer remains allocation-free. Buffer looping,
pause and per-buffer volume are future work.

`SoundConfig::sb2()` installs a DSP reporting version 2.01 and an OPL2 at 220h,
IRQ 5, DMA 1. To select other resources, use
`SoundConfig::none().sound_blaster(SoundBlasterConfig { base: 0x240, irq: 7, dma: 3 })`.
The card's 2x8h/2x9h FM ports share address latches and timers with 388h/389h;
DRO captures preserve accesses through either address pair.

The DSP supports reset (hold 2x6h high for at least 3 microseconds, then lower it
and read AAh from 2xAh), version and complemented-byte queries, direct DAC output,
speaker enable/status, and time-constant-driven 8-bit mono DMA. Single-cycle
command 14h takes a little-endian byte count minus one. Auto-init command 1Ch
uses the block size set by 48h. D0h/D4h halt and continue DMA; DAh exits auto-init
after the current block. D1h/D3h gate the DAC without stopping DMA. F2h asserts
a software IRQ immediately. Unsupported DSP commands are ignored.

For host-serviced DMA, `IoDevice::dma_request(at)` reports a memory-to-device
request on the configured channel. Retain concrete `DosAudio` access to supply
bytes or words; the shared bus trait defines request observation.

Alternatively, `ports.attach_dma(Arc<ConventionalMemory>)` installs an emulated
8237 controller and 1 MiB of shared conventional memory. Program its address,
count, page, mode and mask registers through normal ports. Time advances then
supply due samples automatically, with 64 KiB byte-page wrapping or 128 KiB word
windows. `poll_dma_completion()` returns bounded block/IRQ events on the control
thread, where the host acknowledges the DSP IRQ and refills a consumed ring
half. Polling does not acknowledge an IRQ. Queue backpressure and memory errors
leave controller/sample/time state unchanged. `examples/dma_refill.rs` demonstrates
40 exact half-buffer completions and four overlapping voices from `SfxMixer`.
Mixer source ownership and replacement stay on the caller's thread.

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

The first byte request occurs one sample period after starting DMA. Service
successive requests at `next_dma_tick()` to retain the exact fractional clock
phase. Requests remain asserted until serviced; a late host stalls DMA and
resumes the cadence from delivery time. A completed DSP block asserts IRQ
independently of rendering. Reading 2xEh clears it; observing IRQ never does.
Only commit the host DMA address/count after `try_dma_write_at` succeeds, since
a full audio queue rejects the byte without changing DSP state or machine time.

Output holds the last DAC value between bytes and integrates changes within an
output sample. SB2 uses an ideal-DAC response. Other models apply documented
rate limits and approximate analog response; these are not component-level
circuit simulations. ADPCM, recording and DSP MIDI are unsupported. Time
advances supply data when a PCM buffer or attached memory controller owns DMA;
otherwise the host must service port-level requests.
`OfflineMachine` exposes `next_dma_tick`, `dma_write`
and `irq_request` for the same protocol without an audio device. See
[`examples/sb_pcm.rs`](examples/sb_pcm.rs) for a complete original-tone WAV demo.

Model constructors are `sb1()`, `sb2()`, `sb_pro()`, `sb_pro2()` and `sb16()`;
`SoundConfig::from_blaster("A220 I5 D1 H5 T6")` parses ISA resources and type.
Existing `SoundBlasterConfig` literals remain valid; model/high-DMA selection
uses the separate builders. DSP versions and FM hardware are:

| Model | DSP | FM | DMA playback |
| --- | --- | --- | --- |
| SB1 | 1.05 | OPL2 | Unsigned 8-bit mono single-cycle |
| SB2 | 2.01 | OPL2 | Unsigned 8-bit mono, single/auto-init |
| Pro | 3.00 | Dual OPL2 | 8-bit mono/stereo, high-speed |
| Pro2 | 3.02 | OPL3 | 8-bit mono/stereo, high-speed |
| SB16 | 4.05 | OPL3 | 8/16-bit, signed/unsigned, mono/stereo |

Pro mixer 0Eh enables alternating-byte stereo; the time-constant rate is the
aggregate byte rate. SB16 41h sets per-channel rate; Bxh/Cxh modes count scalar
words/bytes and use distinct byte/word IRQ acknowledgement at 2xEh/2xFh.
D5h/D6h and D9h control word DMA; D0h/D4h and DAh control byte DMA. CT1345/CT1745
master, PCM and FM gains affect output; analog input/recording controls have
readback without recording synthesis. Unknown mixer registers read FFh.
Pro's common OPL address pair broadcasts to both logical chips; dedicated pairs
retain independent latches. SB16 resource register changes select DMA/IRQ lines.
Mixer gain steps and filter curves are approximations documented in source.

## Speaker, PSG and MIDI

`Melody::parse("T180 O3 L8 C E G > C")` compiles QBasic `PLAY`. Pass it to
`engine.play_speaker(melody, options)` for independent speaker controls alongside
FM/MIDI. `SpeakerHandle` supports pause, resume, volume and stop; its last drop
stops playback. Runtime speaker changes use `engine.speaker_controller(capacity)`.
The grammar supports A–G, accidentals, O0–6, octave shifts, T32–255, L/P1–64,
N0–84, dots and MN/ML/MS articulation. MF/MB both prepare asynchronous playback;
variables and substring macros are unsupported. `speaker_pwm(&duties, rate)`
prepares binary pulses from unsigned 0–255 duty samples, with sub-sample area
integration and a final disconnect.

With `psg`, `SoundConfig::none().psg(true)` installs the Tandy/PCjr port C0h.
`ports.psg()` exposes tone, attenuation and noise helpers. The clock is the PC
crystal divided by four; divisor zero means 1024. Noise uses a 15-bit LFSR with
white-noise taps 0/4, seed 4000h and deterministic control-write reset. Periodic
noise recirculates bit zero; tone channel two can clock either noise mode.
Attenuation steps are 2 dB, with level 15 muted. Sega/Game Gear variants are
unsupported.

With `gm`, load a caller-owned SF2 using `GmSynth::load_soundfont(&mut reader)`
and pass the shared font to `SoundConfig::gm(font)`. `mpu401` also supports
`MidiBackend::Custom` implementing the allocation-free `MidiSynth` contract.
UART defaults to 330h/331h: command 3Fh enters UART mode, FFh resets it and FEh
acknowledges both. Running status and interleaved real-time bytes are supported;
complete SysEx is buffered without callback allocation, up to 65,536 bytes.
Intelligent mode is unsupported. GM uses RustySynth with an eight-native-frame
event quantum, including program, sustain, pan, volume and pitch bend. GM1/GM2
System On and System Reset reset in place; other SysEx and aftertouch are ignored.
No SoundFont or ROM is distributed. MT-32 is not yet available: the evaluated
backend allocates on SysEx/reset/reverb changes and needs adaptation first.

## Additional file formats

`Smf::load(bytes)` prepares type 0/1 MIDI. Tracks merge stably by tick, track and
event order; tempo changes retain fractional clock phase. PPQN and SMPTE 24/25/
30000:1001/30 divisions are supported. Split SysEx completes at its final packet's
timestamp. Type 2, RIFF, raw F7 escapes and malformed/truncated tracks are errors.

`Vgm::load(bytes)` prepares uncompressed VGM OPL/PSG register logs with explicit
intro/loop offsets and cumulative 44,100 Hz waits. Supported FM families are
YM3812, YM3526 and YMF262 at fixed DOS clocks; dual YM3812 requires SB Pro hard
panning. PSG logs must declare the modeled Tandy 15-bit noise variant; Sega's
default PSG is incompatible. VGZ, unrelated chips, data blocks, extra headers,
clock overrides and unsupported commands return errors. The API documentation
lists version, clock and flag limits. File parsing occurs outside rendering.

## Features and examples

| Feature | Purpose |
| --- | --- |
| `opl` (default) | MIT-licensed `oplon` FM core |
| `sb` (default) | SB1/2/Pro/Pro2/16, DMA/memory/SFX, IRQ and FM |
| `formats` (default) | IMF/DRO/VGM/SMF parsers (optional `midly`) |
| `toolkit` (default) | game-toolkit-audio stream integration |
| `mpu401` | MIDI UART and custom synthesis |
| `gm` | MIDI UART and caller-supplied SF2 via RustySynth |
| `psg` | Tandy SN76489 tone/noise |
| `nuked` | Optional LGPL-2.1-or-later Nuked OPL3 core |

Without default features, PC speaker, timeline, capture and offline WAV output
remain available. This crate uses `std`; the shared bus crate is `no_std`.

```sh
cargo run --no-default-features --features opl --example adlib -- target/adlib.wav
cargo run --no-default-features --features sb --example sb_pcm -- target/sb_pcm.wav
cargo run --example live_adlib
cargo run --example render_file -- song.dro target/song.wav
cargo run --example render_file -- song.imf target/song.wav 560
cargo run --features psg --example render_file -- song.vgm target/song.wav
cargo run --features gm --example render_file -- song.mid target/song.wav /path/to/font.sf2
task test
task check
task demo           # Play the AdLib phrase live (also demo:adlib).
task demo:sb        # Render and play the Sound Blaster PCM tone.
task demo:pcm       # Play a PCM buffer live, advancing the game clock.
task render:adlib   # Export WAV without playback.
task render:sb      # Export WAV without playback.
task render:pcm     # Export the buffered PCM chirp.
task demo:models    # Hear all five Sound Blaster models.
task demo:dma       # Four overlapping effects and 40 memory-DMA refills.
task demo:music     # Live runtime replacement and fades.
task demo:speaker   # Original PLAY phrase and PWM chirp.
task demo:psg       # Tandy arpeggio and noise percussion.
task demo:gm SF2=/path/to/font.sf2  # Uses your SoundFont.
```

WAV demo tasks play their generated output with the first available player: `ffplay`,
`afplay`, `pw-play`, `paplay`, or `aplay`. `demo:adlib` uses the toolkit's audio
output directly, as does `demo:pcm`. Playback tasks require a working audio device.

Requires Rust 1.95 or later. The tests cover hardware detection, timer phases,
PIT readback, sub-sample speaker pulses, queue overflow, malformed files, stereo
routing, DSP reset and DMA/IRQ protocols, callback-size independence, offline/live equivalence and allocation-free
rendering. No ROMs, SoundFonts or third-party music are included.

Reference interfaces: [oplon](https://docs.rs/oplon/1.0.0/oplon/),
[rtrb](https://docs.rs/rtrb/0.4.1/rtrb/), and the
[DRO format specification](https://moddingwiki.shikadi.net/wiki/DRO_Format).

## License

Dosaudio's own code is **EUPL-1.2 OR MIT OR Apache-2.0**, at your option. See
`LICENSE-EUPL`, `LICENSE-MIT` and `LICENSE-APACHE`.

Dependencies have their own terms: `game-toolkit-dosbus` and the optional toolkit
host are EUPL-1.2, `oplon` is MIT, `rtrb` is MIT OR Apache-2.0, and optional `nuked-opl3` is
LGPL-2.1-or-later. The triple license applies to this crate's own code; it does
not relicense its dependencies. See [THIRD-PARTY.md](THIRD-PARTY.md).
