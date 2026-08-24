//! Integration test: the yield syscall, and that the EL0 return path
//! restores the process's stack pointer.
//!
//! The program stores a marker to the stack, moves distinct values
//! into x0 and x19, yields twice (`svc #1`), and after each yield
//! checks that all three survived; only if both yields return the
//! process to exactly the right place does it exit with the success
//! status.  The load-bearing part is the second half of the
//! handler's first yield: without the vector restoring SP_EL0 from
//! the frame, the process's user stack pointer holds whatever EL0
//! boot garbage it held (0), the first marker store data-aborts, and
//! the test fails before it can fail in any other way.  A host-side
//! test could never prove the return path works at all.
#![no_std]
#![no_main]

use aarch64::vm::RootPageTableType;
use aarch64::{boot, mailbox, process, qemu, vm};
use port::println;

#[macro_use]
mod common;

/// The whole program, with each line's instruction commented.
/// Assembled with `clang -target arm64` and re-checked by objdump;
/// there is no assembler in the tree.
///
/// x8 is the syscall register (1 = yield, 5 = exit success), so the
/// markers use x0/x19.  The first `cmp`/`b.ne` pair puts NZCV *live*
/// across the yield: `b.ne fail_d` after the svc branches on flags
/// set before the trap, so a return that clobbers PSTATE (NZCV) fails
/// the image instead of the later marker checks.  The registers and
/// the stack slot start at 0 (the entry context zeroes them), so the
/// markers are distinct from their starting values.
const YIELD_PROGRAM: [u8; 0x6c] = [
    // 0x00
    0xb3, 0x00, 0x80, 0xd2, // mov x19, #5
    // 0x04
    0xe0, 0x00, 0x80, 0xd2, // mov x0, #7
    // 0x08
    0xe0, 0x07, 0x00, 0xf9, // str x0, [sp, #8]
    // 0x0c
    0x1f, 0x1c, 0x00, 0xf1, // cmp x0, #7
    // 0x10
    0xa1, 0x02, 0x00, 0x54, // b.ne fail_d
    // 0x14
    0x28, 0x00, 0x80, 0xd2, // mov x8, #1
    // 0x18
    0x01, 0x00, 0x00, 0xd4, // svc #0
    // 0x1c
    0x41, 0x02, 0x00, 0x54, // b.ne fail_d
    // 0x20
    0x1f, 0x1c, 0x00, 0xf1, // cmp x0, #7
    // 0x24
    0x41, 0x01, 0x00, 0x54, // b.ne fail_a
    // 0x28
    0x7f, 0x16, 0x00, 0xf1, // cmp x19, #5
    // 0x2c
    0x41, 0x01, 0x00, 0x54, // b.ne fail_b
    // 0x30
    0xe0, 0x07, 0x40, 0xf9, // ldr x0, [sp, #8]
    // 0x34
    0x1f, 0x1c, 0x00, 0xf1, // cmp x0, #7
    // 0x38
    0x21, 0x01, 0x00, 0x54, // b.ne fail_c
    // 0x3c
    0x28, 0x00, 0x80, 0xd2, // mov x8, #1
    // 0x40
    0x01, 0x00, 0x00, 0xd4, // svc #0
    // 0x44
    0xa8, 0x00, 0x80, 0xd2, // mov x8, #5
    // 0x48
    0x01, 0x00, 0x00, 0xd4, // svc #0
    // 0x4c (fail_a)
    0x68, 0x00, 0x80, 0xd2, // mov x8, #3
    // 0x50
    0x01, 0x00, 0x00, 0xd4, // svc #0
    // 0x54 (fail_b)
    0x88, 0x00, 0x80, 0xd2, // mov x8, #4
    // 0x58
    0x01, 0x00, 0x00, 0xd4, // svc #0
    // 0x5c (fail_c)
    0xc8, 0x00, 0x80, 0xd2, // mov x8, #6
    // 0x60
    0x01, 0x00, 0x00, 0xd4, // svc #0
    // 0x64 (fail_d)
    0x48, 0x00, 0x80, 0xd2, // mov x8, #2
    // 0x68
    0x01, 0x00, 0x00, 0xd4, // svc #0
];

/// Where the user text and stack are mapped.
const USER_TEXT_VA: usize = 0x1000;
const USER_STACK_VA: usize = 0x10000;

#[unsafe(no_mangle)]
pub extern "C" fn main9(dtb_va: usize) {
    // Vectors first: without them a fault here goes nowhere at all
    // rather than reaching a handler that can report it.
    boot::irq_ops();
    let dt = unsafe { boot::device_tree(dtb_va) };
    boot::page_allocator(&dt, dtb_va).unwrap();
    mailbox::init(&dt);
    boot::console(&dt);
    // The process enters with IRQs unmasked, so the interrupt
    // machinery must be up: the timer keeps firing while it runs.
    boot::interrupts(&dt);

    println!("running user_yield");

    unsafe {
        vm::init_user_page_tables();
        vm::switch(vm::user_pagetable(), RootPageTableType::User);
    }

    println!("starting yield process");
    let id = process::spawn(&process::Image::Raw {
        text: &YIELD_PROGRAM,
        text_va: USER_TEXT_VA,
        stack_va: USER_STACK_VA,
    });
    process::run_all();
    let status = process::status(id);
    println!("yield process returned, status {status:?}");

    // 5 is the program's success exit; 3, 4 and 6 are its failure
    // exits (a marker did not survive a yield).
    check!(status == Some(5), "both yields returned the process intact, status {status:?}");

    println!("user_yield passed");
    qemu::exit(qemu::PASS);
}
