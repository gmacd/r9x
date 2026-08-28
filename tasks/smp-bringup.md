---
id: 124
status: open
wave: 1
---

# Task 124: bring the secondaries up, then make the scheduler per-CPU

## Status: open — wave 1 (bring-up) and wave 2 (restructure)

## Problem

`AGENTS.md` states the SMP charter — "The project is designed for
multi-core SMP.  All concurrency primitives, initialization patterns, and
shared data structures must be correct under multi-core execution — never
assume single-core."  The secondaries are parked at `aarch64/src/l.S:53-55`
and the scheduler state is global: one `STARTER_CTX` (`process.rs:328`,
whose own comment says "task #4 makes it per-core"), one `NEED_RESCHED`
(`:394`), one `CURSOR` (`:293`), and a single `TABLE` lock taken by every
syscall that touches a process.

**Multi-core was ruled imminent during the 2026-08-28 review**, so the
races the review found are live defects, not deferrals: tasks 117, 110,
113, 109, 108, 105 and 107's neighbours all become reachable the moment a
second core runs.

The single `TABLE` lock is both the scalability ceiling and the direct
cause of task 109's interrupt-context deadlock.  `NEED_RESCHED` has a
concrete consequence too: each core has its own CVAL
(`timer.rs:150-159`), so all four take the timer PPI, exactly one wins
the `swap(false)` at `:478`, and on a 4-core boot only one core ever
preempts per tick.

## Design, in two parts

**Part 1 — bring-up first, before the fixes are finished.**  This is the
deliberate inversion and the highest value-to-effort item on the backlog.
Concurrent code cannot be validated on one core: every race in the
review is currently invisible to the entire test suite, which is exactly
how a project with an explicit SMP charter accumulated a dozen of them.

- Unpark the secondaries in `l.S`.
- An integration image running a known-parallel workload across cores.
- An SMP soak image hammering IPC, spawn and heap growth from every core,
  wired into `xtask ci`.  A race that survives one pass will not survive
  ten thousand.

Expect this to fail loudly and immediately.  That is the deliverable.
Task 97's loom/miri host tests are the companion for the pieces a soak
cannot reach.

**Part 2 — per-CPU restructure.**

- A per-CPU block reached through `TPIDR_EL1`, holding `STARTER_CTX`,
  `NEED_RESCHED`, the run queue and the current thread.
- Per-CPU deadline wheel, so `check_deadlines` stops taking the global
  table lock in interrupt context (task 109's structural fix).
- Separate the table lock (slot allocation, teardown) from run-queue
  locks (scheduling).  Establish and **document** a lock order — the
  current one is implicit and already violated once.
- Fix `switch_out`'s context-save ordering (task 117).
- Cross-core reschedule IPI, which task 112 also needs.
- `run_all` (`:1084`) picks the first `Runnable` slot ignoring priority,
  diverging from `pick_next`; fold in.

## Tests

- The parallel-workload and soak images above, in CI.
- Task 91's VM matrix gains its SMP rows for real rather than in theory.

## Done when

- The secondaries run, the soak image is in `xtask ci`, and it is green.
- No scheduler state is global that should be per-CPU.
- The lock order is written down in `AGENTS.md` and the module docs.
- Full `cargo xtask ci` green.

Origin: architecture and correctness review of `f76d96a`, 2026-08-28.
Ruling recorded in plans/architecture-review-2026-08.md.
