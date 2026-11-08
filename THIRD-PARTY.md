# Third-party dependencies

| Package | License | Use |
| --- | --- | --- |
| game-toolkit-dosbus 0.1 | EUPL-1.2 | Shared hardware time and I/O contracts |
| rtrb 0.4 | MIT OR Apache-2.0 | Bounded SPSC command queue |
| oplon 1.0 | MIT | Default FM synthesis core |
| game-toolkit-audio 0.11 | EUPL-1.2 | Optional output host, with its transitive dependencies |
| nuked-opl3 0.1 | LGPL-2.1-or-later | Optional reference synthesis core |
| allocation-counter 0.8 | MIT OR Apache-2.0 | Test-only allocator instrumentation |

Cargo resolves dependencies under their own licenses. The audio crate's source
license does not change the terms of a combined build. No chip-core source,
firmware ROMs, SoundFonts, extracted instrument banks, or third-party songs are
vendored here. Dependency source packages retain their own notices.
