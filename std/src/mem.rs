//! Memory services: the user-space MMIO mapping and the process heap.
//!
//! `map_mmio` is the `SYS_MAP_MMIO` passthrough (a device server maps a
//! device's registers at a chosen VA).  The heap is a `brk`-style top watermark
//! in the process's own address space, backed by the kernel: `alloc` grows it
//! with `SYS_ALLOC` (page-granular — a sub-page request rounds up to a page);
//! `dealloc` is `brk`-style free-the-top — the most-recent allocation can be
//! given back with `SYS_FREE`, any other is a no-op (a general free that
//! coalesces middle holes is a refinement; the kernel keeps the released pages
//! mapped and reuses them on the next grow, so nothing is unmapped).

use alloc::alloc::{GlobalAlloc, Layout};
use core::cell::Cell;
use core::ptr;
use r9x_abi::{PAGE_SIZE, SYS_ALLOC, SYS_ALLOC_PAGE, SYS_FREE, SYSMAPMMIO};

use crate::sys::sys;

/// The whole-page count an allocation of `size` bytes takes (a sub-page
/// request rounds up to one page): the kernel's grant granularity.  Shared by
/// `alloc` (the request) and `dealloc` (free-the-top's grant size), so the two
/// agree on how many pages a grant spans.  Pure, so the rounding is
/// host-checkable even though the `r9x_std` test binary cannot link (its
/// lang items clash with the host's).
fn pages_for(size: usize) -> usize {
    size.div_ceil(PAGE_SIZE).max(1)
}

/// Map a physical MMIO page into this process's address space at `va`.  The
/// kernel maps one 4 KiB Device page; the caller knows the register layout.
/// Returns the kernel's result (an error code on failure — an access to an
/// unmapped page is a fault the kernel's EL0 path handles).
pub fn map_mmio(phys: u64, va: u64, size: u64) -> u64 {
    unsafe { sys(SYSMAPMMIO, phys, va, size, 0, 0).0 }
}

/// Allocate a page in this process's heap and return both the virtual and
/// physical address.  The physical address is needed by a server that talks
/// to a device which DMA-reads or writes a buffer (the BCM283x Mailbox takes
/// a physical address in its write register).  Returns `None` on failure.
pub fn alloc_page() -> Option<(usize, u64)> {
    let (va, pa, _) = unsafe { sys(SYS_ALLOC_PAGE, 0, 0, 0, 0, 0) };
    let va = va as usize;
    if va < PAGE_SIZE {
        return None;
    }
    Some((va, pa))
}

/// The process's heap: a `brk`-style top watermark in its own address space,
/// backed by the kernel.  `brk` is the current top (page-aligned; 0 before the
/// first grant) and moves with the kernel's: `alloc` extends it, free-the-top
/// lowers it.
struct Heap {
    /// The current top watermark (page-aligned; 0 before the first grant).
    brk: Cell<usize>,
}

// SAFETY: the heap's watermark is a plain `usize`; the process owns the memory
// it bounds (the kernel maps it into this process's TTBR0).  It is read and
// written behind `&self` from the global allocator; there are no threads this
// arc, so the `Cell`'s interior mutability is never raced.
unsafe impl Sync for Heap {}

static HEAP: Heap = Heap { brk: Cell::new(0) };

#[global_allocator]
static GLOBAL: Global = Global;

struct Global;

unsafe impl GlobalAlloc for Global {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // Whole pages: the kernel's granularity, so a sub-page request rounds
        // up — and every grant is page-aligned, which satisfies any alignment a
        // real type asks for (none exceeds the page size).
        let pages = pages_for(layout.size());
        let needed = pages * PAGE_SIZE;
        let va = unsafe { sys(SYS_ALLOC, needed as u64, 0, 0, 0, 0).0 } as usize;
        // The kernel returns the grant's start (page-aligned, at or above the
        // first heap page) or a small error code (1) when the grant is refused
        // (top bound or OOM); a heap VA is never that small.
        if va < PAGE_SIZE {
            return ptr::null_mut();
        }
        HEAP.brk.set(va + needed);
        va as *mut u8
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        let ptr = ptr as usize;
        let pages = pages_for(layout.size());
        let top = ptr + pages * PAGE_SIZE;
        // Free-the-top: only the most-recent allocation can be given back —
        // lower the watermark and tell the kernel (which keeps the pages mapped
        // for the next grow).  Any other `ptr` is a no-op (the bump limitation;
        // a general free is a refinement).
        if top == HEAP.brk.get() {
            HEAP.brk.set(ptr);
            unsafe {
                sys(SYS_FREE, ptr as u64, 0, 0, 0, 0);
            }
        }
    }
}
