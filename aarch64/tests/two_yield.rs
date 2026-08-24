//! Two processes that yield back and forth and then exit.
//!
//! A (yield, yield, exit 6) and B (yield, exit 7) are spawned, and
//! `run_all` runs the table to empty.  The expected dance: A yields,
//! B first-enters through the vector tail, B yields, A resumes inside
//! its own suspended yield handler, A yields again, B resumes inside
//! its handler, B exits, A resumes a third time and exits.  The
//! image proves, end to end, the two resume shapes (first entry
//! through the vector tail, and resume inside a suspended handler),
//! the TPIDR current-pointer handoff at every switch — a stale
//! TPIDR makes B's yield resched from A and starve the table — and
//! the round-robin cursor.
//!
//! No host-side test could prove any of the switch machinery works;
//! the assertions on both statuses make it a test rather than a
//! smoke run.

#![no_std]
#![no_main]

use aarch64::vm::RootPageTableType;
use aarch64::{boot, mailbox, process, qemu, vm};
use port::println;

#[macro_use]
mod common;

/// A: yield, yield, exit with status 6.  x8 is the syscall register.
/// Assembled with `clang -target arm64` and re-checked by objdump;
/// there is no assembler in the tree.
const PROG_A: [u8; 0x18] = [
    0x28, 0x00, 0x80, 0xd2, // mov x8, #1
    0x01, 0x00, 0x00, 0xd4, // svc #0
    0x28, 0x00, 0x80, 0xd2, // mov x8, #1
    0x01, 0x00, 0x00, 0xd4, // svc #0
    0xc8, 0x00, 0x80, 0xd2, // mov x8, #6
    0x01, 0x00, 0x00, 0xd4, // svc #0
];

/// B: yield, exit with status 7.
const PROG_B: [u8; 16] = [
    0x28, 0x00, 0x80, 0xd2, // mov x8, #1
    0x01, 0x00, 0x00, 0xd4, // svc #0
    0xe8, 0x00, 0x80, 0xd2, // mov x8, #7
    0x01, 0x00, 0x00, 0xd4, // svc #0
];

/// Where the user text and stack are mapped.  Distinct per process:
/// the user page table is shared, so a second spawn at the same VA
/// overwrites the first process's code and it resumes into the
/// other's.
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
    // machinery must be up: the timer keeps firing while they run.
    boot::interrupts(&dt);

    println!("running two_yield");

    unsafe {
        vm::init_user_page_tables();
        vm::switch(vm::user_pagetable(), RootPageTableType::User);
    }

    println!("spawning two yielding processes");
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
    println!("status a {status_a:?}, status b {status_b:?}");

    check!(status_a == Some(6), "A exited 6, got {status_a:?}");
    check!(status_b == Some(7), "B exited 7, got {status_b:?}");
    println!("two_yield passed");
    // A failed check! panics above and hangs the image; the timeout
    // then fails it.  qemu::exit is diverging, so there is nothing
    // after it.
    qemu::exit(0);
}
