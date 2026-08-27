//! The console client: the test user program for `r9x_std::console`.
//!
//! It is the load-bearing proof that the client API works end to end: the
//! process writes its line through the console server — the first write
//! resolving `/dev/console` through the nameserver — and then blocks alive
//! on its own console reply channel (no one sends there after the write).
//!
//! When the spawner also handed it a report pair, it first reports the
//! successful write to the image over that pair — the image's check that
//! the client's reply channel really serialized its reply (the
//! `two_clients` image).  The `display` image hands only the nameserver's
//! handles (the `Handles::for_server` form, main fields zero), so no report
//! goes out there.
//!
//! A failed write ends the process with a clean, nonzero status: a panic
//! would exit 0 and a fault records the fault status, so the image's
//! checks tell the three deaths apart.

#![no_std]
#![no_main]

use r9x_std::console;
use r9x_std::ipc;
use r9x_std::println;
use r9x_std::process;
use r9x_std::rt;

/// The line the client writes: the `display` image's "display passed"
/// verdict, now on the console server's terminal path instead of the
/// kernel's print path.
const LINE: &[u8] = b"display passed\n";

#[inline(never)]
#[unsafe(no_mangle)]
pub extern "C" fn start(dtb_va: usize) -> ! {
    rt::run(dtb_va, main)
}

fn main() {
    // Write the line through the console server.  A failure ends the process
    // with a clean, nonzero status (a panic would exit 0, a fault records the
    // fault status) — the image's checks tell the three deaths apart.
    if let Err(e) = console::write(LINE) {
        println!("consclient: write failed: {e:?}");
        process::exit(1);
    }

    // Report the successful write to the image, when the spawner handed a
    // report pair: it sits in the main fields, so `handles()[0]` nonzero
    // means one was handed.  The `two_clients` image creates the
    // nameserver's pair before any report pair, so a report pair can never
    // be channel 0 or 1 — the rule is sound for it.  (The handle count
    // alone cannot distinguish: the `for_server` form also carries four.)
    let (rep_in, _rep_out) = rt::handles();
    if rt::n_handles() >= 4 && rep_in != 0 {
        let ok: u32 = 1;
        let _ = ipc::send(rep_in as u64, 0, 0, &ok.to_le_bytes());
    }

    // Block, alive, on this process's own console reply channel: after the
    // write completed, no one sends there again.
    let reply_h = match console::reply_channel() {
        Ok(h) => h,
        Err(e) => {
            println!("consclient: no reply channel: {e:?}");
            process::exit(1);
        }
    };
    let mut buf = [0u8; ipc::MSG_MAX];
    loop {
        let _ = ipc::receive(reply_h, &mut buf);
    }
}
