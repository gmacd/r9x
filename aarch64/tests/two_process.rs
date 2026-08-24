//! Integration test: two processes in the table, run serially.
//!
//! A (exits 3) and B (exits 4) are spawned, and `run_all` runs the
//! table to empty.  With no tick yet, B can run only because A's exit
//! drives `resched`: the image proves the table, the kstack vector
//! entry, the TPIDR current-pointer, the forkret entry (both processes
//! enter through the vector tail), the depth bracketing around a
//! mid-handler `swtch`, and the serial reschedule chain, end to end.
//! A host-side test could never prove any of the switch machinery
//! works at all.
//!
//! The programs exit straight away: there is no preemption this arc
//! (the tick that gives a spinning process a share of the CPU is the
//! next task), so a spin loop would only delay the very exit that
//! drives the reschedule.  The asserts on both statuses are what make
//! it a test: a broken `resched` would leave B Runnable forever,
//! `run_all` would never return, and the image would time out — but a
//! `resched` that switched to the wrong slot, or lost a status, would
//! return with a wrong or missing status and fail here.
#![no_std]
#![no_main]

use aarch64::vm::RootPageTableType;
use aarch64::{boot, mailbox, process, qemu, vm};
use port::println;

#[macro_use]
mod common;

/// A: exit with status 3 (`mov x8, #3; svc #0`; x8 is the syscall
/// register).  Assembled with `clang -target arm64` and re-checked by
/// objdump; there is no assembler in the tree.
const PROG_A: [u8; 8] = [
    0x68, 0x00, 0x80, 0xd2, // mov x8, #3
    0x01, 0x00, 0x00, 0xd4, // svc #0
];

/// B: exit with status 4.
const PROG_B: [u8; 8] = [
    0x88, 0x00, 0x80, 0xd2, // mov x8, #4
    0x01, 0x00, 0x00, 0xd4, // svc #0
];

/// Where each process's text and stack are mapped.  Distinct per
/// process: the user table maps one page per (va) per process, but
/// the pages themselves are separate allocations.
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
    mailbox::init(&dt);
    boot::console(&dt);
    // The processes enter with IRQs unmasked, so the interrupt
    // machinery must be up: the timer keeps firing while they run,
    // and its tick is what makes the kstack entry and the depth
    // bracketing work under interrupt load.
    boot::interrupts(&dt);

    println!("running two_process");

    unsafe {
        vm::init_user_page_tables();
        vm::switch(vm::user_pagetable(), RootPageTableType::User);
    }

    println!("spawning two processes");
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

    // A's exit drives resched to B; B's exit unwinds run_all.
    println!("running the table");
    process::run_all();
    println!("table ran to empty");

    let sa = process::status(a);
    let sb = process::status(b);
    println!("status a {sa:?}, status b {sb:?}");

    check!(sa == Some(3), "A exited 3, got {sa:?}");
    check!(sb == Some(4), "B exited 4, got {sb:?}");

    println!("two_process passed");
    qemu::exit(qemu::PASS);
}
