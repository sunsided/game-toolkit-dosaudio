# Changelog

## [Unreleased]

## [0.1.5] - 2026-11-09

- `Mt32Roms::from_rom_dir` also finds MAME's versioned ROM names
  (`cm32l_ctrl_1_02.rom`, `cm32l_ctrl_1_00.rom`, `cm32l_pcm.rom`).
- Add `demo:file`/`render:file` for your own DRO, IMF, VGM and MIDI files, and
  optional `MIDI=`/`BANK=` for `demo:opl-midi`. Example errors name the missing file.

## [0.1.4] - 2026-11-08

- Add optional ROM-validated CM-32L synthesis for MT-32 arrangements, adapting
  moont 1.0.0 with safe decoding and preallocated SysEx/reset/reverb resources.
  Supports CM-32L control v1.00/v1.02 and full PCM images supplied by the caller.
- Include the private core's LGPL-2.1-or-later notices and license texts; update
  package SPDX metadata to account for included third-party source.
- Add equal-timeline soundtrack arrangements, explicit device fallback and live
  device switching, with bounded control-thread restoration checkpoints and
  callback-safe ownership. Restored sounding notes retrigger their envelopes.
- Add optional synthie 0.8.0 preparation into native PCM, starter-bank OPL and
  monophonic speaker arrangements; native PCM output requires explicit opt-in.
- Restore OPL2 mono routing when changing an OPL3 core's compatibility mode.
- Add audible MT-32, synthie comparison and uninterrupted-switching demo tasks,
  isolated feature checks and allocation/lifetime regression coverage.

## [0.1.3] - 2026-11-08

- Add validated SBI/OP2/WOPL/AdLib BNK instrument banks and a small original starter set.
- Add `OplMidiDriver` for MIDI-to-OPL preparation and transactional live messages,
  with controllers, sustain, bend, percussion, layered voices and deterministic stealing.
- Share patch/tuning register mapping with existing OPL helpers; preserve compiled
  intro/loop boundaries with chip-state snapshots.
- Add audible patch/rhythm and MIDI-on-AdLib/dual-OPL2/OPL3 tasks.
- Model the 8253 timer with `game_toolkit_dosbus::Pit` (requires dosbus 0.1.2). With
  the speaker installed, `DosAudio` handles ports 40h-43h and exposes the timer
  through `DosAudio::pit()` for IRQ 0 timing; speaker audio is unchanged.
- PIT channel 2 mode 0 is a one-shot instead of a square wave, and
  `DosAudio::speaker_tone` reports no tone for it.

## [0.1.2] - 2026-11-08

- Add `SpeakerHandle::tone`, the frequency of the note a speaker melody sounds.

## [0.1.1] - 2026-11-08

- Add `DosAudio::speaker_tone`, the frequency of the gated PIT tone port I/O plays.
- Add `AudioEngine::play_speaker_at` to start a speaker melody at a given output tick.

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

[Unreleased]: https://github.com/sunsided/game-toolkit-dosaudio/compare/v0.1.5...HEAD
[0.1.5]: https://github.com/sunsided/game-toolkit-dosaudio/compare/v0.1.4...v0.1.5
[0.1.4]: https://github.com/sunsided/game-toolkit-dosaudio/compare/v0.1.3...v0.1.4
[0.1.3]: https://github.com/sunsided/game-toolkit-dosaudio/compare/v0.1.2...v0.1.3
[0.1.2]: https://github.com/sunsided/game-toolkit-dosaudio/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/sunsided/game-toolkit-dosaudio/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/sunsided/game-toolkit-dosaudio/releases/tag/v0.1.0
