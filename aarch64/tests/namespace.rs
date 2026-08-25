//! Integration test: the file metaphor — a client resolves a server by name
//! and round-trips a byte through it.
//!
//! The image spawns the nameserver (handed its own channel pair — the
//! first-server asymmetry) and the console server (handed the nameserver's
//! pair so it can `BIND`).  After both are up (the console server has bound
//! `/dev/console` and is blocked waiting for a client), the kernel — acting
//! as the client — resolves `/dev/console` by name through the nameserver
//! (no console-server handle is hardcoded), sends a byte on the resolved
//! inbound channel, and checks the echo on the resolved outbound channel.
//!
//! The client is the kernel itself (`port::ipc::try_send` / `try_receive`
//! from init context): the channel table is full with the
//! nameserver's pair + the console server's pair, so a separate client
//! process that `SYCCREATECHAN`s its own pair would overflow it.  The
//! user-space syscall path is already proven by the servers themselves.

#![no_std]
#![no_main]

use aarch64::io::{read_reg, write_reg};
use aarch64::uartpl011::UART0_CR;
use aarch64::{boot, deviceutil, ipc, mailbox, process, qemu, vm};
use port::fdt::DeviceTree;
use port::ipc::{Message, try_receive, try_send};
use port::println;

#[macro_use]
mod common;

static CONSOLE_ELF: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/console.elf"));
static NAMESERVER_ELF: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/nameserver.elf"));

const SERVER_PL011_PHYS: u64 = 0xfe20_1000;

const UARTEN: u32 = 1 << 0;
const TXE: u32 = 1 << 8;
const RXE: u32 = 1 << 9;

/// The nameserver protocol opcodes (mirrors `servers/nameserver`).
const OP_RESOLVE: u16 = 1;
const R_OK: u16 = 0;
const R_ENOENT: u16 = 1;

const NAME: &[u8] = b"/dev/console";

fn enable_pl011(dt: &DeviceTree) {
    let pl011_range = deviceutil::find_dt_physrange(dt, &["arm,pl011"], "can't find pl011")
        .unwrap_or_else(|e| panic!("namespace: {e}"));
    check!(
        pl011_range.start.addr() == SERVER_PL011_PHYS,
        "device tree PL011 base {:#x} matches the server's {:#x}",
        pl011_range.start.addr(),
        SERVER_PL011_PHYS
    );
    let vrange = deviceutil::map_device_register("pl011-test", pl011_range, vm::PageSize::Page4K)
        .unwrap_or_else(|e| panic!("namespace: map pl011: {e:?}"));
    let cr = read_reg(&vrange, UART0_CR);
    write_reg(&vrange, UART0_CR, cr | UARTEN | TXE | RXE);
    let cr = read_reg(&vrange, UART0_CR);
    check!(
        cr & (UARTEN | TXE | RXE) == (UARTEN | TXE | RXE),
        "CR has UARTEN+TXE+RXE after enable, got {cr:#x}"
    );
}

#[unsafe(no_mangle)]
pub extern "C" fn main9(dtb_va: usize) {
    boot::irq_ops();
    let dt = unsafe { boot::device_tree(dtb_va) };
    boot::page_allocator(&dt, dtb_va).unwrap();
    mailbox::init(&dt);
    boot::console(&dt);
    boot::interrupts(&dt);

    println!("running namespace");

    unsafe {
        vm::init_user_page_tables();
        vm::switch(vm::user_pagetable(), vm::RootPageTableType::User);
    }

    enable_pl011(&dt);
    println!("pl011 enabled (kernel side)");

    // The nameserver's channel pair: created kernel-side (the image is
    // init-context) and handed to the nameserver (its own pair — the
    // first-server asymmetry) and to the console server (the pair it
    // `BIND`s to).
    let ns_in = ipc::create();
    let ns_out = ipc::create();
    let ns_handles = process::Handles { inbound: ns_in as u32, outbound: ns_out as u32 };

    let ns =
        process::spawn(&process::Image::Elf { bytes: NAMESERVER_ELF, handles: Some(ns_handles) });
    let server =
        process::spawn(&process::Image::Elf { bytes: CONSOLE_ELF, handles: Some(ns_handles) });
    println!("nameserver + console server spawned, running");

    // Run to fixpoint: the nameserver is blocked on its first receive;
    // the console server has mapped the PL011, written 'A', created its
    // pair, sent the BIND (which woke the nameserver), received the BIND
    // reply, and is now blocked on its post-bind receive (waiting for a
    // client).
    process::run_all();
    println!("servers at fixpoint; resolving /dev/console");

    // RESOLVE: send the name to the nameserver's inbound channel, run the
    // nameserver (it wakes, looks up the name, replies with the pair),
    // then read the reply.
    let resolve_req = Message::new(OP_RESOLVE, 1, NAME);
    let ns_in_ch = ipc::channel(ns_in).expect("ns_in channel exists");
    try_send(&ipc::KernSched, ns_in_ch, resolve_req).expect("resolve send");
    process::run_all();
    let ns_out_ch = ipc::channel(ns_out).expect("ns_out channel exists");
    let reply = try_receive(&ipc::KernSched, ns_out_ch)
        .unwrap_or_else(|e| panic!("namespace: resolve receive failed: {e:?}"));
    check!(
        reply.opcode == R_OK,
        "resolve returned R_OK, got opcode {} (ENoent={})",
        reply.opcode,
        R_ENOENT
    );
    check!(reply.len == 8, "resolve reply carries an 8-byte pair, got len {}", reply.len);
    // Extract the console server's (in, out) pair from the reply payload.
    let con_in = u32::from_le_bytes(reply.buf[0..4].try_into().unwrap()) as usize;
    let con_out = u32::from_le_bytes(reply.buf[4..8].try_into().unwrap()) as usize;
    println!("resolved /dev/console: in={} out={}", con_in, con_out);

    // Round-trip: send a byte on the console server's inbound channel,
    // run the console server (it wakes, echoes the byte back), then read
    // the echo.
    let byte = b'x';
    let roundtrip_req = Message::new(0, 2, &[byte]);
    let con_in_ch = ipc::channel(con_in).expect("con_in channel exists");
    try_send(&ipc::KernSched, con_in_ch, roundtrip_req).expect("roundtrip send");
    process::run_all();
    let con_out_ch = ipc::channel(con_out).expect("con_out channel exists");
    let echo = try_receive(&ipc::KernSched, con_out_ch)
        .unwrap_or_else(|e| panic!("namespace: roundtrip receive failed: {e:?}"));
    check!(
        echo.len == 1 && echo.buf[0] == byte,
        "roundtrip echoed byte {byte}, got len {} buf[0]={:?}",
        echo.len,
        echo.buf[0]
    );
    println!("roundtrip byte ok");

    // The console server exited after its one-shot reply.
    check!(
        process::status(server) == Some(0),
        "console server exited 0, got {:?}",
        process::status(server)
    );
    // The nameserver is still alive (blocked on receive, waiting for the
    // next request): it has no clean exit this arc.
    check!(
        process::status(ns).is_none(),
        "nameserver still alive (blocked), got {:?}",
        process::status(ns)
    );

    println!("namespace passed");
    qemu::exit(qemu::PASS);
}
