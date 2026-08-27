//! Integration test: the fault backtrace prints a frame-pointer walk for a
//! Rust-compiled process.
//!
//! Spawns the `faulttest` binary (a Rust ELF with three frames: `main` →
//! `level1` → `level2` → `level3`, which writes to an unmapped address).  The
//! fault handler prints the FAR/ESR line, then the backtrace (return
//! addresses from the FP chain), then kills the process with FAULT_STATUS.
//! The image checks the exit status; the backtrace is visible on the serial
//! output (not machine-checked — it's a debug aid, not a protocol).

#![no_std]
#![no_main]

use aarch64::vm::RootPageTableType;
use aarch64::{boot, mailbox, process, qemu, vm};
use port::println;

#[macro_use]
mod common;

static FAULTTEST_ELF: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/faulttest.elf"));

#[unsafe(no_mangle)]
pub extern "C" fn main9(dtb_va: usize) {
    boot::irq_ops();
    let dt = unsafe { boot::device_tree(dtb_va) };
    boot::page_allocator(&dt, dtb_va).unwrap();
    mailbox::init(&dt);
    boot::console(&dt);
    boot::interrupts(&dt);

    println!("running fault-backtrace");

    unsafe {
        vm::init_user_page_tables();
        vm::switch(vm::user_pagetable(), RootPageTableType::User);
    }

    let pid = process::spawn(&process::Image::Elf { bytes: FAULTTEST_ELF, handles: None });
    println!("spawned faulttest (pid {pid})");

    process::run_all();

    let status = process::status(pid);
    println!("faulttest status: {status:?}");

    // The process died with FAULT_STATUS (0xff = 255): the backtrace was
    // printed (visible on serial), and the kill is the in-image check.
    check!(status == Some(255), "faulttest died with FAULT_STATUS (255), got {status:?}");

    println!("fault-backtrace passed");
    qemu::exit(qemu::PASS);
}
