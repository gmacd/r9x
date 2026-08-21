//! Integration test: the tick preempts.
//!
//! A and B are both busy loops (no yield, a counter in x19) of
//! 0xFFFFFFFF iterations — several seconds under QEMU TCG, so each
//! process survives many 100 ms tick periods; A exits status 3, B
//! status 4, via the catch-all.  The decrement is `subs`, not `sub`:
//! the loop's `b.ne` reads the flag the decrement sets (plain `sub`
//! leaves NZCV untouched and the branch tests a stale flag).  After
//! `run_all`: assert both statuses **and** `preemptions() >= 2`.
//!
//! The status pair alone would pass on a timeline where nothing is
//! preempted (A runs to completion, B runs second).  `>= 2` requires
//! repeated tick *delivery* while processes run — exactly what a
//! stranded-EOI regression breaks: switching from inside the timer
//! callback leaves the suspended context holding the CVAL re-arm and
//! the EOI, so it allows precisely one preemption and then self-heals
//! when an exit resumes the suspended handler.  The count is
//! incremented only on a switch that actually happened, so the
//! self-heal (which resumes tick delivery but switches only on exits
//! after that) cannot pad it back over the threshold.
//!
//! No host-side test could prove any of the switch machinery works;
//! the assertions make this a test rather than a smoke run.

#![no_std]
#![no_main]

use aarch64::vm::RootPageTableType;
use aarch64::{boot, process, qemu, vm};
use port::println;

#[macro_use]
mod common;

/// Busy loop of 0xFFFFFFFF iterations (x19 = 0xffffffff), then exit
/// with status 3 (`mov x8, #3; svc #0`; x8 is the syscall register).
/// Assembled with `clang -target arm64` and re-checked by objdump;
/// there is no assembler in the tree.
const PROG_A: [u8; 0x18] = [
    0xf3, 0xff, 0x9f, 0xd2, // movz x19, #0xffff
    0xf3, 0xff, 0xbf, 0xf2, // movk x19, #0xffff, lsl #16
    0x73, 0x06, 0x00, 0xf1, // a_loop: subs x19, x19, #1
    0xe1, 0xff, 0xff, 0x54, // b.ne a_loop
    0x68, 0x00, 0x80, 0xd2, // mov x8, #3
    0x01, 0x00, 0x00, 0xd4, // svc #0
];

/// Same shape, exits status 4.
const PROG_B: [u8; 0x18] = [
    0xf3, 0xff, 0x9f, 0xd2, // movz x19, #0xffff
    0xf3, 0xff, 0xbf, 0xf2, // movk x19, #0xffff, lsl #16
    0x73, 0x06, 0x00, 0xf1, // b_loop: subs x19, x19, #1
    0xe1, 0xff, 0xff, 0x54, // b.ne b_loop
    0x88, 0x00, 0x80, 0xd2, // mov x8, #4
    0x01, 0x00, 0x00, 0xd4, // svc #0
];

/// Distinct per process: the user page table is shared, so a second
/// spawn at the same VA overwrites the first process's code.
const A_TEXT_VA: usize = 0x1000;
const A_STACK_VA: usize = 0x10000;
const B_TEXT_VA: usize = 0x2000;
const B_STACK_VA: usize = 0x20000;

#[unsafe(no_mangle)]
pub extern "C" fn main9(dtb_va: usize) {
    // Vectors first: without them a fault here goes nowhere at all
    // rather than reaching a handler that can report it.
    boot::irq_ops();
    let dt = unsafe { boot::device_tree(dtb_va) };
    boot::page_allocator(&dt, dtb_va).unwrap();
    boot::console(&dt);
    // The processes enter with IRQs unmasked, so the interrupt
    // machinery must be up: the tick is what this image tests.
    boot::interrupts(&dt);

    println!("running preempt");

    unsafe {
        vm::init_user_page_tables();
        vm::switch(vm::user_pagetable(), RootPageTableType::User);
    }

    println!("spawning two busy loops");
    let a = process::spawn(&PROG_A, A_TEXT_VA, A_STACK_VA);
    let b = process::spawn(&PROG_B, B_TEXT_VA, B_STACK_VA);

    println!("running the table");
    process::run_all();

    let status_a = process::status(a);
    let status_b = process::status(b);
    let preemptions = process::preemptions();
    println!("status a {status_a:?}, status b {status_b:?}, preemptions {preemptions}");

    check!(status_a == Some(3), "A exited 3, got {status_a:?}");
    check!(status_b == Some(4), "B exited 4, got {status_b:?}");
    check!(preemptions >= 2, "tick preemption, got {preemptions}");

    println!("preempt passed");
    // A failed check! panics above and hangs the image; the timeout
    // then fails it.  qemu::exit is diverging, so there is nothing
    // after it.
    qemu::exit(0);
}
