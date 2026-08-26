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
use port::ipc::{self as ipc, IpcErr, IpcScheduler, MSG_MAX, Message, ProcId};
#[cfg(target_os = "none")]
use r9x_abi::RECEIVE_TIMEOUT;
// The channel table itself is host-testable (a plain `Channel` array and an
// atomic count; `port::ipc::Channel` carries no target gate), so the host
// build can unit-test `try_create`/`channel` — the observable of SYCCREATECHAN.
// The atomic import is widened to match (the target-only IRQ/table code and
// the host test both need `AtomicUsize`/`Ordering`).
#[cfg(any(target_os = "none", test))]
use core::sync::atomic::{AtomicUsize, Ordering};
#[cfg(any(target_os = "none", test))]
use port::ipc::Channel;

/// The number of channels: a fixed table (no allocation).  A channel handle
/// is an index into it.
#[cfg(any(target_os = "none", test))]
const NCHANNELS: usize = 6;

/// The channel table.  `Channel` is `!Copy` (it holds a lock), so the array
/// is spelled out rather than repeated.  A channel is not reclaimed this arc:
/// it lives for the program, so a handle is valid while in use and the lookup
/// is a plain index into a `static`.
#[cfg(any(target_os = "none", test))]
static CHANNELS: [Channel; NCHANNELS] = [const { Channel::new(0) }; NCHANNELS];

/// How many channels have been created: the next handle is the old count.
#[cfg(any(target_os = "none", test))]
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

/// Reserve a channel's table slot; the handle is its index, `None` when the
/// table is full.  The counter only grows (a channel is not reclaimed this
/// arc), so a `fetch_add` that lands at or past `NCHANNELS` is a full table:
/// roll the count back and report it.  A full table from a live process is an
/// error the caller maps, not a panic — the kernel-side [`create`] panics
/// (its callers are init-context), the `SYCCREATECHAN` dispatch returns it.
#[cfg(any(target_os = "none", test))]
pub(crate) fn try_create() -> Option<ChannelHandle> {
    let h = NUSED.fetch_add(1, Ordering::AcqRel);
    if h < NCHANNELS {
        Some(h)
    } else {
        // Over-allocated: undo the reservation.  A concurrent over-allocation
        // rolls back in turn, so the count settles back to the true high-water
        // mark (a channel is never reclaimed, so it only ever grows).
        NUSED.fetch_sub(1, Ordering::AcqRel);
        None
    }
}

/// Create a channel; the handle is its table index.  The channel's owner is
/// 0 this arc (the close-on-owner-death hook is not wired; see the module
/// docs).
///
/// # Panics
///
/// When the table is full: the callers are init-context (the test images and
/// `main9`), where a panic is the failure report.  A live process uses
/// `SYCCREATECHAN`, whose dispatch goes through the non-panicking
/// [`try_create`].
#[cfg(any(target_os = "none", test))]
pub fn create() -> ChannelHandle {
    try_create().unwrap_or_else(|| panic!("ipc: no free channel slot ({NCHANNELS})"))
}

/// Host stub: the binary's test target compiles the library as a dependency
/// (not in test mode, on a host-like target) where the channel table does not
/// exist.  Never called: `main9` is `no_main` and never runs on the host.
#[cfg(not(any(target_os = "none", test)))]
pub fn create() -> ChannelHandle {
    loop {
        core::hint::spin_loop();
    }
}

/// The channel for `handle`, if it has been created.
#[cfg(any(target_os = "none", test))]
pub fn channel(handle: ChannelHandle) -> Option<&'static Channel> {
    if handle >= NCHANNELS || handle >= NUSED.load(Ordering::Acquire) {
        return None;
    }
    Some(&CHANNELS[handle])
}

/// The kernel scheduler: [`IpcScheduler`] bound to the process table and
/// TPIDR.  `block` is always of the current process (the one in the blocking
/// syscall); `wake`/`boost`/`unboost` are by id.
#[cfg(target_os = "none")]
pub struct KernSched;

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

    fn now(&self) -> u64 {
        crate::timer::counter()
    }

    fn block_at(&self, id: ProcId, deadline: u64) {
        debug_assert_eq!(process::current_id(), Some(id), "a block is of the current process");
        process::block_at(deadline);
    }
}

/// Read up to `dst.len()` bytes from the user buffer at `src` into `dst`.
/// A no-op when `len` is 0 (the common case: an empty payload never touches
/// user memory).  `pub(crate)`: the message syscalls and `SYS_SPAWN` (reading
/// the spawner's child-state page) both run on this arc — during a `svc` the
/// calling process's `TTBR0` is still installed, so its user VAs are
/// reachable in EL1.
#[cfg(target_os = "none")]
pub(crate) unsafe fn copy_from_user(dst: &mut [u8], src: *const u8, len: usize) {
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
const ERR_EMPTY: u64 = 4;
#[cfg(target_os = "none")]
const ERR_BAD_TAG: u64 = 5;
#[cfg(target_os = "none")]
const ERR_BAD_INTID: u64 = 6;
#[cfg(target_os = "none")]
const ERR_ALREADY_CLAIMED: u64 = 7;
#[cfg(target_os = "none")]
const ERR_NO_SLOT: u64 = 8;
#[cfg(target_os = "none")]
const ERR_BAD_KIND: u64 = 9;

#[cfg(target_os = "none")]
fn err_code(e: IpcErr) -> u64 {
    match e {
        IpcErr::Closed => ERR_CLOSED,
        IpcErr::Full => ERR_FULL,
        IpcErr::Empty => ERR_EMPTY,
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
pub fn sys_irq_claim(intid: u64, handle: u64) -> u64 {
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

/// SYSMAPMMIO: x0 = physical address (page-aligned), x1 = user VA.  Maps the
/// physical page into the current process's TTBR0 with Device memory
/// attributes.  Returns 0 on success, 1 on failure (bad address or mapping
/// error).  The kernel is device-dumb: it provides the capability, the
/// server decides which MMIO to map (the QNX model).
#[cfg(target_os = "none")]
pub(crate) fn sys_map_mmio(pa: u64, va: u64) -> u64 {
    const PAGE: usize = port::mem::PAGE_SIZE_4K;
    if !(pa as usize).is_multiple_of(PAGE) {
        return 1;
    }
    let Some(aspace) = process::current_aspace() else {
        return 1;
    };
    let range = port::mem::PhysRange::with_pa_len(port::mem::PhysAddr::new(pa), PAGE);
    match aspace.map_mmio(&range, va as usize) {
        Ok(()) => {
            // The mapping is live in the page table but the process's TLB
            // may hold a stale entry (or a translation fault for this VA
            // from before the mapping).  Invalidate the user TLB so the
            // process's first access to the new mapping takes the fresh
            // walk.
            unsafe {
                core::arch::asm!("tlbi vmalle1is", "dsb ish", "isb", options(nomem, nostack),);
            }
            0
        }
        Err(_) => 1,
    }
}

/// SYSALLOC: x0 = byte count.  Grows the current process's heap (page-
/// granular, `brk`-style) into its TTBR0 and returns x0 = the first user VA of
/// the grant (page-aligned, the old watermark).  On failure — the grant would
/// cross the user-half edge, or a page cannot be mapped — returns 1: a small
/// error code that can never be a heap VA (those are page-aligned, at or above
/// the first heap page).  The kernel is device-dumb: it provides the pages, the
/// process decides how to use them (the QNX model).
#[cfg(target_os = "none")]
pub(crate) fn sys_alloc(count: u64) -> u64 {
    match process::heap_grow(count) {
        Some(va) => va as u64,
        None => 1,
    }
}

/// SYSFREE: x0 = a heap VA.  Lowers the current process's heap top to it
/// (`brk`-style free-the-top; the released pages stay mapped for a later grow).
/// A `va` outside the heap or not page-aligned is a no-op.  Always returns 0.
#[cfg(target_os = "none")]
pub(crate) fn sys_free(va: u64) -> u64 {
    process::heap_shrink(va);
    0
}

/// SYCCREATECHAN: no arguments.  Returns the x0 result — a fresh channel
/// handle on success, `ERR_NO_SLOT` when the table is full (a live process
/// must not panic the table; the kernel-side [`create`] panics, this does not).
#[cfg(target_os = "none")]
pub(crate) fn sys_createchan() -> u64 {
    match try_create() {
        Some(h) => h as u64,
        None => ERR_NO_SLOT,
    }
}

/// SYSCLOCK: x0 = the clock kind (0 = monotonic; other kinds are a stated
/// refinement, refused).  On return x0 = the tick count (the arch counter's
/// value, increasing at the counter frequency).  A register read: no lock, no
/// allocation, so the hot path stays within the three-thing budget.  The
/// counter frequency is a hardware constant the user reads separately (EL0
/// opt-in to `CNTFRQ_EL0`), not a return of this syscall this arc.
#[cfg(target_os = "none")]
pub fn sys_clock(kind: u64) -> u64 {
    if kind != 0 {
        return ERR_BAD_KIND;
    }
    crate::timer::counter()
}

/// SYSRECEIVEAT: `handle` from channel, `buf`/`cap` the payload destination,
/// `deadline` the wake deadline (a counter tick).  Returns `(x0, x3, x4)`: on
/// a message, like `sys_receive` (x0 = opcode, x3 = bytes copied, x4 = tag);
/// on a timeout, x0 = `RECEIVE_TIMEOUT` (x3/x4 = 0); on a closed channel, x0
/// = the error code.  The wait is bounded: the process is blocked (off the
/// ready set) until the deadline or a message, whichever is first — no spin.
#[cfg(target_os = "none")]
pub(crate) fn sys_receive_at(
    handle: u64,
    buf: *mut u8,
    cap: u64,
    deadline: u64,
) -> (u64, u64, u64) {
    let Some(ch) = channel(handle as ChannelHandle) else {
        return (ERR_BAD_HANDLE, 0, 0);
    };
    match ipc::receive_at(&KernSched, ch, deadline) {
        Ok(msg) => {
            if msg.opcode == RECEIVE_TIMEOUT {
                // The timeout: the deadline beat the message.  No payload.
                return (RECEIVE_TIMEOUT as u64, 0, 0);
            }
            let n = unsafe { copy_to_user(buf, &msg.buf, (msg.len as usize).min(cap as usize)) };
            (msg.opcode as u64, n as u64, msg.tag as u64)
        }
        Err(e) => (err_code(e), 0, 0),
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
            // Copy only the message's actual payload (`msg.len` bytes), not
            // the full `MSG_MAX` buffer: the receiver's `bytes` return is the
            // payload length, and a protocol that reads `buf[..bytes]` would
            // see trailing zeros if the full buffer were copied.
            let n = unsafe { copy_to_user(buf, &msg.buf, (msg.len as usize).min(cap as usize)) };
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
// `create`/`channel` have no host stub: the channel table is host-testable,
// so the host build runs the real ones (see the `use` gate above).
#[cfg(not(target_os = "none"))]
pub(crate) fn sys_send(_handle: u64, _buf: *const u8, _len: u64, _opcode: u64, _tag: u64) -> u64 {
    0
}

#[cfg(not(target_os = "none"))]
pub(crate) fn sys_receive(_handle: u64, _buf: *mut u8, _cap: u64) -> (u64, u64, u64) {
    (0, 0, 0)
}

#[cfg(not(target_os = "none"))]
pub(crate) fn sys_clock(_kind: u64) -> u64 {
    0
}

#[cfg(not(target_os = "none"))]
pub(crate) fn sys_receive_at(
    _handle: u64,
    _buf: *mut u8,
    _cap: u64,
    _deadline: u64,
) -> (u64, u64, u64) {
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

#[cfg(not(target_os = "none"))]
pub(crate) fn sys_map_mmio(_pa: u64, _va: u64) -> u64 {
    0
}

#[cfg(not(target_os = "none"))]
pub(crate) fn sys_alloc(_count: u64) -> u64 {
    0
}

#[cfg(not(target_os = "none"))]
pub(crate) fn sys_free(_va: u64) -> u64 {
    0
}

#[cfg(not(target_os = "none"))]
pub(crate) fn sys_createchan() -> u64 {
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    // The channel table is host-testable (the `use` gate above), so this is
    // the observable of SYCCREATECHAN: a fresh create resolves through
    // `channel`, and a create past the table's size is an error, not a panic.
    // It is the only test that touches `NUSED`, so it resets the count first
    // (the target build shares no state with it).
    #[test]
    fn createchan_fills_the_table_then_errors_not_panics() {
        NUSED.store(0, Ordering::Relaxed);
        // Four fresh creates fill the table; each resolves through `channel`.
        for want in 0..NCHANNELS {
            assert_eq!(try_create(), Some(want), "create #{want}");
            assert!(channel(want).is_some(), "handle {want} must resolve");
        }
        // The next create is a full table: `None`, not a panic.
        assert_eq!(try_create(), None, "a full table is an error, not a panic");
        // An out-of-range handle does not resolve either.
        assert!(channel(NCHANNELS).is_none());
        // The kernel-side `create()` panics on a full table (its callers are
        // init-context); `try_create` is the non-panicking form the
        // SYCCREATECHAN dispatch uses, so it is what is tested here.
    }
}
