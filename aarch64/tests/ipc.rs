//! Integration test: message passing with priority inheritance is
//! deterministic.
//!
//! A high-urgency client (level 16) and a low-urgency server (level 200)
//! exchange one request and one reply over two unidirectional channels,
//! while a mid-urgency busy process (level 128) waits its turn.  The client
//! sends a request, then blocks in receive for the reply.  The send's fast
//! path (server-at-client) boosts the server to the client's level 16, so
//! the server — not the busy process at 128 — is picked next and services
//! the request before the busy process can run.
//!
//! The assertion is a promptness ordering on the switch-in trace
//! (`process::run_order`): while the client is alive the busy process is
//! never switched in.  That is load-bearing — without the inheritance the
//! server stays at 200, the busy at 128 outranks it, and it is switched in
//! between the client's send and the client switching back in with its
//! reply.

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

/// The channel handles: the first created channel is the request channel,
/// the second is the reply channel.
const REQ_CH: u32 = 0;
const REPLY_CH: u32 = 1;

/// The client: level 16.  Sends a request on `REQ_CH` (opcode 1, tag 1),
/// then blocks in receive for the reply on `REPLY_CH`, then exits 10.
fn client_body() -> [u8; 56] {
    let mut b = [0u8; 56];
    let mut i = 0;
    // SYCSEND: x8=2, x0=REQ_CH, x1=0, x2=0, x3=opcode 1, x4=tag 1.
    b[i..i + 4].copy_from_slice(&mov(8, process::SYCSEND as u32));
    i += 4;
    b[i..i + 4].copy_from_slice(&mov(0, REQ_CH));
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
    // SYCRECEIVE: x8=17, x0=REPLY_CH, x1=0, x2=0.
    b[i..i + 4].copy_from_slice(&mov(8, process::SYCRECEIVE as u32));
    i += 4;
    b[i..i + 4].copy_from_slice(&mov(0, REPLY_CH));
    i += 4;
    b[i..i + 4].copy_from_slice(&mov(1, 0));
    i += 4;
    b[i..i + 4].copy_from_slice(&mov(2, 0));
    i += 4;
    b[i..i + 4].copy_from_slice(&SVC);
    i += 4;
    // Exit with status 10.
    b[i..i + 4].copy_from_slice(&mov(8, 10));
    i += 4;
    b[i..i + 4].copy_from_slice(&SVC);
    i += 4;
    assert_eq!(i, b.len());
    b
}

/// The server: level 200.  Blocks in receive for a request on `REQ_CH`,
/// replies on `REPLY_CH` (opcode 2, tag 1), then blocks in receive for the
/// next request (there is none, so it stays blocked and `run_all` ends).
fn server_body() -> [u8; 68] {
    let mut b = [0u8; 68];
    let mut i = 0;
    // SYCRECEIVE: x8=17, x0=REQ_CH, x1=0, x2=0.
    b[i..i + 4].copy_from_slice(&mov(8, process::SYCRECEIVE as u32));
    i += 4;
    b[i..i + 4].copy_from_slice(&mov(0, REQ_CH));
    i += 4;
    b[i..i + 4].copy_from_slice(&mov(1, 0));
    i += 4;
    b[i..i + 4].copy_from_slice(&mov(2, 0));
    i += 4;
    b[i..i + 4].copy_from_slice(&SVC);
    i += 4;
    // SYCREPLY: x8=18, x0=REPLY_CH, x1=0, x2=0, x3=opcode 2, x4=tag 1.
    b[i..i + 4].copy_from_slice(&mov(8, process::SYCREPLY as u32));
    i += 4;
    b[i..i + 4].copy_from_slice(&mov(0, REPLY_CH));
    i += 4;
    b[i..i + 4].copy_from_slice(&mov(1, 0));
    i += 4;
    b[i..i + 4].copy_from_slice(&mov(2, 0));
    i += 4;
    b[i..i + 4].copy_from_slice(&mov(3, 2));
    i += 4;
    b[i..i + 4].copy_from_slice(&mov(4, 1));
    i += 4;
    b[i..i + 4].copy_from_slice(&SVC);
    i += 4;
    // SYCRECEIVE: x8=17, x0=REQ_CH, x1=0, x2=0.  Blocks (no more requests).
    b[i..i + 4].copy_from_slice(&mov(8, process::SYCRECEIVE as u32));
    i += 4;
    b[i..i + 4].copy_from_slice(&mov(0, REQ_CH));
    i += 4;
    b[i..i + 4].copy_from_slice(&mov(1, 0));
    i += 4;
    b[i..i + 4].copy_from_slice(&mov(2, 0));
    i += 4;
    b[i..i + 4].copy_from_slice(&SVC);
    i += 4;
    assert_eq!(i, b.len());
    b
}

/// The busy process: level 128.  Yields once, then exits 12.
fn busy_body() -> [u8; 16] {
    let mut b = [0u8; 16];
    let mut i = 0;
    b[i..i + 4].copy_from_slice(&mov(8, 1)); // yield
    i += 4;
    b[i..i + 4].copy_from_slice(&SVC);
    i += 4;
    b[i..i + 4].copy_from_slice(&mov(8, 12)); // exit 12
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
const BUSY_TEXT_VA: usize = 0x3000;
const BUSY_STACK_VA: usize = 0x30000;

#[unsafe(no_mangle)]
pub extern "C" fn main9(dtb_va: usize) {
    boot::irq_ops();
    let dt = unsafe { boot::device_tree(dtb_va) };
    boot::page_allocator(&dt, dtb_va).unwrap();
    mailbox::init(&dt);
    boot::console(&dt);
    boot::interrupts(&dt);

    println!("running ipc-pi");

    unsafe {
        vm::init_user_page_tables();
        vm::switch(vm::user_pagetable(), RootPageTableType::User);
    }

    // Two unidirectional channels: requests one way, replies the other.
    let req_ch = ipc::create();
    let reply_ch = ipc::create();
    assert_eq!(req_ch as u32, REQ_CH, "req channel is the first handle");
    assert_eq!(reply_ch as u32, REPLY_CH, "reply channel is the second handle");

    println!("spawning client, server, busy");
    // The server is spawned first: it is picked first (lowest index) and
    // blocks in receive, so the client's first send is the fast path.
    let server = process::spawn(&process::Image::Raw {
        text: &server_body(),
        text_va: SERVER_TEXT_VA,
        stack_va: SERVER_STACK_VA,
    });
    let client = process::spawn(&process::Image::Raw {
        text: &client_body(),
        text_va: CLIENT_TEXT_VA,
        stack_va: CLIENT_STACK_VA,
    });
    let busy = process::spawn(&process::Image::Raw {
        text: &busy_body(),
        text_va: BUSY_TEXT_VA,
        stack_va: BUSY_STACK_VA,
    });

    process::set_priority(client, process::Priority::new(16));
    process::set_priority(server, process::Priority::new(200));
    process::set_priority(busy, process::Priority::new(128));

    println!("running the table");
    process::run_all();

    let order = process::run_order();
    println!(
        "statuses c {:?} s {:?} b {:?}, preemptions {}, run_order {:?}",
        process::status(client),
        process::status(server),
        process::status(busy),
        process::preemptions(),
        order
    );

    check!(
        process::status(client) == Some(10),
        "client finished and exited 10, got {:?}",
        process::status(client)
    );
    check!(
        process::status(server).is_none(),
        "server blocked on the next request (still alive), got {:?}",
        process::status(server)
    );

    // The busy's first switch-in, if any, comes after the client's last:
    // the exchange kept the busy behind it for the client's whole lifetime.
    let client_last = order.iter().rposition(|&x| x == client);
    let busy_first = order.iter().position(|&x| x == busy);
    check!(
        match (client_last, busy_first) {
            (Some(cl), Some(bf)) => bf > cl,
            (Some(_), None) => true,
            _ => false,
        },
        "the busy process was not switched in while the client was alive (server-at-client inheritance); order {:?}",
        order
    );

    println!("ipc-pi passed");
    qemu::exit(qemu::PASS);
}
