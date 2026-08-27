//! The console server: the first real r9 user-space server, built as a
//! separate bare-metal Rust executable (the user-binary-loading plan).
//!
//! It does what the hand-assembled `SERVER_TEXT` in the old
//! `console_server` image did, written in Rust: map the PL011 UART's
//! physical register page into this process's own address space via
//! `SYSMAPMMIO`, write `'A'` to the data register, and exit.  From stage 6 it
//! also takes a name: it creates its own channel pair and publishes it under
//! `/dev/console` in the nameserver, so a client can find it by name instead
//! of by a hardcoded handle.
//!
//! It links `r9x_std` — the curated r9 facade that replaces the platform
//! `std`: the message and memory syscalls, the runtime entry, and the heap.
//! The server's own code is the PL011 constant, the nameserver protocol, and
//! the `/dev/console` binding — the device-dumb half of the QNX model.

#![no_std]
#![no_main]

use r9x_std::ipc;
use r9x_std::mem::map_mmio;
use r9x_std::rt;

/// The PL011 UART's physical base on the BCM2711 (QEMU `raspi4b`); a constant
/// the server knows, not something the kernel looks up for it.
const PL011_PHYS: u64 = 0xfe20_1000;
/// The VA the server chooses for the PL011 mapping: in the user (TTBR0) half,
/// far above the image and its stack, so it is clear of both by a wide margin.
/// `SYSMAPMMIO` maps one 4 KiB Device page here; the data register is the
/// first word of that page.
const MMIO_VA: u64 = 0x8000_0000;

/// The nameserver protocol the server speaks (mirrors `nameserver`): `BIND`
/// is the verb the server sends to publish its name; `R_OK` / `R_EFULL` are
/// the results the nameserver replies.
const OP_BIND: u16 = 0;
const R_OK: u16 = 0;
const R_EFULL: u16 = 2;

/// The name the server publishes under.  Absolute, the way a file server's
/// path is — the client resolves this, not a raw channel handle.
const NAME: &[u8] = b"/dev/console";

/// The entry point: where the loader sets `e_entry`.  Forwards to `r9x_std`'s
/// runtime, which records the DTB VA the kernel mapped in and calls this
/// server's [`main`].  (The kernel does not yet pass a DTB to a user entry, so
/// the argument is zero for now — the console knows its own PL011 address.)
#[inline(never)]
#[unsafe(no_mangle)]
pub extern "C" fn start(dtb_va: usize) -> ! {
    rt::run(dtb_va, main)
}

/// Write one byte to the PL011 data register (the kernel console): the
/// server's output to the terminal.
unsafe fn trace(dr: *mut u32, b: u8) {
    // SAFETY: `dr` is the PL011 data register (a Device-memory page); a
    // 32-bit volatile write is the register access the hardware expects.
    unsafe { core::ptr::write_volatile(dr, b as u32) };
}

/// The server body: map the PL011 into this process's own address space,
/// write `'A'` to the data register, create its own channel pair, publish it
/// under `/dev/console` in the nameserver, serve one echo, and exit.
fn main() {
    // If the map fails, the write below faults and the kernel's EL0 fault path
    // kills this process — the failure policy — so the result is not checked.
    let _ = map_mmio(PL011_PHYS, MMIO_VA, 4096);
    let dr = MMIO_VA as *mut u32;
    // SAFETY: `dr` is the PL011 data register, mapped by `map_mmio` above as a
    // Device-memory page; a 32-bit volatile write is the register access the
    // hardware expects.
    unsafe { trace(dr, b'A') };

    // Create this server's own channel pair: the inbound channel clients send
    // to and the outbound channel it replies on.
    let (in_h, out_h) = ipc::create_pair();

    // Read the nameserver's inbound channel: the one to send the `BIND` to.
    // The spawner handed this process the nameserver's handles (extra fields;
    // the main pair is this server's own, created at runtime via SYCCREATECHAN).
    let ns_in = rt::handle_at(2);
    let _ns_out = rt::handle_at(3);

    // Create a reply channel: the nameserver sends the result here, not on its
    // own outbound (which would be shared by all clients and race).
    let reply_chan = ipc::create_chan();

    // Build the `BIND` request: `[name][in:4 LE][out:4 LE][reply:4 LE]`.
    let mut req = [0u8; NAME.len() + 12];
    {
        let n = NAME.len();
        // SAFETY: `req[..n]` and `NAME` are the same length and do not
        // overlap; the 4-byte LE halves are disjoint from each other and from
        // the name.
        unsafe {
            core::ptr::copy_nonoverlapping(NAME.as_ptr(), req.as_mut_ptr(), n);
            let ib = in_h.to_le_bytes();
            core::ptr::copy_nonoverlapping(ib.as_ptr(), req.as_mut_ptr().add(n), 4);
            let ob = out_h.to_le_bytes();
            core::ptr::copy_nonoverlapping(ob.as_ptr(), req.as_mut_ptr().add(n + 4), 4);
            let rc = (reply_chan as u32).to_le_bytes();
            core::ptr::copy_nonoverlapping(rc.as_ptr(), req.as_mut_ptr().add(n + 8), 4);
        };
    }

    // Publish: send the `BIND` to the nameserver's inbound channel.
    let _ = ipc::send(ns_in as u64, OP_BIND, 0, &req);

    // Receive the result on our own reply channel.  It is `R_OK` or `R_EFULL`;
    // on a non-`OK` the server still proceeds — binding is the namespace's
    // concern and the image asserts the bind landed, so a failure here is
    // reported by the image, not the server.
    let mut reply = [0u8; 8];
    let (op, _, _) = ipc::receive(reply_chan, &mut reply);
    let _ = (op == R_OK) || (op == R_EFULL);

    // Persistent console loop: each message's payload is text to write to the
    // UART.  Reply R_OK with no payload.  The console server never exits — it
    // owns the terminal for the lifetime of the system.
    let mut req_buf = [0u8; 256];
    loop {
        let (_, bytes, tag) = ipc::receive(in_h, &mut req_buf);
        let n = bytes.min(256);
        // SAFETY: `dr` is the PL011 data register (Device-memory page).
        unsafe {
            for &b in &req_buf[..n] {
                trace(dr, b);
            }
        };
        ipc::reply(out_h, R_OK, tag, &[]);
    }
}
