# Plan: Preemption — yield, the proc table, and a tick that switches

Status: ready for build (three Claude review rounds + six-lens panel;
hardware claims verified empirically against QEMU)

## Problem

r9 can run one user process from `process::run` to its `svc #0` exit, but the
kernel cannot return from a syscall that keeps the process alive, has no notion
of more than one process, and never switches between processes. Every `svc`
today is a one-way exit: the EL0 trap path that restores a live register file
and `eret`s back into EL0 with the process continuing exists in `trap.S` but
has never been taken in CI, because the only syscall is one that never
returns.

Two latent defects this arc closes, both found in review and one of them
verified empirically:

- **SP_EL0 is never established for a process.** AArch64 `eret` restores PC
  ← ELR and PSTATE ← SPSR and touches *no* stack pointer (Arm ARM DDI 0487,
  `AArch64.ExceptionReturn`; SP_EL0 is a banked register written only by an
  explicit `msr sp_el0` — Linux arm64 does exactly that before every return
  to EL0). r9's EL0 vector saves the interrupted user sp into the frame
  (slot 280) and never writes it back, and nothing else ever sets SP_EL0.
  Verified on a live QEMU: the process's SP_EL0 at its `svc` trap is `0x0`
  (a scratch program doing `mov x0, sp; str x0, [x0]` data-aborted at
  FAR=0 with `sp: 0x0` in the frame dump; the kernel reading frame slot 280
  on the `svc` printed `0x0`). The first process never notices because its
  whole program is one `svc` that exits; the first process that stores to
  its own stack across a syscall will.
- **The vector's final `eret` trusts live ELR_EL1/SPSR_EL1.** `save_frame`
  copies ELR into the frame but the return path never restores it, and SPSR
  is not in the frame at all. Sound today only because nothing between trap
  entry and `eret` writes those registers. `swtch` destroys that the moment a
  suspended trap context is resumed: swtch's resume path does
  `msr spsr_el1 / msr elr_el1 / eret`, and the vector's later `eret` would
  then fire with a kernel PC and EL1 PSTATE — the "process" continues
  mid-kernel-function with the user's register file loaded.

This arc adds preemption in three independently landable steps:

1. **yield** — a syscall that returns to the process, with SP_EL0 made
   honest (the first process with a real stack);
2. **the PCB and the proc table** — the two process statics become a table
  of processes, each with its own kernel trap stack; every switch-in
   completes the EL0 vector tail;
3. **the tick** — a second pair of processes and a periodic timer whose
   interrupt reschedules, so the kernel switches between runnable processes
   without the processes' cooperation.

## Constraints

- aarch64 code only. `x86_64/` and `riscv64/` must not appear in any diff;
  their gates keep passing untouched.
- Single core for now. SMP is task #4; per-core state goes in TPIDR_EL1
  (current process) from the start — the register the done-task note already
  earmarked for it — so #4 finds per-core state where it expects it.
- Every task lands with `cargo xtask ci` green: fmt, check, clippy ×3,
  host tests, dist ×3, integration tests (images grow 4 → 5 → 6 → 7). A test
  image is raw AArch64 bytes in a `no_std` image — hand-encoded instructions
  (encode with `clang -target arm64 --assemble` + objdump, verify the
  round-trip), no assemblers in the repo, no linker scripts.
- `#![forbid(unsafe_op_in_unsafe_fn)]`: every unsafe block explicit.
- The kernel is *outside* the process table: main9 (and the test images)
  start processes and keep a kernel identity. There is no proc 0.
- syscall numbers: 0 = `SYSEXIT` (existing), 1 = `SYSYIELD` (new).  The
  number is read from **x8 at the trap** (landed during task 2; the plan
  originally said the svc immediate in ESR_EL1.ISS — ISS encodes 16 bits
  and leaves no room for arguments, and the test programs need
  terminate-with-status, so x8 = number with x0–x5 reserved for future
  arguments, Linux-arm64-style).  The exit status *is* the number for
  the catch-all: test programs exit via numbers ≥ 3 (3, 4, 6, 7) so no
  status collides with yield (1).  Known wart to revisit before a third
  syscall: status 1 is unrepresentable and each new number retires one
  exit status.

## Prior art

Plan 9 4th edition kernel (the local `/Volumes/Code/repos/plan9` checkout is
userland only; the kernel tree is not vendored, so these references are from
memory of that tree, weight accordingly): `struct proc` with `state`,
`kstack` (per-proc kernel stack), and the saved machine context; `sched()`
as the single funnel from tick/yield/exit that picks *the next runnable*
proc; `swtch()` saving the live context on the running proc's own kstack
because a shared stack is reset by the next exception and would destroy
every live context on it; the reschedule decision made at the tail of the
interrupt path after the hardware is fully acknowledged.

xv6 (RISC-V): the **forkret** trick — a first-time context whose link
register points at the interrupt-return path, so the very first entry
"resumes" like any other and there is no special first-entry code. This arc
uses it for process first entry.

Linux arm64 (reference `/Volumes/Code/repos/linux`): `kernel_ventry` does
`sub sp, sp, #S_FRAME_SIZE` with **zero scratch registers** because SP_EL1
is arranged to be the task kstack; `entry.S` writes `msr sp_el0, <user sp>`
explicitly before every return to EL0 (nothing else can — see Problem);
`schedule()` as the single funnel, invoked after EOI. What this arc takes
from it is shape (SP_EL1-as-kstack, explicit SP_EL0 switch, one reschedule
funnel, register-file-preserving preemption), not code.

## Hardware assumptions (verified where marked)

- **ERET** (Arm ARM DDI 0487): PC ← ELR, PSTATE ← SPSR, and *no* stack
  pointer is copied — SP_EL0/SP_EL1 are banked and written only by explicit
  `msr`/`mov sp` (in SP0 state). *(Empirically verified on QEMU for r9:
  process SP_EL0 was 0x0 at its first trap — see Problem.)* Consequence
  used throughout: a process's user stack pointer is a piece of per-process
  state the kernel must switch explicitly, and SP_EL1 — never touched by
  eret — is free to carry the kernel's per-process stack.
- **SP_EL1 across a process's lifetime**: an EL0 exception preserves SP_EL1;
  the EL0 vector's entry (`sub sp, sp, #304`) and tail (`add sp, sp, #304`)
  keep it at the kstack top for as long as the process is on CPU; nothing
  else runs in EL1 in between.
- **GICv2 lifecycle**: IAR makes the intid active at the CPU interface; the
  active priority masks delivery of every interrupt at or below it until the
  matching EOI. The generic timer line is level-triggered: it stays asserted
  until CVAL is re-armed into the future. Consequence: a context suspended
  *before* the re-arm and EOI strands the whole interrupt system while
  suspended — the tick must reschedule only after both.
- QEMU `virt` machine (what xtask runs); the timer frequency is read at
  runtime by the timer module, so the tick is specified in time (100 ms),
  never in ticks — no CNTFRQ assumption.
- AArch64 `svc` immediate: the syscall number, in ESR_EL1.ISS for EC 0x15.

## Design

### The one thing to get right: where a preempted process's state lives

A process's live state while the kernel runs is (a) the trap frame on a
kernel stack — x0–x30, ESR, ELR, FAR, type, the interrupted user sp, and
(new) SPSR — and (b) the suspended kernel call chain below it. Both must
outlive however long the process is off-CPU, so both live on the process's
own 16 KiB kstack, never on the shared interrupt stack (which the next
kernel exception resets).

**SP_EL1 is the kstack pointer.** While a process is on CPU, SP_EL1 = its
kstack top. The EL0 vector entry is `daifset; sub sp, sp, #304` — zero
scratch registers before the stores (the reason Linux's `kernel_ventry` is
shaped this way; any `mrs`/`ldr` into x9/x10 before `save_frame` would save
and later restore the *clobbered* values, silently corrupting user
registers on every trap). The vector's TPIDR_EL1 x0-parking hack is deleted
in the same change: the new entry needs no scratch, and TPIDR_EL1 becomes
the current-process pointer for the Rust side (mutually exclusive uses —
the parking hack overwrote TPIDR on every trap).

**The frame grows 288 → 304 bytes.** New slot 288 = SPSR (saved by the
shared `save_frame` macro into a scratch register *after* the GPR stores,
the macro's existing discipline). Slot 280 stays the interrupted user sp
(EL0) / interrupted SP_EL1 (EL1). The EL1 vector shares `save_frame` and
the size constants, so its `sub`/`add` and the `TrapFrame` struct move in
lockstep; the EL1 tail is otherwise unchanged (no swtch ever suspends an
EL1 trap context — all resched call sites are EL0 traps, and `resched`
no-ops when TPIDR is null).

**Every switch-in completes the EL0 vector tail.** The tail, reached with
sp = frame base on all paths, is straight-line asm with a `trapret` label at
its head:

```asm
trapret:
        ldr   x6, [sp, #288]      // x0-x18 dead until restore_frame
        msr   spsr_el1, x6        // the process's PSTATE (staged, not live)
        ldr   x6, [sp, #256]
        msr   elr_el1, x6
        ldr   x6, [sp, #280]
        msr   sp_el0, x6          // the process's user stack pointer
        restore_frame             // x0-x30 back, incl. x30 (the process's LR)
        add   sp, sp, #304        // SP_EL1 = kstack top (invariant)
        eret
```

- **First entry (forkret):** `spawn` fabricates the 304-byte frame at
  kstack top − 304 (x0–x30 = 0, ESR = 0, FAR = 0, ELR = text_va, SPSR = 0
  SPSR = 0 — EL0, M[3:0] = 0b00000; EL0 has no h/t split — sp slot = user
  stack top) and an ordinary 112-byte `Context` at
  frame base − 112 (x19–x30 = 0, **x30 = `trapret`**, sp = frame base,
  spsr = `SPSR_EL1H | 0x3c0`). `run_all`'s first `swtch` erets into
  `trapret`; the tail stages ELR = text_va and SP_EL0 = user top and
  `eret`s — the process starts with x30 = 0 (from the frame), not the
  `trapret` label. The entry context stops living on the user stack page —
  kernel state out of user-writable memory.
- **Preemption resume:** `resched` switches from mid-handler (see below);
  the suspended context is the handler's context on the kstack. Resuming it
  erets back into `resched` and execution **returns up the call chain** —
  resched's epilogue, `trap()`'s, `trap_unsafe`'s (which balances
  interrupt depth) — and the `ret` after `bl trap_unsafe` arrives at
  `trapret` with sp = frame base, exactly as on the non-suspended path, by
  AAPCS construction (sp is callee-preserved through swtch because it is in
  the Context). The tail then restores SPSR/ELR/SP_EL0 *from the frame* —
  the live registers hold kernel values after the resume, which is why the
  frame must hold them and why this is the arc's load-bearing invariant.
- **DAIF invariant, stated:** DAIF.I stays masked from vector entry through
  the final `eret`, and every `swtch` spsr argument for a context saved
  inside a handler is `SPSR_EL1H | 0x3c0`. The tail's staged-SPSR→`eret`
  window is safe only because of this: SPSR_EL1 is a holding register, the
  live PSTATE stays masked, and no nested exception can land between the
  `msr`s and the `eret`.

swtch.S is **untouched**: it still saves/restores 112-byte Contexts; the
frame is restored solely by the tail.

### Task 1 — yield: a syscall that returns, honestly

- `process.rs` gains `SYSYIELD = 1`. The SVC case in `trap()` becomes a
  dispatch: 0 kills as today; 1 returns to the process (the handler does
  nothing — the vector's restore + `eret` is the yield); n > 1 terminates
  with status n (today's catch-all, kept). The Svc arm can now complete,
  so the restructure must not fall into the unhandled-exception spin loop.
- `trap.S` EL0 tail: before `restore_frame`, `ldr x6, [sp, #280]; msr
  sp_el0, x6` — the SP_EL0 fix from the Problem section. This is a
  prerequisite of an honest yield test, not a task-2 concern. (The frame
  stays 288 in this task; the parking hack stays — the vector still loads
  the shared stack address into x0, which is what the hack works around.)
- Test image `user_yield`: a program that (a) stores a marker to `[sp, #8]`
  and loads distinct values into x0 and x19, (b) yields, (c) checks all
  three survived, (d) yields again, (e) exits with a status reachable only
  if both `svc`s returned with the register file *and the user stack*
  intact. Without the SP_EL0 fix, SP_EL0 is 0 and the store faults (or,
  landing in the interrupt-stack frame region, is clobbered by the next
  trap's `save_frame`); either way the image fails.

### Task 2 — the PCB, the proc table, and the kstack vector

The two statics (`KERNEL_SLOT`, `EXIT_STATUS`) become a table.

- `static KSTACKS: [[u8; 16 * 4096]; NPROCS]` in `.bss` (mirrors
  `interruptstackbase`; NPROCS = 8 is a compile constant, so no allocation
  story). A magic canary word at each kstack base, debug-asserted in
  `resched` — five lines that catch silent overflow, whose failure mode
  (overflow walking into the neighbour) is otherwise invisible. Statics
  mean no guard *pages* (you can't unmap inside the array); that is
  recorded under Not building, not papered over.
- `pub struct Process`: `state: State` (Running | Runnable | Exited),
  `context: *mut Context` (resume point — the invariant above), `kstack`
  base pointer into `KSTACKS`, `text: Page, text_va: usize`,
  `stack: Page, stack_va: usize` (user pages, mapped in both tables as
  today), `exit_status: u64` (valid once Exited).
- `static TABLE: Lock<[Option<Process>; NPROCS]>` (MCS lock, the house
  pattern from `timer.rs`).
- `spawn(text, text_va, stack_va) -> ProcessId`: allocate text/stack pages
  (kernel table, as today) + map into the user table (as today) +
  fabricate the frame + `Context` on the process's kstack (forkret, above);
  then take the table lock, fill the slot (state = Runnable), drop it.
  Allocations outside the lock; a slot Exited in this batch is reused.
- `run_all()`: if nothing is Runnable, return. Mask IRQs; set TPIDR_EL1 to
  the first Runnable and mark it Running (under the lock, dropped before
  the switch); `swtch(&mut STARTER_CTX, first.context, SPSR_EL1H | 0x3c0)`
  (saving the kernel starter context in `STARTER_CTX` — the successor of
  `KERNEL_SLOT`'s role). The mask covers the TPIDR-write → swtch window:
  an IRQ landing there would see TPIDR non-null while the kernel is still
  on the shared stack. On resume (last process exited): unmask. The table
  is **not** reset: slots reclaim lazily at the next `spawn`, so
  `status(id)` survives `run_all`'s return.
- `status(id) -> Option<u64>`: `Some` iff the slot is Exited. main9 becomes
  `spawn` + `run_all` + `status`; `process::run` is deleted.
- TPIDR_EL1 is set/cleared in the process module (Rust `msr`), always
  immediately before the `swtch` it belongs to; swtch never sees it.
  `current()` (tpidr read) is module-private; the public surface is
  `spawn`/`run_all`/`status`/`SYSEXIT`/`SYSYIELD`/`preemptions` plus a
  narrow crate-private seam for trap.rs (`exit_current`, `yield_current`).
- `resched() -> bool`, born here (the serial two-process test needs it):
  - first action — TPIDR null → return false (no process running; nothing
    to switch *from*);
  - under the lock: if current.state == Running, demote it to Runnable
    (so a preempted/yielding process is selectable again; the condition
    keeps the exit path from resurrecting a process it just marked
    Exited); scan from the cursor (an `AtomicUsize`, per the
    SMP-don't-assume rule) for a Runnable other than current, wrapping;
    mark it Running; advance the cursor; copy out `*mut Process` for
    current and next; **drop the lock**. If the scan finds no next: re-mark
    current Running (the demote was premature — it is still on CPU; the
    stale state is benign this arc, but state should match reality) and
    return false;
  - set TPIDR to next; `port::irq::exit_interrupt()` (depth 1 → 0: the
    other process runs outside interrupt context — without this, depth
    stays 1 across the switch and every `println` while it runs trips the
    `in_interrupt` debug assert); `swtch(&mut (*current).context,
    next.context, SPSR_EL1H | 0x3c0)`; on resume: `port::irq::enter_interrupt()`
    (0 → 1: we are back inside the suspended handler, whose `trap_unsafe`
    epilogue will exit it) and set TPIDR back to current; return true.
    No next → return false, no bracket.
  - The table lock is *never* held across a `swtch` (MCS is not
    reentrant; the suspended context's eventual `resched`, same core,
    would self-deadlock). The lock invariant stated for the next arc:
    kernel-side lock holders run only when TPIDR is null, and `resched`
    checks TPIDR before taking the lock — the moment any syscall touches
    the table from a running process, this dies and acquisition must
    move under a DAIF mask.
- The exit path: `exit_current(n)` marks Exited + records status under the
  lock; `if !resched() { set TPIDR null; port::irq::exit_interrupt()
  (the discarded context's epilogue would never balance); swtch to
  STARTER_CTX }`. The last process's exit thus unwinds `run_all` exactly
  as today's `syscall_exit` does, and a non-last exit hands off to the
  next Runnable.
- Yield handler: `yield_current()` = `resched()`; with one process it
  returns false and the vector returns the process to EL0.
- `trap.S` EL0 vector (task 2's part): the kstack entry (`sub sp, sp, #304`
  from SP_EL1), the parking-hack deletion, the 304-byte frame with the SPSR
  slot, and the `trapret` tail above — all one task, because TPIDR-as-scratch
  and TPIDR-as-pointer are mutually exclusive and the tail must exist before
  any switch-in can use it. EL1 vector: `sub`/`add` and struct lockstep
  only.
- Aliasing discipline (AGENTS.md forbids assuming single-core):
  `Process.context` is the only field written without the table lock (by
  swtch, through the raw pointer copied out under the lock); a slot's
  `Process` is never freed or reused while a raw pointer to it is live
  (enforced by lazy reclaim); every other field is touched under the lock;
  `cursor` is atomic; `STARTER_CTX` is set and read only in IRQ-masked
  kernel context this arc (task #4 makes it per-core, as it makes
  `INTERRUPT_DEPTH` and the cursor).
- Test image `two_process`: spawn A (busy loop, exits status 3) and B
  (busy loop, exits status 4); `run_all`; assert both statuses. With no
  tick yet, B can run only because A's exit drives `resched` — the image
  proves the table, the kstack vector, TPIDR, the forkret entry, the depth
  bracketing, and the serial reschedule chain end to end.

### Task 3 — the tick that preempts

- `static TICK_TIMER: Timer = Timer::periodic(100 ms, &TICK)` in the
  process module; started once by `run_all` (a flag guards the one-time
  start; the timer is never restarted — "not designed for concurrent
  restarts" per its own docs). It keeps firing after the batch as a no-op
  (stopping it is a timer-arc change, noted not built).
- `TICK.fire` does **not** switch. It sets a `NEED_RESCHED` flag (atomic,
  consumed by swap-false at the tail so a stale flag from a drained batch
  can't fire a later resched). The switch happens at the tail of the IRQ
  path in `trap()`, after
  `timer::interrupt_handler()` has re-armed CVAL (deasserting the level
  line) and `gic::end_interrupt` has completed the EOI. This is the Plan 9
  / Linux shape and it is not optional: rescheduling from inside `fire()`
  would suspend a context that still holds both the re-arm and the EOI —
  with the intid active, its priority masks every interrupt at or below
  it, and no further tick is ever delivered while the other process runs.
  Round robin between two busy loops would silently degenerate to exactly
  one preemption total.
- `preemptions() -> u64` (AtomicU64): incremented at the IRQ tail *before*
  `resched`, gated on the flag and a live current process. It counts timer
  ticks that fired while a process was running — the delivery proof.
  (Incrementing inside `resched` would count yield/exit switches too, and
  executing it only when the preempted context later resumes would make
  the count depend on suspended contexts draining — luck, not design.)
- The `preempt` image: A and B are both busy loops (counters in x19, no
  yield), N sized in the build (empirically, running the image under
  `cargo xtask qemu --timeout` several times) so each runs for several
  tick periods; A exits status 3, B status 4. Assert both statuses **and**
  `preemptions() >= 2`. The status pair alone would pass on a timeline
  where nothing is ever preempted; `>= 2` requires *repeated tick
  delivery* while processes run, which is exactly what a stranded-EOI
  regression breaks (it allows precisely one preemption, then self-heals
  when an exit resumes the suspended handler).

### Not building

- Freeing pages at batch end or exit: `pagealloc` has no free API and `vm`
  no unmap — that is a sub-project, not a line in `run_all`. Pages leak
  this arc; with NPROCS = 8 and test-sized batches that is honest, and it
  is said here rather than hidden behind "the table's pages are freed".
- kstack guard pages / overflow detection (Linux's VMAP_STACK machinery):
  statics can't be unmapped per-page; the canary assert is the coverage.
- Sleep/wait states, zombies, parent/child, signals — the next arc. Two
  arc-boundary notes: (1) the exit path's "`resched` returned false ⇒
  everyone exited ⇒ starter" inference is the sleep arc's breaking point —
  with sleeping processes, no-runnable means nothing, and the exit path
  needs an explicit all-exited scan and an idle context, not the starter;
  (2) the table-lock invariant dies the first time a syscall touches the
  table from a running process.
- Per-process address spaces (per-process TTBR0): the user page table
  stays global; processes get disjoint VA ranges and can see each other's
  pages. A namespace arc, not a scheduling one.
- Scheduling policy beyond round robin; priorities; quantum as a *slice*
  (the tick is the whole quantum: 1 tick = 1 reschedule decision).
- Process spawning from userland (no fork).
- Stopping the tick when idle.

### Decision records

1. **Yield before the PCB.** The EL0 return path with a live register file
   is the one unproven thing; the PCB is a data rearrangement. Task 1 also
   carries the SP_EL0 fix, where the first honest test needs it, and it is
   tiny because the frame slot (280) already exists.
2. **SP_EL1 carries the kstack; TPIDR_EL1 carries the process pointer; the
   asm never reads TPIDR.** ERET touches no stack pointer (verified
   empirically), so SP_EL1 is free to hold the kstack top and the vector
   entry needs zero scratch registers — the Linux `kernel_ventry` shape.
   The alternative (read the kstack from the process via TPIDR) requires
   an `mrs`/`ldr` into x9/x10 before `save_frame`, which saves and later
   restores the clobbered values: silent user-register corruption on every
   trap. TPIDR remains the Rust side's per-core current pointer (task #4
   keeps it there).
3. **Forkret first entry; every switch-in completes the vector tail.**
   One mechanism, no first-entry special case, kernel entry state off the
   user stack page, and swtch.S untouched (it still handles 112-byte
   Contexts only; the 304-byte frame is the tail's business).
4. **Mid-handler swtch, with the depth bracket.** `resched` switches from
   inside the trap handler — the suspended context *is* the process's
   durable state (frame + call chain, both on the kstack), and the resume
   unwinds to the tail with sp = frame base by AAPCS construction. The
   `exit_interrupt`/`enter_interrupt` bracket makes interrupt depth
   per-context and zero across switches; without it, the next process runs
   at depth 1 and every `println` panics.
5. **The frame carries SPSR, and the tail restores SPSR/ELR/SP_EL0 from
   the frame before `restore_frame`.** swtch's resume path overwrites the
   live ELR/SPSR; the tail's `eret` must stage them from durable state,
   while x0–x18 are still dead (scratch-safe) and before the frame's
   x0–x30 are reloaded. 288 → 304 moves in lockstep across the shared
   macro, both vectors, and `TrapFrame`.
6. **The tick reschedules at the IRQ tail, never from `fire()`.** The
   GIC active-priority + level-line lifecycle makes an in-`fire` switch a
   one-preemption-per-boot system (Hardware assumptions).
7. **Static kstacks with canary asserts.** NPROCS is a compile constant;
   no allocation story; 64 KiB (16 pages) is the proven size of the
   shared stack the frame depth already requires (the plan's "16 KiB"
   prose was off by 4x — the constant is `16 * 4096`, mirroring the
   interrupt stack); the kstacks need an `UnsafeCell` wrapper to land
   in `.bss`; the **hard-asserted** canary (not `debug_assert`: a
   release image must keep its only overflow detector) catches the one
   failure mode statics can't guard.
8. **Pages leak this arc; slots reclaim lazily.** No free API exists
   (Not building); `status(id)` stays honest after `run_all` returns
   because nothing resets the table.
9. **`SPSR_EL1H | 0x3c0` at every scheduler switch, and DAIF.I masked from
   vector entry through the final `eret`.** All switch sites run masked;
   the documented swtch contract requires the resume state to match.
10. **The table lock is never held across `swtch`; `resched` checks TPIDR
    before locking.** The MCS lock is not reentrant (self-deadlock, not a
    race); the stated kernel-side/IRQ-side lock invariant is what the
    sleep arc must break loudly.

### Open questions (answered by build)

- Stale-`now` clamp in `interrupt_handler` when a tick fires while the
  process set is empty: no-op path; the build confirms no fire storm in
  the `preempt` image (a storm hangs — bounded by the image's qemu-test
  budget and `cargo xtask qemu --timeout`).
- NPROCS, kstack size, tick period, and the test counters are constants
  picked for zero justification pressure; all one-line changes if the
  build says otherwise.

## Tasks

- [x] `tasks/svc-yield.md` — yield + the SP_EL0 return-path fix (task 1)
- [x] `tasks/proc-table.md` — the PCB, the proc table, the kstack vector, forkret (task 2)
- [x] `tasks/tick-preemption.md` — the tick that preempts (task 3)

## Verification

Per task: `cargo xtask ci` green (gates above; integration images grow 4 → 5
→ 6 → 7). Task 3 additionally: run the `preempt` image under
`cargo xtask qemu --arch aarch64 --image preempt --timeout 60` several
times to size both counters so each process runs for several tick periods
under QEMU jitter, and confirm `preemptions() >= 2` on every run before it
is relied on in CI. (Per the QEMU rule: `cargo xtask qemu --timeout` only,
never a backgrounded QEMU.)
