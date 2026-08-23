//! DAIF-based implementation of the portable interrupt masking hooks.

use port::irq::IrqOps;

static IRQ_OPS: IrqOps = IrqOps { mask: mask_irqs, restore: restore_irqs, core_id: current_core };

/// The I bit of DAIF: set masks IRQs on the core.
#[cfg(test)]
const DAIF_IRQ_BIT: u64 = 1 << 7;

/// Hosted test builds run at EL0, where the DAIF accesses below trap as
/// undefined, so the tests run against a mock of the register in the
/// `reg::cnt_el0` style.  It starts masked, like a booting core.
#[cfg(test)]
static MOCK_DAIF: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(DAIF_IRQ_BIT);

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
#[cfg(not(test))]
pub fn unmask_irqs() {
    unsafe {
        core::arch::asm!("msr DAIFClr, #2", options(nostack, preserves_flags));
    }
}

#[cfg(test)]
pub fn unmask_irqs() {
    MOCK_DAIF.fetch_and(!DAIF_IRQ_BIT, core::sync::atomic::Ordering::Relaxed);
}

/// Mask IRQs on this core, returning the previous DAIF state.
#[cfg(not(test))]
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

#[cfg(test)]
fn mask_irqs() -> u64 {
    MOCK_DAIF.fetch_or(DAIF_IRQ_BIT, core::sync::atomic::Ordering::Relaxed)
}

/// Restore a DAIF state previously returned by `mask_irqs`.
#[cfg(not(test))]
fn restore_irqs(daif: u64) {
    unsafe {
        core::arch::asm!(
            "msr daif, {daif}",
            daif = in(reg) daif,
            options(nostack, preserves_flags)
        );
    }
}

#[cfg(test)]
fn restore_irqs(daif: u64) {
    MOCK_DAIF.store(daif, core::sync::atomic::Ordering::Relaxed);
}

/// The calling core's id: MPIDR_EL1 Aff0, the core-within-cluster id
/// (bits [7:0]; Arm ARM DDI 0487, MPIDR_EL1) — the same field `l.S` masks to
/// decide whether to run at all.  Aff0 is a unique core index only on
/// single-cluster targets; every supported target (Pi 4, QEMU `q35`/`virt`)
/// is single-cluster with fewer cores than `port::irq::MAX_CPUS`, so this is
/// a valid depth-table index.  A multi-cluster target would need Aff0 + Aff1;
/// none is supported, so that is refused here, not guessed at.
#[cfg(not(test))]
fn current_core() -> usize {
    let mpidr: u64;
    unsafe {
        core::arch::asm!(
            "mrs {0}, mpidr_el1",
            out(reg) mpidr,
            options(nostack, preserves_flags)
        );
    }
    (mpidr & 0xff) as usize
}

#[cfg(test)]
fn current_core() -> usize {
    0
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::sync::atomic::Ordering;

    /// The calls themselves are the point: before the mock, each of
    /// these trapped as undefined at EL0 and killed the test binary
    /// with SIGILL.
    #[test]
    fn mask_restore_roundtrip() {
        unmask_irqs();
        let prev = mask_irqs();
        assert_eq!(prev & DAIF_IRQ_BIT, 0, "unmask_irqs should have cleared the I bit");
        assert_ne!(
            MOCK_DAIF.load(Ordering::Relaxed) & DAIF_IRQ_BIT,
            0,
            "mask_irqs should set the I bit"
        );
        restore_irqs(prev);
        assert_eq!(
            MOCK_DAIF.load(Ordering::Relaxed) & DAIF_IRQ_BIT,
            0,
            "restore_irqs should reinstate the unmasked state"
        );
    }
}
