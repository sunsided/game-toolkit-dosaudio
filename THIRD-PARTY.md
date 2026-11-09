# Third-party dependencies

| Package | License | Use |
| --- | --- | --- |
| game-toolkit-dosbus 0.1 | EUPL-1.2 | Shared hardware time and I/O contracts |
| rtrb 0.4 | MIT OR Apache-2.0 | Bounded SPSC command queue |
| oplon 1.0 | MIT | Default FM synthesis core |
| game-toolkit-audio 0.11 | EUPL-1.2 | Optional output host, with its transitive dependencies |
| nuked-opl3 0.1 | LGPL-2.1-or-later | Optional reference synthesis core |
| rustysynth 1.3.8 | MIT | Optional caller-supplied SF2 synthesis |
| sha1 0.10 | MIT OR Apache-2.0 | Optional ROM identity checks |
| synthie 0.8.0 | EUPL-1.2 | Optional control-thread native synthesis and song model |
| moont 1.0.0 (adapted, included source) | LGPL-2.1-or-later | Optional private CM-32L core |
| midly 0.5.3 | MIT OR Apache-2.0 | Optional SMF event decoding |
| proptest 1.12 | MIT OR Apache-2.0 | Test-only generated inputs and shrinking |
| libfuzzer-sys 0.4 | (MIT OR Apache-2.0) AND NCSA (upstream metadata); bundled LLVM source retains its Apache-2.0 WITH LLVM-exception notices | Separate development-only fuzz workspace |
| allocation-counter 0.8 | MIT OR Apache-2.0 | Test-only allocator instrumentation |

Cargo resolves dependencies under their own licenses. The audio crate's source
license does not change the terms of a combined build. Dependency source
packages retain their own notices. No firmware ROMs, SoundFonts, extracted
instrument banks or third-party songs are vendored here.

The private CM-32L core in `src/midi/mt32/cm32l.rs` and `src/midi/mt32/cm32l/`
is adapted from moont 1.0.0 and retains upstream copyright notices under
LGPL-2.1-or-later. See [UPSTREAM.md](src/midi/mt32/cm32l/UPSTREAM.md) for
source/archive provenance, local modifications, and the included LGPL/GPL
license texts. It compiles only with `mt32`, but its source is included in the
package; the Cargo license expression accounts for this. Separately authored
dosaudio code remains EUPL-1.2 OR MIT OR Apache-2.0.

The OPL starter instruments in `src/opl/bank/starter.rs` are original register
values authored for this crate on 2026-11-08, under the crate's triple license.
They contain no samples, ROM data or values extracted from a historical bank.
Bank readers implement documented file layouts; no reference implementation's
source or external instrument bank is copied into the crate. Format references
and supported subsets are recorded in `tests/fixtures/opl_banks.md`.
