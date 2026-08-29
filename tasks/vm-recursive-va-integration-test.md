---
id: 134
status: open
---

# Task 134: exercise the recursive-slot guard at the syscall (task 104's integration half)

## Status: open

## Problem

Task 104 added the recursive-slot guard to `Table::next_mut` and unit-tested
it: `map_to_refuses_the_recursive_slot` refuses the recursive VA, and
`map_to_allows_index_511_below_level0` pins the L1/L2 case.  But the escape
is proven at the `map_to` level, not at the syscall — no integration image
calls `SYS_MAP_MMIO` with an L0-index-511 VA and checks that it is refused
with an error return rather than a kernel abort.

The guard is on the path (`trap.rs` -> `ipc::sys_map_mmio` ->
`aspace.map_mmio` -> `map_phys_range` -> `map_to`), so the unit test is a
strong proof.  The end-to-end refusal on real hardware is what remains.  This
is the half of task 104's Tests section that was deferred: "a program calls
`SYS_MAP_MMIO` with an L0-index-511 VA and is refused; the VM matrix in task
91 gains a row for it."  It is a separate piece of work — a new image (file +
`[[test]]` entry + an inline-asm `svc` for `SYS_MAP_MMIO`) — not a small
addition to the guard.

## Fix direction

- An integration image that runs a user program calling
  `SYS_MAP_MMIO(pa, 0x0000_ff80_0000_0000, 4096)` and asserts the return is
  the error (not a successful mapping, and no kernel data abort).
- The task-91 VM matrix gains a row for the recursive-VA refusal.

## Done when

- The integration image calls `SYS_MAP_MMIO` with an L0-index-511 VA, is
  refused, and the VM matrix has the row.
- Full `cargo xtask ci` green.

Origin: task 104 (`vm-recursive-index-guard`), whose guard landed with the
unit test and deferred the integration half here.
