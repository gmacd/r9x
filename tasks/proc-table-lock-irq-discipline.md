---
id: 109
status: open
wave: 0
---

# Task 109: the tick takes TABLE in interrupt context while syscalls hold it with IRQs unmasked

## Status: open — wave 0

## Problem

The invariant is already written down, at `aarch64/src/process.rs:24-27`:

> Kernel-side lock holders run only when TPIDR is null, and `resched`
> checks TPIDR before taking the lock: the moment any syscall touches the
> table from a running process, that invariant dies and acquisition must
> move under a DAIF mask.

Both halves of it are now false.

`irq_resched` (`:481`) calls `check_deadlines()` — which does
`TABLE.lock` at `:450` — *before* the `cur.is_null()` check at `:484`.
The comment above it says the scan is deliberately not gated on TPIDR,
so this is intentional and the invariant was not revisited.

Meanwhile `status()` (`:1110`), `any_exited()` (`:1127`),
`set_priority()` (`:1138`), `boost()` (`:1157`), `unboost()` (`:1169`),
`effective_priority()` (`:1182`) and `try_install()` (`:944`, via
`spawn`) all take `TABLE.lock` with **no `IrqGuard`**, and
`boot::interrupts()` (`boot.rs:66`) unmasks IRQs long before any of them
run.

Concrete: `channel_close.rs:197-200` and `namespace.rs:110-149` call
`run_all()`, then `process::status(...)` / `spawn(...)` again.  By then
`TICK_TIMER` is running (started at `:1078`, never cancelled).  A tick
landing inside `any_exited()`'s table scan enters the EL1 IRQ vector,
reaches `check_deadlines`, and the MCS node enqueues *behind this core's
own live guard* — spinning forever with IRQs masked, so the outer guard
is never dropped.  Narrow window, total and undiagnosable failure.

`timer::with_timers` (`timer.rs:123-128`) gets this right with an
`IrqGuard`, which is the local precedent.

## Design

Two candidate fixes; pick one and write down which.

- **Guard every holder.**  Add `IrqGuard` to all seven call sites.
  Cheapest, keeps the deadline scan where it is, but relies on nobody
  adding an eighth unguarded holder — a rule the codebase has already
  broken once.
- **Move the deadline scan off the table** (preferred, and the direction
  task 124 wants anyway).  A per-CPU deadline wheel means the tick never
  touches the global table, and the invariant becomes structural rather
  than remembered.

Either way, update the module doc at `:24-27` to state what is actually
true afterwards — a stated invariant the code violates is worse than no
comment, because reviewers trust it.

## Tests

- Integration: an image that spawns with the tick running and takes
  `status()` in a loop, run long enough to sample the window.  Task 124's
  soak image is the natural home.

## Done when

- No path takes `TABLE` in interrupt context that another path can hold
  with IRQs unmasked.
- The module doc matches the code.
- Full `cargo xtask ci` green.

Origin: architecture and correctness review of `f76d96a`, 2026-08-28.
