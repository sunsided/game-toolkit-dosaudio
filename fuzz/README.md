# Fuzzing dosaudio

This standalone nightly workspace exercises public music/bank parsers,
prepared SysEx ranges, runtime playback controls and MPU-401 MIDI framing.

From the repository root:

```sh
rustup toolchain install nightly --component rust-src
cargo install cargo-fuzz --version 0.13.1 --locked
task fuzz:smoke
task fuzz:run FUZZ_TARGET=music_formats FUZZ_SECONDS=60
```

Targets: `music_formats`, `opl_banks`, `playback`, `midi_uart`. The default
AddressSanitizer, debug assertions and overflow checks remain enabled.
`seeds.py` creates original corpus inputs; generated corpora/artifacts/builds
stay ignored. No ROMs, SoundFonts, third-party banks or songs are used.

See [testing.md](../tests/fixtures/testing.md) for bounds, invariants, selectors,
reproduction/minimization and longer property-test runs.
