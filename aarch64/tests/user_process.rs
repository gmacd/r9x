//! Integration test: running the first user process to its exit.
//!
//! The kernel starts a process -- whose entire program is a
//! `svc` for sysexit -- in EL0, and the exit trap switches back to
//! the context that started it.  The prints on either side of the
//! process's lifetime are the assertion: a host-side test could
//! never prove that a process entered EL0, faulted-free, and that
//! the kernel resumed at the point that started it.
#![no_std]
#![no_main]

use aarch64::vm::RootPageTableType;
use aarch64::{boot, process, qemu, vm};
use port::println;

#[macro_use]
mod common;

/// The whole program: `svc #0` (sysexit).  AArch64 `svc` is
/// 0xd4000001 | (number << 8), little-endian: 0xd4000001 is `svc #0`
/// (Arm ARM DDI 0487; Linux's arm64 syscall instruction).
const SYSCALL_EXIT: [u8; 4] = [0x01, 0x00, 0x00, 0xd4];

/// Where the user text and stack are mapped.  Both have to be
/// addresses TTBR0 translates: the scratch code this replaced put
/// the stack at KZERO - 0x1000, which is in the TTBR1 half, so the
/// mapping went into the user page table at an address the user page
/// table never sees.
const USER_TEXT_VA: usize = 0x1000;
const USER_STACK_VA: usize = 0x10000;

#[unsafe(no_mangle)]
pub extern "C" fn main9(dtb_va: usize) {
    // Vectors first: without them a fault here goes nowhere at all
    // rather than reaching a handler that can report it.
    boot::irq_ops();
    let dt = unsafe { boot::device_tree(dtb_va) };
    boot::page_allocator(&dt, dtb_va).unwrap();
    boot::console(&dt);
    // The process enters with IRQs unmasked, so the interrupt
    // machinery must be up: the timer keeps firing while it runs,
    // which is the point of entering EL0 that way.
    boot::interrupts(&dt);

    println!("running user_process");

    // Build the user address space and make it the live one, exactly
    // as the kernel binary does before it would run a process.
    unsafe {
        vm::init_user_page_tables();
        vm::switch(vm::user_pagetable(), RootPageTableType::User);
    }

    // The whole test: start the process and check what comes back.
    // If the switch-back were broken, the resume would fault or
    // wander before this line runs at all.
    println!("starting first process");
    let id = process::spawn(&SYSCALL_EXIT, USER_TEXT_VA, USER_STACK_VA);
    process::run_all();
    let status = process::status(id);
    println!("first process returned, status {status:?}");

    check!(status == Some(process::SYSEXIT), "process exited by sysexit, status {status:?}");

    println!("user_process passed");
    qemu::exit(qemu::PASS);
}
