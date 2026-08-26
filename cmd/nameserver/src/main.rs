//! The nameserver: the process the whole file metaphor resolves through.
//!
//! It owns the bind table — a fixed `name -> (in, out) channel-pair` map — and
//! serves three verbs over the message syscalls:
//!
//! - `BIND`: a file server publishes its channel pair under a name.
//! - `RESOLVE`: a client asks for a name and gets the pair back.
//! - `UNBIND`: a file server withdraws its name.
//!
//! A channel is unidirectional, so a request/reply runs over a *pair*: the
//! client sends on the server's inbound channel and receives on its outbound
//! channel.  The bind entry therefore stores the pair, not one handle — a name
//! that resolved to only the inbound channel would let a client send but never
//! receive the reply.
//!
//! It is the first server: its own pair is passed to it by its spawner (a page
//! at the `HANDLES_VA`), not created by it — nothing exists yet that a client
//! could ask to find the nameserver, so the spawner hands it the pair
//! directly.
//!
//! It links `r9x_std` — the curated r9 facade that replaces the platform
//! `std`.  It owns its data (the bind table) entirely in its own address
//! space; the kernel stays a message-passing broker.

#![no_std]
#![no_main]

/// The bind table and the result codes it replies with (a pure-`core` module,
/// unit-tested on the host).
mod bind_table;
use bind_table::{BindTable, Pair, R_ENOENT, R_OK};

use r9x_std::ipc;
use r9x_std::rt;

/// The request verbs (the message opcode a client sends).
const OP_BIND: u16 = 0;
const OP_RESOLVE: u16 = 1;
const OP_UNBIND: u16 = 2;

/// The entry point: where the loader sets `e_entry`.  Forwards to `r9x_std`'s
/// runtime, which records the DTB VA the kernel mapped in and calls this
/// server's [`main`].
#[inline(never)]
#[unsafe(no_mangle)]
pub extern "C" fn start(dtb_va: usize) -> ! {
    rt::run(dtb_va, main)
}

/// The server body: read its own pair and serve the bind table.  A server runs
/// until it is killed; there is no clean exit this arc, so the loop is
/// unbounded.
fn main() {
    // This server's own pair, passed by its spawner.
    let (in_h, out_h) = rt::handles();
    let pair = Pair { in_h, out_h };
    let mut table = BindTable::new();
    let mut buf = [0u8; ipc::MSG_MAX];
    loop {
        let (op, bytes, tag) = ipc::receive(pair.in_h as u64, &mut buf);
        let payload = &buf[..bytes.min(ipc::MSG_MAX)];
        // Every request carries a `reply_chan` (4 bytes LE) as its last field:
        // the channel the nameserver sends the reply on.  The client creates
        // it and receives the reply there, so no two clients share the
        // nameserver's outbound channel.
        let reply_chan =
            u32::from_le_bytes(payload[payload.len() - 4..].try_into().unwrap()) as u64;
        // Dispatch the verb.  The payload layout: a BIND carries the name
        // followed by the bound pair and the reply channel
        // (`[name][in:4][out:4][reply:4]`); a RESOLVE / UNBIND carries the
        // name and the reply channel (`[name][reply:4]`).
        let (result, found) = match op {
            OP_BIND => {
                let name_len = payload.len() - 12;
                let name = &payload[..name_len];
                let in_h = u32::from_le_bytes(payload[name_len..name_len + 4].try_into().unwrap());
                let out_h =
                    u32::from_le_bytes(payload[name_len + 4..name_len + 8].try_into().unwrap());
                (table.bind(name, Pair { in_h, out_h }), None)
            }
            OP_RESOLVE => match table.resolve(&payload[..payload.len() - 4]) {
                Some(p) => (R_OK, Some(p)),
                None => (R_ENOENT, None),
            },
            OP_UNBIND => (table.unbind(&payload[..payload.len() - 4]), None),
            // An unknown verb is answered like an unknown name.
            _ => (R_ENOENT, None),
        };
        // A RESOLVE that found a name replies with the pair; every other reply
        // is empty.  `ptr::copy_nonoverlapping` rather than `copy_from_slice`:
        // the latter is a non-inlined call in this `no_std` build and its stack
        // frame exceeds the mapped user stack.
        let mut out = [0u8; 8];
        let out_slice: &[u8] = match found {
            Some(p) => {
                // SAFETY: the two 4-byte halves of `out` are disjoint and the
                // source arrays are 4 bytes each.
                unsafe {
                    let ib = p.in_h.to_le_bytes();
                    core::ptr::copy_nonoverlapping(ib.as_ptr(), out.as_mut_ptr(), 4);
                    let ob = p.out_h.to_le_bytes();
                    core::ptr::copy_nonoverlapping(ob.as_ptr(), out.as_mut_ptr().add(4), 4);
                };
                &out[..8]
            }
            None => &[],
        };
        ipc::reply(reply_chan, result, tag, out_slice);
    }
}
