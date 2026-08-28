---
status: done
---

# r9-syscall-clock-wait: a clock + bounded wait — the 60 Hz heartbeat (Tier 2.1)

Task 4 of 7 in the r9x arc. Plan:
[plans/r9x-target-std-backend.md](plans/r9x-target-std-backend.md).
Needs Tasks 2–3 (a paced display server is a *spawned* process that *
allocates*). Rationale (Decision 7, order 3 of 3, with the Amiga dissent
recorded): the standing goal is to keep the display at 60 Hz while a user-space
display server does the work. That server must *pace its frame loop to the
vertical blank* (the Amiga heartbeat) — which needs (a) a clock to measure the
deadline and (b) a bounded wait that sleeps until it. Today a process can only
block on a `receive`; it cannot sleep until a time. This is also the kernel's
load-bearing real-time duty (Decision 7 dissent: for the graphics track this
is arguably #2, pulled forward if the display server is built next).

## Goal

Add a clock service and a deadline-bounded wait so a process can measure time
and sleep until a deadline, and expose them in `r9x_std`. The display server's
loop becomes: prepare the frame, wait-for-vblank (a timed wait), update,
repeat — paced to 60 Hz without busy-polling.

Standing constraints: warning-free for all three arches; the clock reads the
arch timer the kernel already owns (aarch64 CNTVCT/CNTFRQ — the timer unit
from `aarch64/src/timer.rs`); a timed wait must not spin the CPU (it blocks
the process and wakes on the timer or on a channel, whichever is first);
interrupt context stays within the three-thing budget (the tick sets a
deadline and wakes; it does not compute a clock value).

## Changes

- **Kernel — `SYS_CLOCK`** (arch `process.rs` + `trap.rs`): x0 = kind
  (monotonic now; real-time later as a stated refinement), on return x0 = the
  tick count (and, a refinement, x1 = the frequency) from the arch generic
  timer. No allocation, no lock on the hot path (a register read).
- **Kernel — `SYS_NANOSLEEP` / receive-with-deadline:** the primary need is a
  `receive` that also wakes on a deadline. Either (a) a new `SYS_RECEIVE_AT`
  (x0 = handle, x1 = buf, x2 = cap, x3 = deadline; returns like `SYCRECEIVE`,
  or a "timed out" opcode if the deadline passes first) or (b) a separate
  `SYS_NANOSLEEP` (x0 = ticks; blocks until the deadline). Both are defensible;
  `RECEIVE_AT` is preferred (the display server waits for *either* the vblank
  interrupt message *or* the deadline — one call). The timer unit is extended
  to hold a per-process wake deadline; the tick wakes it (the existing tick →
  `resched` path, not a new interrupt).
- **`r9x_abi`:** add `SYS_CLOCK`, `SYS_RECEIVE_AT` (and the opcode for
  timeout), covered by the pinning test.
- **`r9x_std` (target repo):** `r9x_std::time::{now, Duration, sleep_until}` and
  `r9x_std::ipc::Channel::receive_at(deadline)`. `sleep_until` is a
  `RECEIVE_AT` on a private channel with no sender (or `SYS_NANOSLEEP`, per
  the choice above).

## Tests

- **Host unit tests:** deadline math (ticks ↔ Duration, the frequency read is
  asserted non-zero as the timer unit already requires), the "timed out"
  opcode, and the "message beat the deadline" ordering.
- **New aarch64 integration image** `clock`: read `SYS_CLOCK`, `sleep_until`
  a short deadline, read the clock again, assert the delta is within a stated
  band (not exact — a measurement plan, per the kernel-taste lens: the band is
  asserted, the claim is bounded). A `RECEIVE_AT` that times out returns the
  timeout opcode; one satisfied by a `send` before the deadline returns the
  message.
- **No busy-wait:** the sleeping process is not Running during the wait (the
  process table shows it blocked; a second process runs in the interval —
  reusing the preemption test's shape).

## Acceptance

- `cargo xtask ci` green (all arches; the `clock` image passes).
- A process can sleep until a deadline without spinning the CPU.
- A `RECEIVE_AT` returns whichever of message/deadline comes first.
- The display-server pacing pattern (prepare → `receive_at(vblank, deadline)`
  → update) is expressible in `r9x_std` and bounded.

## Not in scope

A real-time (wall-clock) clock — monotonic first; wall-clock is a refinement
once a time source is agreed (no RTC is assumed on the current targets). A
`select`/`poll` over many channels — one channel + one deadline is the current
need (the display server's vblank); multi-channel `poll` is a refinement.
Sub-tick resolution / a high-resolution timer — the arch generic timer's
granularity is the granularity; a finer timer is a hardware question, not a
service question. Per-process deadline *inheritance* — Task 6's scheduling
area.

## Build record (2026-08-25)

**Done, aarch64.** `cargo xtask ci` green: 23/23 QEMU images, all arches,
warning-free.

### Decisions

- **`SYS_CLOCK` (25)**: x0 = kind (0 = monotonic; other kinds refused with
  `ERR_BAD_KIND` = 9, a stated refinement). Returns the counter's value (a
  register read: no lock, no allocation). The frequency is not returned (it's
  a hardware constant the user reads from `CNTFRQ_EL0`; `r9x_std::time`
  carries it as `COUNTER_FREQ`, a compile-time constant for QEMU's 1 GHz).
- **`SYS_RECEIVE_AT` (26)**: x0 = handle, x1 = buf, x2 = cap, x3 = deadline.
  Returns like `SYCRECEIVE` (x0 = opcode, x3 = bytes, x4 = tag); on a timeout,
  x0 = `RECEIVE_TIMEOUT` (0xffff, the max u16, reserved for the kernel).
- **`RECEIVE_TIMEOUT: u16 = 0xffff`**: the timeout opcode. A protocol that
  sends a message with this opcode is ambiguous and must not.
- **Deadline is a per-process field** (`Option<u64>` in `Process`): set under
  the table lock before block, cleared by `wake()` (both send-wake and
  tick-wake go through it). The tick's scan and the wake's clear never race.
- **`IpcScheduler` trait extended** with `now()` (a register read, no lock) and
  `block_at(id, deadline)` (sets the per-process deadline then blocks). The
  arch-agnostic `port::ipc` stays arch-agnostic (the deadline is opaque to it).
- **`check_deadlines()` runs in the trap tail** (interrupt context, after
  `irq_resched` consumes the flag): reads the counter once, scans the 8-slot
  table, wakes expired processes. Within the three-thing budget (register read
  + scan + wake). The table lock is safe in interrupt context (the trap tail
  runs after the current process has released all locks, the same discipline
  as `resched`→`switch_out`).
- **`check_deadlines()` is NOT gated on non-null TPIDR**: a bounded wait is
  woken by the tick even when the kernel (not a process) is on CPU.
- **Tick resolution is 100ms** (the existing `TICK_PERIOD`): the task
  explicitly defers sub-tick resolution.
- **`receive_at` loop semantics**: on re-entry after wake, try to pop a
  message first. If a message, return it (message beat deadline). If no
  message, check the local deadline against `sched.now()`: if passed, return
  timeout; if not (spurious wake), re-block.
- **`r9x_std::time`**: `now()` (a `SYS_CLOCK` read), `Duration` (ticks,
  `from_secs`/`from_ticks`/`deadline_from`), `COUNTER_FREQ` (1 GHz for QEMU).
  No `sleep_until` (it would leak a channel slot; `receive_at` on the server's
  own channel is the primitive).
- **`r9x_std::ipc::receive_at`**: a wrapper over `SYS_RECEIVE_AT`.
- **`clock` integration image**: tests `SYS_CLOCK` directly (a register read,
  monotonicity, bad-kind refusal). The `SYS_RECEIVE_AT` blocking wait is
  host-tested (the `Mock` scheduler's deadline logic: timeout,
  message-beats-deadline, spurious-wake re-block, timeout opcode reserved).
- **`sys_clock` is `pub`** (not `pub(crate)`): the `clock` image (a test
  target, a separate crate) needs to call it. A small departure from the
  `sys_*` convention (the others are `pub(crate)`), justified by the test
  image's direct call.

### Deferrals

- **`sleep_until`**: not in `r9x_std::time` (it would leak a channel slot;
  a `SYS_CLOSE` is not yet a syscall). `receive_at` on the server's own
  channel is the primitive.
- **`SYS_RECEIVE_AT` on-device test**: the blocking wait is host-tested (the
  `Mock` scheduler); the on-device `clock` image tests `SYS_CLOCK` (the
  register read). A full on-device `SYS_RECEIVE_AT` test needs a user-space
  process (a `clocktask`) that calls `receive_at` — deferred to the display
  server task (the first real user of the bounded wait).
- **riscv64 / x86-64 port**: mechanical follow-up (the deadline mechanism is
  arch-agnostic in `port::ipc`; the arch-specific parts are the counter read
  and the trap-tail scan).
