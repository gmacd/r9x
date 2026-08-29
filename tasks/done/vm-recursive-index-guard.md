---
id: 104
status: done
wave: 0
commit: faf2a77
---

# Task 104: next_mut is missing the recursive-index guard, giving EL0 arbitrary physical write

## Status: done (faf2a77) — wave 0

## Problem

`Table::next_mut` (`aarch64/src/vm.rs:359`) has no `index == 511` check.
Its pre-MMU twin does — `pre_mmu/vminit.rs:418` returns
`PageTableError::MappingRecursiveIndex` for exactly this case — so the
guard was known to be necessary and was lost in the post-MMU copy.

The consequence is a full escape.  Call
`sys_map_mmio(pa, va = 0x0000_ff80_0000_0000, 4096)`:

- `va_index(va, Level0) == 511`, the recursive slot, so `next_mut`
  follows the self-pointer and treats the root as the level-1 table.
- `va1 == va2 == va3 == 0`, so the walk descends the process's real L1
  and L2 tables.
- The leaf is written into the **L2** table's slot 0 — with
  `with_page_or_table(true)` forced on at `vm.rs:471`.  At level 2 that
  bit means *table descriptor*, not page.

The user's chosen physical page is now walked by the MMU as a level-3
page table for VA `[0, 2 MiB)`.  Fill it first (its PA is obtainable from
`SYS_ALLOC_PAGE`) with hand-built `AllRw` descriptors and you have
arbitrary physical read/write from EL0 — without needing the `pa`
validation hole that tasks 99/120 close.

`heap_grow`'s bound is `param::KZERO`, so the heap path can reach the
same region.

## Precedents

Every recursive-mapping kernel treats the self-slot as reserved and
refuses to walk through it; Zircon and seL4 avoid the question entirely
by not using recursive mapping for the user tables.  The local precedent
is stronger than either: the guard already exists twenty lines away in
this repo's own pre-MMU walker.

## Design

- Port the `index == 511` check from `vminit.rs:418` into
  `Table::next_mut`, returning the same `MappingRecursiveIndex` error.
- Audit the other consumers of `va_index` for the same assumption:
  `user_leaf_entry` (`vm.rs:617`) walks the same tables and shares the
  aliasing (see task 116).
- Consider making the reserved slot structural rather than checked — a
  `const RECURSIVE_SLOT: usize = 511` with both walkers reading it, so a
  third walker cannot be written without meeting it.

## Tests

- Host: `next_mut` at each level with `index == 511` returns the error.
- Integration: a program calls `SYS_MAP_MMIO` with an L0-index-511 VA and
  is refused; the VM matrix in task 91 gains a row for it.

## Done when

- The guard is present and unit-tested; the escape is refused at the
  syscall.
- Full `cargo xtask ci` green.

## Outcome

The guard is **level-0 only**, not the blanket `index == 511` the plan
assumed.  The two walkers build their tables differently: the pre-MMU sets
entry 511 of *every* table it allocates (so its blanket check is correct),
but the post-MMU sets it in the *root* only — `next_mut` allocates every
other table `clear()`ed — so a blanket post-MMU guard would refuse
legitimate VAs whose L1 or L2 index is 511.  `RECURSIVE_SLOT` is shared by
both walkers and `recursive_table_addr`.  Both halves are unit-tested: the
recursive VA is refused (`map_to_refuses_the_recursive_slot`) and an
L2-index-511 VA still maps (`map_to_allows_index_511_below_level0`).  The
`SYS_MAP_MMIO` integration test is filed as task 134.  See the lesson in
`docs/lessons.md`.

Origin: architecture and correctness review of `f76d96a`, 2026-08-28.
