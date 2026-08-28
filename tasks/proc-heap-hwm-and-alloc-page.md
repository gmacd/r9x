---
id: 111
status: open
wave: 3
---

# Task 111: heap_alloc_page returns the wrong VA for its PA, and both heap paths rewind the watermark

## Status: open — wave 3

## Problem

`heap_hwm` is documented at `aarch64/src/process.rs:258-261` as "the
highest top reached", so that a regrow after a `SYS_FREE` reuses the
already-mapped pages in `[heap_brk, heap_hwm)` "instead of re-mapping
(and double-allocating) them".  Two paths break it.

**`heap_alloc_page` returns a VA that is not the page it mapped
(`:1670`).**  It maps at `p.heap_hwm` but returns `old_brk`
(= `p.heap_brk`) as the virtual address, paired with the *physical*
address of the page it mapped at `heap_hwm`.  These differ after any
`SYS_FREE`.  Grow four pages (`brk = hwm = base + 16K`), free three
(`brk = base + 4K`, `hwm unchanged`), then call `SYS_ALLOC_PAGE`: a fresh
page is mapped at `base + 16K`, the caller is told the VA is `base + 4K`,
and is handed the PA of the page at `base + 16K`.

This is the DMA path.  `cmd/mailbox` uses `SYS_ALLOC_PAGE` for exactly
this — it needs a (VA, PA) pair because the BCM283x mailbox takes a
physical address — so it writes its request at one address and points the
VideoCore at an unrelated physical page.

**Both paths rewind `heap_hwm` (`:1649`, `:1673`).**  `p.heap_hwm =
new_brk` is unconditional in `heap_grow`, and `heap_alloc_page` sets it
to `old_brk + 4096`.  After a free-then-smaller-grow the watermark drops
below the true mapped extent, so the next grow re-maps live VAs — and
`Aspace::map_user_data_page_pa` (`aspace.rs:192`) allocates a *fresh*
physical page and overwrites the leaf.  The old page is orphaned and
never freed (nothing frees pages this arc), and the heap contents at that
VA silently vanish.  Ordinary alloc/free churn in `r9x_std`'s
`GlobalAlloc` reaches it.

**Neither mapping path zeroes.**  `map_user_page` (`aspace.rs:141`) does
not zero, but `process.rs:828` explicitly relies on it doing so ("The
page is zeroed by `map_user_page`, so only the header words are
written") — so the handles page leaks 4076 bytes of prior DRAM into every
child.  `spawn_raw`'s stack page (`:622-624`) and the tail of its text
page beyond `text.len()` (`:619`) are the same.  `load_elf` (`:762`) is
the only path that zeroes.  `map_user_data_page_pa` (`:192`), the heap
path, does not either; today that is firmware residue rather than another
process's data only because nothing recycles physical pages yet.

## Design

- `heap_alloc_page` returns the VA it actually mapped.
- `p.heap_hwm = new_brk.max(p.heap_hwm)` in both places.
- Zero at the allocator (see task 105, which needs the same thing for
  page tables) rather than at each call site — one fix, three consumers,
  and it cannot be forgotten by the fourth.
- Correct the stale claim in the comment at `process.rs:828`.

## Tests

- Host: `brk_grow`/`brk_shrink` already have pure-function tests; add the
  watermark invariant (`hwm` is monotonic) to them.
- Integration: grow, free, grow smaller, grow again — assert the heap
  contents below the old watermark survive.
- Integration: `SYS_ALLOC_PAGE` after a free returns a (VA, PA) pair that
  actually corresponds — write through the VA, read back via a device or
  a second mapping of the PA.

## Done when

- The (VA, PA) pair is consistent on every path.
- `heap_hwm` is monotonic and the docs match.
- Pages handed to user space are zero.
- Full `cargo xtask ci` green.

Origin: architecture and correctness review of `f76d96a`, 2026-08-28.
