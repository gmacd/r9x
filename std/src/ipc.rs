//! The message service: r9's request/reply primitive, wrapped over the raw
//! syscall core.  A channel is unidirectional, so a request/reply runs over a
//! *pair* — the client sends on the server's inbound channel and receives the
//! reply on its outbound channel.

use r9x_abi::{SYCCREATECHAN, SYCRECEIVE, SYCREPLY, SYCSEND};

use crate::sys::sys;

/// The message payload bound the kernel enforces: a server sizes its receive
/// buffer to this to accept a full message.
pub use r9x_abi::MSG_MAX;

/// Create one channel.  Returns the channel handle (the kernel's error code on
/// failure — the caller checks it against the kernel's success value).
pub fn create_chan() -> u64 {
    let (h, _, _) = unsafe { sys(SYCCREATECHAN, 0, 0, 0, 0, 0) };
    h
}

/// Create a channel pair: an inbound channel to receive on and an outbound
/// channel to send and reply on.  Two [`create_chan`]s — the connected-pair
/// convenience is not yet a syscall.
pub fn create_pair() -> (u64, u64) {
    (create_chan(), create_chan())
}

/// Send a message on `handle`: the payload `buf` with `opcode` and `tag`.
/// Returns the kernel's result (the channel's error code).
pub fn send(handle: u64, opcode: u16, tag: u32, buf: &[u8]) -> u64 {
    unsafe {
        sys(SYCSEND, handle, buf.as_ptr() as u64, buf.len() as u64, opcode as u64, tag as u64).0
    }
}

/// Receive a message on `handle` into `buf` (at most `buf.len()` bytes).
/// Blocks until one arrives.  Returns `(opcode, bytes, tag)`.
pub fn receive(handle: u64, buf: &mut [u8]) -> (u16, usize, u32) {
    let (op, bytes, tag) =
        unsafe { sys(SYCRECEIVE, handle, buf.as_mut_ptr() as u64, buf.len() as u64, 0, 0) };
    (op as u16, bytes as usize, tag as u32)
}

/// Reply on `handle` with `opcode` as the result and `payload` as the bytes,
/// correlated to `tag`.  The kernel bounds the payload to the message size.
pub fn reply(handle: u64, opcode: u16, tag: u32, payload: &[u8]) {
    let _ = unsafe {
        sys(
            SYCREPLY,
            handle,
            payload.as_ptr() as u64,
            payload.len() as u64,
            opcode as u64,
            tag as u64,
        )
    };
}
