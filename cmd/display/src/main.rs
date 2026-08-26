//! The display server: the first user-space process that owns a frame buffer
//! and paces its frame loop.
//!
//! It is the Amiga's demoscene routine in user-space: prepare the frame, wait
//! for the vertical blank (a timer deadline on QEMU — there is no vblank
//! interrupt), update the display, repeat.
//!
//! Double buffering: the server writes to a back buffer in its own heap
//! (cached Normal memory), then copies it to the front buffer (the VideoCore's
//! framebuffer in VC RAM, mapped by the kernel at `FB_VA` with Device memory
//! attributes).  The copy is ~70 µs at the Pi 4's memory bandwidth — far
//! less than one frame period (16 ms), so the tearing window is negligible.
//!
//! The frame loop:
//! 1. Write a moving color bar to the back buffer (driven by the frame
//!    number: the bar's position is `frame_number % width`).
//! 2. Copy the back buffer to the front buffer (the framebuffer).
//! 3. Block on the pacing channel with a timer deadline (`SYS_RECEIVE_AT`):
//!    no spin, the process is off the ready set during the wait.
//! 4. Advance the frame number.
//!
//! The color bar is a vertical bar that moves left-to-right across the frame.
//! The rest of the frame is black.  The pattern proves the frame loop is
//! running (the frame number advances, the bar moves).
//!
//! The server configures the framebuffer itself (via `SYS_FB_CONFIGURE`):
//! the kernel sends the Mailbox `SET_*` + `ALLOCATE` sequence and maps the
//! result into the server's page table.  The server then writes to `FB_VA`.
//!
//! It publishes its name in the nameserver (`/dev/display`), like the console
//! server publishes `/dev/console`.  The nameserver must be up before the
//! BIND is processed.
//!
//! The frame loop is infinite — the display server never exits.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::vec;
use alloc::vec::Vec;
use r9x_abi::{FB_HEIGHT, FB_SIZE, FB_VA, FB_WIDTH};
use r9x_std::fb;
use r9x_std::ipc;
use r9x_std::rt;
use r9x_std::time;

const WIDTH: usize = FB_WIDTH;
const HEIGHT: usize = FB_HEIGHT;

/// The color bar's width, in pixels.
const BAR_WIDTH: usize = 32;

/// The nameserver protocol (mirrors the console server).
const OP_BIND: u16 = 0;
const R_OK: u16 = 0;

/// The name the server publishes under.
const NAME: &[u8] = b"/dev/display";

/// The entry point.
#[inline(never)]
#[unsafe(no_mangle)]
pub extern "C" fn start(dtb_va: usize) -> ! {
    rt::run(dtb_va, main)
}

/// Write a moving color bar to the back buffer.  The bar is a vertical
/// stripe at `x = frame_number % WIDTH`, `BAR_WIDTH` pixels wide, bright
/// green (RGB 0, 255, 0, fully opaque).  The rest of the frame is black.
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
/// The front buffer is mapped with Device memory attributes (uncached), so
/// each byte is a memory access.  At the Pi 4's bandwidth (~17 GB/s) this
/// takes ~70 µs for 1.2 MB — far less than one frame period.
fn flip(back: &[u8]) {
    let front = FB_VA as *mut u8;
    // SAFETY: the kernel mapped the framebuffer at FB_VA (FB_SIZE bytes,
    // Device memory, writable).  `back` is a valid slice of the same size.
    unsafe {
        core::ptr::copy_nonoverlapping(back.as_ptr(), front, FB_SIZE);
    };
}

/// The server body: configure the framebuffer, allocate the back buffer,
/// publish the name, and run the frame loop forever.
fn main() {
    // Configure the framebuffer: the kernel sends the Mailbox SET_* + ALLOCATE
    // sequence and maps the result into this process's page table at FB_VA.
    let result = fb::configure();
    // A failure means the framebuffer is already configured (should not happen
    // in this arc) or the Mailbox request failed.  Either way, the frame loop
    // would fault on the first write to FB_VA, so exit now.
    if result != 0 {
        // The process exits with the svc number (0 = SYSEXIT).  The kernel
        // records the fault status.
        r9x_std::process::exit(1);
    }

    // Allocate the back buffer in this process's heap (cached Normal memory).
    let mut back: Vec<u8> = vec![0u8; FB_SIZE];

    // Write and flip the first frame (frame_number = 0: the bar is at x = 0).
    write_frame(&mut back, 0);
    flip(&back);

    // Create the pacing channel.
    let pacing_chan = ipc::create_chan();

    // Publish the name in the nameserver (like the console server).
    let (ns_in, ns_out) = rt::handles();
    let (in_h, out_h) = ipc::create_pair();
    let mut req = [0u8; NAME.len() + 8];
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
        };
    }
    let _ = ipc::send(ns_in as u64, OP_BIND, 0, &req);
    let mut reply = [0u8; 8];
    let (op, _, _) = ipc::receive(ns_out as u64, &mut reply);
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
