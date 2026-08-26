//! Integration test: `SYS_SETPRIO` — priority control.
//!
//! A whole kernel image: sets a process's priority, verifies success;
//! tries the idle sentinel (refused); tries a bad id (error).

#![no_std]
#![no_main]

use aarch64::{boot, process, qemu, vm};
use port::println;

#[macro_use]
mod common;

/// A busy loop (yield loop) so the child is live when we set its priority.
const PROG_BUSY: [u8; 8] = [
    0x00, 0x00, 0x00, 0xd5, // yield
    0x00, 0x00, 0x00, 0x14, // b .
];

const BUSY_TEXT_VA: usize = 0x1000;
const BUSY_STACK_VA: usize = 0x10000;

#[unsafe(no_mangle)]
pub extern "C" fn main9(dtb_va: usize) {
    boot::irq_ops();
    let dt = unsafe { boot::device_tree(dtb_va) };
    boot::page_allocator(&dt, dtb_va).unwrap();
    aarch64::mailbox::init(&dt);
    boot::console(&dt);
    boot::interrupts(&dt);

    println!("running sched");

    unsafe {
        vm::init_user_page_tables();
        vm::switch(vm::user_pagetable(), vm::RootPageTableType::User);
    }

    // Spawn a live child.
    let child = process::spawn(&process::Image::Raw {
        text: &PROG_BUSY,
        text_va: BUSY_TEXT_VA,
        stack_va: BUSY_STACK_VA,
    });
    println!("child {child} spawned");

    // Set the child's priority to 10 (high urgency).
    let result = process::sys_setprio(child as u64, 10);
    check!(result == 0, "setprio succeeded (result {result})");
    println!("set child {child} prio to 10");

    // Set self (u64::MAX) priority.  From the kernel context there is no
    // current process, so this should fail with SETPRIO_BAD_ID.
    let result = process::sys_setprio(u64::MAX, 20);
    check!(
        result == process::SETPRIO_BAD_ID,
        "self from kernel context is bad id (result {result})"
    );
    println!("self from kernel context refused");

    // The idle sentinel (255) is refused.
    let result = process::sys_setprio(child as u64, 255);
    check!(result == process::SETPRIO_BAD_PRIO, "idle sentinel refused (result {result})");
    println!("idle sentinel refused");

    // A bad id is an error.
    let result = process::sys_setprio(200, 10);
    check!(result == process::SETPRIO_BAD_ID, "bad id refused (result {result})");
    println!("bad id refused");

    // Kill the child so the table is clean.
    let result = process::sys_kill(child as u64);
    check!(result == 0, "kill succeeded");
    process::sys_wait(child as u64, 0);

    check!(!process::any_exited(), "no un-reaped zombies");

    println!("sched passed");
    qemu::exit(qemu::PASS);
}
