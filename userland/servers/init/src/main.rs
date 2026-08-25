//! The init process: the first "real" user process.
//!
//! In this arc it does nothing but exist and block: it creates a channel
//! pair and waits on a receive that no one satisfies.  Stage 7 fills it in
//! as the process manager (spawn servers, handle crashes, restart them).
//!
//! It links `r9x_std` — the curated r9 facade that replaces the platform
//! `std`.

#![no_std]
#![no_main]

use r9x_std::ipc;
use r9x_std::rt;

/// The entry point: where the loader sets `e_entry`.  Forwards to `r9x_std`'s
/// runtime, which records the DTB VA the kernel mapped in and calls this
/// process's [`main`].
#[inline(never)]
#[unsafe(no_mangle)]
pub extern "C" fn start(dtb_va: usize) -> ! {
    rt::run(dtb_va, main)
}

/// The process body: create a channel pair to block on (stage 7's event
/// delivery will target the inbound channel) and wait forever.
fn main() {
    // Create a channel pair to block on.  The inbound channel is the one
    // stage 7's event delivery will target; the outbound is unused for now.
    let (in_h, _out_h) = ipc::create_pair();

    // Block forever: receive on the inbound channel (no one sends).  The
    // buffer is never read — a wakeup is a stage-7 concern.
    let mut buf = [0u8; ipc::MSG_MAX];
    loop {
        let _ = ipc::receive(in_h, &mut buf);
    }
}
