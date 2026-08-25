//! Initialisation, as separately callable steps.
//!
//! The kernel binary runs all of these in order.  An integration test runs
//! the prefix it needs and stops, so that what it exercises is not buried
//! under initialisation it does not care about.
//!
//! The steps are ordered by dependency, and the dependencies are real: the
//! console maps its UART registers, so it cannot come up before the page
//! allocator that backs those mappings.

use port::mem::{PhysAddr, PhysRange};
use port::pagealloc::PageAllocError;
use r9x_core::fdt::DeviceTree;

use crate::kmem::{
    boottext_physrange, bss_physrange, data_physrange, rodata_physrange, text_physrange,
};
use crate::param::KZERO;
use crate::vm::PageSize;
use crate::{devcons, gic, irq, pagealloc, timer, trap};

/// Parse the device tree the boot code left us.
///
/// # Safety
/// `dtb_va` must be the virtual address of a device tree blob, as passed to
/// main9 by the boot code.
pub unsafe fn device_tree(dtb_va: usize) -> DeviceTree<'static> {
    unsafe { DeviceTree::from_usize(dtb_va).unwrap() }
}

/// Hand the page allocator every physical range the kernel already owns, so
/// that it hands out only free memory.  Everything that maps anything —
/// including the console — depends on this.
pub fn page_allocator(dt: &DeviceTree, dtb_va: usize) -> Result<(), PageAllocError> {
    let dtb_physrange = PhysRange::with_pa_len(PhysAddr::new((dtb_va - KZERO) as u64), dt.size());
    let mut physranges = [
        dtb_physrange.round(PageSize::Page4K.size()),
        boottext_physrange().add(&text_physrange()),
        rodata_physrange(),
        data_physrange().add(&bss_physrange()),
    ];
    physranges.sort_by_key(|a| a.start);
    pagealloc::init_page_allocator(dt, physranges.into_iter())
}

/// Bring up the console.  Requires [`page_allocator`].
pub fn console(dt: &DeviceTree) {
    devcons::init(dt);
}

/// Register the DAIF operations IrqGuard needs.  Takes no device tree and
/// must precede anything that takes a lock shared with interrupt context.
pub fn irq_ops() {
    irq::init();
    trap::init();
}

/// GIC, then timer, then unmask — the order is load bearing, and is
/// explained at the call site in the kernel binary.  Requires [`irq_ops`].
/// Both inits panic on failure (see their modules' failure policy), so
/// reaching the unmask means a driver is there to acknowledge whatever
/// arrives.
pub fn interrupts(dt: &DeviceTree) {
    gic::init(dt);
    timer::init(dt);
    irq::unmask_irqs();
}
