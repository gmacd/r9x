//! Integration test: the state SBI and l.S left us in.
//!
//! This is a whole kernel image.  It links the same library the kernel
//! binary does, so `start` in l.S runs the usual early boot and then calls
//! the `main9` below instead of the real one.
//!
//! None of this can be checked by a host unit test: every assertion is
//! about what firmware handed a supervisor that has just started.
#![no_std]
#![no_main]

use port::println;
use r9x_core::fdt::DeviceTree;
use riscv64::platform::{devcons, platform_init};
use riscv64::qemu;

#[macro_use]
mod common;

#[unsafe(no_mangle)]
pub extern "C" fn main9(hartid: usize, dtb_ptr: usize) -> ! {
    // The console is all this test needs, and it is what lets it report.
    let dt = unsafe { DeviceTree::from_usize(dtb_ptr).unwrap() };
    devcons::init(&dt);
    platform_init();

    println!("running boot");

    // SBI hands the supervisor the hart it started on.  Any hart may be
    // the boot hart, but virt has four, so the number has to be one of
    // them rather than uninitialised rubbish.
    check!(hartid < 4, "boot hart is one of the machine's ({hartid})");

    // The device tree pointer is the other half of the SBI handover, and
    // has to point at a blob that parses -- it already has, above.
    check!(dtb_ptr != 0, "dtb handed over ({dtb_ptr:#x})");
    check!(dt.size() > 0, "dtb has a size ({} bytes)", dt.size());

    // We are running from where the linker script put us, which for this
    // kernel is the address SBI was told to jump to.
    let here = main9 as *const () as usize;
    check!(here >= 0x8020_0000, "executing from the load address ({here:#x})");

    // Reading the DTB back through the same pointer must still work after
    // the console has mapped its UART: nothing has clobbered the blob.
    let again = unsafe { DeviceTree::from_usize(dtb_ptr).unwrap() };
    check!(again.size() == dt.size(), "dtb still intact after console init");

    println!("boot passed");
    qemu::exit(qemu::PASS);
}
