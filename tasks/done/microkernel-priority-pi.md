---
status: done
---

# microkernel-priority-pi: a priority scheduler with priority inheritance

Task 1 of 7 in the microkernel-substrate arc. Plan:
[plans/microkernel-substrate.md](plans/microkernel-substrate.md). Lands after the
tier-1 SMP correctness tasks; lands before microkernel-ipc-core.

## Goal

Extend the aarch64 scheduler from round-robin to **priority-based with priority
inheritance (PI)**, so the kernel can actually *keep* the determinism claim that
IPC is built on. Round-robin cannot bound priority inversion: a high-priority
thread waiting on a resource held by a low-priority one waits as long as the
low-priority thread runs. PI fixes that by *temporarily boosting* the holder to
the waiter's priority. Without this, `port::ipc` (stage 2) is a monolith with RPC
and no latency guarantee — which is exactly the thing the substrate exists to
not be.

This task builds the **capability**: a process has an *effective* priority that
can be raised and later restored, and the ready selection is by highest
priority (round-robin *within* a priority class, so existing fairness is nested,
not deleted). The *triggering* of PI (on a blocking send) is stage 2; here we
prove the boost/restore and the priority-ordered selection stand on their own.

## Changes

All in `aarch64/` for the reference implementation. x86-64/riscv64 schedulers
are gate-green only (they keep their current scheduling; this arc ports per-arch
later, aarch64 is the source of truth — same policy as the preemption arc).

- `src/process.rs`:
  - A `priority` per proc-table slot. A small fixed set, not an unbounded int —
    an `enum Priority` (or a `u8` with a named const range) so illegal values
    are unrepresentable and the compare is trivial. Give the existing kernel
    process and the timer a stated priority; new user processes start at a
    defined default. (No `nice`/renice from userspace — that is not this task.)
  - Ready selection: replace "scan from the cursor for *a* Runnable other than
    the current" with "find the highest-priority Runnable; among those tied,
    round-robin from the cursor." One pass to find the max, then the existing
    cursor walk restricted to that class. The cursor and the round-robin
    fairness stay — they are *nested under* priority, not replaced.
  - **PI boost/restore**: `fn boost(slot, to: Priority)` raises a process's
    *effective* priority and remembers its original; `fn unboost(slot)` restores
    it. A process's slot carries `base` and `effective` (or a single `effective`
    plus a remembered `base` while boosted). These are the hooks stage 2 calls.
    Document the invariant: a process is boosted at most once (no stacking) —
    boosting an already-boosted process is a no-op or an assertion, stated
    explicitly.
  - Every priority read/write goes under the existing table lock; the
    `swtch`-written `context` field stays the only lock-free write (the aliasing
    discipline in the module header is unchanged).

## Tests

- **Host unit tests** (the existing `#[cfg(test)]` style in the module, as
  `bitmapalloc.rs` does): selection picks the highest-priority Runnable; two
  processes in the same class round-robin; `boost`/`unboost` raise and restore
  exactly; a boosted process sorts above its base-priority peers and drops back
  on restore; boosting an already-boosted process is the stated no-op/assert.
- **Integration image** (aarch64, `harness = false`, `required-features =
  ["qemu-test"]`, `[[test]]` entry in `aarch64/Cargo.toml`): two user processes
  at different priorities both runnable; assert the high-priority one is
  scheduled before the low-priority one over a sequence of ticks (the
  preemption counters already in `process.rs` make this observable from the
  kernel; the image exits with a status the kernel checks). The boost path is
  exercised by a kernel-side call to `boost`/`unboost` on the low-priority
  process and a re-check of ordering — stage 2 will replace that manual call
  with a real blocking send.

## Acceptance

- `cargo xtask ci` green across all three arches (x86-64/riscv64 untouched
  beyond compiling).
- The new image passes: `cargo xtask qemu --arch aarch64 --image <prio> --timeout 60`.
- The selection change is observable: remove the "highest priority" pass and the
  image's ordering assertion fails — the test is load-bearing, not decorative.

## Not in scope

- `port::ipc`, channels, and the *triggering* of PI on a blocking send — stage
  2. Here PI is a capability exercised by a manual boost.
- x86-64/riscv64 scheduler ports — gate-green only, ported when this arc reaches
  them (aarch64 is the reference).
- Tick-quantum / timeslice changes, `nice`/renice from userspace, and any
  real-time (deadline) scheduling — priority + PI is the scope; nothing above it.
- SMP: this task assumes the tier-1 SMP correctness tasks are done. If they are
  not, this task is **blocked on them**, not ahead of them.

## Outcome

Done. aarch64 is the reference; x86-64/riscv64 are gate-green only.

What landed in `aarch64/src/process.rs` (+ the `prio` image and its
`[[test]]` entry):

- `Priority` is a two-level `enum` (`User` < `Kernel`), `PartialOrd`/`Ord`
  derived, so "higher runs first" is the enum order and illegal values are
  unrepresentable. A new `spawn` starts at `DEFAULT_PRIORITY` (`User`).
  `KERNEL_PRIORITY`/`BOOST_PRIORITY` were *not* added: a fixed constant is the
  wrong shape, because stage 2 boosts to the *waiter's* priority (a parameter),
  not a fixed level — the concept lives in the `Priority::Kernel` doc instead.
- `PriorityState { base, effective }` per slot. `is_boosted()` is
  `effective > base` (not `!=`), so a call that sets `effective` below `base`
  can never be misreported as boosted; `boost`'s contract is "to a priority at
  or above base." No-stacking (boost-of-boosted is a no-op) is enforced and
  tested.
- `pick_next(&[(State, Priority)], current, cursor) -> Option<usize>` is the
  host-testable pure selection: one pass for the highest effective priority
  among the Runnable (not the current), then the existing cursor walk
  restricted to that class. `resched` builds the per-slot `(state, effective)`
  view and calls it; the cursor and round-robin fairness are nested under
  priority, not replaced. `run_all`'s initial pick is unchanged (position-based).
  The `slots` array is rebuilt per resched — O(NPROCS) in a per-switch path —
  and is the price of keeping `pick_next` a pure function; accepted (resched is
  per-switch, not per-interrupt).
- `boost(id, to)`, `unboost(id)`, `effective_priority(id)`: the PI capability.
  All take the table lock; the `context` field stays the only lock-free write.
  `run_order()` + the `RUN_ORDER` buffer are qemu-test-only observability so
  the image can assert *order* (the preemption counters count switches, not
  order). The hot-path `record_run` call sites are unconditional, but the
  function compiles out to nothing outside qemu-test builds, so production
  codegen is unchanged.
- Seven host unit tests cover the pure selection and `PriorityState` (priority
  wins; same-class round-robin; boost raises/restore; boosted sorts above peers
  and drops back; re-boost is a no-op; current never reselected; a Running slot
  is not selectable).

Load-bearing proof (the task's acceptance): the `prio` image runs three
processes — L, M at `User`, H boosted to `Kernel` — and asserts H is switched
in before M. Real run_order is `[0, 2, 0, 2, 0, 2, 0, 2, 1]` (L, H, …, M last).
Temporarily replacing `pick_next` with a pure round-robin gives
`[0, 1, 2, …]` (M before H) and the image **fails** — so the assertion is
load-bearing, not decorative. (The switches here are yield-driven; the
preemption counters only count tick-driven switches, so the assertion is on
the order trace, not on `preemptions()`.)

Gates: clippy 0 ×3, dist OK ×3, 31 aarch64 host tests (24 prior + 7), all 11
aarch64 integration images pass.
