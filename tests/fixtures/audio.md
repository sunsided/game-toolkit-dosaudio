# Original audio regression fixtures

`../audio_goldens.rs` constructs every input in memory. These are original test
signals written for this repository on 2026-11-08, under the repository's
licenses. No game music, recordings, downloaded files or third-party musical
material is used. The tests need no audio device, window, network or new dependency.

## Inputs and capture contract

Each signal uses a sustained sine carrier and a nearly silent FM modulator:
operator multiplier 1, modulator total level 63, carrier total level 8, attack
15, decay 0, sustain 0, release 8, sustain enabled, feedback/connection 0.
F-number 580 at block 4 gives approximately 440 Hz; F-number 290 gives 220 Hz.
The slight remaining modulation produces a measurable small harmonic component.

- **IMF type 0:** 11 register/value/u16-delay setup records at time zero,
  followed by key-off at 400 ms and 350 ms delay. Its explicit tick rate is
  1,000 Hz; the instrument's final key-on record supplies the 400 ms delay.
  OPL2 compatibility mode sends identical channels.
- **DRO v2:** uncompressed, OPL3 hardware, an explicit register code map and
  high-bank `0x105 = 1` mode enable. The 440 Hz note starts left, switches right
  at 200 ms, and releases at 400 ms. Short delays encode 200 ms twice; long
  256 ms plus short 94 ms encode the release tail. Total duration is 750 ms.
- **Direct register trace:** dual OPL2, 440 Hz left from 0–400 ms and 220 Hz
  right from 100–500 ms, followed by release tails. Uses explicit `TimedEvent`
  writes including bank `0x100`, with a declared duration of 750 ms.

All captures use `OfflineMachine`, 48,000 Hz, zero latency, playback gain 1.0
(the `PlayOptions::default()` value), and only the selected FM hardware.
No PC speaker or Sound Blaster is installed. Explicit rendering lasts 750 ms;
it does not use `render_to_end` or add its automatic tail. Conversion from
integer master-crystal ticks floors this endpoint, yielding **35,999 stereo
frames** with the current timeline. Every float sample must be finite and
unclipped (`abs(sample) < 1`).

Hashes cover exactly the 143,996 PCM bytes after the 44-byte WAV header, in
interleaved left/right order. `Wav::to_bytes` clamps each f32 sample to [-1,1],
multiplies by 32767, rounds, casts to signed i16 and emits little-endian bytes.
FNV-1a-64 starts at `0xcbf29ce484222325`; for each byte, XOR then multiply by
`0x100000001b3` with wrapping u64 arithmetic. Container metadata is excluded.

## Pinned baseline and review metadata

Captured and checked on 2026-11-08 against repository baseline
`12c3731a62bd5cf8ca8d1f063e47fa2902cc6bf3`, before synthesis renderer changes.
Concurrent work on Sound Blaster configuration was present, but these fixtures
install no Sound Blaster and the OPL adapter/renderer were unchanged.

- Platform: Linux x86_64.
- `rustc 1.99.1 (58cf8f9c3 2026-10-13)`.
- `cargo 1.99.1 (5f94df478 2026-08-27)`.
- `oplon` **1.0.0**, registry SHA-256
  `0df1044d4a29bc6db30e653b263b80599bcb4f15c0c0e2c61f9ac32f71d58de3`.
- `nuked-opl3` **0.1.0**, registry SHA-256
  `fe763ce652db623a34372b1628588faaee86af83195175acee070d92d1fe27b7`.

| Input | oplon 1.0.0 FNV-1a-64 | nuked-opl3 0.1.0 FNV-1a-64 |
| --- | ---: | ---: |
| IMF OPL2 | 8251495682276887477 | 7503558725369065500 |
| DRO OPL3 | 11970232720812347942 | 832608049917537739 |
| Direct dual OPL2 | 18132943180728515164 | 16305653591510393370 |

Review checks: exact frame count and encoding are asserted; all six baseline
assertions pass; both cross-core comparison and deliberate wrong-pitch and
swapped-pan rejection pass. These literals protect this implementation and
backend versions; they are not claims of physical-chip accuracy. Backend
waveforms are deliberately not required to match each other byte for byte.

## Cross-core measurements and tolerances

The IMF comparison measures each core independently over a 100–300 ms steady
window. A direct DFT scans 200–500 Hz in 2 Hz steps for pitch; the 200 ms window
contains 88 nominal 440 Hz cycles. Fundamental and harmonic fractions are
`2 * abs(DFT(f))^2 / (sample_count * sum(sample^2))`, requiring no FFT library.
The first 10 ms measure attack, 425–450 ms measure release progress after the
400 ms key-off, and 650–700 ms measure the final tail. Envelope values are RMS
relative to the steady window. Pan windows (50–150 and 250–350 ms) avoid edges.

| Property | Required comparison | Measured baseline (oplon / nuked) | Reason |
| --- | --- | --- | --- |
| Pitch | reference within 4 Hz of 440; cores within 4 Hz | 440 / 440 Hz | Allows two 2 Hz scan steps and slight clock/window error; rejects octave changes. |
| Fundamental energy | both ≥95%; difference ≤2 percentage points | 99.7114% / 99.7118% | Leaves room for core approximation and finite-window leakage while preserving the sine spectrum. |
| Second + third harmonic energy | both ≤2% | 0.2867% / 0.2864% | Allows residual modulation; catches strong distortion/wrong instruments. |
| Sustained RMS | reference ≥0.005; candidate/reference 0.9–1.1 | 0.0439624 / 0.0439480 | Excludes silence and permits 10% core level variation. |
| Attack/steady RMS | both 0.8–1.2; difference ≤0.05 | 1.01016 / 1.00863 | The programmed attack is effectively immediate; allows phase and startup differences. |
| Mid-release/steady RMS | both 0.05–0.4; difference ≤0.05 | 0.17437 / 0.17138 | Requires audible decay progress and similar release timing, without sample alignment. |
| Final tail/steady RMS | both ≤0.02 | 0 / 0.000488 | Allows quantization-floor differences while requiring substantial release. |
| Stereo routing | intended/silent channel RMS ≥100 in both windows, both cores | silent channel zero in both cores | Requires ≥40 dB isolation and correct left-then-right direction; avoids dividing by zero with a 1e-12 floor. |

A separate test renders an intentional octave error (220 Hz) and intentionally
swapped OPL3 pan bits through Nuked, then requires the same comparison functions
to return the specific pitch and stereo errors. It must remain passing: a test
that merely compares two identical broken signals would fail this sanity check.
These small signals cover two-operator tones, key edges and stereo routing;
they do not establish rhythm, four-operator or every envelope setting's accuracy.

## Deliberate regeneration

Run the existing baseline before editing the renderer or upgrading cores:

```sh
cargo test --test audio_goldens --no-default-features --features opl,formats,nuked --offline
```

For an intentional change, retain the old literals while running the tests;
assertion failures show the actual hash (or pair of hashes). Independently review
the renderer/core diff, input bytes/register schedule, timing, gain and PCM
encoding. Inspect or listen to PCM if the change affects sound. Review the
cross-core measurements and retain the mutation tests; do not widen tolerances
just to hide a failure. Only then replace the affected literals and this table
in the same change, recording the reason, source commit, exact tool/core
versions, platform and measured properties here. Run the command twice, plus
`--release`, and check identical pinned results. No environment variable,
ignored test, snapshot auto-update or generation-only passing test exists.

Feature checks: the direct trace runs with `opl` alone; IMF/DRO checks require
`formats`; Nuked baselines require `nuked`; semantic comparisons require all
three. With no features this integration target contains zero tests.
