---
id: 112
status: open
wave: 3
---

# Task 112: sys_kill does not stop its target, and exit_current picks the wrong slot

## Status: open — wave 3

## Problem

**`sys_kill` marks a Running process Exited without stopping it
(`aarch64/src/process.rs:1816`).**  Nothing rejects `pid == self`, and
the syscall returns to EL0 normally.  The doc at `:1798-1801` claims a
Running target "will not be re-selected", which is true and insufficient
— it is still running *now*.  Two consequences:

- *Slot reuse under a live process.*  `try_install` (`:945-948`) treats
  any `state == Exited` slot as free.  After a self-kill the process
  keeps running and its next `SYS_SPAWN` picks its own slot;
  `forkret_context(id)` (`:1001`) writes the canary and
  `write_bytes(0, FRAME_SZ + CONTEXT_SZ)` over the top 416 bytes of that
  kstack — which is the live trap frame of the in-flight syscall.
  `trapret` then erets from a zeroed frame.  `sys_wait`'s
  `table[idx] = None` (`:1791`) has the same exposure, and so do the
  `exit_current`/`fault` paths, which mark `Exited` at `:1220`/`:1437`
  and only then run `close_all_for` and `resched`.
- *Panic on the follow-up exit.*  When the self-killed process later
  calls `SYSEXIT`, `exit_current` finds no `Running` slot and hits
  `panic!("exit_current: no Running process in the table")` (`:1216`).

**`exit_current` matches the first Running slot (`:1214`).**  `current`
is read from TPIDR at `:1200` and null-checked, then discarded; the
lookup is `find(|slot| p.state == Running)`.  `fault()` (`:1417-1428`)
does it correctly by pointer-matching TPIDR.  Under SMP with two
processes Running on two cores, an exit on one core marks the other
core's process `Exited`, reports the wrong id, and closes the wrong
channels via `close_all_for`.

## Design

- `sys_kill` on a Running target must stop it, not just label it.  For
  `pid == self` that is the existing exit path; for another core's
  process it needs a cross-core reschedule IPI, which is task 124's
  machinery — so land the self-kill rejection now and the cross-core case
  with 124.
- `exit_current` pointer-matches TPIDR, exactly as `fault()` does.
  Factor the lookup into one helper both call, so the third caller cannot
  get it wrong.
- Slot reclamation must not reuse a slot whose process is still on a
  core.  Once threads exist (task 125) this becomes a refcount; for now,
  a "not the current process on any core" check.
- While here: `sys_setprio` (`:1866`) validates the `u64` against 255 but
  then does `Priority::new(prio as u8)`, so `sys_setprio(id, 511)`
  returns 0 and installs the never-scheduled idle sentinel.  `sys_spawn`
  gets this right at `:879` by checking the `u64`.  And `:1854`'s
  `.unwrap_or(0)` turns "TPIDR matches no slot" into "change process 0's
  priority", where every other TPIDR lookup in the file panics.

## Tests

- Integration: a process kills itself, then spawns; the spawn must not
  land in its own slot and the parent must see a sane exit status.
- Integration: `sys_setprio(id, 511)` is refused.
- Host: the TPIDR→slot helper has a unit test for the no-match case.

## Done when

- A killed process stops; a dead slot is never reused under a live
  process.
- `exit_current` and `fault` share one slot-identity path.
- Full `cargo xtask ci` green.

Origin: architecture and correctness review of `f76d96a`, 2026-08-28.
