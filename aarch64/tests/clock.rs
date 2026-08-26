//! Integration test: `SYS_CLOCK` — the arch's monotonic counter, a register
//! read.
//!
//! A whole kernel image like the others: it links the kernel library and runs
//! its own `main9`.  It reads the clock twice (with the counter guaranteed to
//! advance between them — the kernel's own tick runs) and checks the delta is
//! positive (the clock is monotonic).  It checks a bad kind is refused (the
//! error code, not a fault).  The `SYS_RECEIVE_AT` blocking wait is host-tested
//! (the `Mock` scheduler's deadline logic); this image exercises the register
//! read, the part only on-device.

#![no_std]
#![no_main]

use aarch64::{boot, ipc, mailbox, qemu};
use port::println;

#[macro_use]
mod common;

#[unsafe(no_mangle)]
pub extern "C" fn main9(dtb_va: usize) {
    boot::irq_ops();
    let dt = unsafe { boot::device_tree(dtb_va) };
    boot::page_allocator(&dt, dtb_va).unwrap();
    mailbox::init(&dt);
    boot::console(&dt);
    boot::interrupts(&dt);

    println!("running clock");

    // The clock is a register read: two reads, the second strictly after the
    // first (the counter is always running, no tick needed in between).
    let t0 = ipc::sys_clock(0);
    let t1 = ipc::sys_clock(0);
    println!("clock: t0={t0:#x}, t1={t1:#x}, delta={:#x}", t1 - t0);
    check!(t1 > t0, "clock must be monotonic: t0={t0:#x}, t1={t1:#x}");

    // A bad kind is refused: the error code (not a fault, not a tick count).
    let bad = ipc::sys_clock(1);
    println!("clock kind 1: {bad:#x}");
    check!(bad != 0, "a bad clock kind must be refused (non-zero)");

    println!("clock passed");
    qemu::exit(qemu::PASS);
}
