//! The heaptask: a minimal user process that drives the `r9x_std` heap.
//!
//! It is the load-bearing proof that `alloc` is real: it allocates a small
//! block, then a `Vec` past the old 16 KiB static footprint (the case the
//! static heap could not serve), writes and reads markers through the granted
//! pages, frees the top back with `SYS_FREE`, and regrows.  A mapping fault
//! would end the process with the fault status before it reaches the end, and a
//! failed check would panic (the panic handler exits) — either way it would no
//! longer be blocked at the end, which is the `heap` image's success check.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::vec;
use alloc::vec::Vec;
use r9x_std::{ipc, rt};

/// The entry point: where the loader sets `e_entry`.  Forwards to `r9x_std`'s
/// runtime, which records the DTB VA the kernel mapped in and calls this
/// process's [`main`].
#[inline(never)]
#[unsafe(no_mangle)]
pub extern "C" fn start(dtb_va: usize) -> ! {
    rt::run(dtb_va, main)
}

/// A small allocation: one whole page, round-tripped.  Exercises the first
/// grant, which learns the kernel's heap base.
fn small_block() -> Vec<u32> {
    let mut v = vec![0x1234u32; 4];
    v[0] = 0xdead;
    v[3] = 0xbeef;
    assert_eq!(v[0], 0xdead);
    assert_eq!(v[3], 0xbeef);
    v
}

/// A `Vec` past the old 16 KiB static footprint: 64 KiB is sixteen whole pages,
/// so it cannot be served from a fixed buffer — only from a heap that grows by
/// `SYS_ALLOC`.  Mark the first and last bytes and read them back; a bad
/// mapping would fault on the write, not return a wrong byte.
fn large_block() -> Vec<u8> {
    let mut v = vec![0u8; 64 * 1024];
    let last = v.len() - 1;
    v[0] = 0xa5;
    v[last] = 0x5a;
    assert_eq!(v[0], 0xa5);
    assert_eq!(v[last], 0x5a);
    v
}

/// The process body: allocate through the kernel-backed heap, free the top,
/// regrow, and block (alive).
fn main() {
    // A small block: the first grant, which learns the kernel's heap base.
    let mut small = small_block();

    // A large block past the old 16 KiB static footprint: the load-bearing case
    // (sixteen whole pages), unserviceable from a fixed buffer.
    let large = large_block();

    // Free-the-top: drop the large block — the `dealloc` lowers the heap top
    // and calls `SYS_FREE`; the pages stay mapped.  The regrow below reuses
    // them (a grow does not re-map pages already mapped), so the allocation
    // that follows a free is the case that would break if the watermark were
    // forgotten.
    drop(large);

    // Regrow past the freed top: 24 pages, reusing the sixteen freed plus eight
    // new.  Mark and re-read as the others.
    let regrown = large_block_regrown();

    // The small block — a live, non-top allocation — survived the free and the
    // regrow untouched: re-check its first marker.
    small[0] = 0xcafe;
    assert_eq!(small[0], 0xcafe);

    // Done: give both back (regrown first — the top — then small, a no-op free
    // because it is no longer the most recent allocation; the bump limitation).
    drop(regrown);
    drop(small);

    // Block on a receive no one satisfies: the live, done state.  The `heap`
    // image checks this process is still here (a fault or a panic would have
    // ended it first).
    let (in_h, _out_h) = ipc::create_pair();
    let mut buf = [0u8; ipc::MSG_MAX];
    loop {
        let _ = ipc::receive(in_h, &mut buf);
    }
}

/// A regrow block: 96 KiB is twenty-four whole pages — more than the freed
/// sixteen, so the grant reuses the released pages (which stay mapped) and maps
/// the rest fresh.  Mark the first and last bytes and read them back.
fn large_block_regrown() -> Vec<u8> {
    let mut v = vec![0u8; 96 * 1024];
    let last = v.len() - 1;
    v[0] = 0x3c;
    v[last] = 0xc3;
    assert_eq!(v[0], 0x3c);
    assert_eq!(v[last], 0xc3);
    v
}
