//! Integration test: two concurrent console clients — per-client
//! reply-channel serialization.
//!
//! The image spawns the nameserver, the console server, and two console
//! clients (each handed the nameserver's handles in the extra fields and a
//! report pair in the main fields).  Each client writes its line through
//! `r9x_std::console` — the first write resolving `/dev/console` through the
//! nameserver — reports the success to the image over its report pair, and
//! then blocks alive on its own console reply channel.
//!
//! The check: both reports arrive, and no process exited.  That pins the
//! property the client API's correctness rests on: the console server must
//! reply `R_OK` on the per-client reply channel embedded in the request
//! payload, not on its own outbound channel, which every client shares.  If
//! a reply went out on the shared outbound channel, one client's waiting
//! receive could take the other client's `R_OK` (or neither would take a
//! valid one) and the victim would block inside its write forever — its
//! report would never arrive, and this image fails.
//!
//! Both clients write the same built-in line, so the terminal text is
//! incidental: the test checks the reports, not the text.
//!
//! The bringup is phased: the servers are run to fixpoint first (the console
//! server has bound `/dev/console` and is in its serve loop), and only then
//! are the clients spawned and run.  A client resolves `/dev/console` by name
//! on its first write and does not retry a not-yet-bound name, so this
//! ordering guarantees the name is present — with no dependence on scheduler
//! interleaving (the same phased bringup the `namespace` image uses).

#![no_std]
#![no_main]

use aarch64::{boot, ipc, mailbox, process, qemu, vm};
use port::ipc::try_receive;
use port::println;

#[macro_use]
mod common;

/// The built console server's ELF, embedded: xtask's `ServerStep` builds it
/// (static, non-PIE, linked at the shared image base), the kernel's
/// `build.rs` stages it into `OUT_DIR`, and `include_bytes!` pulls the bytes
/// in.
static CONSOLE_ELF: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/console.elf"));
/// The built nameserver's ELF, embedded (same as the `system` image).
static NAMESERVER_ELF: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/nameserver.elf"));
/// The built console client's ELF, embedded (same staging as the servers).
static CONSCCLIENT_ELF: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/consclient.elf"));

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

    println!("running two_clients");

    unsafe {
        vm::init_user_page_tables();
        vm::switch(vm::user_pagetable(), vm::RootPageTableType::User);
    }

    // The nameserver's channel pair: created kernel-side (the image is
    // init-context) and handed to the nameserver (its own pair — the
    // first-server asymmetry) and, in the extra fields, to the console
    // server and the two clients.
    let ns_in = ipc::create();
    let ns_out = ipc::create();
    let _ns = process::spawn(&process::Image::Elf {
        bytes: NAMESERVER_ELF,
        handles: Some(process::Handles {
            inbound: ns_in as u32,
            outbound: ns_out as u32,
            ns_inbound: 0,
            ns_outbound: 0,
        }),
    });

    // The console server: owns the terminal.  The nameserver's handles go in
    // the extra fields (it makes its own pair for serving).
    let ns_extra = process::Handles {
        inbound: 0,
        outbound: 0,
        ns_inbound: ns_in as u32,
        ns_outbound: ns_out as u32,
    };
    let _server =
        process::spawn(&process::Image::Elf { bytes: CONSOLE_ELF, handles: Some(ns_extra) });

    // Phase 1: the servers.  Run to fixpoint: the console server has bound
    // `/dev/console` and is blocked on its serve loop, the nameserver on its
    // next request.
    process::run_all();
    println!("servers at fixpoint; spawning the clients");

    // Phase 2: the clients, now that the console is bound — a client's
    // `RESOLVE /dev/console` is guaranteed to find the name, with no
    // dependency on scheduler ordering (the same phased bringup the
    // `namespace` image uses).  Each client gets a report pair in the main
    // fields and the nameserver's handles in the extra fields.  The report
    // pairs are created after the nameserver's pair, so a report pair can
    // never be channel 0 or 1 — the rule the client's report detection
    // relies on (it reports only when `handles()[0]` is nonzero).
    let a_in = ipc::create();
    let a_out = ipc::create();
    let _client_a = process::spawn(&process::Image::Elf {
        bytes: CONSCCLIENT_ELF,
        handles: Some(process::Handles {
            inbound: a_in as u32,
            outbound: a_out as u32,
            ns_inbound: ns_in as u32,
            ns_outbound: ns_out as u32,
        }),
    });
    let b_in = ipc::create();
    let b_out = ipc::create();
    let _client_b = process::spawn(&process::Image::Elf {
        bytes: CONSCCLIENT_ELF,
        handles: Some(process::Handles {
            inbound: b_in as u32,
            outbound: b_out as u32,
            ns_inbound: ns_in as u32,
            ns_outbound: ns_out as u32,
        }),
    });

    println!("two clients spawned, running");

    process::run_all();

    // Both clients report: opcode 0, payload `[1:4 LE]` — "one successful
    // write".  A client whose write failed would have exited with a clean,
    // nonzero status instead; one whose reply was stolen would be blocked
    // inside the write, alive, and have no report to send.
    let a = try_receive(&ipc::KernSched, ipc::channel(a_in).expect("client A's report channel"))
        .unwrap_or_else(|e| panic!("two_clients: report A receive failed: {e:?}"));
    check!(
        a.opcode == 0 && a.len == 4 && a.buf[0] == 1,
        "client A reported one successful write, got opcode {} len {}",
        a.opcode,
        a.len
    );
    let b = try_receive(&ipc::KernSched, ipc::channel(b_in).expect("client B's report channel"))
        .unwrap_or_else(|e| panic!("two_clients: report B receive failed: {e:?}"));
    check!(
        b.opcode == 0 && b.len == 4 && b.buf[0] == 1,
        "client B reported one successful write, got opcode {} len {}",
        b.opcode,
        b.len
    );

    // Every process alive: the clients are blocked on their own console
    // reply channels, the console server on its next receive.
    check!(!process::any_exited(), "no process exited (both clients wrote and are blocked alive)");

    println!("two_clients passed");
    qemu::exit(qemu::PASS);
}
