//! Integration test: the tick preempts.
//!
//! A and B are both busy loops (no yield) that each run for half a
//! second of *real* time, paced against the physical counter
//! (CNTPCT_EL0/CNTFRQ_EL0, EL0-readable because `timer::init` set the
//! CNTKCTL_EL1 enable bits); A exits status 3, B status 4, via the
//! catch-all.  Real-time pacing is what makes the test machine-
//! independent: both the loop duration and the 100 ms tick period are
//! real time, so any runner yields roughly ten preemptions whether
//! its TCG is fast or slow, while a fixed instruction count is a
//! different duration per machine and the count rides on machine
//! speed.  After `run_all`: assert both statuses **and**
//! `preemptions() >= 2`.
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

/// Run for half a second of *real* time, then exit with status 3.
/// Reads the counter frequency and paces the loop against the
/// physical counter (both EL0-readable now that `timer::init` set
/// CNTKCTL_EL1's EL0 enable bits), so the duration is the same on a
/// fast or slow TCG runner: a fixed instruction count would be a
/// different duration per machine and the preemption count would ride
/// on machine speed.  `mov x8, #3` first: x8 survives the loop, so
/// the exit needs no setup after it.  Assembled with
/// `clang -target arm64` and re-checked by objdump; there is no
/// assembler in the tree.
const PROG_A: [u8; 0x24] = [
    0x68, 0x00, 0x80, 0xd2, // mov x8, #3
    0x00, 0xe0, 0x3b, 0xd5, // mrs x0, cntfrq_el0
    0x00, 0xfc, 0x41, 0xd3, // lsr x0, x0, #1
    0x21, 0xe0, 0x3b, 0xd5, // mrs x1, cntpct_el0
    0x24, 0x00, 0x00, 0x8b, // add x4, x1, x0
    0x22, 0xe0, 0x3b, 0xd5, // a_loop: mrs x2, cntpct_el0
    0x5f, 0x00, 0x04, 0xeb, // cmp x2, x4
    0xcb, 0xff, 0xff, 0x54, // b.lt a_loop
    0x01, 0x00, 0x00, 0xd4, // svc #0
];

/// Same shape, exits status 4.
const PROG_B: [u8; 0x24] = [
    0x88, 0x00, 0x80, 0xd2, // mov x8, #4
    0x00, 0xe0, 0x3b, 0xd5, // mrs x0, cntfrq_el0
    0x00, 0xfc, 0x41, 0xd3, // lsr x0, x0, #1
    0x21, 0xe0, 0x3b, 0xd5, // mrs x1, cntpct_el0
    0x24, 0x00, 0x00, 0x8b, // add x4, x1, x0
    0x22, 0xe0, 0x3b, 0xd5, // b_loop: mrs x2, cntpct_el0
    0x5f, 0x00, 0x04, 0xeb, // cmp x2, x4
    0xcb, 0xff, 0xff, 0x54, // b.lt b_loop
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
    let a = process::spawn(&process::Image::Raw {
        text: &PROG_A,
        text_va: A_TEXT_VA,
        stack_va: A_STACK_VA,
    });
    let b = process::spawn(&process::Image::Raw {
        text: &PROG_B,
        text_va: B_TEXT_VA,
        stack_va: B_STACK_VA,
    });

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
