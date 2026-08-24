//! Integration test: the EL0 fault path kills only the faulting process
//! (aspace-fault).
//!
//! A writes to an unmapped VA (`0x5000`): a data abort taken in EL0 walks
//! A's own TTBR0, finds no mapping, and the kernel calls `process::fault`,
//! which kills A with `FAULT_STATUS` (0xff) and reschedules to B.  B exits
//! cleanly with status 5.  The peer (B) and the kernel survive the fault —
//! the isolation proof at the fault level.
//!
//! Without the fault handler (before this task), the `str` faults, the
//! vector falls through to print-and-spin, and the machine hangs instead of
//! killing only A.
#![no_std]
#![no_main]

use aarch64::vm::RootPageTableType;
use aarch64::{boot, process, qemu, vm};
use port::println;

#[macro_use]
mod common;

/// A: write to an unmapped VA (`0x5000`) — a data abort in EL0.  The `str`
/// faults; the `svc` after it is unreachable.  `mov x0, #1` (0xd2800020),
/// `mov x1, #0x5000` (0xd28a0001), `str x0, [x1]` (0xf9000020), `svc #0`
/// (0xd4000001).
const PROG_A: [u8; 16] = [
    0x20, 0x00, 0x80, 0xd2, // mov x0, #1
    0x01, 0x00, 0x8a, 0xd2, // mov x1, #0x5000
    0x20, 0x00, 0x00, 0xf9, // str x0, [x1]
    0x01, 0x00, 0x00, 0xd4, // svc #0
];

/// B: exit with status 5 (`mov x8, #5; svc #0`).
const PROG_B: [u8; 8] = [
    0xa8, 0x00, 0x80, 0xd2, // mov x8, #5
    0x01, 0x00, 0x00, 0xd4, // svc #0
];

#[unsafe(no_mangle)]
pub extern "C" fn main9(dtb_va: usize) {
    boot::irq_ops();
    let dt = unsafe { boot::device_tree(dtb_va) };
    boot::page_allocator(&dt, dtb_va).unwrap();
    boot::console(&dt);
    boot::interrupts(&dt);

    println!("running aspace_fault");

    unsafe {
        vm::init_user_page_tables();
        vm::switch(vm::user_pagetable(), RootPageTableType::User);
    }

    // A faults on the `str`; the kernel kills it with FAULT_STATUS and
    // reschedules to B.  B exits cleanly with 5.
    println!("spawning a faulting process and a peer");
    let a =
        process::spawn(&process::Image::Raw { text: &PROG_A, text_va: 0x1000, stack_va: 0x10000 });
    let b =
        process::spawn(&process::Image::Raw { text: &PROG_B, text_va: 0x2000, stack_va: 0x20000 });

    println!("running the table");
    process::run_all();
    println!("table ran to empty");

    let sa = process::status(a);
    let sb = process::status(b);
    println!("status a {sa:?}, status b {sb:?}");

    check!(sa == Some(process::FAULT_STATUS), "A died with the fault status, got {sa:?}");
    check!(sb == Some(5), "B exited 5 (its own clean exit), got {sb:?}");

    println!("aspace_fault passed");
    qemu::exit(qemu::PASS);
}
