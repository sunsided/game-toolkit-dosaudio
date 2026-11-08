# Private CM-32L core

Source: [moont 1.0.0](https://crates.io/crates/moont/1.0.0),
<https://gitlab.gnome.org/geoffhill/moont>.
Original crate archive SHA-256: `a40ac0fccd2350245fcb1054c31dd19fc78a2a645b2841b50285b89683b8240c`.
The original source files were checked against that archive before adaptation.
Adapted for dosaudio on 2026-11-09.

This directory and its parent `cm32l.rs` retain the upstream copyright notices
and are LGPL-2.1-or-later. `COPYING.LESSER.txt` and `COPYING.txt` contain the
license texts. Dosaudio's separately authored code retains its triple license;
the package SPDX expression accounts for this included LGPL source. Compiling
the `mt32` feature incorporates this core. No ROMs, ROM-derived data, samples,
bundled-ROM build script, external GM bank, or extracted instruments are included.

Changes from the published core:

- Move the library into a private `cm32l.rs` plus `cm32l/` module and adjust
  imports. Omit the optional ROM-bundling, GM mapper and SMF convenience modules;
  dosaudio already supplies MIDI transport and file parsing.
- Replace packed-struct pointer casts with safe, field-by-field byte decoders.
  The crate continues to forbid unsafe Rust.
- Reset existing SysEx memory in place rather than allocate a new boxed state.
- Allocate all four reverb filter networks during construction, then exchange
  retained networks and clear their buffers on mode changes.
- Use sixteen retained, 65,536-byte reference-counted SysEx blocks. Enqueue,
  clone, processing, reset cancellation and overflow never free the last owner
  in render. Full queues return false to the adapter's drop counter.
- Preserve insertion order and the core's one-message-per-native-frame/partial-
  abort processing. Dosaudio supplies completed-message timestamps, so remove
  the second simulated wire-transfer delay. Compare queued timestamps with
  wrapping arithmetic across the native u32 clock rollover.
- Immediate MIDI System Reset clears queued messages; an ordered Roland SysEx
  reset preserves following messages. Both reuse allocated state, and reset
  clears the reverb tail.
- Disable optional tracing, omit upstream ROM-dependent tests/document examples,
  and format the private sources using this repository's formatter. Formatting
  fixes do not change oscillator/envelope/filter algorithms or constant tables.

Dosaudio tests construct original, synthetic control tables and zero PCM outside
the public hash gate; these are not Roland firmware. They exercise audible LA
synthesis, every reverb mode, SysEx, reset ordering, clock wrapping, queue bounds,
chunk independence, and allocation/deallocation instrumentation. A separate,
explicitly ignored test accepts caller-owned ROMs via `DOSAUDIO_MT32_ROM_DIR`.

Firmware identification and revision offsets are checked against Munt's
[ROMInfo.cpp](https://github.com/munt/munt/blob/master/mt32emu/src/ROMInfo.cpp) and
[ControlROMMaps](https://github.com/munt/munt/blob/master/mt32emu/src/Synth.cpp).
The wrapper validates original v1.00/v1.02 images, then normalizes only v1.00's
reserve/pan/program/max tables to the v1.02 offsets expected by this core.
