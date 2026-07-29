//! DAIF-based implementation of the portable interrupt masking hooks.

use port::irq::IrqOps;

static IRQ_OPS: IrqOps = IrqOps { mask: mask_irqs, restore: restore_irqs };

/// Register DAIF masking with `port::irq`.  Must be called before
/// interrupts are enabled.
pub fn init() {
    port::irq::set_ops(&IRQ_OPS);
}

/// Unmask IRQs on this core.  Call once the core's interrupt controller
/// is initialised — on aarch64 that means after a successful `gic::init`
/// on the boot core, and after the per-core GIC bringup on each
/// secondary.
///
/// Boot path only.  This writes DAIF directly rather than going through
/// `IrqOps`, so it does not compose with `IrqGuard`: called inside a
/// guard's scope it unmasks, and the guard then re-masks on drop with
/// no diagnostic.  Named to match `mask_irqs`/`restore_irqs` below, and
/// deliberately *not* `enable_interrupts` — `gic::enable_interrupt`
/// already means something else entirely (enable one INTID at the
/// distributor).
///
/// Deliberately not `nomem`: this is the point where the core starts
/// taking interrupts, so the compiler must not move memory accesses
/// across it.
pub fn unmask_irqs() {
    unsafe {
        core::arch::asm!("msr DAIFClr, #2", options(nostack, preserves_flags));
    }
}

/// Mask IRQs on this core, returning the previous DAIF state.
fn mask_irqs() -> u64 {
    let daif: u64;
    unsafe {
        core::arch::asm!(
            "mrs {daif}, daif",
            "msr daifset, #2",
            daif = out(reg) daif,
            options(nostack, preserves_flags)
        );
    }
    daif
}

/// Restore a DAIF state previously returned by `mask_irqs`.
fn restore_irqs(daif: u64) {
    unsafe {
        core::arch::asm!(
            "msr daif, {daif}",
            daif = in(reg) daif,
            options(nostack, preserves_flags)
        );
    }
}
