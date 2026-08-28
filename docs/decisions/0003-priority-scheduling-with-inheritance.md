---
status: accepted
---

# 0003 — Priority scheduling with priority inheritance

- **Status**: accepted — implemented (`aarch64/src/process.rs`, the `IpcScheduler` trait in `port/src/ipc.rs`)
- **Date**: 2026-08-28 (record written from `tasks/plans/microkernel-substrate.md`)
- **Context**: `tasks/plans/microkernel-substrate.md`; priority levels at `aarch64/src/process.rs:130`, the inheritance hook at `port/src/ipc.rs:82`

## Decision

The scheduler is priority-based over QNX's 256-level range (0 most urgent),
with priority inheritance on IPC: a sender blocked on a lower-priority
receiver boosts that receiver to the sender's level until the reply. This was
a prerequisite to IPC, not a follow-on.

## Why

Round-robin cannot bound priority inversion, so "deterministic IPC" on a
fair-share scheduler is a false claim. Priority inheritance is the reason
send/receive/reply exists in the QNX model, and without it the substrate's
determinism test cannot pass. The work was forced by the property, not
preferred on taste.

## Alternatives rejected

- **Keep round-robin and add IPC on top.** Lost: unbounded inversion, and a
  determinism claim the system could not honour.

**Dissent** (kernel-taste lens): don't change a scheduler that works;
round-robin is simpler. Accepted as a real cost, overridden because the
inheritance is load-bearing rather than speculative.

## Consequences

- `port::ipc` depends on an `IpcScheduler` trait (`priority`, `boost`,
  `unboost`) rather than on any concrete scheduler — the seam that keeps IPC
  arch-neutral.
- Every new blocking path must state what happens to the blocked party's
  priority; a path that cannot answer is a defect.
- IRQ delivery deliberately does *not* inherit — see
  [0008](0008-irq-to-message-routing.md).
