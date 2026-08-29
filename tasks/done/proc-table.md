---
status: done
---

# proc-table: the PCB, the proc table, and the kstack vector

Task 2 of 3 in the preemption arc. Plan: [plans/preemption.md](../plans/preemption.md).
Lands after svc-yield; lands before tick-preemption.

## Goal

Replace the two process statics (`KERNEL_SLOT`, `EXIT_STATUS`) with a proc
table, give each process its own kernel trap stack (kstack), make SP_EL1 the
kstack pointer (the EL0 vector frames on it with zero scratch registers),
make TPIDR_EL1 the current-process pointer, and introduce `resched()` so
that a process's exit hands off to the next runnable process. First entry is
forkret-style: every switch-in completes the EL0 vector tail, no
first-entry special case. swtch.S is untouched.

## Changes

All in `aarch64/`.

- `src/trap.S`, frame grows 288 → 304:
  - shared `save_frame` macro: after the interrupt-type store, `mrs x4,
    spsr_el1; str x4, [sp, #288]` (x4 was already stored at [sp, #32] — the
    macro's existing scratch-after-store discipline).
  - EL0 entry: `daifset; sub sp, sp, #304` (SP_EL1 = kstack top — no stack
    address load, so the TPIDR_EL1 x0-parking hack is **deleted**, not
    bypassed); `save_frame`; `mrs x0, sp_el0; str x0, [sp, #280]`;
    `mov x0, sp; bl trap_unsafe`.
  - EL0 tail, with a new `.globl trapret` label at its head:
    `ldr x6, [sp, #288]; msr spsr_el1, x6; ldr x6, [sp, #256]; msr
    elr_el1, x6; ldr x6, [sp, #280]; msr sp_el0, x6; restore_frame; add sp,
    sp, #304; eret`. The msr's stage the process's PSTATE/PC/SP_EL0 from
    the frame while x0–x18 are dead; `add` restores SP_EL1 = kstack top
    (the invariant); `eret` consumes the staged values.
  - EL1 vector: `sub`/`add` move 288 → 304 in lockstep (shared macro and
    size constants); its tail is otherwise unchanged (no swtch ever
    suspends an EL1 trap context).
- `src/trap.rs`: `TrapFrame` gains `spsr: u64` (repr(C) slot order: after
  `sp`); debug print gains the field. The Svc dispatch reads the syscall
  number from **x8** (below), and the yield arm must `return` after
  `yield_current` or it falls into the unhandled-exception spin. The
  layout is triple-maintained (trap.S offsets, the `process::FRAME_*`/
  `CONTEXT_SZ` constants, the structs), so host tests pin
  `size_of`/`offset_of` against the constants.
- `src/process.rs` — the module is rewritten around the table:
  - `static KSTACKS: [[u8; 16 * 4096]; NPROCS]` (`.bss` via an
    `UnsafeCell` wrapper — a plain `[[u8; N]; M]` static lands in
    `.rodata`, and a store into `.rodata` permission-faults; mirrors
    `interruptstackbase`; NPROCS = 8).  16 * 4096 = 64 KiB per kstack,
    the same size as the interrupt stack (the plan's "16 KiB" prose was
    off by 4x; the constant matches the task file and the intent).
    A canary magic word at each kstack base, **hard-asserted** in
    `resched` before every switch (a `debug_assert` would die in a
    release image and lose the only overflow detector; two loads per
    switch is the cost).
  - `struct Process { state: State, context: *mut Context,
    kstack: *mut u8, exit_status: u64 }` (the plan's
    `text`/`stack: Page` fields were dropped: the pages are allocated
    and immediately leaked — consistent with the no-free-API stance,
    but the handles will need re-plumbing when freeing lands);
    `enum State { Running, Runnable, Exited }`.
  - `static TABLE: Lock<[Option<Process>; NPROCS]>` (MCS lock, house
    pattern). `Process.context` is the only field written without the lock
    (by swtch, via the raw pointer copied out under the lock); a slot is
    never reused while a raw pointer to it is live (lazy reclaim at the
    next `spawn`).
  - `spawn(text, text_va, stack_va) -> u8`: allocate text/stack pages
    (kernel table, as today) + map into the user table (as today);
    fabricate the forkret state on the process's kstack: the 304-byte
    frame at kstack top − 304 (x0–x30 = 0, ESR = 0, FAR = 0, ELR =
    text_va, SPSR = 0, sp slot = user stack top) and an ordinary
    112-byte `Context` at frame base − 112 (x19–x30 = 0, x30 = `trapret`,
    sp = frame base, spsr = `SPSR_EL1H | 0x3c0`); publish the slot
    (Runnable).  **Finding and claiming the slot plus the frame
    fabrication are one critical section** (review finding: two
    concurrent spawns must not pick the same slot and interleave frame
    writes into one kstack); the page allocations stay outside the
    lock.
  - `run_all()`: if nothing Runnable, **panic** (plan said return; the
    doc comment says panic — a table with nothing to run is a caller
    bug and spinning/returning silently hides it). Mask IRQs; under the
    lock pick the first Runnable, mark it Running, set TPIDR_EL1 to it,
    drop the lock; `swtch(&mut STARTER_CTX, first.context,
    SPSR_EL1H | 0x3c0)`. On resume (last process exited): unmask. No
    table reset; pages leak (no free API — see the plan's Not
    building).
  - `status(id) -> Option<u64>` (Some iff Exited; valid until slot reuse).
    `process::run` and `kernel_slot`/`set_exit_status` are deleted;
    main9 becomes `spawn` + `run_all` + `status`.
  - `resched() -> bool`: TPIDR null → false. Under the lock: if current is
    Running, demote to Runnable (condition prevents the exit path
    resurrecting an Exited process); scan from the cursor (`AtomicUsize`)
    for a Runnable other than current, wrapping; if none: re-mark current
    Running and return false (one line; state matches reality); if found:
    mark Running, advance cursor, copy out `*mut Process` for current and
    next, **drop the lock** (never held across swtch — MCS is not
    reentrant); set TPIDR to next; `port::irq::exit_interrupt()`;
    `swtch(&mut (*current).context, next.context, SPSR_EL1H | 0x3c0)`; on
    resume: `port::irq::enter_interrupt()`; set TPIDR back to current;
    return true.
  - `exit_current(n)`: under the lock mark Exited + record status;
    `if !resched() { set TPIDR null; port::irq::exit_interrupt() (the
    discarded context's epilogue would never balance); swtch(&mut slot,
    STARTER_CTX, SPSR_EL1H | 0x3c0) }` — the starter address is read
    out **before** the switch and the from slot is a local: the
    process's own context is not the target of the unwind (it is a
    stale context, and ereting into one is how a resumed kernel ends
    up in EL0 at a stale address — a real bug found while testing).
  - `current()` (tpidr read) is module-private; trap.rs reaches the module
    through the narrow crate-private seam (`exit_current`, `yield_current`).
- `src/main.rs`: `process::run(...)` → `let id = process::spawn(...);
  process::run_all(); let status = process::status(id)`.

## Deviations from the plan (recorded)

- **Syscall number = x8 at the trap** (plan: the svc immediate in
  ESR_EL1.ISS).  Linux-arm64-style: x8 the number, x0–x5 future
  arguments; the ISS decode (`svc_aarch64_decode` + host test) stays
  but is unused.  Constraint to revisit before a third syscall: x8
  doubles as the exit status, so status 1 is unrepresentable (1 =
  yield) and every new syscall number retires one exit status —
  a real syscall set will want `x8 = number, x0 = status`.
- **Kstacks are 64 KiB** (`16 * 4096`), not the plan's "16 KiB" prose;
  they need the `UnsafeCell` wrapper to land in `.bss` (a plain
  zero-initialised static is emitted to `.rodata` and stores into it
  permission-fault).
- **`run_all` panics on an empty table** (plan: return).
- **`Process` drops the page handles** (plan: `text`/`stack: Page`);
  pages are allocated and leaked.
- **`save_frame` stores SPSR with x3** (plan said x4 — immaterial,
  x3 is free at that point in the current macro).

## Invariants (stated in code comments, enforced by the design)

- While a process is on CPU, SP_EL1 = its kstack top (vector entry `sub`,
  tail `add`; nothing else runs in EL1 in between).
- A process's off-CPU state is its frame **plus** the suspended kernel call
  chain below it, both on its kstack; every switch-in erets to `trapret`
  (first entry) or into `resched` and unwinds to it (preemption), arriving
  with sp = frame base by AAPCS construction.
- DAIF.I masked from vector entry through the final `eret`; every swtch
  spsr for a handler-saved context is `SPSR_EL1H | 0x3c0`.
- The table lock is never held across a swtch; `resched` checks TPIDR
  before taking the lock (kernel-side lock holders run only when TPIDR is
  null — the invariant the sleep arc must break loudly).

## Tests

- New integration image `aarch64/tests/two_process.rs` (+ `[[test]]`
  entry): A (exit 3) and B (exit 4), **distinct text/stack VAs** (the
  user table is shared: a second spawn at the same VA overwrites the
  first process's code and it resumes into the other's — a real bug
  found while testing); `run_all`; assert both statuses.  The programs
  exit straight away rather than busy-looping (plan said busy loop):
  with no tick, a spin would only delay the exit that drives the
  reschedule.  B runs only because A's exit drives `resched` — the
  image proves the table, the kstack vector, TPIDR, forkret, the depth
  bracketing, and the serial handoff end to end.
- New integration image `aarch64/tests/two_yield.rs` (+ `[[test]]`
  entry): A (yield, yield, exit 6) and B (yield, exit 7) with distinct
  VAs; the expected dance (A yields → B first-enters → B yields → A
  resumes in its suspended handler → …) proves both resume shapes, the
  TPIDR handoff at every switch (a stale TPIDR starves the table),
  and the cursor.  This is the regression test for the commented-out
  `tpidr_set(next)` bug the review found.
- `user_process` unchanged.  `user_yield` re-encoded for x8, and its
  first `cmp`/`b.ne` pair now puts **NZCV live across the yield**
  (the `b.ne` after the svc branches on flags set before the trap) —
  the regression test for the missing SPSR store the review found.
- Host tests: layout pins (`size_of`/`offset_of` for `TrapFrame`
  against `FRAME_*`, `size_of` for `Context` against `CONTEXT_SZ`).
  The plan's state-machine host tests (slot reclaim, demote/re-mark)
  were not done: `Process`/`State`/`TABLE` are `target_os = "none"`
  only, and the images above cover those paths end to end instead.

## Acceptance

- `cargo xtask ci` green (integration images are now 6).
- `cargo xtask qemu --arch aarch64 --image two_process --timeout 60` passes
  (A and B both reach their exit statuses; the canary asserts fire if a
  kstack overflows).
- `cargo xtask qemu --arch aarch64 --image two_yield --timeout 60` passes.

## Resolution

Landed.  Review (Claude, working-tree diff) found and confirmed two
real bugs both silent under the then-current test set: `tpidr_set(next)`
was commented out (a debug-instrumentation removal glued the call onto a
comment line), and `save_frame` never stored SPSR so every switch-in
staged the slot's initial PSTATE (NZCV destroyed on resume).  Also:
the spawn find+claim was two lock acquisitions (SMP: fixed to one
critical section), the canary asserts were `debug_assert` (now hard),
and the syscall convention moved from ESR.ISS to x8 (deviation
recorded above).  The two regression images (two_yield, the
NZCV-live user_yield) and the layout pins exist because of the review.

## Not in scope

The tick (task 3): no timer is started, no preemption of a running
process, no `preemptions()`. Per-process address spaces, sleep states,
page freeing — the plan's Not building.
