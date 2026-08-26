//! Integration test: `SYS_SPAWN` — a running process brings up another by
//! index.
//!
//! A whole kernel image like the others: it links the kernel library and runs
//! its own `main9`.  It registers the kernel's image registry with the child
//! (a Rust-built ELF, built by xtask's `ServerStep`, embedded here) and spawns
//! init — itself a Rust-built ELF, handed a channel pair by the image (the
//! init-context path, `process::spawn`).  init then does what a process
//! manager does: it `SYS_SPAWN`s the child by index, handing it a child-state
//! page (the same pair, so the child reports back), drives the two error cases
//! — a bad index and a full process table are both *error codes* the spawner
//! recovers from, not faults — and receives the child's round-trip message,
//! which proves the child-state reached the child intact.
//!
//! The image checks, after `run_all`, that no process exited: a fault or a
//! panic (a failed check in init or the child) ends a process, so an all-alive
//! table is the success.  The child has no id the image knows (its spawner
//! learned it), so the check is over the whole table, not one slot.

#![no_std]
#![no_main]

use aarch64::{boot, ipc, mailbox, process, qemu, registry, vm};
use port::println;

#[macro_use]
mod common;

/// The built init's ELF, embedded: xtask's `ServerStep` builds it (static,
/// non-PIE, linked at the shared image base), this crate's `build.rs` stages it
/// into `OUT_DIR`, and `include_bytes!` pulls the bytes in.  The loader reads
/// it through `Image::Elf`.
static INIT_ELF: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/init.elf"));
/// The built child's ELF, embedded: the registry's one entry, the image init
/// spawns by index.
static CHILD_ELF: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/child.elf"));

/// The registry's one entry: the child.  init spawns it by index 0.
static CHILD: registry::EmbeddedElf = registry::EmbeddedElf { bytes: CHILD_ELF, name: "child" };

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

    println!("running spawn");

    unsafe {
        vm::init_user_page_tables();
        vm::switch(vm::user_pagetable(), vm::RootPageTableType::User);
    }

    // The image's channel pair: created kernel-side (the image is init-context)
    // and handed to init, whose `SYS_SPAWN` children report back over it.  The
    // image keeps the pair so the servers never see each other's handles by
    // constant.
    let in_h = ipc::create();
    let out_h = ipc::create();
    let handles = process::Handles {
        inbound: in_h as u32,
        outbound: out_h as u32,
        extra_inbound: 0,
        extra_outbound: 0,
    };

    // Register the image registry before any spawn can reference an index: the
    // load-bearing ordering (a spawn by an unregistered index is the error, not
    // a fault).  init is spawned by the image (the init-context path), not by
    // index, so the registry holds only the child (index 0).
    registry::register(&[&CHILD]);
    let init = process::spawn(&process::Image::Elf { bytes: INIT_ELF, handles: Some(handles) });
    println!("init spawned ({init}), registry has the child (index 0), running");

    process::run_all();

    let init_status = process::status(init);
    println!("init status {init_status:?}, any_exited {}", process::any_exited());
    // No process exited: init drove the spawn, the error cases, and the
    // round-trip, and is blocked; the child and the fillers are blocked too.
    // A fault or a panic (a failed check) ends a process, so an exited process
    // is the failure.
    check!(
        init_status.is_none(),
        "init alive (drove spawn + errors + round-trip), got {init_status:?}"
    );
    check!(!process::any_exited(), "no process exited (child read its child-state and is alive)");

    println!("spawn passed");
    qemu::exit(qemu::PASS);
}
