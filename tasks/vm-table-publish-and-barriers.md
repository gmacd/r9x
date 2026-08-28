---
id: 105
status: open
wave: 0
---

# Task 105: page-table pages are published before they are cleared, and TLBI ordering is wrong

## Status: open — wave 0

## Problem

Three ordering defects in the same walker, all in `aarch64/src/vm.rs`.

**Publish before clear (`:384`).**  `write_entry` makes the parent entry
valid — and issues a broadcast TLBI — at `:384`; the new page is only
resolved and `clear()`ed at `:388-391`.  `pagealloc::allocate_physpage`
returns raw DRAM (`aspace.rs:101-103` says so explicitly, which is why the
root page is zeroed by hand there).  Between the two, the hardware table
walker — speculatively on this core, or a real walk on another once task
124 lands — reads stale DRAM as descriptors and can cache translations to
arbitrary physical addresses with arbitrary permissions.

`vminit.rs:436-449` gets the order right: clear, then publish.  The
recursive-addressing design is what forces the inversion here (the entry
must exist before the page is reachable through the recursive VA), so the
fix needs either a scratch mapping or a zeroing allocator.

**No `dsb ishst` before the TLBI (`:812`).**  `VmTraitImpl::write_entry`
does `write_volatile(entry)` then `invalidate_all_tlb_entries()` —
`tlbi; dsb ish; isb`.  The architecture requires the descriptor store to
be ordered *before* the TLBI with a `DSB ISHST`; without it the TLBI can
complete before the new descriptor is visible to the walkers, so a walk
re-fetches and re-caches the old one after the invalidate.

**No break-before-make (`:475`).**  `map_to` writes a new descriptor
straight over a live one.  Changing the output address, attributes or
block size of a valid entry in a single store is CONSTRAINED
UNPREDICTABLE and can raise TLB conflict aborts.  Reachable today via two
`sys_map_mmio` calls at one VA, and via the heap regrow in task 111.

## Design

- Zero at the source: make `allocate_physpage` hand back zeroed pages, or
  add a `alloc_zeroed_physpage` the table walker uses.  That fixes the
  publish-order bug without a scratch mapping and also closes the
  info-leak half of task 111.
- Add `dsb ishst` between the store and the TLBI.
- Implement break-before-make in `map_to`: invalidate, write invalid,
  DSB, write new, DSB, ISB.
- While here, decide whether `invalidate_all_tlb_entries` (a full
  `vmalle1is` per entry) should become an address-scoped `tlbi vae1is` —
  the current shape is correct but is a broadcast full flush per mapped
  page, and `sys_map_mmio` does one per 4 KiB.

## Tests

- Integration: the task 91 VM matrix rows for BBM and for live-permission
  change are the acceptance evidence.
- Host: none directly (barriers are target-only); the zeroing change is
  host-testable in the allocator.

## Done when

- New table pages are zero before any walker can reach them.
- The store→TLBI ordering is architecturally correct at every site.
- Overwriting a valid descriptor uses break-before-make.
- Full `cargo xtask ci` green.

Origin: architecture and correctness review of `f76d96a`, 2026-08-28.
