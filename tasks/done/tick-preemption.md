---
status: done
---

# tick-preemption: the tick that preempts

Task 3 of 3 in the preemption arc. Plan: [plans/preemption.md](plans/preemption.md).
Lands after proc-table.

## Goal

Add a periodic timer tick that reschedules, so the kernel switches between
runnable processes without the processes' cooperation. The tick never
switches from inside the timer callback: it sets a flag, and the switch
happens at the tail of the IRQ path, after CVAL is re-armed (deasserting
the level line) and the EOI is done — otherwise the intid stays active at
the GIC and its priority masks every interrupt at or below it for as long
as the other process runs, degenerating round robin to exactly one
preemption total.

## Changes

All in `aarch64/`.

- `src/process.rs`:
  - `static TICK_TIMER: Timer = Timer::periodic(Duration::from_millis(100),
    &TICK);` with `struct Tick` implementing `TimerCallback::fire` as: set
    a `NEED_RESCHED` flag (atomic, consumed by swap-false at the tail) and
    return true. Started once by `run_all` (a one-time flag; the timer is
    never restarted — "not designed for concurrent restarts" per its docs).
    It keeps firing after the batch as a no-op.
  - `preemptions() -> u64` backed by an `AtomicU64`.
- `src/trap.rs`, the IRQ arm: after `timer::interrupt_handler()` and
  `gic::end_interrupt(iar)` — i.e. re-arm done, EOI done — add the trap
  tail: `process::irq_resched()` — the process module consumes the flag
  (swap-false), and if a process is current, `resched()`s, counting a
  switch as a preemption. The count bump lives in the process module so
  `resched` (shared with yield/exit) stays uncounted.

## Why the tail, not `fire()` (keep this comment in the code)

IAR makes the intid active; the active priority masks delivery of every
interrupt at or below it until the matching EOI; the timer line is
level-triggered and stays asserted until CVAL is re-armed. A context
suspended before both holds the deassert and the EOI — no further tick is
ever delivered while it is suspended. `fire()` therefore sets a flag; the
switch happens only once the hardware is fully acknowledged. This is the
Plan 9 / Linux shape (tick sets a flag, the trap tail schedules).

## Tests

- New integration image `aarch64/tests/preempt.rs` (+ `[[test]]` entry):
  A and B are both busy loops (no yield) that each run for half a second
  of *real* time, paced against the physical counter (read
  CNTPCT_EL0/CNTFRQ_EL0 at EL0); A exits status 3, B status 4, via the
  catch-all. After `run_all`: assert both statuses **and**
  `preemptions() >= 2` (observed ~4). The status pair alone would pass on
  a timeline where nothing is preempted (A runs to completion, B runs
  second); `>= 2` requires repeated tick *delivery* while processes run —
  exactly what a stranded-EOI regression breaks (it allows precisely one
  preemption, then self-heals when an exit resumes the suspended handler).
  **Real-time pacing, added after landing:** the first version used a
  fixed instruction count (0xFFFFFFFF), which cost ~10 s here and ~35 s
  on the CI runner; shrinking the count to bring the image back under
  ~2 s left exactly 2 preemptions — at the assert's edge, because a
  fixed count is a different duration per machine while the 100 ms tick
  is real time. Pacing against the counter makes both sides of the
  comparison real time, so the preemption count is machine-independent.
  This required `timer::init` to set CNTKCTL_EL1's EL0 counter-enable
  bits (reset value 0 traps EL0 counter reads); a host test pins the
  opt-in.
- **The decrement must be `subs`, not `sub`** (bit 29 of the encoding):
  a plain `SUB` (immediate) does not update NZCV, so the loop's branch
  tests a stale flag and the loop never stops — it wraps the counter
  through zero and runs forever. The kernel was exonerated of the
  resulting symptoms (endless preemptions of a never-exiting process)
  only after the program was fixed.
- `two_process`, `user_process`, `user_yield`, `two_yield` pass unchanged.
- Debugging note: temporary instrumentation in `irq_resched` that held the
  proc-table lock across the `resched()` call self-deadlocked the MCS
  lock and produced spurious "kernel executing kstack" faults; all debug
  was stripped before the gates. The lldb/GDB session (`xtask qemu
  --gdb`) confirmed every `swtch`/`trapret` round trip is structurally
  correct (ctx pointers, frame slots, handler resume chain).

## Acceptance

- `cargo xtask ci` green (aarch64 integration images are now 7; 9 total across arches).
- `preemptions() >= 2` holds on every manual run (several) before the image
  is relied on in CI. Per the QEMU rule: `cargo xtask qemu --timeout` only,
  never a backgrounded QEMU.

## Not in scope

Sleep/wait states (the exit path's "nothing runnable ⇒ everyone exited ⇒
starter" inference is the sleep arc's breaking point), priorities, a
quantum as a slice, per-process address spaces, stopping the tick when
idle, page freeing.
