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

/// The number of IRQ routes: a fixed table (no allocation).  The Amiga's
/// interrupt-to-message-port path generalised: the kernel's interrupt context
/// budget is lookup, enqueue, wake — three things, no allocation, no lock
/// held across a switch.
#[cfg(target_os = "none")]
const NIRQS: usize = 16;

/// A GICv2 SPI range: INTIDs 32..=1019 (Arm GICv2 Architecture Specification,
/// section 3.4.1).  SGIs (0..15) and PPIs (16..31) are banked per core and
/// are not claimable by a user-space process.
#[cfg(target_os = "none")]
const INTID_SPI_MIN: u16 = 32;
#[cfg(target_os = "none")]
const INTID_SPI_MAX: u16 = 1019;

/// One IRQ route: an INTID mapped to a channel handle and an owning process.
#[cfg(target_os = "none")]
struct IrqRoute {
    intid: u16,
    channel: ChannelHandle,
    /// The owning process: set by `sys_irq_claim`; read by the close-on-
    /// owner-death hook (not wired this arc).
    #[allow(dead_code)]
    owner: ProcId,
}

/// A sync-safe wrapper around `UnsafeCell<Option<IrqRoute>>`: the
/// write-then-publish pattern (write the route, then `NIRQUEUED.fetch_add`
/// with `Release`) paired with the read's `Acquire` load makes the access
/// safe: the read sees a fully-written route, never a torn one.  The write is
/// a single `sys_irq_claim` (a single writer); the read is the trap handler's
/// IRQ dispatch (a reader per core).
#[cfg(target_os = "none")]
struct IrqRouteCell(core::cell::UnsafeCell<Option<IrqRoute>>);

// SAFETY: the write-then-publish pattern makes the access safe (see docs).
#[cfg(target_os = "none")]
unsafe impl Sync for IrqRouteCell {}

/// The IRQ routing table.  Set by `SYSIRQCLAIM`; read by the trap handler's
/// IRQ dispatch.  A linear scan over `NIRQS` entries (16 comparisons per IRQ
/// — acceptable: the IRQ handler is not the display server's hot path).
#[cfg(target_os = "none")]
static IRQ_ROUTES: [IrqRouteCell; NIRQS] = [
    IrqRouteCell(core::cell::UnsafeCell::new(None)),
    IrqRouteCell(core::cell::UnsafeCell::new(None)),
    IrqRouteCell(core::cell::UnsafeCell::new(None)),
    IrqRouteCell(core::cell::UnsafeCell::new(None)),
    IrqRouteCell(core::cell::UnsafeCell::new(None)),
    IrqRouteCell(core::cell::UnsafeCell::new(None)),
    IrqRouteCell(core::cell::UnsafeCell::new(None)),
    IrqRouteCell(core::cell::UnsafeCell::new(None)),
    IrqRouteCell(core::cell::UnsafeCell::new(None)),
    IrqRouteCell(core::cell::UnsafeCell::new(None)),
    IrqRouteCell(core::cell::UnsafeCell::new(None)),
    IrqRouteCell(core::cell::UnsafeCell::new(None)),
    IrqRouteCell(core::cell::UnsafeCell::new(None)),
    IrqRouteCell(core::cell::UnsafeCell::new(None)),
    IrqRouteCell(core::cell::UnsafeCell::new(None)),
    IrqRouteCell(core::cell::UnsafeCell::new(None)),
];

/// How many IRQ routes have been claimed: the next route is the old count.
#[cfg(target_os = "none")]
static NIRQUEUED: AtomicUsize = AtomicUsize::new(0);

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
pub(crate) struct KernSched;

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
const ERR_BAD_INTID: u64 = 5;
#[cfg(target_os = "none")]
const ERR_ALREADY_CLAIMED: u64 = 6;
#[cfg(target_os = "none")]
const ERR_NO_SLOT: u64 = 7;

#[cfg(target_os = "none")]
fn err_code(e: IpcErr) -> u64 {
    match e {
        IpcErr::Closed => ERR_CLOSED,
        IpcErr::Full => ERR_FULL,
        IpcErr::BadTag => ERR_BAD_TAG,
    }
}

/// The channel for `intid`, if it has been claimed via `SYSIRQCLAIM`.
/// A linear scan over the routing table (16 comparisons per IRQ).
#[cfg(target_os = "none")]
pub fn route(intid: u16) -> Option<&'static Channel> {
    let n = NIRQUEUED.load(Ordering::Acquire);
    for cell in IRQ_ROUTES.iter().take(n) {
        // SAFETY: the routing table is written only by `sys_irq_claim`
        // (a single writer, the syscall path) and read here (the trap
        // handler).  The write-then-publish pattern (write the route, then
        // `NIRQUEUED.fetch_add`) with `Acquire`/`Release` ordering
        // makes the read see a fully-written route.
        let r = unsafe { &*cell.0.get() };
        if let Some(route) = r
            && route.intid == intid
        {
            return channel(route.channel);
        }
    }
    None
}

/// SYSIRQCLAIM: x0 = INTID, x1 = channel handle.  The kernel checks the
/// INTID is in the SPI range, the channel handle is valid, and the INTID is
/// not already claimed.  It adds the routing table entry and enables the
/// interrupt at the GIC distributor.  Returns the x0 result code.
#[cfg(target_os = "none")]
pub(crate) fn sys_irq_claim(intid: u64, handle: u64) -> u64 {
    // The INTID must be in the SPI range (32..=1019 on GICv2).
    let intid = match intid.try_into() {
        Ok(i) if (INTID_SPI_MIN..=INTID_SPI_MAX).contains(&i) => i,
        _ => return ERR_BAD_INTID,
    };
    // The channel handle must be valid (created via `ipc::create()`).
    let handle = handle as ChannelHandle;
    if channel(handle).is_none() {
        return ERR_BAD_HANDLE;
    }
    // The INTID must not already be claimed.
    let n = NIRQUEUED.load(Ordering::Acquire);
    for cell in IRQ_ROUTES.iter().take(n) {
        // SAFETY: same as `route` above.
        let r = unsafe { &*cell.0.get() };
        if let Some(route) = r
            && route.intid == intid
        {
            return ERR_ALREADY_CLAIMED;
        }
    }
    // Add the routing table entry.
    let slot = NIRQUEUED.fetch_add(1, Ordering::AcqRel);
    if slot >= NIRQS {
        NIRQUEUED.fetch_sub(1, Ordering::AcqRel);
        return ERR_NO_SLOT;
    }
    let owner = process::current_id().unwrap_or(0);
    // SAFETY: `slot` was just allocated (the `fetch_add` returned it), so no
    // other core is writing to it.  The write is published by the `AcqRel`
    // ordering on the `fetch_add` above.
    unsafe { *IRQ_ROUTES[slot].0.get() = Some(IrqRoute { intid, channel: handle, owner }) };
    // Enable the interrupt at the GIC distributor.
    crate::gic::enable_interrupt(intid);
    OK
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

#[cfg(not(target_os = "none"))]
pub fn route(_intid: u16) -> Option<&'static ()> {
    None
}

#[cfg(not(target_os = "none"))]
pub(crate) fn sys_irq_claim(_intid: u64, _handle: u64) -> u64 {
    0
}
