---
status: done
---

# r9-syscall-proc-control: wait/reap + kill — completing the process model (Tier 3.1)

Task 5 of 7 in the r9x arc. Plan:
[plans/r9x-target-std-backend.md](plans/r9x-target-std-backend.md).
Needs Task 3 (`sys_spawn` produces children to reap) and Task 4 (reaping
should be able to wait with a deadline). Rationale: the process manager (init,
stage 7) must *manage* — detect a dead server and restart it. Today a process
can exit but no one can ask "did my child finish, and with what status." This
completes the process model: spawn (Task 3) + wait/kill (here).

## Goal

Add process-control services so a parent can learn a child's exit status
(reap) and a process can be terminated (kill), and expose them in `r9x_std`.
The process manager becomes real: it reaps children, sees their status, and
restarts the dead ones.

Standing constraints: warning-free for all three arches; a `wait` blocks the
parent (woken by the child's exit or a deadline, per Task 4); reaping is what
frees a `Process` slot (a child that exits but is never reaped holds its slot
— the `NPROCS` table must not leak slots); `kill` is bounded (it marks the
target for termination on the next switch; interrupt context only sets a
flag).

## Changes

- **Kernel — a death record:** when a process exits or faults, record its exit
  status in the `Process` slot and mark it Zombie (a new state beside
  Runnable/Running/Blocked). The slot is not reusable until reaped. This is the
  data-structure change that makes "who owns a dead slot" a fact, not an
  assumption (kernel-taste: an edge-case branch is a data-representation smell
  — a zombie state removes the "is this slot free or just-dead" branch).
- **Kernel — `SYS_WAIT`** (arch `process.rs` + `trap.rs`): x0 = a child id (or
  0 = any child), x1 = deadline (0 = block forever; a value, per Task 4).
  Reaps a matching zombie: on return x0 = the reaped child id, x1 = its exit
  status; a timeout returns a stated opcode. Reaping frees the slot.
- **Kernel — `SYS_KILL`** (x0 = a process id): mark the target for
  termination (its status set to a kill code; it dies on the next switch or
  immediately if not Running). A kill of a non-existent id is an error.
- **`r9x_abi`:** add `SYS_WAIT`, `SYS_KILL`, the Zombie state's exit-status
  conventions, the timeout opcode — covered by the pinning test.
- **`r9x_std` (target repo):** `r9x_std::process::ProcessId::wait(deadline) ->
  Result<ExitStatus>` and `ProcessId::kill()`. A `Process` handle type that
  owns a `ProcessId` and reaps on drop (a stated convenience; the manual
  `wait` is the primitive).

## Tests

- **Host unit tests:** the slot lifecycle (spawn → run → exit → zombie → reap
  → slot reusable), the "any child" match, the timeout, a kill of a bad id.
- **New aarch64 integration image** `procctl`: init spawns a child that exits
  with a known status; the parent `SYS_WAIT`s and asserts the status matches
  (the assertion a host test cannot make); the slot is then reusable (a second
  spawn succeeds); a `SYS_KILL` of a running child yields the kill status on
  the next `wait`.
- **No slot leak:** spawn N children, let all exit, reap all, then spawn N
  more — the table does not exhaust (this is the property a missing reap
  would break).
- The pinning test covers the new syscalls and status conventions.

## Acceptance

- `cargo xtask ci` green (all arches; the `procctl` image passes).
- A parent learns a child's exact exit status.
- Reaping frees slots; the table does not leak under spawn/exit/reap cycles.
- `kill` terminates a process; a bad id is an error, not a fault.
- init (the process manager) can spawn, reap, and restart a dead server.

## Not in scope

Signals beyond a single kill (no signal numbers, no per-process signal mask —
a kill is a termination, Plan 9/QNX shape; richer signals are a large
refinement with no current user). `waitpid` options (`WNOHANG` is the
deadline=0 fast path; a full option set is a refinement). Reparenting an
orphaned child to init — for now a child whose parent is reaped is reaped by
init (a stated rule); general reparenting is a refinement. Per-process exit
*code* granularity beyond a small status set (the status set is the eight
exit-range values + FAULT + kill; more codes are a refinement).
