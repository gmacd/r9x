//! The display server: the first user-space process that owns a frame buffer
//! and paces its frame loop.
//!
//! It is the Amiga's demoscene routine in user-space: prepare the frame, wait
//! for the vertical blank (a timer deadline on QEMU — there is no vblank
//! interrupt), update the display, repeat.  The framebuffer is in VC RAM
//! (allocated by the VideoCore firmware via the Mailbox property interface);
//! the kernel maps it into this process's page table at `FB_VA`.
//!
//! The frame loop:
//! 1. Write a moving color bar to the frame buffer (driven by the frame
//!    number: the bar's position is `frame_number % width`).
//! 2. Block on the pacing channel with a timer deadline (`SYS_RECEIVE_AT`):
//!    no spin, the process is off the ready set during the wait.
//! 3. Advance the frame number.
//!
//! The color bar is a vertical bar that moves left-to-right across the frame.
//! The rest of the frame is black.  The pattern proves the frame loop is
//! running (the frame number advances, the bar moves).
//!
//! It publishes its name in the nameserver (`/dev/display`), like the console
//! server publishes `/dev/console`.  The nameserver must be up before the
//! BIND is processed (the bringup order: nameserver first, display second).
//!
//! The frame loop is infinite — the display server never exits.  It blocks
//! on the pacing channel's deadline (`SYS_RECEIVE_AT`) between frames, so the
//! scheduler can run the other processes.

#![no_std]
#![no_main]

extern crate alloc;

use r9x_abi::{FB_HEIGHT, FB_SIZE, FB_VA, FB_WIDTH};
use r9x_std::ipc;
use r9x_std::rt;
use r9x_std::time;

const WIDTH: usize = FB_WIDTH;
const HEIGHT: usize = FB_HEIGHT;

/// The color bar's width, in pixels.  A 32-pixel-wide bar — wide enough to
/// see, narrow enough to move noticeably across the frame.
const BAR_WIDTH: usize = 32;

/// The nameserver protocol (mirrors the console server): `BIND` is the verb
/// the server sends to publish its name; `R_OK` is the result.
const OP_BIND: u16 = 0;
const R_OK: u16 = 0;

/// The name the server publishes under.
const NAME: &[u8] = b"/dev/display";

/// The entry point: where the loader sets `e_entry`.  Forwards to
/// `r9x_std`'s runtime, which records the DTB VA and calls [`main`].
#[inline(never)]
#[unsafe(no_mangle)]
pub extern "C" fn start(dtb_va: usize) -> ! {
    rt::run(dtb_va, main)
}

/// Write a moving color bar to the frame buffer.  The bar is a vertical
/// stripe at `x = frame_number % WIDTH`, `BAR_WIDTH` pixels wide, bright
/// green (RGB 0, 255, 0, fully opaque).  The rest of the frame is black.
///
/// # Safety
/// `fb` must point to a valid, writable region of at least `FB_SIZE` bytes.
unsafe fn write_frame(fb: *mut u8, frame_number: u64) {
    let bar_x = (frame_number as usize) % WIDTH;
    // SAFETY: fb points to a valid, writable region of at least FB_SIZE bytes.
    unsafe {
        core::ptr::write_bytes(fb, 0, FB_SIZE);
        let mut y = 0;
        while y < HEIGHT {
            let row_start = (y * WIDTH + bar_x) * 4;
            let mut x = 0;
            while x < BAR_WIDTH && bar_x + x < WIDTH {
                let px = row_start + x * 4;
                core::ptr::write_volatile(fb.add(px), 0);
                core::ptr::write_volatile(fb.add(px + 1), 255);
                core::ptr::write_volatile(fb.add(px + 2), 0);
                core::ptr::write_volatile(fb.add(px + 3), 255);
                x += 1;
            }
            y += 1;
        }
    }
}

/// The server body: write to the kernel-mapped framebuffer, publish the
/// name, and run the frame loop forever.
fn main() {
    // The framebuffer is in VC RAM (a physical address returned by the
    // VideoCore firmware via the Mailbox property interface).  The kernel
    // maps it into this process's page table at `FB_VA` with Device memory
    // attributes (uncached), so the VideoCore sees writes immediately.
    //
    // SAFETY: the kernel mapped the framebuffer into this process's page
    // table at `FB_VA` before spawning us.  The region is `FB_SIZE` bytes,
    // readable and writable, and no other pointer aliases it (the VideoCore
    // reads it, but the ARM never writes it through any other pointer).
    let fb_ptr = FB_VA as *mut u8;

    // Write the first frame (frame_number = 0: the bar is at x = 0).
    // SAFETY: the kernel mapped the framebuffer at `FB_VA` before spawning us.
    unsafe { write_frame(fb_ptr, 0) };

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

    // The frame loop: prepare the frame, wait for the deadline, repeat.
    // This is the Amiga's demoscene routine.  The wait is a `SYS_RECEIVE_AT`
    // on the pacing channel — no spin, the process is off the ready set.
    let mut frame_number: u64 = 0;
    loop {
        frame_number = frame_number.wrapping_add(1);
        // SAFETY: the kernel mapped the framebuffer at `FB_VA` before spawning us.
        unsafe { write_frame(fb_ptr, frame_number) };
        let mut buf = [0u8; 0];
        let deadline = time::now().saturating_add(time::FRAME_PERIOD);
        let _ = ipc::receive_at(pacing_chan, &mut buf, deadline);
    }
}
