//! The compiler's memory builtins for the r9 target: `memcpy`, `memmove`,
//! `memset`, and `memcmp`, referenced by `core`'s slice and pointer
//! operations.
//!
//! A target whose `os` is not `"none"` (this one's is `"r9"`) is not expected
//! to get these from `compiler_builtins` — the OS is.  So `r9x_std` provides
//! them, the way a platform's C runtime would (Redox's `relibc` plays the same
//! role for its targets).  The bodies are plain byte loops: correct first, and
//! fast enough for the message-sized copies the servers make.
//!
//! These are the r9 target's runtime, provided exactly as a platform's C runtime
//! would provide them, so the lint that flags redefining them is allowed here.
#![allow(suspicious_runtime_symbol_definitions)]

/// Copy `n` bytes from `src` to `dst`, returning `dst`.
///
/// # Safety
///
/// `src` and `dst` must each be valid for `n` bytes, and the regions must not
/// overlap (use [`memmove`] for overlapping copies).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn memcpy(dst: *mut u8, src: *const u8, n: usize) -> *mut u8 {
    // SAFETY: the caller (core's slice and pointer code) upholds the contract
    // above.
    unsafe {
        let mut d = dst;
        let mut s = src;
        for _ in 0..n {
            *d = *s;
            d = d.add(1);
            s = s.add(1);
        }
    }
    dst
}

/// Move the `n` bytes from `src` to `dst`, which **may overlap** (unlike
/// [`memcpy`]), returning `dst`.
///
/// # Safety
///
/// `src` must be valid for `n` bytes to read and `dst` for `n` bytes to write;
/// the regions may overlap.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn memmove(dst: *mut u8, src: *const u8, n: usize) -> *mut u8 {
    // SAFETY: the caller upholds the contract above.
    unsafe {
        // Copy away from the overlap: when `dst` is below `src` copy from the
        // head (each source byte is read before a later write can reach it),
        // otherwise from the tail.
        if (dst as usize) < (src as usize) {
            for i in 0..n {
                *dst.add(i) = *src.add(i);
            }
        } else {
            for i in (0..n).rev() {
                *dst.add(i) = *src.add(i);
            }
        }
    }
    dst
}

/// Set the `n` bytes at `dst` to `c`, returning `dst`.
///
/// # Safety
///
/// `dst` must be valid for `n` bytes to write.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn memset(dst: *mut u8, c: u8, n: usize) -> *mut u8 {
    // SAFETY: the caller upholds the contract above.
    unsafe {
        let mut d = dst;
        for _ in 0..n {
            *d = c;
            d = d.add(1);
        }
    }
    dst
}

/// Compare the first `n` bytes at `a` and `b`: negative if `a < b`, zero if
/// equal, positive if `a > b` (byte-wise, most significant first).
///
/// # Safety
///
/// `a` and `b` must each be valid for `n` bytes to read.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn memcmp(a: *const u8, b: *const u8, n: usize) -> i32 {
    // SAFETY: the caller upholds the contract above.
    unsafe {
        let mut x = a;
        let mut y = b;
        for _ in 0..n {
            let (u, v) = (*x, *y);
            if u != v {
                return (u as i32) - (v as i32);
            }
            x = x.add(1);
            y = y.add(1);
        }
    }
    0
}
