use port::Result;
use port::mem::{PhysRange, VirtRange};
use r9x_core::fdt::DeviceTree;

use crate::vm::{PhysPageAllocator, VmTraitImpl};
use crate::{pagealloc, vm};

/// Probe the device tree for a node matching one of the given compatible
/// strings (tried in order), take its first `reg` block, and return a
/// `PhysRange`.  Returns an error if no node or its `reg` is found.
pub fn find_dt_physrange(
    dt: &DeviceTree,
    compatibles: &'static [&'static str],
    err: &'static str,
) -> Result<PhysRange> {
    let reg = compatibles
        .iter()
        .find_map(|c| dt.find_compatible(c).next())
        .and_then(|node| dt.property_translated_reg_iter(node).next())
        .and_then(|reg| reg.regblock())
        .ok_or(err)?;
    // A reg with no length is a probe error, not an extent: reject it here
    // rather than map a zero-size range that would abort on first access.
    PhysRange::from_regblock(&reg).ok_or("device reg has no length (size_cells == 0)")
}

/// Map a device register to device memory
/// TODO Maybe make this a macro and wrap the error reporting?
pub fn map_device_register(
    id: &'static str,
    physrange: PhysRange,
    page_size: vm::PageSize,
) -> Result<VirtRange> {
    let page_physrange = physrange.round(page_size.size());

    let mut physpage_allocator = PhysPageAllocator {};
    let mut vmtrait_impl = VmTraitImpl {};

    if let Ok(vr) = vm::kernel_pagetable().map_phys_range(
        &mut physpage_allocator,
        &mut vmtrait_impl,
        id,
        &page_physrange,
        vm::next_free_device_page4k(),
        vm::Entry::rw_device(),
        page_size,
        vm::RootPageTableType::Kernel,
    ) {
        let offset = vr.start - page_physrange.start.addr() as usize;
        Ok(VirtRange::from_physrange(&physrange, offset))
    } else {
        Err("failed to map device register")
    }
}

/// Map a buffer to device memory
/// TODO Maybe make this a macro and wrap the error reporting?
pub fn alloc_device_page(
    id: &'static str,
    page_size: vm::PageSize,
) -> Result<(VirtRange, PhysRange)> {
    let page_pa = pagealloc::allocate_physpage().expect("couldn't allocate page");
    let page_physrange = PhysRange::with_pa_len(page_pa, page_size.size());

    let mut physpage_allocator = PhysPageAllocator {};
    let mut vmtrait_impl = VmTraitImpl {};

    if let Ok(vr) = vm::kernel_pagetable().map_phys_range(
        &mut physpage_allocator,
        &mut vmtrait_impl,
        id,
        &page_physrange,
        vm::next_free_device_page4k(),
        vm::Entry::rw_device(),
        page_size,
        vm::RootPageTableType::Kernel,
    ) {
        Ok((vr, page_physrange))
    } else {
        Err("failed to map device buffer")
    }
}
