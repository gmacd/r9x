//! `r9x_std::console`: writing to the console server — the user-space
//! driver that owns the terminal.
//!
//! The client discovers the server by name: a one-shot `RESOLVE` of
//! `/dev/console` through the nameserver (the same convention the mailbox
//! server uses), cached for the process's life — the resolved pair does not
//! change.  A write uses the console server's `OP_WRITE` protocol: the
//! payload is `[reply_chan:4 LE][data...]`, and the server replies `R_OK`
//! on the client's own reply channel — not on the server's outbound
//! channel, which every client shares and on which two concurrent clients
//! would steal each other's replies.
//!
//! A message payload is bounded by `MSG_MAX`, so a write longer than
//! `MSG_MAX - 4` (the reply-channel field) is chunked into several
//! messages.  A message is the atomicity unit — two clients' output may
//! interleave at message boundaries, never within one — so a caller that
//! cares keeps one line within one message.
//!
//! The caller must have been spawned with the nameserver's handles (the
//! `Handles::for_server` form, or the legacy main-pair form) — the same
//! requirement as every other server that resolves by name.

use core::cell::Cell;
use core::fmt;
use core::fmt::Write;

use r9x_abi::RECEIVE_CLOSED;

use crate::ipc;
use crate::rt;

/// The console server's write verb.
const OP_WRITE: u16 = 0;
/// The nameserver's resolve verb: the same number under a different verb,
/// as each server numbers its own verbs from zero.
const OP_RESOLVE: u16 = 1;
/// The result code both servers reply with on success.
const R_OK: u16 = 0;

/// The name the console server publishes under.
const CONSOLE_NAME: &[u8] = b"/dev/console";

/// A console write failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConsoleError {
    /// The message path failed: a `send` error, a reply that is not `R_OK`,
    /// or use of the console before a successful write.
    Ipc,
    /// The console server is gone: the kernel closed its channels on its
    /// death, so the reply a write is waiting on never comes — a
    /// `RECEIVE_CLOSED` where a hang would be.
    Closed,
}

/// The resolved console server, and this process's reply channel.
#[derive(Clone, Copy)]
struct Console {
    /// The console server's inbound channel: where `OP_WRITE`s go.
    in_h: u64,
    /// This process's reply channel: where the server's `R_OK`s come back.
    reply_h: u64,
}

/// The one-shot resolve, cached: the pair does not change for the server's
/// life, and a process is single-threaded (a "thread" is a process), so the
/// `Cell` needs no lock.
struct ConsoleCache {
    value: Cell<Option<Console>>,
}

// SAFETY: the cache holds a pair of plain channel indices (no pointers);
// the process is single-threaded (a "thread" is a process), so the `Cell`'s
// interior mutability is never raced.
unsafe impl Sync for ConsoleCache {}

static CONSOLE: ConsoleCache = ConsoleCache { value: Cell::new(None) };

/// Write bytes to the console.  On first use this resolves `/dev/console`
/// through the nameserver and caches the result; the write is then sent in
/// chunks of at most `MSG_MAX - 4` bytes (the reply-channel field), each
/// awaited for the server's `R_OK`.
pub fn write(data: &[u8]) -> Result<(), ConsoleError> {
    let c = resolve()?;
    const CHUNK: usize = ipc::MSG_MAX - 4; // the reply-channel field
    let mut payload = [0u8; ipc::MSG_MAX];
    payload[0..4].copy_from_slice(&(c.reply_h as u32).to_le_bytes());
    let mut rest = data;
    while !rest.is_empty() {
        let n = rest.len().min(CHUNK);
        payload[4..4 + n].copy_from_slice(&rest[..n]);
        if ipc::send(c.in_h, OP_WRITE, 0, &payload[..4 + n]) != 0 {
            return Err(ConsoleError::Ipc);
        }
        // The reply carries no payload (the server writes `&[]`), so the
        // buffer only needs to hold one.
        let mut reply = [0u8; 4];
        let (op, _, _) = ipc::receive(c.reply_h, &mut reply);
        if op == RECEIVE_CLOSED {
            return Err(ConsoleError::Closed);
        }
        if op != R_OK {
            return Err(ConsoleError::Ipc);
        }
        rest = &rest[n..];
    }
    Ok(())
}

/// A convenience for [`write`] that formats with `core::format_args`: into
/// a 256-byte stack buffer (truncating with `...` when it does not fit, as
/// `r9x_std::print` does), plus the newline, written as one or more
/// messages.
pub fn println(args: fmt::Arguments<'_>) -> Result<(), ConsoleError> {
    let mut buf = [0u8; 257]; // 256 of content + the newline
    let (pos, truncated) = {
        let mut w = BufWriter { buf: &mut buf, pos: 0 };
        let truncated = w.write_fmt(args).is_err();
        (w.pos, truncated)
    };
    if truncated {
        buf[pos - 3..pos].copy_from_slice(b"...");
    }
    buf[pos] = b'\n';
    write(&buf[..pos + 1])
}

/// This process's reply channel for the console server: the channel a
/// finished client can block on — after a completed write, no one sends
/// there again.  Requires a successful [`write`] or [`println`] first, so
/// that the resolve has happened.
pub fn reply_channel() -> Result<u64, ConsoleError> {
    CONSOLE.value.get().map(|c| c.reply_h).ok_or(ConsoleError::Ipc)
}

/// Resolve `/dev/console` through the nameserver, once: create this
/// process's reply channel, send the `RESOLVE`, and cache the result.
fn resolve() -> Result<Console, ConsoleError> {
    if let Some(c) = CONSOLE.value.get() {
        return Ok(c);
    }
    // The nameserver's inbound channel: the spawner handed this process the
    // nameserver's handles, as every other server gets them.
    let ns_in = rt::handle_at(2) as u64;
    // This process's reply channel: the nameserver sends the resolve result
    // here, and the console server sends its `R_OK`s here — never on the
    // server's outbound, which every client shares.
    let reply_h = ipc::create_chan();
    let mut req = [0u8; CONSOLE_NAME.len() + 4];
    req[..CONSOLE_NAME.len()].copy_from_slice(CONSOLE_NAME);
    req[CONSOLE_NAME.len()..].copy_from_slice(&(reply_h as u32).to_le_bytes());
    if ipc::send(ns_in, OP_RESOLVE, 0, &req) != 0 {
        return Err(ConsoleError::Ipc);
    }
    let mut reply = [0u8; 8];
    let (op, bytes, _) = ipc::receive(reply_h, &mut reply);
    if op == RECEIVE_CLOSED {
        return Err(ConsoleError::Closed);
    }
    // `R_OK` carries the server's `(in, out)` pair; only the inbound half
    // is kept — replies come on this process's own channel.
    if op != R_OK || bytes != 8 {
        return Err(ConsoleError::Ipc);
    }
    let in_h = u32::from_le_bytes(reply[0..4].try_into().unwrap()) as u64;
    let c = Console { in_h, reply_h };
    CONSOLE.value.set(Some(c));
    Ok(c)
}

/// A `fmt::Write` adapter over a fixed stack buffer.  Tracks the write
/// position; returns `Err` when the buffer is full (the caller truncates) —
/// the same shape as `r9x_std::print`'s.
struct BufWriter<'a> {
    buf: &'a mut [u8],
    pos: usize,
}

impl fmt::Write for BufWriter<'_> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let n = s.len();
        if self.pos + n > self.buf.len() - 1 {
            return Err(fmt::Error);
        }
        self.buf[self.pos..self.pos + n].copy_from_slice(s.as_bytes());
        self.pos += n;
        Ok(())
    }
}
