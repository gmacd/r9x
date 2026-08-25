//! Memory services: the user-space MMIO mapping and the heap front.
//!
//! The heap is a static buffer for now: the current servers allocate nothing
//! (fixed per-request buffers, no `Vec`), so a static heap is the honest
//! starting point.  Tier 1.1 swaps the backing for the kernel's
//! `SYS_ALLOC` / `SYS_FREE` — the same `#[global_allocator]`, a different
//! backing, and no change to the servers.

use alloc::alloc::{GlobalAlloc, Layout};
use core::ptr;
use core::sync::atomic::{AtomicUsize, Ordering};
use r9x_abi::SYSMAPMMIO;

use crate::sys::sys;

/// Map a physical MMIO page into this process's address space at `va`.  The
/// kernel maps one 4 KiB Device page; the caller knows the register layout.
/// Returns the kernel's result (an error code on failure — an access to an
/// unmapped page is a fault the kernel's EL0 path handles).
pub fn map_mmio(phys: u64, va: u64) -> u64 {
    unsafe { sys(SYSMAPMMIO, phys, va, 0, 0, 0).0 }
}

/// The static heap's size: 16 KiB.  A constant, not a guess about a load that
/// does not exist yet — it covers the small per-request buffers a server may
/// grow to use, and Tier 1.1's kernel heap makes it moot.
const HEAP_SIZE: usize = 16 * 1024;

/// The process heap, installed as the r9 target's global allocator.
#[global_allocator]
static ALLOC: Bump = Bump { base: [0u8; HEAP_SIZE], offset: AtomicUsize::new(0) };

/// A bump allocator over a static buffer: allocations are aligned and handed
/// out from a single monotonically advancing offset, and nothing is ever freed
/// (a bump heap's trade, and acceptable while no server allocates).  Tier 1.1
/// replaces the backing with the kernel's heap without changing this type.
struct Bump {
    base: [u8; HEAP_SIZE],
    offset: AtomicUsize,
}

// `Bump` is shared by every thread that touches the allocator; `AtomicUsize`
// makes the offset race-free, and the `base` bytes are never mutated after
// start, so the type is `Sync`.
unsafe impl GlobalAlloc for Bump {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let size = layout.size();
        let align = layout.align();
        // Claim a suitably-aligned slot for `layout` with a compare-and-swap so
        // a concurrent allocation cannot land in the same bytes.  The alignment
        // is computed on the *absolute* address, not the offset: the `base` bytes
        // sit at an address the compiler only aligns to the struct (8 here), so
        // aligning the offset alone would hand back a misaligned pointer for any
        // `align` above that.
        let base_addr = self.base.as_ptr() as usize;
        let limit = base_addr + self.base.len();
        loop {
            let off = self.offset.load(Ordering::SeqCst);
            let start = (base_addr + off).div_ceil(align) * align;
            let end = start + size;
            if end > limit {
                return ptr::null_mut();
            }
            if self
                .offset
                .compare_exchange_weak(
                    off,
                    start - base_addr + size,
                    Ordering::SeqCst,
                    Ordering::SeqCst,
                )
                .is_ok()
            {
                // The returned pointer is in-bounds of `base` (checked above),
                // aligned to `layout.align()`, and disjoint from every prior
                // allocation (the offset only ever advances).
                return start as *mut u8;
            }
        }
    }

    unsafe fn dealloc(&self, _ptr: *mut u8, _layout: Layout) {
        // A bump heap never frees: allocations live to the end of the process.
    }
}
