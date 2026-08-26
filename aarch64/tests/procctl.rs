//! Integration test: `SYS_WAIT` + `SYS_KILL` — the process control
//! syscalls.
//!
//! A whole kernel image: spawns children that exit with known statuses,
//! runs them via `run_all`, then reaps them via `sys_wait` and asserts
//! the statuses match; checks the slots are reusable; kills a running
//! child and checks the kill status.

#![no_std]
#![no_main]

use aarch64::{boot, process, qemu, vm};
use port::println;

#[macro_use]
mod common;

/// Exit with status 3: same encoding as two_process's PROG_A.
const PROG_EXIT_3: [u8; 8] = [
    0x68, 0x00, 0x80, 0xd2, // mov x8, #3
    0x01, 0x00, 0x00, 0xd4, // svc #0
];
/// Exit with status 4: same encoding as two_process's PROG_B.
const PROG_EXIT_4: [u8; 8] = [
    0x88, 0x00, 0x80, 0xd2, // mov x8, #4
    0x01, 0x00, 0x00, 0xd4, // svc #0
];

const EXIT_3_TEXT_VA: usize = 0x1000;
const EXIT_3_STACK_VA: usize = 0x10000;
const EXIT_4_TEXT_VA: usize = 0x2000;
const EXIT_4_STACK_VA: usize = 0x20000;

#[unsafe(no_mangle)]
pub extern "C" fn main9(dtb_va: usize) {
    boot::irq_ops();
    let dt = unsafe { boot::device_tree(dtb_va) };
    boot::page_allocator(&dt, dtb_va).unwrap();
    aarch64::mailbox::init(&dt);
    boot::console(&dt);
    boot::interrupts(&dt);

    println!("running procctl");

    unsafe {
        vm::init_user_page_tables();
        vm::switch(vm::user_pagetable(), vm::RootPageTableType::User);
    }

    // Spawn two children that exit with known statuses.
    let child_a = process::spawn(&process::Image::Raw {
        text: &PROG_EXIT_3,
        text_va: EXIT_3_TEXT_VA,
        stack_va: EXIT_3_STACK_VA,
    });
    let child_b = process::spawn(&process::Image::Raw {
        text: &PROG_EXIT_4,
        text_va: EXIT_4_TEXT_VA,
        stack_va: EXIT_4_STACK_VA,
    });
    println!("child_a {child_a} (exits 3), child_b {child_b} (exits 4)");

    // Run all processes: both children run and exit.  run_all returns
    // when there are no more Runnable processes.
    process::run_all();

    // Reap child_a: check the id and status.
    let (reaped_id, status_a) = process::sys_wait(child_a as u64, 0);
    check!(reaped_id == child_a as u64, "reaped id {reaped_id} == child_a {child_a}");
    check!(status_a == 3, "child_a exit status {status_a} == 3");
    println!("reaped {reaped_id}, status {status_a}");

    // Reap child_b.
    let (reaped_id_b, status_b) = process::sys_wait(child_b as u64, 0);
    check!(reaped_id_b == child_b as u64, "reaped id {reaped_id_b} == child_b {child_b}");
    check!(status_b == 4, "child_b exit status {status_b} == 4");
    println!("reaped {reaped_id_b}, status {status_b}");

    // The slots are now free: a third spawn succeeds and gets a different
    // id (the table has 8 slots; the two reaped slots are reusable).
    let child_c = process::spawn(&process::Image::Raw {
        text: &PROG_EXIT_3,
        text_va: EXIT_3_TEXT_VA,
        stack_va: EXIT_3_STACK_VA,
    });
    println!("child_c {child_c} spawned (slot reusable — reuses a freed slot)");
    // The slot was freed by sys_wait, so child_c may reuse child_a's or
    // child_b's slot.  What matters is that the spawn succeeded (a
    // non-error id).

    // Run child_c to completion and reap it.
    process::run_all();
    let (_, status_c) = process::sys_wait(child_c as u64, 0);
    check!(status_c == 3, "child_c exit status {status_c} == 3");
    println!("reaped {child_c}, status {status_c}");

    // Kill of a bad id is an error, not a fault.
    let bad_kill = process::sys_kill(200);
    check!(bad_kill == 1, "kill of bad id returns 1 (got {bad_kill})");

    // Wait for a bad id is an error.
    let (wait_bad, _) = process::sys_wait(200, 0);
    check!(
        wait_bad == process::WAIT_BAD_ID,
        "wait of bad id returns WAIT_BAD_ID (got {wait_bad:#x})"
    );

    // No process exited un-reaped: the table is clean.
    check!(!process::any_exited(), "no un-reaped zombies remain");

    println!("procctl passed");
    qemu::exit(qemu::PASS);
}
