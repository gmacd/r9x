---
status: done
---

# fdt: property_value_as_u32_iter over-reads a non-multiple-of-4 property

Confirmed and located (2026-08-27 audit; the parser moved to
`core/src/fdt.rs` — commit e4a403a). The iterator at
`core/src/fdt.rs:170-181` computes `value_end` (:172) and guards
`value_i >= value_end` (:174), but then slices `value_i..value_i+4`
against the *structures block* (:179). For `value_len = 6`: iteration 2
has `value_i = 4 < 6`, so it reads bytes 4..8 — the last two bytes come
from past the property (alignment padding or the next token). Garbage
final cell, no memory unsafety (`get` bounds the slice; `bytes_to_u32`
returns `None` near the block end).

Exposure is real, not theoretical: `timer_intid_from_dt` feeds this
iterator firmware-supplied `interrupts` cells (timer.rs:230-246).
`property_value_as_u32` (fdt.rs:165-167) is bounded correctly — only
the iterator has the bug.

**The fix pattern is already in the file**: the sibling iterators check
remaining length against `value_end` (`property_reg_iter` at
fdt.rs:226-229, the ranges iterator at :281). Make the u32 iterator's
guard `value_i + 4 > value_end` to match — two lines. Reference
behaviour: libfdt requires cell-property `len` to match exactly and
never manufactures bytes past it
(linux/scripts/dtc/libfdt/fdt_ro.c:519-521); the Rust `fdt` crates
slice the value to `len` first so a trailing partial cell is dropped
structurally.

Done when: the iterator stops at the property's value_end (the partial
trailing cell is dropped) with a host test feeding a non-multiple-of-4
property (host tests exist at `core/tests/fdt_test.rs`; `cargo xtask
test` runs `r9x-core`).

Origin: Claude code review of the timer-intid-source change, 2026-08-21
(flagged as pre-existing, out of scope there); confirmed by the
2026-08-27 backlog audit.
