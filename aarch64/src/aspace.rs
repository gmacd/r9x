//! The per-process address space (stage 3 of the microkernel substrate,
//! `_tasks/plans/microkernel-aspace.md`): a page-table root for the process's
//! TTBR0, and the physical address of that root.
//!
//! The kernel stays in TTBR1 (unreachable from EL0 — its entries are `Priv*`),
//! so switching a process's TTBR0 is one register write and leaves every
//! kernel mapping reachable.  The process's TTBR0 is empty except its own
//! text/stack (mapped by `process::spawn` through [`Aspace::map_user_page`]);
//! a process reaches the kernel by syscall, not a mapped device page.  A fault
//! in one process walks its *own* tables and kills only that process — the
//! isolation property.  A server maps its device's MMIO through
//! [`Aspace::map_mmio`] (stage 5: the console server's UART register page).
//!
//! The real binding is target-only; the host build (unit tests of the
//! process/trap modules) sees a stub so they compile.

#[cfg(target_os = "none")]
use crate::kmem::physaddr_as_ptr_mut_offset_from_kzero;
#[cfg(target_os = "none")]
use crate::pagealloc;
#[cfg(target_os = "none")]
use crate::param::KZERO;
#[cfg(target_os = "none")]
use crate::vm::{
    self, Entry, PhysPageAllocator, RootPageTable, RootPageTableType, VaMapping, VmTraitImpl,
};
#[cfg(target_os = "none")]
use port::mem::{PAGE_SIZE_4K, PhysAddr, PhysRange};
#[cfg(target_os = "none")]
use port::pagealloc::PageAllocError;

/// A process's address space: its TTBR0 page-table root and the physical
/// address of that root (what TTBR0 holds).  The root lives in a pagealloc'd
/// page (one per process, bounded by `NPROCS` — not a `static`).  The raw
/// pointer carries the "valid for the process's life; the page is not freed
/// this arc" lifetime honestly (the established `process.rs` note).
///
/// Teardown (a later arc) must unmap both the TTBR0 entries (the process's
/// text/stack) *and* the corresponding TTBR1 identity entries (the kernel
/// mappings of the same pages, added by `new` and `map_user_page`) before
/// the pages are freed — otherwise the kernel table leaks mappings into
/// freed pages.
#[cfg(target_os = "none")]
pub struct Aspace {
    /// The process's TTBR0 root, in a pagealloc'd page.  Written only by this
    /// AS's `map_user_page` (before the process is reachable) and read by the
    /// fault handler; the process table's slot discipline keeps two owners
    /// apart.
    root: *mut RootPageTable,
    /// The physical address of the root — what TTBR0 holds.
    root_pa: PhysAddr,
}

// SAFETY: the root is written only by this AS's own `map_user_page` (before
// the process is reachable) and read by the fault handler; a slot's `Aspace`
// is never freed or reused while a raw pointer to it is live (the process
// table's discipline), so no two owners interleave.
#[cfg(target_os = "none")]
unsafe impl Sync for Aspace {}

#[cfg(target_os = "none")]
impl Aspace {
    /// Create an address space: allocate a root page, map it into the kernel
    /// table (TTBR1) at its identity VA so the kernel can write it, zero it,
    /// set the index-511 self-pointer, and record its physical address.
    /// Panics on allocation or mapping failure (init-only context, as
    /// `spawn`'s page allocs do); there is no `Default` because a default AS
    /// would allocate.
    ///
    /// The map-into-TTBR1 step is the load-bearing one: a pagealloc page is in
    /// *available* memory, which the kernel table does not identity-map (only
    /// the kernel's own sections are), so the root page must be mapped before
    /// it can be written — the same step `deviceutil::alloc_device_page`
    /// takes.
    #[allow(clippy::new_without_default)]
    pub fn new() -> Aspace {
        let root_pa = pagealloc::allocate_physpage()
            .unwrap_or_else(|err| panic!("aspace: root page: {err:?}"));
        let range = PhysRange::with_pa_len(root_pa, PAGE_SIZE_4K);
        // Map the root page into the kernel table at its identity VA (pa +
        // KZERO), so the kernel can write it.  Reuses the intermediate tables
        // the kernel's own mapping already built.
        let mut physpage_allocator = PhysPageAllocator {};
        let mut vmtrait_impl = VmTraitImpl {};
        vm::kernel_pagetable()
            .map_phys_range(
                &mut physpage_allocator,
                &mut vmtrait_impl,
                "aspace-root",
                &range,
                VaMapping::Offset(KZERO),
                Entry::rw_kernel_data(),
                crate::vm::PageSize::Page4K,
                RootPageTableType::Kernel,
            )
            .unwrap_or_else(|err| panic!("aspace: map root page: {err:?}"));
        // The mapped VA is pa + KZERO (the identity offset); the root table
        // lives there for the kernel's writes.
        let root = physaddr_as_ptr_mut_offset_from_kzero::<RootPageTable>(root_pa);
        unsafe {
            // A fresh pagealloc page is not zeroed; the table must be all
            // invalid entries before the self-pointer is written.
            core::ptr::write_bytes(root as *mut u8, 0, PAGE_SIZE_4K);
            // The index-511 self-pointer: lets the recursive walk build this
            // table while a different one is live in TTBR0.
            vm::init_empty_root_page_table(&mut *root);
        }
        Aspace { root, root_pa }
    }

    /// The physical address to install in TTBR0 for this AS.
    pub fn ttbr0(&self) -> PhysAddr {
        self.root_pa
    }

    /// Install this AS's root in TTBR0: the process's address space becomes
    /// live.  The kernel stays in TTBR1 (unreachable from EL0).  The caller
    /// holds the IRQ mask (the switch path does) so the TLBI/DSB/ISB window
    /// is not preempted.
    ///
    /// # Safety
    /// The AS must be fully constructed (its root mapped in TTBR1 by `new`)
    /// and remain live for as long as it is installed.
    pub unsafe fn install(&self) {
        // SAFETY: the caller guarantees the AS is live; the root is mapped in
        // TTBR1 (the kernel table) by `new`, so the dereference is valid and
        // the table remains live while installed.
        unsafe { vm::switch(&*self.root, RootPageTableType::User) };
    }

    /// Map a physical page into this AS at `va` (the process's own text/stack)
    /// *and* into the kernel table (TTBR1) at its identity VA, so the kernel
    /// can reach the page.  Returns the kernel pointer (the identity VA,
    /// `pa + KZERO`) — the one the kernel writes through to load initial
    /// contents (the text).  The user mapping at `va` is what the process
    /// sees; the two map the same physical page.
    ///
    /// The kernel mapping is load-bearing: the kernel runs in TTBR1, which does
    /// not map the user half, so it cannot write through the user VA — the
    /// same physical page must be reachable in TTBR1 for the text copy.
    pub fn map_user_page(&self, entry: Entry, va: usize) -> Result<*mut u8, PageAllocError> {
        // Allocate the physical page first (unmapped), so it can be mapped
        // into both tables.
        let page_pa = pagealloc::allocate_physpage()?;
        let range = PhysRange::with_pa_len(page_pa, PAGE_SIZE_4K);
        let mut physpage_allocator = PhysPageAllocator {};
        let mut vmtrait_impl = VmTraitImpl {};
        // The user mapping: the process sees the page at `va`.
        unsafe { &mut *self.root }
            .map_phys_range(
                &mut physpage_allocator,
                &mut vmtrait_impl,
                "aspace-user",
                &range,
                VaMapping::Addr(va),
                entry,
                crate::vm::PageSize::Page4K,
                RootPageTableType::User,
            )
            .map_err(|_| PageAllocError::UnableToMap)?;
        // The kernel mapping: the kernel reaches the same page at its identity
        // VA (pa + KZERO) in TTBR1, so it can write the text.
        vm::kernel_pagetable()
            .map_phys_range(
                &mut physpage_allocator,
                &mut vmtrait_impl,
                "aspace-kern",
                &range,
                VaMapping::Offset(KZERO),
                Entry::rw_kernel_data(),
                crate::vm::PageSize::Page4K,
                RootPageTableType::Kernel,
            )
            .map_err(|_| PageAllocError::UnableToMap)?;
        Ok(physaddr_as_ptr_mut_offset_from_kzero::<u8>(page_pa))
    }

    /// Map a device's MMIO range into this AS at `va` (the process sees the
    /// device registers).  Does NOT map into TTBR1 (the kernel does not need
    /// the device page; the server owns it exclusively).  The range must be
    /// page-aligned and ≤ one page for this arc.
    pub fn map_mmio(&self, range: &PhysRange, va: usize) -> Result<(), PageAllocError> {
        let mut physpage_allocator = PhysPageAllocator {};
        let mut vmtrait_impl = VmTraitImpl {};
        unsafe { &mut *self.root }
            .map_phys_range(
                &mut physpage_allocator,
                &mut vmtrait_impl,
                "aspace-mmio",
                range,
                VaMapping::Addr(va),
                Entry::rw_user_mmio(),
                crate::vm::PageSize::Page4K,
                RootPageTableType::User,
            )
            .map_err(|_| PageAllocError::UnableToMap)?;
        Ok(())
    }
}

// Host builds (unit tests of the process/trap modules) see a stub so those
// modules compile; it is never called (the aspace path is target-only).
#[cfg(not(target_os = "none"))]
pub struct Aspace;

#[cfg(not(target_os = "none"))]
impl Aspace {
    #[allow(clippy::new_without_default)]
    pub fn new() -> Aspace {
        loop {
            core::hint::spin_loop();
        }
    }
    pub fn ttbr0(&self) -> usize {
        0
    }
    pub fn map_user_page(&self, _entry: u64, _va: usize) -> *mut u8 {
        core::ptr::null_mut()
    }
    pub fn map_mmio(&self, _range: &port::mem::PhysRange, _va: usize) {}
}
