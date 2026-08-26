//! The init process: the first "real" user process, and now the process
//! manager.
//!
//! It is the load-bearing proof that a *running* process can launch another
//! by image index — not just the boot image (which hard-starts a server from
//! `main9`).  init reads the channel pair the image handed it (the generalized
//! `HANDLES_VA` header), writes a child-state page on its own heap, and
//! `SYS_SPAWN`s the child the registry registered.  It then drives the two
//! error cases — a bad index and a full process table are both *error codes*
//! the spawner recovers from, not faults — and receives the child's
//! round-trip message, which proves the child-state reached the child intact.
//!
//! It links `r9x_std` — the curated r9 facade that replaces the platform
//! `std`.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::vec;
use alloc::vec::Vec;
use r9x_std::{ipc, process, rt};

/// The entry point: where the loader sets `e_entry`.  Forwards to `r9x_std`'s
/// runtime, which records the DTB VA the kernel mapped in and calls this
/// process's [`main`].
#[inline(never)]
#[unsafe(no_mangle)]
pub extern "C" fn start(dtb_va: usize) -> ! {
    rt::run(dtb_va, main)
}

/// The image registry's index of the child (the image registers `[child]`).
const CHILD_INDEX: u64 = 0;

/// The process body: read the spawner's pair, bring up the child by index,
/// drive the two error cases, check the child's round-trip, and block — alive.
fn main() {
    // The image's pair, read from the `HANDLES_VA` page the kernel wrote
    // (the generalized header's first two handles).  The count — not the first
    // handle — is the check: channel 0 is a valid handle (the table is indexed
    // from 0), so `in_h` may be 0.
    let (in_h, out_h) = rt::handles();
    assert!(rt::n_handles() == 2, "init: no channel pair in the child-state");

    // The child's child-state page: a whole page on this process's heap, laid
    // out as the generalized header `[2, in_h, out_h]` — the same pair, so the
    // child reports back over `in_h`, which init receives.  The kernel reads
    // this page from init's address space during the spawn and writes it to
    // the child's `HANDLES_VA` page.
    let mut state: Vec<u8> = vec![0u8; 4096];
    state[0..4].copy_from_slice(&2u32.to_le_bytes()); // n_handles
    state[4..8].copy_from_slice(&in_h.to_le_bytes()); // handles[0]
    state[8..12].copy_from_slice(&out_h.to_le_bytes()); // handles[1]
    let state_va = state.as_ptr() as usize;

    // Spawn the child by index: the process-manager primitive.  A running
    // process launches another, handing it a child-state page — the boot image
    // never hard-starts it.
    let _child = match process::spawn(CHILD_INDEX, state_va, 128) {
        Ok(id) => id,
        Err(e) => panic!("init: spawn by index failed: {e:?}"),
    };

    // A bad index is an error, not a fault: out of range (the registry has one
    // entry, so 999 is past it).  The spawner recovers (it checks the error),
    // it does not fault the kernel.
    match process::spawn(999, state_va, 128) {
        Err(process::SpawnErr::BadIndex) => {}
        other => panic!("init: bad index must be BadIndex, got {other:?}"),
    }

    // A full process table is an error, not a fault.  init doesn't know how
    // many slots the kernel's other servers (nameserver, console) took, so it
    // fills the table by spawning until the kernel refuses: the spawn returns
    // `NoSlot` when the table is full (however many other processes share it),
    // and that refusal — not a fault — is the check.
    loop {
        match process::spawn(CHILD_INDEX, state_va, 128) {
            Ok(_) => {}
            Err(process::SpawnErr::NoSlot) => break,
            other => panic!("init: full table must be NoSlot, got {other:?}"),
        }
    }

    // Receive the child's round-trip message and check it: the child reported
    // its handle count (2) back over `in_h`, so the round-trip proves the
    // child-state reached the child intact.
    let mut buf = [0u8; ipc::MSG_MAX];
    let (_op, bytes, _tag) = ipc::receive(in_h as u64, &mut buf);
    assert!(bytes >= 4, "init: child's round-trip message too short: {bytes}");
    let got = u32::from_le_bytes(buf[..4].try_into().unwrap());
    assert_eq!(got, 2, "init: child read the wrong child-state: {got}");

    // Block (alive): the `spawn` image checks init is still here — a fault or
    // a panic (a failed check) would have ended it first.
    loop {
        let _ = ipc::receive(in_h as u64, &mut buf);
    }
}
