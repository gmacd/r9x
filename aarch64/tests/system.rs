//! Integration test: the full system bringup — the same one the kernel image
//! runs.  This is the test the default `cargo xtask qemu` is missing: the
//! kernel's `main9` bringup is a separate code path from every other image,
//! and an init-context assumption there (init spawned with `handles: None`
//! after init came to require a channel pair) faulted a process with no test to
//! catch it.
//!
//! The fix is structural: the bringup is extracted into
//! [`aarch64::system::bringup`], and the kernel image *and* this image call the
//! **same** function, so they cannot drift apart.  This image runs the shared
//! bringup, runs the system to a fixpoint, and checks that no process exited:
//! a fault or a panic (a failed check in init or the child) ends a process, so
//! an all-alive table is the success.

#![no_std]
#![no_main]

use aarch64::{boot, mailbox, process, qemu, system, vm};
use port::println;

#[macro_use]
mod common;

#[unsafe(no_mangle)]
pub extern "C" fn main9(dtb_va: usize) {
    // Vectors first: without them a fault in a process goes nowhere at all
    // rather than reaching the handler that records the fault status.
    boot::irq_ops();
    let dt = unsafe { boot::device_tree(dtb_va) };
    boot::page_allocator(&dt, dtb_va).unwrap();
    mailbox::init(&dt);
    boot::console(&dt);
    boot::interrupts(&dt);

    println!("running system");

    unsafe {
        vm::init_user_page_tables();
        vm::switch(vm::user_pagetable(), vm::RootPageTableType::User);
    }

    // The same bringup the kernel image runs: register the child, create the
    // pairs, spawn the nameserver, the console, and init (which SYS_SPAWNs the
    // child by index).  No `set_console_live` here — the image still uses
    // `println!` for its check.
    let _ns_handles = system::bringup();

    process::run_all();

    println!("any_exited {}", process::any_exited());
    // No process exited: init drove the spawn, the error cases, and the
    // round-trip, and is blocked; the child and the fillers are blocked too.
    // A fault or a panic (a failed check) ends a process, so an exited process
    // is the failure.  The check is over the whole table (the image can't know
    // the children's ids — init learned them), not one slot.
    check!(!process::any_exited(), "no process exited (the full system brought up cleanly)");

    println!("system passed");
    qemu::exit(qemu::PASS);
}
