---
id: 117
status: open
wave: 2
---

# Task 117: switch_out publishes the outgoing process as Runnable before saving its context

## Status: open — wave 2 (SMP)

## Problem

`switch_out` (`aarch64/src/process.rs:1315`) marks the outgoing process
`Runnable` and drops the table lock at `:1354`, but `swtch` does not save
its context until `:1374`.

In that window the process is advertised as selectable while its
`context` field still holds the *previous* switch-out's saved value, and
while this core is still executing on its kstack.  Another core in
`switch_out` can `pick_next` it, mark it `Running`, `tpidr_set` it, and
`swtch` into that stale context — two cores executing the same process on
the same 64 KiB kstack.  Core 0's `swtch(&mut (*cur).context, ...)` then
overwrites the pointer core 1 is running from.

Not reachable while the secondaries are parked; reachable the moment task
124 lands, which is why it is filed at wave 2 rather than deferred.

## Design

- The demotion must not become visible before the context is saved.
  Either hold the table lock across the save, or introduce an
  intermediate state (`Switching`) that `pick_next` skips and that the
  save transitions out of.  The intermediate state is preferable: holding
  the table lock across `swtch` is exactly what the module doc at
  `:20-23` forbids, and for good reason.
- Audit the neighbouring paths for the same shape: `block_current`
  (`:1479`), `exit_current` (`:1214`) and `fault` (`:1417`) all mutate
  state, drop the lock, then switch.

## Tests

- Integration: task 124's soak image, with two cores repeatedly yielding
  between the same pair of processes — the natural detector is a kstack
  canary check on entry to `swtch`.

## Done when

- No process is selectable between its demotion and its context save.
- The soak image runs clean.
- Full `cargo xtask ci` green.

Origin: architecture and correctness review of `f76d96a`, 2026-08-28.
