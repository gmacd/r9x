//! The debug print facility: `println!` to the kernel's PL011 via `SYS_PRINT`.
//!
//! A debug/boot facility, not a production I/O path. The production path is
//! IPC to the console server (`/dev/cons`, task 88). This exists so a server
//! can report what it is doing before the console server is up (or while
//! debugging the console server itself).
//!
//! Not thread-safe: bytes within a single call are contiguous, but calls from
//! different processes may interleave (the same guarantee as the kernel's
//! `iprintln!`).

use core::fmt::Write;

use crate::sys;

/// The format buffer size: 256 bytes of content + room for a newline.
/// Longer output is truncated with `...`.
const BUF_SIZE: usize = 259;

/// Write a string to the kernel debug console.
pub fn print_str(s: &str) {
    let mut buf = [0u8; BUF_SIZE];
    let n = s.len().min(BUF_SIZE - 1);
    buf[..n].copy_from_slice(&s.as_bytes()[..n]);
    buf[n] = b'\n';
    // SAFETY: `buf.as_ptr()` is a valid user VA for `n+1` bytes (the buffer
    // is on the caller's stack); the kernel reads it via `copy_from_user`
    // and writes to the PL011.
    unsafe {
        sys::sys(r9x_abi::SYS_PRINT, buf.as_ptr() as u64, (n + 1) as u64, 0, 0, 0);
    }
}

/// Format into a 256-byte stack buffer, then `SYS_PRINT`. Truncated with
/// `...` if the formatted output exceeds the buffer.
pub fn print_fmt(args: core::fmt::Arguments<'_>) {
    let mut buf = [0u8; BUF_SIZE];
    let (pos, truncated) = {
        let mut w = BufWriter { buf: &mut buf, pos: 0 };
        let truncated = w.write_fmt(args).is_err();
        (w.pos, truncated)
    };
    if truncated {
        // Overwrite the last 3 bytes of the content with "..."
        let end = pos.min(BUF_SIZE - 1);
        let start = end.saturating_sub(3);
        buf[start..end].copy_from_slice(b"...");
    }
    let n = pos.min(BUF_SIZE - 1);
    buf[n] = b'\n';
    // SAFETY: `buf.as_ptr()` is a valid user VA for `n+1` bytes (the buffer
    // is on the caller's stack); the kernel reads it via `copy_from_user`
    // and writes to the PL011.
    unsafe {
        sys::sys(r9x_abi::SYS_PRINT, buf.as_ptr() as u64, (n + 1) as u64, 0, 0, 0);
    }
}

/// A `fmt::Write` adapter over a fixed stack buffer. Tracks the write
/// position; returns `Err` when the buffer is full (the caller truncates).
struct BufWriter<'a> {
    buf: &'a mut [u8],
    pos: usize,
}

impl Write for BufWriter<'_> {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        let n = s.len();
        if self.pos + n > self.buf.len() - 1 {
            return Err(core::fmt::Error);
        }
        self.buf[self.pos..self.pos + n].copy_from_slice(s.as_bytes());
        self.pos += n;
        Ok(())
    }
}

#[macro_export]
macro_rules! println {
    () => {
        $crate::print::print_str("\n")
    };
    ($($t:tt)*) => {
        $crate::print::print_fmt(core::format_args!($($t)*))
    };
}
