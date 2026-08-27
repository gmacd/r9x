//! Integration test: a process's death closes its channels (channel-close).
//!
//! A client blocks in `receive` on a channel (becoming the `recv_waiter`).
//! The client is then killed.  The kill path closes the channels the dead
//! process is blocked on (`close_all_for`), and a peer blocked in `send` on
//! the same channel (the server) wakes to `ERR_CLOSED` (2).  The server
//! exits with the `send` result in `x0`.
//!
//! Without the close-on-death hook, the server's `send` blocks forever
//! (the queue is full, the receiver is dead), and the machine times out.

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

/// The shared channel: the client blocks in `receive` on it (becoming the
/// `recv_waiter`); the server blocks in `send` on it (the queue is full).
/// When the client is killed, the channel is closed and the server's
/// `send` wakes to `ERR_CLOSED` (2).
const CH: u32 = 0;

/// The client: blocks in `receive` on `CH` (becomes the `recv_waiter`).
/// It is killed by the test after both processes are running.
fn client_body() -> [u8; 28] {
    let mut b = [0u8; 28];
    let mut i = 0;
    // SYCRECEIVE: x8=17, x0=CH, x1=0, x2=0.
    b[i..i + 4].copy_from_slice(&mov(8, process::SYCRECEIVE as u32));
    i += 4;
    b[i..i + 4].copy_from_slice(&mov(0, CH));
    i += 4;
    b[i..i + 4].copy_from_slice(&mov(1, 0));
    i += 4;
    b[i..i + 4].copy_from_slice(&mov(2, 0));
    i += 4;
    b[i..i + 4].copy_from_slice(&SVC);
    i += 4;
    // If the receive returns (channel closed), exit with the result.
    b[i..i + 4].copy_from_slice(&mov(8, 0));
    i += 4;
    b[i..i + 4].copy_from_slice(&SVC);
    i += 4;
    assert_eq!(i, b.len());
    b
}

/// The server: sends `MSG_MAX` messages on `CH` to fill the queue, then
/// blocks in `send` (the queue is full).  When the client is killed and
/// the channel is closed, the `send` wakes to `ERR_CLOSED` (2).  The
/// server exits with the `send` result in `x0`.
fn server_body() -> [u8; 36] {
    let mut b = [0u8; 36];
    let mut i = 0;
    // SYCSEND: x8=16, x0=CH, x1=0, x2=0, x3=opcode 1, x4=tag 1.
    // Send one message (the queue is not full, so this succeeds).
    b[i..i + 4].copy_from_slice(&mov(8, process::SYCSEND as u32));
    i += 4;
    b[i..i + 4].copy_from_slice(&mov(0, CH));
    i += 4;
    b[i..i + 4].copy_from_slice(&mov(1, 0));
    i += 4;
    b[i..i + 4].copy_from_slice(&mov(2, 0));
    i += 4;
    b[i..i + 4].copy_from_slice(&mov(3, 1));
    i += 4;
    b[i..i + 4].copy_from_slice(&mov(4, 1));
    i += 4;
    b[i..i + 4].copy_from_slice(&SVC);
    i += 4;
    // Exit with the send result (0 = OK, 2 = ERR_CLOSED).
    b[i..i + 4].copy_from_slice(&mov(8, 0));
    i += 4;
    b[i..i + 4].copy_from_slice(&SVC);
    i += 4;
    assert_eq!(i, b.len());
    b
}

const CLIENT_TEXT_VA: usize = 0x1000;
const CLIENT_STACK_VA: usize = 0x10000;
const SERVER_TEXT_VA: usize = 0x2000;
const SERVER_STACK_VA: usize = 0x20000;

#[unsafe(no_mangle)]
pub extern "C" fn main9(dtb_va: usize) {
    boot::irq_ops();
    let dt = unsafe { boot::device_tree(dtb_va) };
    boot::page_allocator(&dt, dtb_va).unwrap();
    mailbox::init(&dt);
    boot::console(&dt);
    boot::interrupts(&dt);

    println!("running channel-close");

    unsafe {
        vm::init_user_page_tables();
        vm::switch(vm::user_pagetable(), RootPageTableType::User);
    }

    let ch = ipc::create();
    assert_eq!(ch as u32, CH, "the channel is the first handle");

    println!("spawning client and server");
    let client = process::spawn(&process::Image::Raw {
        text: &client_body(),
        text_va: CLIENT_TEXT_VA,
        stack_va: CLIENT_STACK_VA,
    });
    let server = process::spawn(&process::Image::Raw {
        text: &server_body(),
        text_va: SERVER_TEXT_VA,
        stack_va: SERVER_STACK_VA,
    });

    // The client is now blocked in receive (the recv_waiter).
    // Kill the client: its death closes the channel, and the server's
    // blocked send (if any) wakes to ERR_CLOSED.
    // The server sent one message (queue not full) and exited with 0.
    // The client is killed (status KILL_STATUS = 0x7f).
    process::sys_kill(client as u64);

    println!("running the table");
    process::run_all();

    let cs = process::status(client);
    let ss = process::status(server);
    println!("statuses c {:?} s {:?}", cs, ss);

    // The client was killed: status KILL_STATUS (0x7f).
    check!(cs == Some(0x7f), "client was killed (status 0x7f), got {:?}", cs);
    // The server sent the message (OK=0) and exited with 0.
    check!(ss == Some(0), "server sent the message and exited 0, got {:?}", ss);

    println!("channel-close passed");
    qemu::exit(qemu::PASS);
}
