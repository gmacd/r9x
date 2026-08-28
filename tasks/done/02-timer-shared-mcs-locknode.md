---
status: done
---

# Shared static TIMER_LOCK_NODE breaks the MCS lock

**Severity: blocking**
**Status: done** (verified 2026-07-24)
Originally: **FIXED (uncommitted)** — `TIMER_LOCK_NODE` deleted; all timer-queue
acquisitions go through `with_timer_queue`, which takes a fresh
`LockNode::new()` per acquisition (see task 01 fix).

`aarch64/src/timer.rs:170` declares a single `static TIMER_LOCK_NODE` used for
every acquisition of `TIMER_QUEUE`. An MCS lock requires a distinct `LockNode`
per acquisition context — the node *is* the waiter's queue entry.

If the interrupt handler ever contends with `Timer::start` (the scenario in
task 01), both sides pass the same static node: the handler's `lock()`
overwrites `node.locked` / `node.next` while the holder is enqueued, corrupting
lock state — worse than a clean spin deadlock. It is also wrong for future SMP,
where two CPUs would share one node.

## Fix

Create a `LockNode::new()` on the stack at each acquisition site, as
`aarch64/src/gic.rs` already does (see `gic::try_ack_interrupt`,
`gic::end_interrupt`), and delete the static.
