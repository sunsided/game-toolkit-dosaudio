# Generated tests, fuzzing and caller assets

## Property tests

`tests/property.rs` uses proptest 1.12 as a development-only dependency. These
properties run as part of ordinary `cargo test`; feature gates keep the
speaker-only build usable. The focused task enables the relevant parser/FM APIs:

```sh
task test:property
PROPTEST_CASES=4096 PROPTEST_RNG_SEED=20261109 task test:property
```

Coverage includes valid MIDI byte preservation and invalid data rejection;
SysEx framing/ranges with u32 overflow; fallback cycles/selection against an
independent graph model; exact cumulative IMF timing and equivalent type-0/type-1
inputs; captured DRO banks/register escapes/final delays; and randomized device
switch, pause, resume, fade, clone/drop and stop traces with identical samples and
handle state across callback partitions. Generators construct meaningful valid
inputs as well as invalid mutations. Proptest shrinks failures and prints a
reproducible seed. Preserve generated regression seed files and add a focused
regression when a failure exposes a defect.

## Fuzz targets

The separate `fuzz/` workspace uses cargo-fuzz/libFuzzer with AddressSanitizer,
coverage feedback, debug assertions and overflow checks. It has no toolkit audio
host or proprietary assets, and its dependencies are absent from normal builds.
Install the development tools once:

```sh
rustup toolchain install nightly --component rust-src
cargo install cargo-fuzz --version 0.13.1 --locked
```

```sh
task fuzz:smoke
task fuzz:run FUZZ_TARGET=playback FUZZ_SECONDS=60
# Direct invocation, including a reproducible random seed:
python3 fuzz/seeds.py
cargo +nightly fuzz run music_formats -- -max_total_time=60 -seed=20261109 -max_len=65536
```

| Target | Inputs and assertions |
| --- | --- |
| `music_formats` | IMF0/1, DRO, VGM, SMF, UTF-8 PLAY, BLASTER/short MIDI and prepared SysEx ranges. Successful music has ordered/bounded events and valid owned payload ranges. |
| `opl_banks` | SBI, OP2, WOPL and BNK. Successful entries have ordered unique addresses, stable lookup and pass public insertion validation. |
| `playback` | Bounded runtime control programs: device switches, pause/resume, fades, clone/drop, stop, live events and reclamation. Samples and handle state must match across callback sizes, and samples stay finite/in range. |
| `midi_uart` | Arbitrary timestamped MPU-401 bytes, running status, real-time bytes, malformed/aborted SysEx and resets. The custom synth accepts only complete valid messages; message digest/count and samples must match across callback sizes. |

Music/bank targets cap each input at 65,536 bytes and do not render arbitrary
file durations. Playback caps input at 256 bytes and renders at most 64 frames
per instruction; UART caps it at 512 bytes and renders 2,304 frames. These
bounds keep fuzz cases bounded without requiring user assets. Checkpoint/queue
limits remain enforced by production code.

`fuzz/seeds.py` deterministically generates original valid and malformed seeds.
Seeds contain no firmware, SoundFonts, copied instrument banks or third-party
songs. Local evolving corpora, sanitizer builds and crash artifacts are ignored.
The `fuzz/Cargo.lock` file is tracked. The parser selector bytes and stateful
instruction layouts are defined in each target and `fuzz/src/lib.rs`.

A failure leaves an artifact under `fuzz/artifacts/<target>/` and prints its
reproduction command. Run or minimize it with the same target:

```sh
cargo +nightly fuzz run playback fuzz/artifacts/playback/crash-<hash>
cargo +nightly fuzz tmin playback fuzz/artifacts/playback/crash-<hash>
```

Promote a minimized failure to a deterministic regression before fixing it.
Do not add caller-owned assets to a corpus. CI runs the properties in its normal
feature matrix and all four seeded fuzz targets for 256 smoke iterations each;
longer campaigns remain available through the Taskfile.

Local sanitizer campaigns on 2026-11-09 completed 100,000 inputs for each
parser target, 2,556 runtime playback programs and 1,831 UART streams, with no
crashes or invariant failures. They used seed `20261109`, up to 15 seconds or
100,000 runs per target. These bounded campaigns establish runnable coverage;
they do not claim exhaustive verification.

## Caller-ROM verification

CM-32L validation remains explicitly opt-in, so ordinary tests and fuzzing need
no firmware. Supply a directory with one supported control image and full PCM:

```sh
task test:mt32-roms ROMS=/path/to/roms
DOSAUDIO_MT32_ROM_DIR=/path/to/roms cargo test --release --no-default-features \
  --features mt32 --test mt32 --locked -- --ignored
```

When a directory has both supported revisions, the loader prefers v1.02. To
verify v1.00 separately, supply a directory containing only its control image
and the full PCM (symlinks to caller-owned files work). See
[music_backends.md](music_backends.md) for the completed two-revision debug/release
verification and measured demo output. Firmware is never packaged or committed.
