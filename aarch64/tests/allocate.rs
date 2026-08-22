//! Integration test: the kernel page allocator and the global heap.
//!
//! A whole kernel image like the others: it links the kernel library and
//! runs its own `main9`, bringing up just enough (page allocator + console)
//! to allocate.  The page-allocator half maps three fresh kernel pages and
//! writes through each one; the heap half drives the global QuickFit
//! allocator with small and large boxes.  Neither is reachable from a host
//! unit test -- the assertions are about pages and heap blocks that only
//! exist on a live, booted machine.
#![no_std]
#![no_main]

extern crate alloc;

use aarch64::param::KZERO;
use aarch64::vm::{Entry, RootPageTableType, VaMapping};
use aarch64::{boot, pagealloc, qemu, vm};
use alloc::boxed::Box;
use port::println;

#[macro_use]
mod common;

#[unsafe(no_mangle)]
pub extern "C" fn main9(dtb_va: usize) {
    // The console maps its UART through the page allocator, so the
    // allocator comes up first.  No mailbox, timer, or interrupts: nothing
    // here takes an interrupt, so the timer and GIC stay off.
    let dt = unsafe { boot::device_tree(dtb_va) };
    boot::page_allocator(&dt, dtb_va).unwrap();
    boot::console(&dt);

    println!("running allocate");

    // --- Kernel page allocator -----------------------------------------
    // Baseline so the growth check below measures only these allocations.
    let (used_before, _total) = pagealloc::usage_bytes();
    let page_table = vm::kernel_pagetable();
    let entry = Entry::rw_kernel_data();
    let mut vas: [usize; 3] = [0; 3];
    for (i, va_slot) in vas.iter_mut().enumerate() {
        let page = match pagealloc::allocate_virtpage(
            page_table,
            "testkernel",
            entry,
            VaMapping::Offset(KZERO),
            RootPageTableType::Kernel,
        ) {
            Ok(page) => page,
            Err(e) => {
                println!("FAIL  kernel page {i} did not allocate: {e:?}");
                qemu::exit(qemu::FAIL);
            }
        };
        let va = page as *mut _ as usize;
        check!(va >= KZERO, "kernel page {i} is in the kernel half ({va:#x})");
        // A write through the returned pointer that reads back is the live
        // mapping: a bad one would fault, not return.
        page.0[0] = 0xa5;
        check!(page.0[0] == 0xa5, "kernel page {i} is readable and writable");
        *va_slot = va;
    }
    check!(
        vas[0] != vas[1] && vas[1] != vas[2] && vas[0] != vas[2],
        "the three kernel pages are distinct ({:#x} {:#x} {:#x})",
        vas[0],
        vas[1],
        vas[2]
    );
    let (used_after, _total) = pagealloc::usage_bytes();
    check!(
        used_after > used_before,
        "usage grew after allocation ({used_before:#x} -> {used_after:#x})"
    );

    // --- Global heap (QuickFit) ----------------------------------------
    let a: Box<u32> = Box::new(0x1234u32);
    check!(*a == 0x1234u32, "small heap allocation round-trips");
    let mut big = Box::new([0u8; 4096]);
    big[0] = 0xff;
    big[big.len() - 1] = 0x5a;
    check!(big[0] == 0xff && big[big.len() - 1] == 0x5a, "large heap allocation is usable");
    let a_addr = &*a as *const u32 as usize;
    let big_addr = big.as_ptr() as usize;
    check!(a_addr != big_addr, "heap allocations are distinct ({a_addr:#x} vs {big_addr:#x})");
    drop(a);
    let c: Box<u64> = Box::new(0xdead_beefu64);
    check!(*c == 0xdead_beefu64, "allocation after free round-trips");

    println!("allocate passed");
    qemu::exit(qemu::PASS);
}
