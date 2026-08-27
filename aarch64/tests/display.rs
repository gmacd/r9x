//! Integration test: the display server — a user-space process that owns a
//! software framebuffer, paces its frame loop with `SYS_RECEIVE_AT`, and
//! writes a moving color bar.
//!
//! The image spawns the nameserver (so the servers can publish their
//! names), the mailbox, display, and console servers, runs them to a
//! fixpoint, then spawns the console client (a `consclient` test program
//! handed the nameserver's handles) and runs to a fixpoint again, and checks
//! that no process exited: a framebuffer allocation failure or a fault in
//! the frame loop ends a process, so an all-alive table is the success.
//!
//! The client's job is the "display passed" verdict line: it writes it
//! through the console server (`r9x_std::console`), the way the init
//! process will once it moves to user space — not through the kernel's
//! print path.  The image's own diagnostics still go over `SYS_PRINT`; only
//! the verdict takes the terminal path.
//!
//! The bringup is phased, on purpose: the client resolves `/dev/console` by
//! name on its first write and does not retry a not-yet-bound name, so it is
//! spawned and run only after the servers have reached fixpoint (the console
//! server has bound the name and is in its serve loop).  That guarantees the
//! name is present, with no dependence on scheduler interleaving — the same
//! phased bringup the `namespace` image uses.
//!
//! The channel budget is exact: nameserver 2, mailbox 5, display 5, console
//! server 3, client 1 — sixteen, the `NCHANNELS` limit.  A channel created
//! for the client's own use (instead of reusing its console reply channel
//! to block on) would overflow it.
//!
//! The display server's frame loop is infinite (it never exits).  It blocks
//! on the pacing channel's deadline (`SYS_RECEIVE_AT`) between frames, so the
//! scheduler can run the other processes.  The tick wakes the display server
//! at each deadline, and the frame loop continues.

#![no_std]
#![no_main]

use aarch64::{boot, ipc, mailbox, process, qemu, vm};
use port::println;

#[macro_use]
mod common;

/// The built display server's ELF, embedded: xtask's `ServerStep` builds it
/// (static, non-PIE, linked at the shared image base), the kernel's
/// `build.rs` stages it into `OUT_DIR`, and `include_bytes!` pulls the bytes
/// in.
static DISPLAY_ELF: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/display.elf"));
/// The built nameserver's ELF, embedded (same as the `system` image).
static NAMESERVER_ELF: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/nameserver.elf"));
/// The built mailbox server's ELF, embedded.
static MAILBOX_ELF: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/mailbox.elf"));
/// The built console server's ELF, embedded.
static CONSOLE_ELF: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/console.elf"));
/// The built console client's ELF, embedded.
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

    println!("running display");

    unsafe {
        vm::init_user_page_tables();
        vm::switch(vm::user_pagetable(), vm::RootPageTableType::User);
    }

    // The nameserver: spawned first so the display server's BIND is
    // processed.  The nameserver blocks on its first receive; the display
    // server's BIND send wakes it (the IPC fast path).
    let ns_in = ipc::create();
    let ns_out = ipc::create();
    let ns_handles = process::Handles {
        inbound: ns_in as u32,
        outbound: ns_out as u32,
        ns_inbound: 0,
        ns_outbound: 0,
    };
    process::spawn(&process::Image::Elf { bytes: NAMESERVER_ELF, handles: Some(ns_handles) });

    // The mailbox server: owns the Mailbox property interface.  The nameserver's
    // handles go in the extra fields (the server makes its own pair for serving).
    process::spawn(&process::Image::Elf {
        bytes: MAILBOX_ELF,
        handles: Some(process::Handles {
            inbound: 0,
            outbound: 0,
            ns_inbound: ns_in as u32,
            ns_outbound: ns_out as u32,
        }),
    });

    // The display server: handed the nameserver's handles (it finds the
    // mailbox server by RESOLVE).
    let display_handles = process::Handles {
        inbound: ns_in as u32,
        outbound: ns_out as u32,
        ns_inbound: 0,
        ns_outbound: 0,
    };
    process::spawn(&process::Image::Elf { bytes: DISPLAY_ELF, handles: Some(display_handles) });

    // The console server: owns the terminal.  The nameserver's handles go in
    // the extra fields (it makes its own pair for serving).
    process::spawn(&process::Image::Elf {
        bytes: CONSOLE_ELF,
        handles: Some(process::Handles {
            inbound: 0,
            outbound: 0,
            ns_inbound: ns_in as u32,
            ns_outbound: ns_out as u32,
        }),
    });

    // Phase 1: the servers.  Run to fixpoint: the console server has bound
    // `/dev/console` and is in its serve loop; the mailbox and display
    // servers are up.
    process::run_all();
    println!("servers at fixpoint; spawning the client");

    // Phase 2: the console client, now that the console is bound — its
    // `RESOLVE /dev/console` is guaranteed to find the name, with no
    // dependence on scheduler ordering (the same phased bringup the
    // `namespace` image uses).  It writes the "display passed" verdict
    // through the console server, then blocks alive on its own console reply
    // channel.
    process::spawn(&process::Image::Elf {
        bytes: CONSCCLIENT_ELF,
        handles: Some(process::Handles {
            inbound: 0,
            outbound: 0,
            ns_inbound: ns_in as u32,
            ns_outbound: ns_out as u32,
        }),
    });

    process::run_all();

    println!("any_exited {}", process::any_exited());
    // No process exited: the display server's frame buffer allocation
    // succeeded, the frame loop is running (blocked on the pacing deadline),
    // the console client wrote its verdict and is blocked alive (a failed
    // write would have ended it), and the nameserver processed the BINDs.
    // A fault or a panic (a failed allocation) ends a process, so an exited
    // process is the failure.
    check!(
        !process::any_exited(),
        "no process exited (the display server and console client are running)"
    );

    // The verdict line itself is on the terminal: the console client wrote
    // it through the console server, not over this image's print path.
    qemu::exit(qemu::PASS);
}
