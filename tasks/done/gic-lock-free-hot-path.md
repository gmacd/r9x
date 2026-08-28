---
status: done
---

# Make the GIC interrupt path lock-free: replace Lock<Option<Gic>> ✅ DONE

`static GIC: Lock<Option<Gic>>` was replaced with `AtomicPtr<Gic>`. `Gic` holds two `VirtRange`s that never change after init, its methods take `&mut self` but mutate no field, and the hot-path registers
(GICC_IAR/GICC_EOIR) are per-CPU banked, needing no mutual exclusion —
verified against Linux, where `gic_handle_irq` reads INTACK with a bare
`readl_relaxed` and its lock guards only SGI cpu-map bookkeeping
(/Volumes/Code/repos/linux/drivers/irqchip/irq-gic.c).

Cost today: every interrupt takes IrqGuard + LockNode + MCS acquisition up
to three separate times (try_ack_interrupt, disable_interrupt,
end_interrupt), plus `Option` unwrap boilerplate in every pub fn. The lock
protects the _placement_ of the driver, not any shared state.

Fix: store the mapped ranges in a set-once static — the `AtomicPtr`
pattern this same commit relies on for `IRQ_OPS` in port/src/irq.rs — and
the interrupt path becomes lock-free and the Option checks disappear.
(Distributor registers are RMW-free by design here too: ISENABLER/ICENABLER
are set/clear-on-write.) Note the mailbox's lock is not precedent: it
protects a shared request buffer; there is no shared mutable state here.

Interacts with: gic-init-ordering.md (publish point becomes the static
store — order it before delivery-enable) and the `with_gic` item in
gic-timer-review-nits.md (moot once the lock is gone).

Done when: no lock acquisition on the ack/EOI path; init publishes
exactly once via atomic store; SMP-correct (init→handler happens-before
guaranteed).

Implementation:
- `static GIC: AtomicPtr<Gic> = AtomicPtr::new(ptr::null_mut())`
- `init()` publishes via `Box::into_raw(Box::new(gic))` + `store(ptr, Release)` with IrqGuard around publish
- `Gic` methods `&mut self` → `&self` (no shared mutable state)
- Hot path: `load(Acquire)` + null-check + unsafe deref — no MCS, no LockNode, no IrqGuard
- `#[inline(always)]` on all three pub functions

Panel review: no blockers or should-fixes (kernel-taste + microkernel-and-firmware). Nits fixed:
- IrqGuard comment made succinct
- `#[inline(always)]` added to interrupt-path functions

Origin: panel review of 46a59c9 (kernel-taste + simplicity lenses).
