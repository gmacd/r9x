---
id: 107
status: open
wave: 0
---

# Task 107: bitmap allocator frees the wrong bit, and three neighbouring off-by-ones

## Status: open — wave 0 (fold in task 9)

## Problem

Four arithmetic defects in `port/src/bitmapalloc.rs`.  All are pure
functions of their inputs and host-testable.

**`deallocate` clears the wrong bit (`:164`).**  The check on `:161` is
`if !bitmap.is_set(8 * byte_idx + bit_idx)`, which is right; the clear on
the next line is `bitmap.set(bit_idx, false)`, which is not.  For any
page whose `byte_idx != 0` the free clears a bit in byte 0 instead:
`deallocate(PhysAddr(4096 * 8))` gives `(0, 1, 0)`, tests bit 8, and
clears bit 0.  Physical page 0 is marked free while still in use — it
will be handed out a second time — and page `0x8000` stays allocated
forever.

Latent only because nothing calls `deallocate` yet (`port/src/allocator.rs:115`
is `unimplemented!()`).  It fires the moment process teardown lands.
The existing unit test at `:399` frees `PhysAddr(4)`, which lands in
byte 0, so it passes.

**`pa > self.end` should be `>=` (`:154`).**  `end` is the exclusive
upper bound — `free_unused_ranges` sets it from `available_mem.end`
(`:119`) and `mark_range` rejects `range.end > self.end` (`:220`).  So
`deallocate(end)` is accepted and clears the bit for a page one past RAM.

**`currpa` advances once per bitmap, not per byte (`:278`).**  It should
step by `bytes_per_bitmap_byte()` (8 × page size) per byte.  After all 32
bitmaps `currpa` has reached 128 KiB rather than the real end, so the
`currpa >= self.end` truncation in `indices_from` (`:260`) is dead code
and `allocate`/`usage_bytes` scan past `end`.  Masked today because
`free_unused_ranges` marks `end..max_bytes` allocated (`:122-123`); it
becomes a live wrong-address bug the moment that marking is skipped.

**`mark_range` rounds outward for frees (`:224`).**  `step_by_rounded`
rounds start down and end up.  That is right for "mark used" and wrong
for "mark free": a free range starting mid-page marks the whole
containing page free.  Safe today only because every range in
`boot.rs:37-41` happens to be aligned by `kernel.ld`; one unaligned
linker symbol silently frees a page of kernel text.

## Design

- Fix `:164` to `bitmap.set(8 * byte_idx + bit_idx, false)`.
- Fix `:154` to `>=`.
- Advance `currpa` by `bytes_per_bitmap_byte()` per byte in the scan.
- Round inward for `mark_free`, outward for `mark_allocated`.  Task 9
  (delete `mark_range`'s `check_end` flag) is the same function and
  should land in the same change.
- Note `aarch64/src/pagealloc.rs:31`: `BitmapPageAlloc<32, PAGE_SIZE_4K>`
  covers exactly 4 GiB, so an 8 GB Pi 4 fails `init_page_allocator`
  rather than clamping.  Filed in task 127; mentioned here because it is
  the same constant.

## Tests

- Host: `deallocate` round-trips for a page in every byte position, not
  just byte 0 — the existing test's blind spot.
- Host: `deallocate(end)` is rejected; `deallocate(end - page)` is not.
- Host: the scan reaches the real `end` and refuses beyond it.
- Host: an unaligned `mark_free` range does not free the containing page.

## Done when

- All four are fixed, each with a host test that fails before the change.
- Task 9 folded in.
- Full `cargo xtask ci` green.

Origin: architecture and correctness review of `f76d96a`, 2026-08-28.
