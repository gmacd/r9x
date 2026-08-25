//! Integration test: the kernel heap — a process's `alloc` grows on demand.
//!
//! A whole kernel image like the others: it links the kernel library and runs
//! its own `main9`.  It brings up the user-space machinery (user page tables,
//! the process switch) and spawns the heaptask — a Rust-built ELF (built by
//! xtask's ServerStep, embedded here, loaded by `spawn`) that drives the
//! `r9x_std` heap: a small block, a `Vec` past the old 16 KiB static footprint
//! (the load-bearing case — the static heap could not serve it), a free-the-top
//! (`SYS_FREE`), and a regrow.  The kernel backs every grant with real pages
//! mapped into the process's own TTBR0 (no kernel identity map — the heap is
//! the process's to use).
//!
//! Two heaptasks are spawned, to show the heap is per-process: each keeps its
//! own watermark in its own `Aspace`, so one's allocation never touches the
//! other's.  The image runs them to a fixpoint and checks both are still alive
//! — blocked at the end of their body.  A mapping fault (a grant not actually
//! mapped) ends the writing process with the fault status, and a failed marker
//! check panics (the handler exits); either way the process is no longer alive,
//! so liveness is the check.  The top-bound *error* (not a fault into the MMIO
//! region) is proven at the cursor, host-side: `brk_grow` returns `None` where
//! it would cross the user-half edge (the arch's unit test) — the bound itself
//! is far above QEMU's physical memory, so it is not reached on-device.

#![no_std]
#![no_main]

use aarch64::{boot, mailbox, process, qemu, vm};
use port::println;

#[macro_use]
mod common;

/// The built heaptask's ELF, embedded: xtask's `ServerStep` builds it (static,
/// non-PIE, linked at the shared image base), this crate's `build.rs` stages it
/// into `OUT_DIR`, and `include_bytes!` pulls the bytes in.  The loader reads
/// it through `Image::Elf`.
static HEAPTASK_ELF: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/heaptask.elf"));

#[unsafe(no_mangle)]
pub extern "C" fn main9(dtb_va: usize) {
    // Vectors first: without them a fault in a process goes nowhere at all
    // rather than reaching the handler that records the fault status.
    boot::irq_ops();
    let dt = unsafe { boot::device_tree(dtb_va) };
    boot::page_allocator(&dt, dtb_va).unwrap();
    mailbox::init(&dt);
    boot::console(&dt);
    boot::interrupts(&dt);

    println!("running heap");

    unsafe {
        vm::init_user_page_tables();
        vm::switch(vm::user_pagetable(), vm::RootPageTableType::User);
    }

    // Two heaptasks: the heap is per-process, so spawn two to show each keeps
    // its own watermark in its own `Aspace` and one's allocation never touches
    // the other's.  No handles — the heaptask allocates and blocks; it does not
    // message the image, so it is handed no spawner-passed pair.
    let a = process::spawn(&process::Image::Elf { bytes: HEAPTASK_ELF, handles: None });
    let b = process::spawn(&process::Image::Elf { bytes: HEAPTASK_ELF, handles: None });
    println!("heaptask x2 spawned (a={a}, b={b}), running");

    process::run_all();

    let sa = process::status(a);
    let sb = process::status(b);
    println!("heaptask a status {sa:?}, b status {sb:?}");
    // Both must be alive — blocked at the end of their body, past the large
    // allocation, the free, and the regrow.  A grant that was not actually
    // mapped would fault the writing process (the fault status); a failed
    // marker check would panic (the handler exits).  Either ends the process,
    // so an ended process is the failure.
    check!(sa.is_none(), "heaptask a alive past its allocations, got {sa:?}");
    check!(sb.is_none(), "heaptask b alive (heap isolated from a), got {sb:?}");

    println!("heap passed");
    qemu::exit(qemu::PASS);
}
