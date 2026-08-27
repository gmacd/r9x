//! Integration test: a process's death closes its channels (close-on-death).
//!
//! The client blocks in `receive` on channel 0 (becoming the `recv_waiter`).
//! The server sends one message on channel 0 (delivered to the client), then
//! blocks in `receive` on channel 1.  The test kills the client: the kill
//! path calls `close_all_for`, which sees the client is the `recv_waiter` on
//! channel 0 and closes it.  The test then wakes the server (a message on
//! channel 1); the server's second `send` on channel 0 returns `ERR_CLOSED`
//! (2) because the channel is closed.  The server exits with status 2.
//!
//! Without the close-on-death hook, the channel stays open and the server's
//! second `send` succeeds (the queue has room), so the server exits 0 — the
//! check below would fail.

#![no_std]
#![no_main]

use aarch64::vm::RootPageTableType;
use aarch64::{boot, ipc, mailbox, process, qemu, vm};
use port::ipc::{Message, try_send};
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

/// Channel 0: the shared channel the client blocks on and the server sends.
const CH0: u32 = 0;
/// Channel 1: the server's wakeup channel (the test sends here to release
/// the server after the kill).
const CH1: u32 = 1;

/// The client: receives on CH0 twice.  The first receive gets the server's
/// message; the second blocks (the client becomes the `recv_waiter`).  If the
/// channel is closed while the client is blocked, the receive returns and the
/// client exits (but the kill sets the status first, so the observable is
/// KILL_STATUS, not the receive result).
fn client_body() -> [u8; 48] {
    let mut b = [0u8; 48];
    let mut i = 0;
    // First SYCRECEIVE: x8=17, x0=CH0, x1=0, x2=0.
    b[i..i + 4].copy_from_slice(&mov(8, process::SYCRECEIVE as u32));
    i += 4;
    b[i..i + 4].copy_from_slice(&mov(0, CH0));
    i += 4;
    b[i..i + 4].copy_from_slice(&mov(1, 0));
    i += 4;
    b[i..i + 4].copy_from_slice(&mov(2, 0));
    i += 4;
    b[i..i + 4].copy_from_slice(&SVC);
    i += 4;
    // Second SYCRECEIVE: same registers (blocks — becomes the recv_waiter).
    b[i..i + 4].copy_from_slice(&mov(8, process::SYCRECEIVE as u32));
    i += 4;
    b[i..i + 4].copy_from_slice(&mov(0, CH0));
    i += 4;
    b[i..i + 4].copy_from_slice(&mov(1, 0));
    i += 4;
    b[i..i + 4].copy_from_slice(&mov(2, 0));
    i += 4;
    b[i..i + 4].copy_from_slice(&SVC);
    i += 4;
    // SYSEXIT: x8=0 (if the receive returned, exit with the result in x0).
    b[i..i + 4].copy_from_slice(&mov(8, 0));
    i += 4;
    b[i..i + 4].copy_from_slice(&SVC);
    i += 4;
    assert_eq!(i, b.len());
    b
}

/// The server: sends on CH0 (delivered to the client), blocks in `receive`
/// on CH1 (the test's wakeup), then sends on CH0 again.  After the kill,
/// CH0 is closed, so the second send returns ERR_CLOSED (2).  Exits with the
/// second send result in x0.
fn server_body() -> [u8; 84] {
    let mut b = [0u8; 84];
    let mut i = 0;
    // First SYCSEND: x8=16, x0=CH0, x1=0, x2=0, x3=opcode 1, x4=tag 1.
    b[i..i + 4].copy_from_slice(&mov(8, process::SYCSEND as u32));
    i += 4;
    b[i..i + 4].copy_from_slice(&mov(0, CH0));
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
    // SYCRECEIVE on CH1: x8=17, x0=CH1, x1=0, x2=0 (blocks for the wakeup).
    b[i..i + 4].copy_from_slice(&mov(8, process::SYCRECEIVE as u32));
    i += 4;
    b[i..i + 4].copy_from_slice(&mov(0, CH1));
    i += 4;
    b[i..i + 4].copy_from_slice(&mov(1, 0));
    i += 4;
    b[i..i + 4].copy_from_slice(&mov(2, 0));
    i += 4;
    b[i..i + 4].copy_from_slice(&SVC);
    i += 4;
    // Second SYCSEND: x8=16, x0=CH0, x1=0, x2=0, x3=opcode 1, x4=tag 2.
    // Returns ERR_CLOSED (2) if the channel was closed by the kill.
    b[i..i + 4].copy_from_slice(&mov(8, process::SYCSEND as u32));
    i += 4;
    b[i..i + 4].copy_from_slice(&mov(0, CH0));
    i += 4;
    b[i..i + 4].copy_from_slice(&mov(1, 0));
    i += 4;
    b[i..i + 4].copy_from_slice(&mov(2, 0));
    i += 4;
    b[i..i + 4].copy_from_slice(&mov(3, 1));
    i += 4;
    b[i..i + 4].copy_from_slice(&mov(4, 2));
    i += 4;
    b[i..i + 4].copy_from_slice(&SVC);
    i += 4;
    // SYSEXIT: x8=0 (exit with x0 = the second send result).
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

    // Two channels: CH0 is the shared channel (client receives, server sends),
    // CH1 is the server's wakeup channel (the test sends here after the kill).
    let ch0 = ipc::create();
    assert_eq!(ch0 as u32, CH0, "first channel is handle 0");
    let ch1 = ipc::create();
    assert_eq!(ch1 as u32, CH1, "second channel is handle 1");

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

    // Phase 1: the client blocks in receive on CH0 (becomes the recv_waiter);
    // the server sends on CH0 (delivered to the client), then blocks in
    // receive on CH1.  After this phase, the client is the recv_waiter on
    // CH0 and the server is the recv_waiter on CH1.
    process::run_all();

    // Kill the client: the kill path calls close_all_for, which sees the
    // client is the recv_waiter on CH0 and closes it.
    process::sys_kill(client as u64);

    // Wake the server: send a message on CH1 (the server's recv_waiter
    // channel).  The server wakes and proceeds to its second send on CH0.
    let wake_msg = Message::new(0, 0, &[]);
    let ch1_handle = ipc::channel(ch1).expect("ch1 channel exists");
    try_send(&ipc::KernSched, ch1_handle, wake_msg).expect("wake send on ch1");

    // Phase 2: the server's second send on CH0 returns ERR_CLOSED (2)
    // because the channel was closed by the kill.  The server exits with
    // status 2.
    process::run_all();

    let cs = process::status(client);
    let ss = process::status(server);
    println!("statuses c {:?} s {:?}", cs, ss);

    // The client was killed: status KILL_STATUS (0x7f).
    check!(cs == Some(0x7f), "client was killed (status 0x7f), got {:?}", cs);
    // The server's second send returned ERR_CLOSED (2): the channel was
    // closed by the client's death.
    check!(ss == Some(2), "server got ERR_CLOSED (2) on second send, got {:?}", ss);

    println!("channel-close passed");
    qemu::exit(qemu::PASS);
}
