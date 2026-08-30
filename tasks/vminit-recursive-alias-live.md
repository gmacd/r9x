---
id: 138
status: open
wave: 3
---

# Task 138: pre-MMU self-pointers leave live writable aliases in the inherited tables

## Status: open

## Problem

**B2 (fable, opus; VERIFIED logic, opus verified experimentally via
`pmemsave`).** Pre-MMU `next_mut` writes a self-pointer into slot 511 of
**every** table it allocates (`aarch64/src/pre_mmu/vminit.rs:452-456`),
with a comment acknowledging the slot is "dangerous at any level here —
unlike the post-MMU, where it is root-only" (`:418-424`). The tables it
builds are then **adopted as the live kernel tables** (TTBR1 at `main9`,
re-read via `aarch64/src/vm.rs:586-592`), but post-MMU
`Table::next_mut` refuses index 511 only at level 0 (`vm.rs:378`), and
its comment claims "Only level 0 has the self-pointer; index 511 at
L1/L2 is an ordinary slot and must still map" (`vm.rs:376-378`) — false
for the inherited tables. Live-guest dump (audit):

```
root L0 @0x20c5000:  [256]->0x20c6000  [511]->0x20c5000  (intended)
L1     @0x20c6000:  [  0]->0x20c7000  [511]->0x20c6000  (STALE SELF)
L2     @0x20c7000:  [0..16] 2M blocks [511]->0x20c7000  (STALE SELF)
L3     @0x20c8000:  [0..28] DTB 4K    [511]->0x20c8000  (STALE SELF)
```

The self-pointer descriptor is valid, table, `AP=PrivRw`, `PXN=0`:
**every page table also exists as a writable, EL1-executable alias** at a
KZERO VA the kernel believes is unmapped (audit probed the L3 table
through `0xffff8000081ff000` and `0xffff80003fe40000`).

**The corruption path:** any post-MMU 4 KiB mapping of a frame in PA
`[0x3fe00000, 0x40000000)` at `VaMapping::Offset(KZERO)` — which
`aarch64/src/aspace.rs:85-96` and `:163-174` do with arbitrary allocator
frames — has L2 index 511 under L1 index 0; `next_mut` finds the stale
self-pointer valid-and-table, returns the L2 table *itself* as the "L3"
table (the recursive VA `0xffffffc0001ff000` resolves
root→root→L1→L2→the L2 table itself), and the leaf write lands at L2[0]
— **the 2 MiB block mapping the kernel's own text**. `write_entry`
follows with `tlbi vmalle1is; dsb ish; isb`, so the image dies on the
next instruction fetch.

**Reachability, stated precisely** (audit ruling 3): *verified* — the
invariant is broken and the aliases are live, writable and
EL1-executable today, and any code path reaching `KZERO +
[0x3fe00000, 0x40000000)` corrupts the kernel-text mapping. *Not
verified* — that a stock-firmware Pi hands out such a frame:
`aarch64/src/pagealloc.rs:46-53` takes only the **first** memory
regblock, and stock firmware ends the first bank below `0x40000000`
(VideoCore reserve — the same convention behind QEMU's 960 MiB), so the
triggering frame may never be allocated on a stock Pi either. The defect
stands on the broken invariant and the live aliases regardless.

## Design

- Delete `vminit.rs:455` — nothing pre-MMU ever follows the pointer; the
  walk uses physical addresses directly.
- Narrow the pre-MMU guard to level 0 to match `vm.rs:378`.
- Deleting line 455 *makes* the texts at `vm.rs:376-378` and
  `docs/lessons.md:209-213` true; keep them and add a note that the
  invariant now depends on the pre-MMU side not installing self-pointers,
  so a future reader doesn't re-introduce them.

## Tests

- D2.1 (write this one first): for every table reachable from the kernel
  root at L1/L2/L3, `entries[511]` does not point at the table's own PA.
  Enumerate with a visited set — pre-fix the stale self-pointers are
  *cycles* and a naive recursive walk never terminates.
- D2.3/D2.4: walking `KZERO + 0x3fe00000` (L2 index 511 under L1 0) and
  `KZERO + 511 GiB` (L1 index 511) finds no valid entry.
- D2.5: the exact trigger, driven explicitly (QEMU's 960 MiB means the
  allocator would never reach that PA). **Pre-fix:** a dedicated image
  whose expected result is a timeout/prefetch abort (needs task 135's
  per-image failure expectations). **Post-fix:** a normal image where the
  mapping succeeds and D2.1/D2.2 still hold.
- D2.13: the (root) self-pointer entry matches `rw_kernel_data()`
  attributes — pairs with task 143's D18 fix.
- D8.6: `Entry::is_table` at `Level3` is always false (`vm.rs:214-216`).

## Done when

- No table carries a stale self-pointer (D2.1/D2.3/D2.4 pass).
- The trigger image maps `KZERO + 0x3fe00000` cleanly (D2.5 post-fix).
- The doc sites carry the dependency note.
- Full `cargo xtask ci` green. Metal end-to-end (only if a firmware maps
  the top of the first GiB to the guest) belongs to task 127's session.

Origin: pre-`main9` boot audit, 2026-08-30
(`plans/premain-boot-review-2026-08.md`, B2, checklist D2).
