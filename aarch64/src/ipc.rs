//! aarch64-side IPC glue: the channel table, the kernel scheduler
//! (`port::ipc`'s [`IpcScheduler`] bound to the process table and TPIDR), and
//! the message syscalls.
//!
//! [`port::ipc`] is arch-agnostic — a channel, the send/receive/reply logic,
//! and priority inheritance.  This module binds it to aarch64: a channel
//! handle is an index into a small table of channels (no allocation); the
//! scheduler's block/wake/boost run against the process table and TPIDR; and
//! the syscalls copy a message's payload across the user/kernel boundary
//! (a user buffer is a user VA, reachable in EL1 this arc because every
//! process shares the user page table).
//!
//! The owner of a channel is tracked in the [`Channel`] but the close-on-
//! owner-death hook is not wired this arc (the test images keep their owners
//! alive, and the explicit [`port::ipc::close`] is what the host tests use).
//! The real binding is target-only; the host build (unit tests of the trap
//! dispatch) sees stub handlers so the dispatch compiles.

/// A channel handle: an index into the channel table.
pub type ChannelHandle = usize;

#[cfg(target_os = "none")]
use crate::process::{self, Priority};
#[cfg(target_os = "none")]
use core::sync::atomic::{AtomicUsize, Ordering};
#[cfg(target_os = "none")]
use port::ipc::{self as ipc, Channel, IpcErr, IpcScheduler, MSG_MAX, Message, ProcId};

/// The number of channels: a fixed table (no allocation).  A channel handle
/// is an index into it.
#[cfg(target_os = "none")]
const NCHANNELS: usize = 4;

/// The channel table.  `Channel` is `!Copy` (it holds a lock), so the array
/// is spelled out rather than repeated.  A channel is not reclaimed this arc:
/// it lives for the program, so a handle is valid while in use and the lookup
/// is a plain index into a `static`.
#[cfg(target_os = "none")]
static CHANNELS: [Channel; NCHANNELS] =
    [Channel::new(0), Channel::new(0), Channel::new(0), Channel::new(0)];

/// How many channels have been created: the next handle is the old count.
#[cfg(target_os = "none")]
static NUSED: AtomicUsize = AtomicUsize::new(0);

/// Create a channel; the handle is its table index.  The channel's owner is
/// 0 this arc (the close-on-owner-death hook is not wired; see the module
/// docs).
#[cfg(target_os = "none")]
pub fn create() -> ChannelHandle {
    let h = NUSED.fetch_add(1, Ordering::AcqRel);
    assert!(h < NCHANNELS, "ipc: no free channel slot ({NCHANNELS})");
    h
}

/// The channel for `handle`, if it has been created.
#[cfg(target_os = "none")]
fn channel(handle: ChannelHandle) -> Option<&'static Channel> {
    if handle >= NCHANNELS || handle >= NUSED.load(Ordering::Acquire) {
        return None;
    }
    Some(&CHANNELS[handle])
}

/// The kernel scheduler: [`IpcScheduler`] bound to the process table and
/// TPIDR.  `block` is always of the current process (the one in the blocking
/// syscall); `wake`/`boost`/`unboost` are by id.
#[cfg(target_os = "none")]
struct KernSched;

#[cfg(target_os = "none")]
impl IpcScheduler for KernSched {
    fn current(&self) -> Option<ProcId> {
        process::current_id()
    }

    fn priority(&self, id: ProcId) -> u8 {
        process::effective_priority(id).map(|p| p.level()).unwrap_or(u8::MAX)
    }

    fn boost(&self, id: ProcId, to: u8) {
        process::boost(id, Priority::new(to));
    }

    fn unboost(&self, id: ProcId) {
        process::unboost(id);
    }

    fn block(&self, id: ProcId) {
        debug_assert_eq!(process::current_id(), Some(id), "a block is of the current process");
        process::block_current();
    }

    fn wake(&self, id: ProcId) {
        process::wake(id);
    }
}

/// Read up to `dst.len()` bytes from the user buffer at `src` into `dst`.
/// A no-op when `len` is 0 (the common case: an empty payload never touches
/// user memory).
#[cfg(target_os = "none")]
unsafe fn copy_from_user(dst: &mut [u8], src: *const u8, len: usize) {
    let n = len.min(dst.len());
    if n == 0 {
        return;
    }
    // SAFETY: `src` is a user VA, mapped read-only and reachable in EL1 this
    // arc (every process shares the user page table); `dst` is a kernel
    // buffer of at least `n` bytes; the regions are disjoint (user vs kernel
    // VA).  A faulting pointer is a user bug the test images do not produce.
    unsafe { core::ptr::copy_nonoverlapping(src, dst.as_mut_ptr(), n) };
}

/// Write up to `len` bytes from `src` to the user buffer at `dst`; returns
/// the bytes written.  A no-op (0 written) when `src` is empty or `len` is 0.
#[cfg(target_os = "none")]
unsafe fn copy_to_user(dst: *mut u8, src: &[u8], len: usize) -> usize {
    let n = len.min(src.len());
    if n == 0 {
        return 0;
    }
    // SAFETY: `dst` is a user VA, mapped read-write and reachable in EL1 this
    // arc; `src` is a kernel buffer of at least `n` bytes; the regions are
    // disjoint (user vs kernel VA).  A faulting pointer is a user bug the
    // test images do not produce.
    unsafe { core::ptr::copy_nonoverlapping(src.as_ptr(), dst, n) };
    n
}

/// A syscall result code in x0: 0 is success, the rest are the
/// [`IpcErr`]s plus a bad handle.  The user maps them back to its own errors.
#[cfg(target_os = "none")]
const OK: u64 = 0;
#[cfg(target_os = "none")]
const ERR_BAD_HANDLE: u64 = 1;
#[cfg(target_os = "none")]
const ERR_CLOSED: u64 = 2;
#[cfg(target_os = "none")]
const ERR_FULL: u64 = 3;
#[cfg(target_os = "none")]
const ERR_BAD_TAG: u64 = 4;

#[cfg(target_os = "none")]
fn err_code(e: IpcErr) -> u64 {
    match e {
        IpcErr::Closed => ERR_CLOSED,
        IpcErr::Full => ERR_FULL,
        IpcErr::BadTag => ERR_BAD_TAG,
    }
}

/// SYCSEND: `handle` on channel, `buf`/`len` the payload, `opcode`/`tag` the
/// envelope.  Returns the x0 result code.
#[cfg(target_os = "none")]
pub(crate) fn sys_send(handle: u64, buf: *const u8, len: u64, opcode: u64, tag: u64) -> u64 {
    let Some(ch) = channel(handle as ChannelHandle) else {
        return ERR_BAD_HANDLE;
    };
    let n = (len as usize).min(MSG_MAX);
    let mut data = [0u8; MSG_MAX];
    unsafe { copy_from_user(&mut data, buf, n) };
    let msg = Message::new(opcode as u16, tag as u32, &data[..n]);
    match ipc::send(&KernSched, ch, msg) {
        Ok(()) => OK,
        Err(e) => err_code(e),
    }
}

/// SYCRECEIVE: `handle` from channel, `buf`/`cap` the payload destination.
/// Returns `(x0, x3, x4)`: on success x0 = opcode, x3 = bytes copied, x4 =
/// tag; on failure x0 = the error code and x3/x4 are 0.
#[cfg(target_os = "none")]
pub(crate) fn sys_receive(handle: u64, buf: *mut u8, cap: u64) -> (u64, u64, u64) {
    let Some(ch) = channel(handle as ChannelHandle) else {
        return (ERR_BAD_HANDLE, 0, 0);
    };
    match ipc::receive(&KernSched, ch) {
        Ok(msg) => {
            let n = unsafe { copy_to_user(buf, &msg.buf, cap as usize) };
            (msg.opcode as u64, n as u64, msg.tag as u64)
        }
        Err(e) => (err_code(e), 0, 0),
    }
}

/// SYCREPLY: `handle` on channel, `buf`/`len` the payload, `opcode` the
/// protocol's reply opcode, `tag` the request's tag.  Returns the x0 result
/// code (a reply whose message tag differs from `tag` is `ERR_BAD_TAG`).
#[cfg(target_os = "none")]
pub(crate) fn sys_reply(handle: u64, buf: *const u8, len: u64, opcode: u64, tag: u64) -> u64 {
    let Some(ch) = channel(handle as ChannelHandle) else {
        return ERR_BAD_HANDLE;
    };
    let n = (len as usize).min(MSG_MAX);
    let mut data = [0u8; MSG_MAX];
    unsafe { copy_from_user(&mut data, buf, n) };
    let msg = Message::new(opcode as u16, tag as u32, &data[..n]);
    match ipc::reply(&KernSched, ch, tag as u32, msg) {
        Ok(()) => OK,
        Err(e) => err_code(e),
    }
}

// Host builds (the unit tests of the trap dispatch) see stub handlers so the
// dispatch compiles; they are never called (the trap path is target-only).
#[cfg(not(target_os = "none"))]
pub fn create() -> ChannelHandle {
    loop {
        core::hint::spin_loop();
    }
}

#[cfg(not(target_os = "none"))]
pub(crate) fn sys_send(_handle: u64, _buf: *const u8, _len: u64, _opcode: u64, _tag: u64) -> u64 {
    0
}

#[cfg(not(target_os = "none"))]
pub(crate) fn sys_receive(_handle: u64, _buf: *mut u8, _cap: u64) -> (u64, u64, u64) {
    (0, 0, 0)
}

#[cfg(not(target_os = "none"))]
pub(crate) fn sys_reply(_handle: u64, _buf: *const u8, _len: u64, _opcode: u64, _tag: u64) -> u64 {
    0
}
