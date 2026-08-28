---
status: accepted
---

# 0006 — `Aspace` is a page-table root; EL0 faults kill the process

- **Status**: accepted — implemented (`aarch64/src/aspace.rs`)
- **Date**: 2026-08-28 (record written from `tasks/plans/microkernel-aspace.md`)
- **Context**: `tasks/plans/microkernel-aspace.md`

## Decision

A process's address space is a page-table root plus a shallow copy of the
user-visible kernel mappings — not a VMA list. TTBR0 holds only the process's
own text and stack; the kernel stays in TTBR1, unreachable from EL0. Every
EL0 fault kills the faulting process; there is no fix path. The TTBR0 switch
happens in `switch_out`, after TPIDR and before `swtch`, using the all-EL1
TLBI.

## Why

The isolation property needs a root, not a map of regions: on inspection there
are no user-visible kernel mappings to copy, because the kernel's mappings are
all `Priv*` in TTBR1. An empty per-process root is the minimal table that makes
a wild write corrupt only the faulting process. Kill is the total function for
faults — a no-op handler that returns to the faulting instruction is a
livelock, and demand paging is a later arc. Switching in `switch_out` keeps
the EL0 return path decoupled from address-space state.

## Alternatives rejected

- **A VMA list (`mm_struct` shape).** Lost: it is what demand paging and
  teardown need, and this arc has neither; the second user earns the
  abstraction.
- **Copy kernel mappings into each `Aspace`.** Lost on inspection — there are
  none to copy.
- **Switch TTBR0 in the vector tail.** Lost: it couples the single EL0 return
  path to the address space.
- **A per-process TLBI.** Lost: a later optimisation that needs a measurement
  to justify it; the all-EL1 form is correct and matches `vm::switch`.

**Dissent** (kernel-taste lens): the all-EL1 TLBI on every context switch
flushes the neighbours' entries too — a real hot-path cost, accepted as
correct-first, with the per-process form filed as the optimisation a
measurement will decide.

**Dissent** (microkernel-and-firmware lens): the fault should be *delivered*
to the process so a server can catch its own. r9x has no signal machinery, and
a server's death is a clean event the nameserver notices — the Plan 9 shape.

**Dissent** (hardware-truth lens): a process that can reach no device mapping
cannot be a device server. Exactly so — the device capability is a separate
verb, `SYS_MAP_MMIO`, see [0007](0007-device-dumb-kernel.md).

## Consequences

- `aarch64/src/aspace.rs` is the isolation boundary; anything that adds a
  mapping to TTBR0 is a capability decision, not a plumbing change.
- Demand paging, teardown and signals each re-open one clause of this record.
- Fault diagnosis leans on the backtrace path (see
  [0013](0013-elf-symtab-backtrace.md)), since there is no recovery to inspect.
