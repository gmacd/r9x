---
id: 116
status: open
wave: 3
---

# Task 116: user_range_ok validates a VA that aliases a different one

## Status: open — wave 3

## Problem

`user_range_ok` (`aarch64/src/vm.rs:582`) bounds the user half at
`KZERO` (2^64 − 2^47).  The actual TTBR0 limit is 2^48 — `T0SZ = 16`
(`pre_mmu/vminit.rs:305`).  The gap `[2^48, KZERO)` is accepted by the
bounds check and is not translatable.

Worse, the walk itself masks the high bits away: `va_index` uses only
bits 12..48, and `recursive_table_addr` masks with
`0x0000_ffff_ffff_f000`.  So `user_range_ok(0x0001_0000_0000_1000, 8,
false)` walks and validates the leaf for VA `0x1000` — a completely
different address — and returns true.

`read_user` (`aarch64/src/ipc.rs:258-264`) then does
`copy_nonoverlapping` through the un-translatable VA, taking a level-0
translation fault at EL1.  That is a user-triggerable kernel data abort,
reachable from `SYSSEND`, `SYSRECEIVE` and `SYS_SPAWN` — every syscall
that names a user buffer.

The same masking is shared by `user_leaf_entry` (`:617`), so task 104's
recursive-index hole and this one are two views of one missing
canonical-address check.

## Design

- Bound at the translation limit, not `KZERO`: reject any VA with bits
  ≥ 48 set before walking.  Derive the constant from `T0SZ` rather than
  writing 48 twice, so a `T0SZ` change cannot desynchronise them.
- Add the same check to `user_leaf_entry`, or route both through one
  canonicalisation helper that returns `None` for a non-canonical VA.
- Note for task 125: the validate-then-copy window is safe today only
  because one thread owns each address space, so nothing can unmap
  between the check and the copy.  Threads break that assumption; design
  the fix (revalidate-under-lock, or copy with a fault-tolerant
  accessor) when threads land, not after.

## Tests

- Host: `user_range_ok` rejects `0x0001_0000_0000_1000` while accepting
  `0x1000` with the same page mapped.
- Integration: a process passes a high non-canonical buffer to
  `SYSSEND`; the syscall returns `ERR_BAD_VA` and the kernel survives.

## Done when

- No VA outside the TTBR0 range validates.
- The aliasing test exists and fails before the change.
- Full `cargo xtask ci` green.

Origin: architecture and correctness review of `f76d96a`, 2026-08-28.
