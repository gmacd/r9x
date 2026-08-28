---
status: done
---

# aspace-switch — install the process's TTBR0 on the switch path

## Problem

`Aspace` exists (aspace-struct) but every process still shares
`USER_PAGETABLE`: `spawn` maps text/stack into the shared table and the switch
path never touches TTBR0, so a process's table is never live. The isolation
property is not yet real — a wild write still corrupts the shared table.

## Evidence

- `process.rs` `spawn`: maps text/stack into `vm::user_pagetable()` (the shared
  table). The process's own `Aspace` (from aspace-struct) is built but unused.
- `process.rs` `switch_out`: sets TPIDR and calls `swtch`; it never writes
  TTBR0. `vm::switch` (the TTBR0/TTBR1 installer with the `TLBI/DSB/ISB`)
  exists but is only called at boot (`main9`).
- `main9`: the no-process TTBR0 is the shared `USER_PAGETABLE` (switched in at
  boot).

## Fix direction

- `spawn`: build the process's `Aspace` (aspace-struct) and map the process's
  text/stack into it via `map_user_page` (not the shared `USER_PAGETABLE`).
- `switch_out`: after `tpidr_set(next)` and before `swtch`, install `next`'s
  TTBR0 (`vm::switch(next.aspace.ttbr0(), User)` or the inline `msr
  ttbr0_el1` + `tlbi vmalle1is` + `dsb ish` + `isb`). The order is TPIDR →
  TTBR0 → `swtch` (the table must be live before the process's first EL0
  instruction; TPIDR must be set before the fault handler can find the
  process if the first instruction faults).
- The kernel's own TTBR0 (the shared `USER_PAGETABLE`, the no-process case) is
  left as is: when `run_all` returns (no process Runnable), the shared table is
  implicitly restored by the next `switch_out` (or stays live; the kernel runs
  in EL1 with TTBR1, so the no-process TTBR0 is not exercised).

## Done-when

- A new integration image (`aarch64/tests/aspace.rs`) spawns two processes at
  the *same* text/stack VAs but with different data, runs both, and asserts
  each sees its own data (the isolation proof at the scheduling level). The
  image is a `[[test]]` entry with `harness = false` and
  `required-features = ["qemu-test"]`.
- `cargo xtask ci` is green, including the new image (the integration-test
  count goes up by one).

## Origin

Stage 3 of `tasks/plans/microkernel-substrate.md`, decomposed in
`tasks/plans/microkernel-aspace.md`. Second of the three sequenced tasks
(struct → switch → fault). Stands on aspace-struct.
