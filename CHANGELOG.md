# Changelog

## [Unreleased]

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
