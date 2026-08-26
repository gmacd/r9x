//! The runtime glue a r9 executable needs that the platform `std` would
//! otherwise provide: the entry's tail (recording the DTB VA, running the
//! program's `main`, exiting), the panic handler, and the runtime facts the
//! loader and spawner pass in (the DTB VA, the spawner-passed handles).

use core::panic::PanicInfo;
use core::sync::atomic::{AtomicUsize, Ordering};
use r9x_abi::HANDLES_VA;

use crate::process::exit;

/// The DTB virtual address the kernel mapped read-only into this process,
/// passed as the first entry argument.  Recorded once by [`run`]; zero until
/// then (a real DTB VA is never zero).
static DTB_VA: AtomicUsize = AtomicUsize::new(0);

/// The runtime entry: record the DTB VA the kernel mapped in (the
/// `main9(dtb_va)` convention) and run the program's `main`.  A binary's
/// `#[no_mangle] start` forwards to this, passing its own `main`; the kernel
/// jumps to `start` with the DTB VA as the first argument.  If `main` returns
/// — a program bug, since a server runs until killed — the process exits
/// cleanly.
pub fn run<F>(dtb_va: usize, main: F) -> !
where
    F: FnOnce(),
{
    DTB_VA.store(dtb_va, Ordering::SeqCst);
    main();
    exit(0);
}

/// The DTB virtual address the kernel mapped into this process (see [`run`]).
/// Zero until the kernel maps and passes the DTB to a user entry — it does not
/// yet (the console-via-FDT arc), so this is zero for the current servers, who
/// know their own device addresses.
pub fn dtb_va() -> usize {
    DTB_VA.load(Ordering::SeqCst)
}

/// The device tree at the DTB VA, ready to query (parsed by
/// [`r9x_core::fdt`]).  Panics if the kernel has not passed a DTB to this
/// process yet (see [`dtb_va`]) or the DTB is malformed.
pub fn device_tree() -> r9x_core::fdt::DeviceTree<'static> {
    let va = dtb_va();
    if va == 0 {
        // The kernel does not yet map and pass the DTB to a user entry (that
        // is the console-via-FDT arc); fail clearly rather than parse address
        // zero.
        panic!("r9x_std::rt::device_tree: no DTB passed to this process");
    }
    // SAFETY: `va` is the VA the kernel mapped the DTB read-only into this
    // process for its whole lifetime, so the `'static` borrow the parser takes
    // of that memory is sound.
    unsafe { r9x_core::fdt::DeviceTree::from_usize(va).unwrap() }
}

/// This process's spawner-passed channel pair, read from the `HANDLES_VA`
/// page the spawner wrote into before this process's first instruction.  The
/// page is the generalized child-state header —
/// `[n_handles:4 LE][handle:4 LE ...][argc:4 LE][argv ...]` (the layout
/// [`r9x_abi::SPAWN_MAX_HANDLES`] documents) — and a server's state is a pair
/// (`n_handles = 2`): this returns the first two handles, the channel pair.
/// A state with fewer handles (a `SYS_SPAWN` child handed a bare value, or
/// none) is not a pair; a caller that must distinguish reads the raw page.
pub fn handles() -> (u32, u32) {
    let p = HANDLES_VA as *const u32;
    // SAFETY: the spawner wrote the header to this page before this process's
    // first instruction; the two 32-bit reads (the first two handles, at
    // offsets 4 and 8, under the count) are in-bounds of that page.
    unsafe { (core::ptr::read_volatile(p.add(1)), core::ptr::read_volatile(p.add(2))) }
}

/// The spawner-passed child-state's handle count: the first word of the
/// `HANDLES_VA` page (the generalized header's `n_handles`).  A server's state
/// is a pair (2); a `SYS_SPAWN` child with no state is a zero page (0).  A
/// spawner that handed a pair and a child that read none (0) have a spawner
/// bug, and the count — not the first handle — is the check (channel 0 is a
/// valid handle: the table is indexed from 0).
pub fn n_handles() -> u32 {
    let p = HANDLES_VA as *const u32;
    // SAFETY: the spawner wrote the header (or the kernel zeroed the page);
    // the read is in-bounds of that page.
    unsafe { core::ptr::read_volatile(p) }
}

/// The panic handler: r9's panic strategy is to end the process.  A server
/// that panics is a bug, but the process must not spin the machine or corrupt
/// other processes — it exits and the kernel reclaims it.
#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    exit(0);
}
