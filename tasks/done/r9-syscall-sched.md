---
status: done
---

# r9-syscall-sched: priority + inheritance — real-time control (Tier 3.2)

Task 6 of 7 in the r9x arc. Plan:
[plans/r9x-target-std-backend.md](../plans/r9x-target-std-backend.md).
Needs Tasks 3–5 (you set a priority on processes you can spawn, wait on, and
kill). Rationale (the QNX doctrine, `microkernel-and-firmware.md`):
determinism is the product, and priority inheritance bounds priority
inversion. Today the scheduler has priorities (the `prio` image exercises
them) but a process cannot *set* its own or a child's priority, and there is
no inheritance — so the display server cannot raise its priority to beat a
burst of other work, and a low-priority process holding a resource a
high-priority process needs inverts the order. This is what makes the 60 Hz
goal *guaranteed*, not just usually-met.

## Goal

Add scheduling-control services so a process can set priorities and the
scheduler applies priority inheritance, and expose the basics in `r9x_std`.
The display server (and the input path) can hold a high priority; a blocked
high-priority process does not starve behind a low-priority resource holder.

Standing constraints: warning-free for all three arches; a priority change is
a syscall (may resched, not interrupt context); inheritance is applied in the
scheduler's existing wake/block path (no new lock on the hot path beyond what
`mcslock` already provides); the interrupt context budget is untouched (a tick
still does lookup/enqueue/wake).

## Changes

- **Kernel — `SYS_SETPRIO`** (arch `process.rs` + `trap.rs`): x0 = a process
  id (or 0 = self), x1 = priority. Validates the range (the existing priority
  set), updates the target, and rescheds if the target is the current or a
  blocked process whose order changed. A bad id/range is an error.
- **Kernel — priority inheritance:** the kernel already tracks, per process,
  the resources it blocks on (a channel a receiver waits for). Extend the
  block/wake path: when a high-priority process blocks on a resource held by a
  lower-priority process, the holder temporarily runs at the waiter's
  priority; on release, it reverts to its base priority. The base priority is
  stored separately from the effective (inherited) priority — a two-field
  fact, not a mutable single value (kernel-taste: the special case "revert to
  base" disappears when base and effective are distinct fields).
- **`r9x_abi`:** add `SYS_SETPRIO` and the priority range — covered by the
  pinning test.
- **`r9x_std` (target repo):** `r9x_std::process::set_priority(Priority)` and a
  `Priority` type (the existing set, a newtype over the kernel's values).

## Tests

- **Host unit tests:** the range validation, the base/effective two-field
  revert, an inheritance chain (A high blocks on B low, B on C low → B and C
  run at A's priority until A unblocks).
- **New aarch64 integration image** `sched`: a low-priority process holds a
  channel a high-priority process is about to receive on; assert the low
  process runs at the high priority for the duration (the holder is not
  starved), and that it reverts on release. A `set_priority` to a bad value is
  an error.
- **No inversion on the vblank path:** the display-server-shaped loop (high
  priority, `receive_at` a vblank) is not delayed by a low-priority process —
  the `prio` image extended with inheritance.

## Acceptance

- `cargo xtask ci` green (all arches; the `sched` image passes).
- A process can set its own and a child's priority.
- Inheritance is applied and reverted correctly (the `sched` image proves the
  holder runs at the waiter's priority and reverts).
- The display-server pacing (Task 4) is not delayed by lower-priority work.

## Not in scope

A full POSIX priority *class* (real-time vs. normal bands) — a single priority
set with inheritance is the current need; classes are a refinement. CPU
*quota*/throttling (time-slicing limits) — inheritance bounds inversion;
quota is a throughput policy, a separate concern. Per-core affinity/pinning —
the scheduler is per-core (AGENTS.md multi-core); affinity is a refinement once
SMP bringup lands. Dynamic priority *aging* (to prevent starvation of
low-priority work) — a refinement; the current set is small enough that
inheritance is the load-bearing mechanism.
