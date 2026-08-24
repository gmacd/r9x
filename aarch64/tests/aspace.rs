//! Integration test: per-process address spaces (aspace-switch).
//!
//! Two processes are spawned at the *same* text and stack VAs (`0x1000`
//! and `0x10000`) but with different programs.  With a shared user table
//! that is impossible — one page per VA — but with per-process ASpaces
//! each VA maps to a different physical page per process.  A exits 5, B
//! exits 6; if the TTBR0 switch on the context-switch path is broken
//! (both processes share one AS, or the wrong one is installed), one of
//! them fetches the other's text and exits with the wrong status.  The
//! status asserts are what make it a test.
//!
//! This is the isolation proof at the scheduling level: the same VA
//! resolves to different physical pages per process, and the switch
//! installs the right one.
#![no_std]
#![no_main]

use aarch64::vm::RootPageTableType;
use aarch64::{boot, process, qemu, vm};
use port::println;

#[macro_use]
mod common;

/// A: exit with status 5 (`mov x8, #5; svc #0`).  `movz x8, #5` =
/// 0xd28000a8 (Arm ARM DDI 0487).
const PROG_A: [u8; 8] = [
    0xa8, 0x00, 0x80, 0xd2, // mov x8, #5
    0x01, 0x00, 0x00, 0xd4, // svc #0
];

/// B: exit with status 6 (`mov x8, #6; svc #0`).  `movz x8, #6` =
/// 0xd28000c8.
const PROG_B: [u8; 8] = [
    0xc8, 0x00, 0x80, 0xd2, // mov x8, #6
    0x01, 0x00, 0x00, 0xd4, // svc #0
];

/// The shared text and stack VAs: the load-bearing part of the test.  A
/// shared user table cannot map two different pages at the same VA; only
/// per-process ASpaces can.  A's text maps to one physical page in A's
/// tables, B's to another in B's, both at `0x1000`.
const TEXT_VA: usize = 0x1000;
const STACK_VA: usize = 0x10000;

#[unsafe(no_mangle)]
pub extern "C" fn main9(dtb_va: usize) {
    boot::irq_ops();
    let dt = unsafe { boot::device_tree(dtb_va) };
    boot::page_allocator(&dt, dtb_va).unwrap();
    boot::console(&dt);
    boot::interrupts(&dt);

    println!("running aspace");

    unsafe {
        vm::init_user_page_tables();
        vm::switch(vm::user_pagetable(), RootPageTableType::User);
    }

    // Both at the same VAs: A's exit drives resched to B; B's exit
    // unwinds run_all.
    println!("spawning two processes at the same VAs");
    let a = process::spawn(&PROG_A, TEXT_VA, STACK_VA);
    let b = process::spawn(&PROG_B, TEXT_VA, STACK_VA);

    println!("running the table");
    process::run_all();
    println!("table ran to empty");

    let sa = process::status(a);
    let sb = process::status(b);
    println!("status a {sa:?}, status b {sb:?}");

    check!(sa == Some(5), "A exited 5 (its own text), got {sa:?}");
    check!(sb == Some(6), "B exited 6 (its own text), got {sb:?}");

    println!("aspace passed");
    qemu::exit(qemu::PASS);
}
