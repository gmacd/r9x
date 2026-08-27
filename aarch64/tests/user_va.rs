//! Integration test: a bad user pointer is an error return, not a kernel
//! data abort (read_user).
//!
//! A calls `SYS_PRINT` with an unmapped pointer (`0x5000`), then `SYCSEND`
//! with an unmapped payload buffer on a real channel.  Both go through the
//! syscall layer's `read_user`, which software-walks A's `TTBR0` and finds
//! no mapping — so each syscall returns `ERR_BAD_VA` instead of copying
//! through the bad pointer (a `copy_nonoverlapping` on an unmapped VA in EL1
//! would be a kernel data abort: the machine would hang, not return).
//!
//! A then exits cleanly.  The proof is that the table runs to empty and A's
//! status is its own clean exit — the kernel survived both bad pointers.
//! Without the walk, the first `SYS_PRINT` aborts in EL1 and this image
//! never reaches the status check.
#![no_std]
#![no_main]

use aarch64::vm::RootPageTableType;
use aarch64::{boot, ipc, mailbox, process, qemu, vm};
use port::println;

#[macro_use]
mod common;

/// movz x<Rd>, #<imm>: `0xd2800000 | (imm << 5) | Rd`.
const fn mov(rd: u8, imm: u32) -> [u8; 4] {
    let w = (0xd2800000u32 | (imm << 5) | rd as u32).to_le_bytes();
    [w[0], w[1], w[2], w[3]]
}

/// svc #0.
const SVC: [u8; 4] = [0x01, 0x00, 0x00, 0xd4];

/// An unmapped VA in the gap between A's text (0x1000) and stack (0x10000):
/// the walk finds no mapping for it.
const BAD_VA: u32 = 0x5000;

/// A: `SYS_PRINT` with a bad pointer (returns `ERR_BAD_VA`, no fault), then
/// `SYCSEND` with a bad buffer on channel 0 (returns `ERR_BAD_VA`, no fault),
/// then exit 0.
fn a_body() -> [u8; 56] {
    let mut b = [0u8; 56];
    let mut i = 0;
    // SYS_PRINT: x8=31, x0=BAD_VA, x1=16.
    b[i..i + 4].copy_from_slice(&mov(8, process::SYS_PRINT as u32));
    i += 4;
    b[i..i + 4].copy_from_slice(&mov(0, BAD_VA));
    i += 4;
    b[i..i + 4].copy_from_slice(&mov(1, 16));
    i += 4;
    b[i..i + 4].copy_from_slice(&SVC);
    i += 4;
    // SYCSEND: x8=16, x0=0 (channel), x1=BAD_VA (buffer), x2=16, x3=0, x4=0.
    b[i..i + 4].copy_from_slice(&mov(8, process::SYCSEND as u32));
    i += 4;
    b[i..i + 4].copy_from_slice(&mov(0, 0));
    i += 4;
    b[i..i + 4].copy_from_slice(&mov(1, BAD_VA));
    i += 4;
    b[i..i + 4].copy_from_slice(&mov(2, 16));
    i += 4;
    b[i..i + 4].copy_from_slice(&mov(3, 0));
    i += 4;
    b[i..i + 4].copy_from_slice(&mov(4, 0));
    i += 4;
    b[i..i + 4].copy_from_slice(&SVC);
    i += 4;
    // Exit 0: x0 is the exit status (the SYCSEND result clobbered it).
    b[i..i + 4].copy_from_slice(&mov(0, 0));
    i += 4;
    b[i..i + 4].copy_from_slice(&mov(8, 0));
    i += 4;
    b[i..i + 4].copy_from_slice(&SVC);
    i += 4;
    assert_eq!(i, b.len());
    b
}

const A_TEXT_VA: usize = 0x1000;
const A_STACK_VA: usize = 0x10000;

#[unsafe(no_mangle)]
pub extern "C" fn main9(dtb_va: usize) {
    boot::irq_ops();
    let dt = unsafe { boot::device_tree(dtb_va) };
    boot::page_allocator(&dt, dtb_va).unwrap();
    mailbox::init(&dt);
    boot::console(&dt);
    boot::interrupts(&dt);

    println!("running user_va");

    unsafe {
        vm::init_user_page_tables();
        vm::switch(vm::user_pagetable(), RootPageTableType::User);
    }

    // A's SYCSEND needs a real channel: the first created one is handle 0.
    let ch = ipc::create();
    assert_eq!(ch as u32, 0, "the first channel is handle 0");

    println!("spawning a process that points syscalls at unmapped memory");
    let a = process::spawn(&process::Image::Raw {
        text: &a_body(),
        text_va: A_TEXT_VA,
        stack_va: A_STACK_VA,
    });

    println!("running the table");
    process::run_all();
    println!("table ran to empty");

    let sa = process::status(a);
    println!("status a {sa:?}");
    // A's own clean exit: both bad pointers were error returns, not faults.
    check!(sa == Some(0), "A exited 0 (bad pointers were refused, not faulted), got {sa:?}");

    println!("user_va passed");
    qemu::exit(qemu::PASS);
}
