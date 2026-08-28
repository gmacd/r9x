---
status: done
---

# Minor cleanups in gic.rs and main.rs

**Severity: minor**

**Status: done** (2026-07-24) — main.rs demo tickers / `test_sysexit` left in place intentionally.

All in code staged on branch `2026-07-08-bump-rust`:

- `aarch64/src/gic.rs:96` — `GicdTyper`'s `Debug` impl prints the struct name
  as `"GiccIidr"` (copy-paste). Also, `GiccIidr`'s own `Debug` impl defines a
  `product_id` field in the bitstruct but omits it from the debug output.
- `aarch64/src/gic.rs:139-147` — leftover scaffolding: commented-out `nr_ppis`
  code, commented-out println lines, and the `// graham - ppis was 16 in old
  code` note should be removed before commit. Resolved questions (verified
  against the GICv2 spec + linux irq-gic.c:1204): PPIs are architecturally
  fixed at 16 in GICv2 (INTIDs 16-31, banked per CPU; nothing to probe — the
  commented formula is a garbled GICv3.1 GICR_TYPER[31:27] recipe and reads
  SecurityExtn/LSPI bits here). FIXED (uncommitted): the old `nr_spis`
  computed total interrupt lines, not SPIs (on QEMU raspi4b,
  ITLinesNumber=6: it reported 224 "SPIs" when the truth is 192). Now
  `nr_lines = min((it_lines_number+1)*32, 1020)` is stored (as Linux stores
  gic_irqs, since per-interrupt register loops iterate `32..nr_lines`) and
  SPIs are derived for display; the nr_ppis scaffolding and stray comments
  are removed.
- `aarch64/src/gic.rs:184,192` — in `find_gicc_gicd_virtranges` the locals are
  named `gicc_physrange` / `gicd_physrange` but hold `VirtRange`s (the result
  of `map_device_register`). Rename.
- `aarch64/src/gic.rs:156` — `1 << TIMER_INTID` into `GICD_ISENABLER` only
  works because INTID 30 < 32. A general `enable_irq(intid)` needs
  `GICD_ISENABLER + 4 * (intid / 32)` with bit `intid % 32`. Fine today; add a
  comment or generalise.
- `aarch64/src/main.rs` — the demo tickers (`PeriodicTicker1/2`,
  `HelloOneShot`) and the commented-out `test_sysexit()` call are marked
  temporary; confirm they are intended to land, or gate/remove them.
- `aarch64/src/gic.rs:120-127` (flagged by pi/Qwen3.6 second-opinion review) —
  `Gic::gicd_virtrange` and `nr_spis` are stored but never used after `new()`,
  both under `#[allow(dead_code)]`. They are holdovers for planned IRQ
  management; either implement `enable_irq`-style methods that use them or
  drop the fields until needed.
