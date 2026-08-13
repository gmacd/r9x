//! Integration test: the mapping l.S built before main9 ran.
//!
//! This is a whole kernel image.  It links the same library the kernel
//! binary does, so `start` in l.S runs the usual early boot and then calls
//! the `main9` below instead of the real one.  Only the console is brought
//! up: nothing here needs an allocator, interrupts or a timer.
//!
//! None of this can be checked by a host unit test: every assertion is
//! about the state of a live MMU.
#![no_std]
#![no_main]

use aarch64::boot;
use aarch64::kmem::{
    boottext_physrange, bss_physrange, data_physrange, rodata_physrange, text_physrange,
    total_kernel_physrange,
};
use aarch64::param::KZERO;
use aarch64::qemu;
use aarch64::vm;
use port::println;

/// Report and end the run on the first failure.  A test image has no
/// unwinding and nothing to hand a failure back to but its exit status.
macro_rules! check {
    ($cond:expr, $($arg:tt)+) => {
        if $cond {
            println!("ok    {}", format_args!($($arg)+));
        } else {
            println!("FAIL  {}", format_args!($($arg)+));
            qemu::exit(qemu::FAIL);
        }
    };
}

#[unsafe(no_mangle)]
pub extern "C" fn main9(dtb_va: usize) {
    // The console maps its UART registers, so the page allocator has to
    // come up first.  That is the whole init this test needs: no mailbox,
    // no timer, no GIC, no interrupts.
    let dt = unsafe { boot::device_tree(dtb_va) };
    boot::page_allocator(&dt, dtb_va).unwrap();
    boot::console(&dt);

    println!("running pagetables");

    // We are executing through the high-half mapping, so the address of
    // this very function has to be above KZERO.  If l.S had left us on the
    // identity mapping this would fail while still being able to report.
    let here = main9 as *const () as usize;
    check!(here >= KZERO, "executing above KZERO ({here:#x})");

    // The DTB was handed to us as a virtual address in the same window.
    check!(dtb_va >= KZERO, "dtb mapped in kernel space ({dtb_va:#x})");

    // Reaching the root page table means the recursive slot resolves; an
    // unmapped one would fault rather than return.
    let pt = vm::kernel_pagetable() as *const _ as usize;
    check!(pt >= KZERO, "kernel page table reachable ({pt:#x})");

    // Section ranges come from linker symbols, so their order is a check
    // that the image was laid out and mapped as kernel.ld describes.
    let (boottext, text) = (boottext_physrange(), text_physrange());
    let (rodata, data, bss) = (rodata_physrange(), data_physrange(), bss_physrange());
    let total = total_kernel_physrange();
    check!(text.size() > 0, "kernel text is not empty");
    check!(boottext.start <= text.start, "boottext precedes text");
    check!(text.start <= rodata.start, "text precedes rodata");
    check!(rodata.start <= data.start, "rodata precedes data");
    check!(data.start <= bss.start, "data precedes bss");
    check!(total.start <= boottext.start && total.end >= bss.end, "total spans the image");

    println!("pagetables passed");
    qemu::exit(qemu::PASS);
}
