# OPL bank and MIDI fixtures

The starter set in `src/opl/bank/starter.rs` contains six melodic patches and
five percussion patches authored as register values for dosaudio on 2026-11-08.
It is available under EUPL-1.2 OR MIT OR Apache-2.0, like the crate. No game bank,
ROM, sample, SoundFont or third-party song was used. Timbres have descriptive
names and are not intended to reproduce a complete General MIDI instrument set.

`tests/opl_banks.rs` constructs SBI, OP2, WOPL and AdLib BNK files in memory.
Register patterns and names are original; no historical bank is redistributed.
`tests/opl_midi.rs` generates original MIDI messages and exercises public driver,
port, capture and synthesis interfaces. Demo phrases are original too.

Layout references consulted (2026-11-08; implementations written independently):

- [OP2 layout and fine tuning](https://github.com/Wohlstand/OPL3BankEditor/blob/master/Specifications/DMX%20OP2%20Bank%20Format.txt).
- [WOPL v1–3 layout](https://github.com/Wohlstand/OPL3BankEditor/blob/master/Specifications/WOPL-and-OPLI-Specification.txt),
  checked against the [format author's reader](https://github.com/Wohlstand/libADLMIDI/blob/master/src/wopl/wopl_file.c).
- [AdLib BNK field ordering](https://github.com/adplug/adplug/blob/master/src/composer.cpp),
  `load_bnk_info`, `read_bnk_instrument`, `read_fm_operator`.
- [DMX bank interpretation](https://github.com/chocolate-doom/chocolate-doom/blob/master/src/i_oplmusic.c),
  fixed-note transposition and second-voice detune semantics.

Coverage is deliberately bounded to two-operator and layered two-operator
instruments. WOPL true-four-operator and hardware-rhythm flags are rejected,
as are MT-32 defaults and ADLMIDI-specific volume models. OP2 delayed vibrato
is rejected. BNK accepts version 1.0 melodic timbres only, assigning programs by
used-name-table order because BNK has no inherent MIDI program mapping. WOPL v1
uses bank order as MSB and zero LSB; v2/v3 use their explicit metadata.

MIDI percussion uses ordinary melodic OPL voices and fixed-note bank entries;
`opl_patches` separately demonstrates all five hardware rhythm drums through
existing helpers. Stereo pan is quantized to the routing available on each chip.
The driver combines velocity/volume/expression as amplitude and rounds to 0.75 dB
OPL total-level steps. Layer detune is quantized to 1/32 semitone, matching OP2.

Compiled loops restore a register snapshot at the exact source event boundary;
notes crossing that point retrigger their envelopes. This is deterministic
register replay, not a hidden synthesizer snapshot. All parsing, compilation,
sequence allocation and final resource reclamation remain outside rendering.
