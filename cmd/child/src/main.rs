//! The child: the reporter a `SYS_SPAWN` spawner brings up by image index.
//!
//! It is the load-bearing proof that a *running* process (init) can launch
//! another by index and hand it a child-state page: the child reads its
//! spawner-passed channel pair from the generalized `HANDLES_VA` header,
//! reports the handle count back over the message, and blocks — alive.  A
//! child-state that is a zero page (the spawner wrote no pair) would make the
//! check below panic (the process ends, the image's `any_exited` check fails),
//! so liveness *and* the reported value together prove the state reached it.
//!
//! It links `r9x_std` — the curated r9 facade that replaces the platform
//! `std`.

#![no_std]
#![no_main]

use r9x_std::{ipc, rt};

/// The entry point: where the loader sets `e_entry`.  Forwards to `r9x_std`'s
/// runtime, which records the DTB VA the kernel mapped in and calls this
/// process's [`main`].
#[inline(never)]
#[unsafe(no_mangle)]
pub extern "C" fn start(dtb_va: usize) -> ! {
    rt::run(dtb_va, main)
}

/// The process body: read the spawner-passed pair (the generalized
/// `HANDLES_VA` header's first two handles), check it is a real pair, report
/// the handle count back over the message, and block — alive.
fn main() {
    // The spawner's pair, read from the `HANDLES_VA` page the kernel wrote
    // from the spawner's child-state (the generalized header: the pair sits
    // under the count).
    let (in_h, _out_h) = rt::handles();
    // A real pair is `n_handles == 2`: a zero page (the spawner wrote no
    // child-state) has `n_handles == 0`, which is not a pair.  The check is on
    // the count, not the first handle — channel 0 is a valid handle (the table
    // is indexed from 0), so `in_h` may be 0.  A failed check ends the process
    // — the image's `any_exited` check fails on it.
    assert!(rt::n_handles() == 2, "child: no channel pair in the child-state");
    // Report the handle count (2 for a pair) back over the message: the
    // spawner receives it and checks it, so the round-trip proves the state
    // reached this process intact.
    let n_handles: u32 = 2;
    let msg = n_handles.to_le_bytes();
    let _ = ipc::send(in_h as u64, 0, 0, &msg);
    // Block on a receive no one satisfies: the live, done state.  The `spawn`
    // image checks this process is still here (a fault or a panic would have
    // ended it first).
    let mut buf = [0u8; ipc::MSG_MAX];
    loop {
        let _ = ipc::receive(in_h as u64, &mut buf);
    }
}
