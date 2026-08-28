---
status: done
---

# BLOCKER: dispatch traps on the saved vector type, not a GIC probe

`trap()` (aarch64/src/trap.rs:113) opens with `gic::try_ack_interrupt()` on
every trap — synchronous exceptions included — and `return`s if anything was
pending, while the `interrupt_type` that trap.S stamps into the TrapFrame
(trap.S:86-87, stored at trap.rs:60) is never read by anything but the Debug
impl. Reading GICC_IAR is the acknowledge (GICv2 spec, IHI 0048B §4.4.4),
not a passive query.

Verified failure modes:
1. An SVC or fault taken while any IRQ is pending is consumed *as* the IRQ:
   ack, EOI, `return` — ELR already points past the SVC, so the syscall is
   silently dropped. Latent only because 46a59c9 comments out
   `test_sysexit()`; with a 1 Hz periodic timer running it is a matter of
   time once syscalls return.
2. A genuine IRQ-vector entry where IAR reads spurious (1023 — allowed under
   GICv2 when a level source deasserts between signal and ack) falls through
   to the ESR decode; ESR_EL1 is stale on IRQ entry, so this lands in the
   terminal spin loop (or misreads as a syscall).
3. Every future syscall/fault pays an acking MMIO read + IrqGuard + MCS lock
   even when nothing is pending — hot-path tax for the common case.

Fix: branch on `frame.interrupt_type` (IRQ vectors → GIC ack/handle/EOI
path, treating spurious IAR as "handled, no-op"; sync vectors → ESR path).
Prior art: Plan 9 `bcm/trap.c` switches on `ureg->type` and only `PsrMirq`
reaches `irq()` (/Volumes/Code/repos/plan9).

Done when: the GIC is touched only from the IRQ arm; a pending IRQ at SVC
entry no longer swallows the syscall; spurious IAR on an IRQ vector is a
harmless return.

Origin: panel review of 46a59c9 — flagged independently by all six lenses.
