---
status: done
---

# aspace-fault — the EL0 fault path kills the faulting process

## Problem

There is no fault handler: a data/instruction abort taken while EL0 runs
prints the frame and spins (`trap.rs`'s `_ =>` arm), so a wild write from one
process hangs the machine instead of killing only that process. The isolation
property (a fault in one server faults only that `Aspace`) is not yet real at
the fault level.

## Evidence

- `trap.rs` `trap`: the `match frame.esr_el1.exception_class_enum()` handles
  `SvcAarch64` and falls through to a print-and-spin for everything else
  (data abort, instruction abort). `FAR_EL1`/`ESR_EL1` are in the frame
  (`far_el1`, `esr_el1`) but unused.
- `process.rs`: `exit_current` is the kill path (marks `Exited`, reschedules);
  there is no fault-status distinct from a clean exit status.
- `aspace-switch` (preceding task) makes the process's TTBR0 live, so a fault
  now walks the process's *own* tables — the prerequisite for "fault only that
  Aspace."

## Fix direction

- `trap.rs`: in the `SyncInvalidEL0_64` path, a data/instruction abort
  (`ESR_EL1.EC` 0x24/0x25) with a current process (TPIDR non-null) calls a new
  `process::fault(far, esr)`; a fault with no current process is a kernel
  fault — print and spin (the existing unknown-exception path).
- `process.rs`: `pub(crate) fn fault(far: u64, esr: EsrEl1) -> !` — print the
  FAR/ESR and the faulting process's id, mark the process `Exited` with a fault
  status (a distinct constant, e.g. `FAULT_STATUS = 0xff`, so an image can tell
  a fault-death from a clean exit), and reschedule (the same path
  `exit_current` takes). This arc has no demand-paging, so *every* EL0 fault is
  a kill (the fix path is a later change to this one function).

## Done-when

- The `aspace` integration image (or a new `aspace_fault` image) spawns a
  faulting process (one that writes to an unmapped VA) and a peer; the faulting
  process dies with `FAULT_STATUS`, the peer (and the kernel) survive, and the
  image asserts both. The isolation proof at the fault level.
- `cargo xtask ci` is green, including the new image.

## Origin

Stage 3 of `tasks/plans/microkernel-substrate.md`, decomposed in
`tasks/plans/microkernel-aspace.md`. Third of the three sequenced tasks
(struct → switch → fault). Stands on aspace-switch. Stage 4 (IRQ→message) is
independent of this task and can interleave with aspace-switch.
