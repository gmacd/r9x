//! The display server: the first user-space process that owns a frame buffer
//! and paces its frame loop.
//!
//! Double buffering: the server writes to a back buffer in its own heap
//! (cached Normal memory), then copies it to the front buffer (the VideoCore's
//! framebuffer in VC RAM, mapped by the server via `SYS_MAP_MMIO` with Device
//! memory attributes).  The copy is ~70 µs at the Pi 4's memory bandwidth —
//! far less than one frame period (16 ms), so the tearing window is
//! negligible.
//!
//! The server configures the framebuffer via IPC to the mailbox server:
//! sends a configure request, receives the physical address and size, then
//! maps the framebuffer via `SYS_MAP_MMIO`.
//!
//! The frame loop: write a moving color bar to the back buffer, copy it to
//! the front buffer, block on the pacing channel's deadline, repeat.
//!
//! It publishes its name in the nameserver (`/dev/display`).  The frame loop
//! is infinite — the display server never exits.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::vec;
use alloc::vec::Vec;
use r9x_abi::{FB_HEIGHT, FB_SIZE, FB_VA, FB_WIDTH};
use r9x_std::ipc;
use r9x_std::mem;
use r9x_std::process::exit;
use r9x_std::rt;
use r9x_std::time;

const WIDTH: usize = FB_WIDTH;
const HEIGHT: usize = FB_HEIGHT;

/// The color bar's width, in pixels.
const BAR_WIDTH: usize = 32;

/// The nameserver protocol.
const OP_BIND: u16 = 0;
const R_OK: u16 = 0;

/// The mailbox server's IPC protocol.
const OP_CONFIGURE_FB: u16 = 0;

/// The nameserver's RESOLVE opcode.
const OP_RESOLVE: u16 = 1;

/// The name of the mailbox server in the nameserver.
const MBOX_NAME: &[u8] = b"/dev/mailbox";

/// The name the server publishes under.
const NAME: &[u8] = b"/dev/display";

/// The entry point.
#[inline(never)]
#[unsafe(no_mangle)]
pub extern "C" fn start(dtb_va: usize) -> ! {
    rt::run(dtb_va, main)
}

/// Write a moving color bar to the back buffer.
fn write_frame(back: &mut [u8], frame_number: u64) {
    let bar_x = (frame_number as usize) % WIDTH;
    back.fill(0);
    let mut y = 0;
    while y < HEIGHT {
        let row_start = (y * WIDTH + bar_x) * 4;
        let mut x = 0;
        while x < BAR_WIDTH && bar_x + x < WIDTH {
            let px = row_start + x * 4;
            back[px] = 0;
            back[px + 1] = 255;
            back[px + 2] = 0;
            back[px + 3] = 255;
            x += 1;
        }
        y += 1;
    }
}

/// Copy the back buffer to the front buffer (the VideoCore's framebuffer).
fn flip(back: &[u8]) {
    let front = FB_VA as *mut u8;
    // SAFETY: the framebuffer is mapped at FB_VA (FB_SIZE bytes, Device
    // memory, writable).  `back` is a valid slice of the same size.
    unsafe {
        core::ptr::copy_nonoverlapping(back.as_ptr(), front, FB_SIZE);
    };
}

/// The server body: configure the framebuffer via IPC to the mailbox server,
/// allocate the back buffer, publish the name, and run the frame loop forever.
fn main() {
    // Read the nameserver's channel pair.
    let (ns_in, ns_out) = rt::handles();
    let ns_in = ns_in as u64;
    let ns_out = ns_out as u64;

    // Look up the mailbox server's channel pair in the nameserver.
    let mut resolve_req = [0u8; MBOX_NAME.len()];
    // SAFETY: `resolve_req` and `MBOX_NAME` are the same length.
    unsafe {
        core::ptr::copy_nonoverlapping(
            MBOX_NAME.as_ptr(),
            resolve_req.as_mut_ptr(),
            MBOX_NAME.len(),
        )
    };
    let _ = ipc::send(ns_in, OP_RESOLVE, 0, &resolve_req);
    let mut resolve_reply = [0u8; 8];
    let (op, _, _) = ipc::receive(ns_out, &mut resolve_reply);
    if op != R_OK {
        exit(1);
    }
    let mbox_in = u32::from_le_bytes([
        resolve_reply[0],
        resolve_reply[1],
        resolve_reply[2],
        resolve_reply[3],
    ]);
    let mbox_out = u32::from_le_bytes([
        resolve_reply[4],
        resolve_reply[5],
        resolve_reply[6],
        resolve_reply[7],
    ]);

    // Configure the framebuffer via IPC to the mailbox server.
    let mut req = [0u8; 14];
    req[0..2].copy_from_slice(&OP_CONFIGURE_FB.to_le_bytes());
    req[2..6].copy_from_slice(&(WIDTH as u32).to_le_bytes());
    req[6..10].copy_from_slice(&(HEIGHT as u32).to_le_bytes());
    req[10..14].copy_from_slice(&32u32.to_le_bytes());
    let _ = ipc::send(mbox_in as u64, R_OK, 0, &req);

    // Receive the reply: [status:1][phys_lo:4][phys_hi:4][size_lo:4][size_hi:4]
    let mut reply = [0u8; 24];
    let (op, _, _) = ipc::receive(mbox_out as u64, &mut reply);
    if op != R_OK || reply[0] != 0 {
        exit(1);
    }
    let phys_lo = u32::from_le_bytes([reply[1], reply[2], reply[3], reply[4]]);
    let phys_hi = u32::from_le_bytes([reply[5], reply[6], reply[7], reply[8]]);
    let phys = (phys_hi as u64) << 32 | phys_lo as u64;
    if phys == 0 {
        exit(2);
    }

    // Map the framebuffer into this process's page table at FB_VA.
    let map_result = mem::map_mmio(phys, FB_VA as u64, FB_SIZE as u64);
    if map_result != 0 {
        exit(3);
    }

    // Allocate the back buffer in this process's heap (cached Normal memory).
    let mut back: Vec<u8> = vec![0u8; FB_SIZE];

    // Write and flip the first frame.
    write_frame(&mut back, 0);
    flip(&back);

    // Create the pacing channel.
    let pacing_chan = ipc::create_chan();

    // Publish the name in the nameserver.
    let (in_h, out_h) = ipc::create_pair();
    let mut bind_req = [0u8; NAME.len() + 8];
    {
        let n = NAME.len();
        // SAFETY: `bind_req[..n]` and `NAME` are the same length.
        unsafe {
            core::ptr::copy_nonoverlapping(NAME.as_ptr(), bind_req.as_mut_ptr(), n);
            let ib = in_h.to_le_bytes();
            core::ptr::copy_nonoverlapping(ib.as_ptr(), bind_req.as_mut_ptr().add(n), 4);
            let ob = out_h.to_le_bytes();
            core::ptr::copy_nonoverlapping(ob.as_ptr(), bind_req.as_mut_ptr().add(n + 4), 4);
        };
    }
    let _ = ipc::send(ns_in, OP_BIND, 0, &bind_req);
    let mut bind_reply = [0u8; 8];
    let (op, _, _) = ipc::receive(ns_out, &mut bind_reply);
    let _ = op == R_OK;

    // The frame loop: prepare the frame, flip, wait for the deadline, repeat.
    let mut frame_number: u64 = 0;
    loop {
        frame_number = frame_number.wrapping_add(1);
        write_frame(&mut back, frame_number);
        flip(&back);
        let mut buf = [0u8; 0];
        let deadline = time::now().saturating_add(time::FRAME_PERIOD);
        let _ = ipc::receive_at(pacing_chan, &mut buf, deadline);
    }
}
