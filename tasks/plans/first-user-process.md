# Running the first user process: start, sysexit, return to the kernel

## Problem and constraints

Get r9 (aarch64, Pi 4 / QEMU raspi4b) to the stage where the kernel starts
a very simple user process in EL0, the process calls sysexit, and the
kernel resumes at the point that started it — printing just before the
start and again once the process has returned.

Standing constraints: warning-free on all three arches (all new code lives
in the `aarch64` crate, so the other arches are untouched); minimal scoped
change; Plan 9 shape; QEMU integration test proves it.

## Prior art

**r9 already has most of this.** The `aarch64/tests/user_process.rs` image
builds the entire scenario — live user page table, a text page holding
`mov x0,#0; mov x1,#1; svc #3`, a user stack, a `Context` placed at the
top of the stack — and deliberately stops one step short of `swtch`,
because the trap handler it would land in spins instead of returning
(stated in the test's own doc comment). `swtch.S` is a 9nen-style context
switch (9nen itself being Plan-9-shaped). The trap machinery has a full
16-vector table, a dedicated kernel interrupt stack, and an `eret`
epilogue that already returns to user mode when left alone. `main9`
already runs with TTBR0 on the *user* page table while the timer tickers
fire, so EL1→EL1 interrupts under the user table are proven.

**Plan 9** (`/Volumes/Code/repos/plan9` — partial checkout, `src/cmd`
only; from the published 9port design): the kernel's own main() runs as
a process; every process is `struct proc` with a `struct context`
saved on the process's own stack; context switch is one assembly
procedure; sysexit is a syscall that kills the current process and
schedules another. Essential parts for us: context-on-stack, one swtch,
exit-as-syscall. Accreted: NPROC table, scheduler, pgroup — refused.

**Linux** (witness only): `el0_svc` → `invoke_syscall`, `do_group_exit`
→ `do_exit` → `schedule`; `switch_to` saves the callee-saved set on the
task's stack. Confirms the shape; everything else (cred, mm_struct,
cgroup) is accretion.

**What is actually missing** (the whole plan):
1. `swtch.S` saves `spsr_el1` — architecturally uninitialized at EL1
   until an exception happens to set it, and clobbered by hardware to the
   *trapped* context's state on every trap taken to EL1 (Arm ARM DDI 0487,
   "Exception model"). After an EL0 exception it holds EL0 state, so the
   "kernel context" saved next would carry EL0 state back. The natural
   fix — reading the current state, as the AArch32 original did with
   `mrs rd, cpsr` — is **not possible in AArch64**: there is no
   instruction that reads the current state (the first build attempt
   failed exactly here). The save side therefore takes the resume state
   from the **caller**, which is where that knowledge lives: `swtch`
   gains a `spsr` argument (`SPSR_EL1H` = 0x7 for kernel callers, DAIF
   bits ORed in when masking at switch-out).
2. The trap handler's synchronous path spins after printing; it needs a
   real syscall dispatch whose exit path returns to a *saved kernel
   context* (hardware `eret` cannot restore x19–x30, so the return must
   go through `swtch`).
3. A place to remember the kernel's return context between `start` and
   the exit trap.
4. The process itself: page setup (exists in the test), context build,
   the switch, and the resumption print.

## Hardware assumptions (required)

Target: aarch64 on QEMU `raspi4b` (the integration path) and Pi 4
hardware. All other arches: **nothing new** — every change is inside the
`aarch64` crate; x86-64/riscv64 gates keep compiling exactly what they do
today.

- **Exception vector table**: 16 entries × 128 B, selected by (EL, SP
  selection, exception class); `VBAR_EL1` armed at `exception_vectors` by
  `trap::init` (Arm ARM DDI 0487, "Exception vector table"). Already
  true; EL0 vectors are exercised for the first time by this feature.
- **`eret`** reads `ELR_EL1`/`SPSR_EL1` and does not consume `SPSR_EL1`;
  the vector preamble's masking therefore needs no manual unmask — the
  interrupted context's DAIF is in the hardware `SPSR_EL1` (DDI 0487,
  ERET / "Saving the current state").
- **`SPSR_EL1` at EL1 is uninitialized** until the first exception to
  EL1 writes it; it then holds the *trapped* context's state (DDI 0487,
  SPSR_EL1 register description). This is the basis of finding #1.
- **SVC from AArch64**: `ESR_EL1.EC = 0x15`, `ISS` = the `svc`
  immediate (imm24) (DDI 0487, ESR_EL1). Syscall number := that
  immediate.
- **User half of the address space**: TTBR0 covers `[0, KZERO)`; the
  user root table already carries the kernel identity mapping (proven by
  `main9` running tickers with TTBR0 = user table), so kernel code, the
  interrupt stack, and the console are all reachable while an EL0
  process runs.
- **Interrupts while EL0 runs**: policy decision — the first process
  starts with **IRQs enabled** (SPSR DAIF = 0) so the live 1 s timer
  proves the EL0→EL1 path under the user table. Consequence handled: the
  vector preamble must mask IRQs (`msr daifset, #2`) so a timer firing
  inside the exit handler cannot push a second frame onto the single
  fixed interrupt stack and overlap the exit frame (see Failure policy).
- **Single core**: MPIDR affinity 0 only; non-zero cores hang in `l.S`.
  No SMP assumptions anywhere in this design.
- **Memory ordering**: no new sharing between contexts of different
  privilege; the existing DSB/ISB around `TTBR0`/TLBI in `vm::switch`
  remains the only barrier on the path. No new ordering requirements.
- **Firmware co-tenancy**: firmware owns the UART and the mailbox (as
  today); this design adds no firmware interaction. EL0 is not granted
  any firmware device mapping — the process's address space contains
  only its text, its stack, and the kernel image (which it must not
  touch; enforcement is the page tables' business, already in place).

## Design

### Data structures

The central structure already exists: `swtch::Context` (x19–x30, sp,
spsr; 112 B, fits the stack page the test asserts). It stays unchanged.
New state is exactly two words, in a new `aarch64::process` module:

```text
kernel_return_ctx: *const Context   // null while no process runs
exit_status:       u64              // written by the exit trap, read by run()
```

No `Process`/PCB struct: with one process and no scheduler, a struct is
a form without content (see Decision 3). Ownership: `process::run`
creates both; the exit trap writes `exit_status` and consumes
`kernel_return_ctx`; `run` nulls the slot on resumption. No fact is
stored twice: the process's register state lives only in its `Context`
on its own stack; the kernel's only in the context `swtch` saved on the
kernel stack.

### Interfaces

One public function (day-one users: `main9`, the `user_process` test
image):

```rust
// aarch64/src/process.rs
pub fn run(text: &[u8], text_va: usize, stack_va: usize) -> u64
```

Preconditions: TTBR0 is the user table, and interrupts are fully
brought up (`boot::interrupts`; `main9` already does this, the test
image adds it). `run` allocates the text page at `text_va` and the
stack page at `stack_va` (existing `pagealloc::allocate_virtpage`),
writes `text`, builds the `Context` at the top of the stack (x30 =
`text_va`, spsr = `0x0`: EL0, SP0, DAIF unmasked, IL = 0 — the CPSR
state encoding, Arm ARM DDI 0487 SPSR_EL1), records
`kernel_return_ctx`, switches, and returns `exit_status` once the exit
trap has switched back.

The process always enters with IRQs unmasked; there is no masked-entry
variant (panel finding: the `irq_on` knob was a decision the author
refused to make, exported as interface — whole-system lens). The test image gets
the EL0→EL1 proof by doing full bringup, not by dodging it.

Syscall ABI (first edition): `svc #<n>`, `n` = `ESR_EL1.ISS`;
**0 = exit**; arguments ignored (status recorded as 0). Defined in
`aarch64::process`, not `port` — a second arch re-derives its own
encoding (Decision 4).

`swtch` gains a `pub unsafe fn` wrapper in `swtch.rs` (today
`pub(crate)` + `#[expect(dead_code)]`), documenting the protocol:
`from` receives the current context's address; `to` must be a context
built by a prior save or by a starter; the call does not return until
that context is switched back to.

### Init and bringup order

Load-bearing orderings, stated:
1. `boot::irq_ops()` before any masking (DAIF ops) — unchanged.
2. User table initialized and made live **before** `process::run`
   (precondition, checked by a debug assert on the root table's
   liveness — or left to the caller; the test and main9 both already do
   it; decision: documented precondition, no runtime check).
3. `kernel_return_ctx` recorded **before** the switch into EL0 — if an
   IRQ preempts between, the worst case is a timer handler running
   with the slot set and no process yet in EL0; the exit trap cannot
   fire without an EL0 `svc`, so the ordering is safe as stated.
4. Vector preamble masks IRQs before any handler work (new; see Failure
   policy).

### Failure policy

- **Unimplemented syscall** (`n != 0`): print the number and the frame,
  then kill the process as an exit with status `n` — a stray `svc`
  must not hang the machine, and returning to a user program that will
  just re-issue it is a livelock. (Decision 5.)
- **Exit trap with no `kernel_return_ctx`**: unreachable; print and spin
  (loud, debuggable), like the existing unknown-exception path.
- **Unknown exception**: unchanged (dump frame, spin).
- **Interrupts in the exit path**: masked by the vector preamble
  (`msr daifset, #2` in `handle_interrupt`; the DAIFSet immediate
  encodes D,A,I,F as bits 3..0, so `#2` is the I bit — the same
  immediate `irq.rs` already uses), so the exit handler's
  `swtch` cannot be preempted by a timer into the still-live trap frame
  on the single fixed interrupt stack. `eret` restores the interrupted
  context's DAIF from the hardware `SPSR_EL1`, so the masking leaks
  nowhere. FIQ/SError nesting remains possible — none is used on this
  target; noted, not fixed.
- Allocation failure in `run`: it runs at boot in `main9` (init-only
  context) and in the test image; `panic!` is acceptable there, matching
  `main9`'s existing style. No panic is reachable from trap context:
  the exit trap only writes two words and calls `swtch`.

## Not building

- **Scheduler / proc table / NPROC**: one process, one caller. The
  moment a second process or a blocking `run` is wanted, this becomes
  the `proc` redesign; designing it now is speculation (same rule as
  `timer-table-portability`).
- **A `Process`/PCB struct**: nothing to hold beyond two words.
- **Syscall table / table dispatch**: one syscall; a `match` on the
  immediate is the table.
- **Signal-style return-to-faulting-instruction for exit**: exit never
  returns to the process; there is no "resume the process" path to
  build.
- **Per-process page tables / teardown**: the first process shares the
  kernel's user table; reclaiming its two pages on exit is deferred
  (they are test allocations in the test, boot-time in `main9`).
- **x86-64/riscv64 equivalents**: out of scope by request; the design
  keeps every change inside the `aarch64` crate so the gates stay green
  without cross-arch surface.

## Decision records

- **Decision: the exit trap returns via `swtch` to the saved kernel
  context, not via `eret`.**
  - Alternatives: `eret` to a hand-built EL1 state — rejected: hardware
    `eret` restores no general registers, so x19–x30 of the kernel
    caller would be whatever the trap left in them.
  - Dissent: none; it follows from the register rules, not taste.
- **Decision: `swtch` takes the saved context's resume state as a
  caller-supplied argument, not from any register.**
  - Alternatives: read CPSR as the AArch32 original does — impossible;
    AArch64 has no CPSR read (caught by the first build: `mrs x5,
    cpsr` does not assemble). Initialize `spsr_el1` at boot and
    maintain an invariant that nothing clobbers it — rejected: the
    hardware writes it on every trap to EL1; the invariant would be
    false by construction. A constant in the assembly — rejected: the
    DAIF half of the value depends on the caller's interrupt state,
    which only the caller knows.
  - Dissent: none on the argument shape; the hardware-truth lens
    supplies the forcing function. Recorded because the plan originally
    specified the impossible CPSR read — the build gate, not the panel,
    caught it.
- **Decision: no `Process` struct; two module statics.**
  - Alternatives: a `Process { entry, context, status }` allocated per
    run — rejected as form without content at one user (clarity
    lens: the struct's fields would each have exactly one writer and one
    reader).
  - Dissent: the kernel-taste lens notes that the second process will
    want the struct; we accept one refactor then rather than carrying
    dead fields now.
- **Decision: syscall encoding stays in `aarch64`, not `port`.**
  - Alternatives: `port::syscall` with per-arch encoding — rejected
    until a second arch needs it (YAGNI; `port` changes cost a gate
    trip on three arches).
  - Dissent: the whole-system lens wants the ABI visible from `port`
    once userspace exists; agreed it moves then, with the userspace,
    not before.
- **Decision: unimplemented syscalls kill the process (status = n).**
  - Alternatives: spin (today's behaviour) — a hang, and the test
    image's `check!` would never report; return to EL0 — a livelock for
    a program that re-issues.
  - Dissent: Plan 9 sends a `syserror` and lets the process continue
    where sensible; with no other syscalls there is no sensible
    continuation for the first process.
- **Decision: the first process starts with IRQs enabled, and there is
  no knob to vary it; both callers do full interrupt bringup.**
  - Alternatives: a `run(…, irq_on: bool)` parameter so the test image
    could enter masked — rejected on panel (whole-system lens: a tunable is a
    decision refused; kernel-taste lens: a boolean parameter makes one function
    do two things). Masked entry itself — rejected as policy: the
    EL0→EL1 IRQ vector would stay unexercised and the timer would
    silently stop during the process, hiding exactly the class of bug
    (kernel state reachable from EL0) this milestone should flush out.
  - Dissent: the firmware/hardware lens prefers least privilege at
    first contact; the vector-preamble masking plus the proven
    interrupt stack make the risk bounded, and the integration test
    will show it immediately.

## Tasks

1. `tasks/swtch-spsr-cpsr.md` — the one-line save-side fix, the `pub
   unsafe fn swtch` wrapper (SAFETY comment stating the protocol),
   protocol docs. First, because everything else stands on the switch
   being trustworthy.
2. `tasks/trap-svc-exit.md` — vector-preamble IRQ masking; SVC dispatch
   in `trap.rs`; the exit path (status + `swtch` back, with a why-
   comment on the switch call — it is the subtlest line in the design);
   unimplemented syscalls kill.
3. `tasks/process-run.md` — `aarch64::process::run` (page setup,
   context build, the switch, resumption) and the two module statics
   (SAFETY comments on the statics' accessors).
4. `tasks/user-process-switch-test.md` — extend the `user_process`
   image across the line it currently stops short of: add
   `boot::interrupts`, take the switch, let the process exit, assert
   after return, `qemu::exit(PASS)`.
5. `tasks/main9-run-first-process.md` — `main9` prints before, runs the
   process, prints after; remove the now-dead scratch in the test
   image's doc comment. Sequenced last: the feature is "done" when the
   kernel image, not just the test, does it.
