//! Integration test: the display server — a user-space process that owns a
//! software framebuffer, paces its frame loop with `SYS_RECEIVE_AT`, and
//! writes a moving color bar.
//!
//! The image spawns the nameserver (so the display server can publish
//! `/dev/display`) and the display server (handed the nameserver's handles),
//! runs the system to a fixpoint, and checks that no process exited: a
//! framebuffer allocation failure or a fault in the frame loop ends a
//! process, so an all-alive table is the success.
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

    process::run_all();

    println!("any_exited {}", process::any_exited());
    // No process exited: the display server's frame buffer allocation
    // succeeded, the frame loop is running (blocked on the pacing deadline),
    // and the nameserver processed the BIND.  A fault or a panic (a failed
    // allocation) ends a process, so an exited process is the failure.
    check!(!process::any_exited(), "no process exited (the display server is running)");

    println!("display passed");
    qemu::exit(qemu::PASS);
}
