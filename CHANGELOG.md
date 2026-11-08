# Changelog

## [Unreleased]

## [0.1.0] - 2026-11-08

`Error` and `MusicEvent` are `#[non_exhaustive]`: matches need a wildcard arm, and
new variants in later releases are not breaking.

- Add SB1/Pro/Pro2/16 DSP formats, indexed mixers, stereo FM aliases and BLASTER parsing.
- Add conventional-memory 8237 DMA, bounded completion/refill events and fixed-voice SFX mixing.
- Add runtime music replacement/live events, control-side resource reclamation and fades.
- Add QBasic PLAY, independent speaker handles/PWM and optional Tandy PSG.
- Add optional MPU-401 UART, custom MIDI synthesis, caller-supplied RustySynth SF2 and SMF loading.
- Add VGM OPL/PSG logs with introduction/loop regions and prepared multi-device events.
- Add bounded synthesis-side OPL capture and six pinned cross-core audio regression baselines.
- Add audible model, DMA/refill, runtime-music, speaker/PWM, PSG and user-SF2 demo tasks.
- Add game-thread hardware control and an allocation-free audio renderer over
  the published `game-toolkit-dosbus` interface.
- Add PC speaker periodic PIT modes, OPL2/dual OPL2/OPL3 ports and timers,
  the default oplon backend and optional Nuked backend.
- Add prepared music playback, pause/loop/volume/stop handles, IMF and DRO loaders,
  game-thread port capture with DRO export, and deterministic offline WAV output.
- Add toolkit stereo streaming, runnable demos, public-interface tests and CI.
- Add Sound Blaster 2.0 DSP reset/detection, direct DAC, 8-bit mono single-cycle
  and auto-init host-serviced DMA, IRQ acknowledgement, halt/resume, relocated
  ISA resources, shared OPL aliases, and an offline PCM demo.
- Use `folder.rs` plus `folder/` throughout instead of `mod.rs`.
- Add `play_pcm` with shared sample buffers, cloneable stop/completion handles,
  automatic DSP DMA servicing, transactional time advances, combined music/PCM
  offline rendering, and live/render task demos.

[Unreleased]: https://github.com/sunsided/game-toolkit-dosaudio/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/sunsided/game-toolkit-dosaudio/releases/tag/v0.1.0
