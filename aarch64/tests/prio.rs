//! Integration test: the scheduler runs by priority, with priority
//! inheritance as a capability.
//!
//! Three user processes L, M, H each yield three times, then exit (6, 7,
//! 8).  L and M are both at the default priority; H is boosted to a
//! high-urgency level by a kernel-side `process::boost` before the table
//! runs — the manual stand-in for the blocking send that will do it in
//! stage 2.  `run_all` starts with the lowest-index Runnable (L), and
//! every switch after is a priority-ordered `resched`.
//!
//! The assertion is on the switch-in order the kernel records
//! (`process::run_order`): H (the boosted one) must be switched in before
//! M (its same-base peer).  That is load-bearing — with only round-robin
//! (no priority pass) the table runs L, M, H, L, M, … and H is switched
//! in *after* M, so the assertion fails.  The trace has one entry from
//! `run_all`'s first pick and the rest from yield-driven rescheds, so a
//! non-trivial order is itself proof the selection (not just the first
//! pick) is priority-ordered.

#![no_std]
#![no_main]

use aarch64::vm::RootPageTableType;
use aarch64::{boot, mailbox, process, qemu, vm};
use port::println;

#[macro_use]
mod common;

/// Yield three times, then exit with status 6.  x8 is the syscall
/// register.  Assembled with `clang -target arm64` and re-checked by
/// objdump; there is no assembler in the tree.
const PROG_L: [u8; 0x20] = [
    0x28, 0x00, 0x80, 0xd2, // mov x8, #1
    0x01, 0x00, 0x00, 0xd4, // svc #0
    0x28, 0x00, 0x80, 0xd2, // mov x8, #1
    0x01, 0x00, 0x00, 0xd4, // svc #0
    0x28, 0x00, 0x80, 0xd2, // mov x8, #1
    0x01, 0x00, 0x00, 0xd4, // svc #0
    0xc8, 0x00, 0x80, 0xd2, // mov x8, #6
    0x01, 0x00, 0x00, 0xd4, // svc #0
];

/// Same shape, exits 7.
const PROG_M: [u8; 0x20] = [
    0x28, 0x00, 0x80, 0xd2, // mov x8, #1
    0x01, 0x00, 0x00, 0xd4, // svc #0
    0x28, 0x00, 0x80, 0xd2, // mov x8, #1
    0x01, 0x00, 0x00, 0xd4, // svc #0
    0x28, 0x00, 0x80, 0xd2, // mov x8, #1
    0x01, 0x00, 0x00, 0xd4, // svc #0
    0xe8, 0x00, 0x80, 0xd2, // mov x8, #7
    0x01, 0x00, 0x00, 0xd4, // svc #0
];

/// Same shape, exits 8.
const PROG_H: [u8; 0x20] = [
    0x28, 0x00, 0x80, 0xd2, // mov x8, #1
    0x01, 0x00, 0x00, 0xd4, // svc #0
    0x28, 0x00, 0x80, 0xd2, // mov x8, #1
    0x01, 0x00, 0x00, 0xd4, // svc #0
    0x28, 0x00, 0x80, 0xd2, // mov x8, #1
    0x01, 0x00, 0x00, 0xd4, // svc #0
    0x08, 0x01, 0x80, 0xd2, // mov x8, #8
    0x01, 0x00, 0x00, 0xd4, // svc #0
];

/// Distinct per process: the user page table is shared, so a second spawn
/// at the same VA overwrites the first process's code.
const L_TEXT_VA: usize = 0x1000;
const L_STACK_VA: usize = 0x10000;
const M_TEXT_VA: usize = 0x2000;
const M_STACK_VA: usize = 0x20000;
const H_TEXT_VA: usize = 0x3000;
const H_STACK_VA: usize = 0x30000;

#[unsafe(no_mangle)]
pub extern "C" fn main9(dtb_va: usize) {
    // Vectors first: without them a fault here goes nowhere at all
    // rather than reaching a handler that can report it.
    boot::irq_ops();
    let dt = unsafe { boot::device_tree(dtb_va) };
    boot::page_allocator(&dt, dtb_va).unwrap();
    mailbox::init(&dt);
    boot::console(&dt);
    // The processes enter with IRQs unmasked, so the interrupt machinery
    // must be up: the tick keeps firing while they run.
    boot::interrupts(&dt);

    println!("running prio");

    unsafe {
        vm::init_user_page_tables();
        vm::switch(vm::user_pagetable(), RootPageTableType::User);
    }

    println!("spawning three yielding processes");
    let l = process::spawn(&process::Image::Raw {
        text: &PROG_L,
        text_va: L_TEXT_VA,
        stack_va: L_STACK_VA,
    });
    let m = process::spawn(&process::Image::Raw {
        text: &PROG_M,
        text_va: M_TEXT_VA,
        stack_va: M_STACK_VA,
    });
    let h = process::spawn(&process::Image::Raw {
        text: &PROG_H,
        text_va: H_TEXT_VA,
        stack_va: H_STACK_VA,
    });

    // H is the process a high-priority waiter would boost (stage 2 does
    // this on a blocking send; here it is a manual kernel-side call, the
    // point of this stage).  Raise it to a high-urgency level.
    process::boost(h, process::Priority::new(16));
    check!(
        process::effective_priority(h) == Some(process::Priority::new(16)),
        "boost raised H to a high-urgency level, got {:?}",
        process::effective_priority(h)
    );

    println!("running the table");
    process::run_all();

    let order = process::run_order();
    let h_before_m = order
        .iter()
        .position(|&x| x == h)
        .is_some_and(|i| order.iter().position(|&x| x == m) > Some(i));
    println!(
        "statuses l {:?} m {:?} h {:?}, preemptions {}, run_order {:?}",
        process::status(l),
        process::status(m),
        process::status(h),
        process::preemptions(),
        order
    );

    check!(process::status(l) == Some(6), "L exited 6, got {:?}", process::status(l));
    check!(process::status(m) == Some(7), "M exited 7, got {:?}", process::status(m));
    check!(process::status(h) == Some(8), "H exited 8, got {:?}", process::status(h));
    check!(
        h_before_m,
        "boosted H was switched in before its same-base peer M (priority, not round-robin); order {:?}",
        order
    );

    println!("prio passed");
    // A failed check! panics above and hangs the image; the timeout then
    // fails it.  qemu::exit is diverging, so there is nothing after it.
    qemu::exit(qemu::PASS);
}
