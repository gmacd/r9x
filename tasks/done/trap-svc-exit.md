---
status: done
---

# Trap: mask IRQs in the vector preamble; dispatch SVC; exit returns to the kernel

Two gaps in the synchronous path, per plans/first-user-process.md:

1. **The vector preamble does not mask IRQs.** `handle_interrupt`
   switches to the interrupt stack and runs handlers with the
   interrupted DAIF. Today all handlers run in EL1 with the single
   fixed interrupt stack, so an IRQ landing inside a handler pushes a
   second frame onto the still-live one. The exit path makes this live:
   the exit handler calls `swtch` while its trap frame is on that
   stack. Add `msr daifset, #2` (I bit; the DAIFSet immediate encodes
   D,A,I,F as bits 3..0 — the same immediate irq.rs uses) at the top of
   `handle_interrupt`. `eret` restores the interrupted context's DAIF
   from the hardware SPSR_EL1, so the masking leaks nowhere.
2. **The SVC path prints and spins.** `trap()`'s `esr_el1.ec() == 0x15`
   arm (EC 0x15 = SVC from AArch64, ISS = the svc immediate, Arm ARM
   DDI 0487 ESR_EL1) must become a dispatch:
   - **exit** (`aarch64::process::SYSEXIT`, 0): record the status in
     the process module, iprintln, then `swtch` back to
     `kernel_return_ctx`. With a why-comment: hardware `eret` cannot
     restore x19–x30, so returning to the kernel that started the
     process is a context switch, not an exception return. If
     `kernel_return_ctx` is null: print and spin (unreachable, loud).
   - **anything else**: print the number and the frame, then treat it
     as an exit with that number as status — a stray `svc` must not
     hang the machine, and returning to a program that re-issues it is
     a livelock.

The exit handler must not allocate and must not be able to panic: it
writes two words and calls `swtch` (the plan's failure policy).

Sequenced after swtch-spsr-cpsr (the switch-back stands on it) and
together with process-run (which owns the two statics and SYSEXIT).

Done when: the preamble masks; SVC dispatch exists with the two arms;
`cargo xtask` gates clean on all three arches; unit tests cover the
decode that is host-testable (ISS extraction) the way trap.rs' existing
tests do.

Origin: plans/first-user-process.md — findings #2–3.

## Status: done

Landed on the first-user-process branch ahead of process-run.  The
exit trap needed the process module's two statics and SYSEXIT to
exist, so that first half of process-run (module, statics, accessors,
with SAFETY comments) landed here; process-run adds `run` on top.
The dispatch treats exit and unimplemented numbers uniformly (kill
with the number as status), so it is one arm, not two.  The swtch
wrapper's scheduled `#[expect(dead_code)]` failed as designed with
the first caller.  `cargo xtask ci` green: 41 tests (aarch64 11→12,
the SVC decode), dist ×3, 4/4 integration.
