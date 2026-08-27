//! The r9 binary-format and syscall ABI — the single source of the constants
//! that both ends of a user binary read, so the two cannot drift (the
//! user-binary-loading plan).
//!
//! The *build* links a server at [`IMAGE_BASE`]; the *loader* rejects any
//! segment placed below it; the *servers* read their spawner-passed state from
//! [`HANDLES_VA`] and send IPC payloads bounded by [`MSG_MAX`].  The trap
//! numbers below are the user-facing syscall ABI.  Each value is a stated
//! convention, not a hardware fact.
//!
//! Both the kernel's `port` (by re-export) and the `r9x_std` target link this
//! crate; a pinning test asserts the kernel's re-exports match these values.

#![no_std]

/// The base a user binary is linked at (`--image-base`): page-aligned, in the
/// TTBR0/user half.  The build links at it and the loader rejects any segment
/// placed below it, so the two agree by construction.  It sits clear of the
/// very top of the user half (where an MMIO a server maps lives) and clear of
/// the kernel's low mappings.
pub const IMAGE_BASE: usize = 0x10_0000;

/// The VA a server reads its spawner-passed state from: the page the
/// spawner writes a server's own channel pair (and, later, its parent
/// handles) into before the server's first instruction.  The spawner writes
/// `[in:4 LE][out:4 LE]` here and the server reads it.  The VA is a stated
/// convention the spawner and the server both read from here, so they cannot
/// drift; it sits in the user half, clear of the image ([`IMAGE_BASE`]) and
/// its stack by a wide margin.
pub const HANDLES_VA: usize = 0x100_0000;

/// The target's page size in bytes (4 KiB on all three architectures).  The
/// kernel's heap grant is page-granular — a `SYS_ALLOC` rounds the request up
/// to whole pages and returns a page-aligned VA — so the user-space allocator
/// shares the constant to round its own requests and to know a grant's size.
pub const PAGE_SIZE: usize = 4096;

/// The payload bound: an IPC message carries at most `MSG_MAX` payload bytes.
/// 256, QNX's classic message size: the fast path is a 256-byte move, not a
/// copy of unbounded data.
pub const MSG_MAX: usize = 256;

// The trap numbers — the user-facing syscall ABI, defined once here.  The
// arch dispatch (`aarch64::process`) re-exports them so the build, the loader,
// and the servers all read the same values and cannot drift.  The argument
// positions (arg0, arg1, …) are the *logical* interface; each arch maps them
// onto its own registers (aarch64: the number in x8, arguments in x0-x4).

/// Terminate the calling process.  arg0 = exit status.  Because the number
/// doubles as the status, status 1 is not expressible (1 is yield) and every
/// new syscall number retires one exit status; revisit when a second real
/// syscall lands.
pub const SYSEXIT: u64 = 0;

/// Voluntarily give up the remainder of this process's timeslice.  No
/// arguments.  If another process is Runnable, the handler reschedules to it
/// first.
pub const SYSYIELD: u64 = 1;

/// Send a message on a channel.  arg0 = channel handle, arg1 = user buffer
/// pointer, arg2 = buffer length, arg3 = opcode, arg4 = tag.  Result in arg0
/// (0 on success, an error otherwise).  The numbers 16-18 sit above the
/// exit-status range (0-15) the test images use.
pub const SYCSEND: u64 = 16;

/// Receive a message on a channel (blocking when none is queued).  arg0 =
/// channel handle, arg1 = user buffer pointer, arg2 = buffer capacity.  On
/// return arg0 = opcode, arg3 = the bytes copied, arg4 = tag (a closed channel
/// puts an error in arg0).
pub const SYCRECEIVE: u64 = 17;

/// Reply on a channel.  arg0 = channel handle, arg1 = user buffer pointer,
/// arg2 = buffer length, arg4 = tag.  Result in arg0.  A reply whose message
/// tag differs from the reply's tag returns an error and sends nothing.
pub const SYCREPLY: u64 = 18;

/// Claim a hardware interrupt for the current process.  arg0 = INTID, arg1 =
/// channel handle.  The kernel adds the routing-table entry and enables the
/// interrupt at the distributor.  Result in arg0 (0 on success, an error code
/// on failure).
pub const SYSIRQCLAIM: u64 = 19;

/// Map a physical page into the current process's user half with Device memory
/// attributes.  arg0 = physical address (page-aligned), arg1 = user VA.  The
/// kernel maps the page into the process's address space only (the server owns
/// the MMIO exclusively).  Result in arg0 (0 on success, 1 on failure).
pub const SYSMAPMMIO: u64 = 20;

/// Create a channel.  No arguments.  Result in arg0 — a fresh channel handle
/// on success, an error code when the channel table is full.
pub const SYCCREATECHAN: u64 = 21;

/// Grow the current process's heap by `arg0` bytes, `brk`-style: the request is
/// rounded up to whole pages and granted from a per-process top watermark.  On
/// return arg0 = the first user VA of the new range (page-aligned, the old
/// watermark), or a non-zero error code when the grant would cross the top of
/// the user half (the region a server may map its MMIO into).  The pages are
/// mapped into the process's TTBR0 only — no kernel identity map — because the
/// heap is the process's to use and the kernel neither reads nor writes it.
pub const SYS_ALLOC: u64 = 22;

/// Lower the current process's heap top to `arg0` (a page-aligned VA within
/// the heap), `brk`-style free-the-top.  The released pages stay mapped and are
/// reused by a later grow, so nothing is unmapped; a general free that
/// coalesces holes in the middle of the heap is a refinement, not this.  Always
/// returns 0.
pub const SYS_FREE: u64 = 23;

/// Spawn a process from the image registry.  arg0 = the image index, arg1 =
/// a child-state VA (a page in the spawner's address space holding the child's
/// `[n_handles, handles..., argc, argv...]`, or 0 for none), arg2 = the child's
/// priority (0 most urgent; 255, the idle sentinel, is refused).  Result in
/// arg0: the child's process id on success, or one of the `SPAWN_*` error
/// codes — a bad index or a full process table is an error the spawner recovers
/// from, not a fault.
pub const SYS_SPAWN: u64 = 24;

/// Read the arch's monotonic counter.  arg0 = the clock kind (0 = monotonic;
/// other kinds are a stated refinement, refused until a time source is
/// agreed).  On return arg0 = the tick count (the counter's value, increasing
/// at the arch's counter frequency).  A register read: no lock, no
/// allocation.  The counter frequency is a hardware constant the user reads
/// separately (EL0 opt-in to `CNTFRQ_EL0`, or a build-time constant).
pub const SYS_CLOCK: u64 = 25;

/// Receive a message on a channel, bounded by a deadline.  arg0 = channel
/// handle, arg1 = user buffer pointer, arg2 = buffer capacity, arg3 = the
/// wake deadline (a counter tick count; the process is woken when the counter
/// reaches it, or by an arriving message, whichever is first).  On return:
/// like `SYCRECEIVE` (arg0 = opcode, arg3 = bytes copied, arg4 = tag) when a
/// message arrives; on timeout arg0 = [`RECEIVE_TIMEOUT`] (arg3/arg4 = 0);
/// on a closed channel arg0 = the error code.  A timed wait that does not
/// spin: the process is blocked (off the ready set) until the deadline or a
/// message, whichever is first.
pub const SYS_RECEIVE_AT: u64 = 26;

/// Allocate a page in the current process's heap and return both the virtual
/// and physical address.  No arguments.  On return: arg0 = the VA (page-
/// aligned, the old watermark), arg1 = the physical address.  On failure
/// (the grant would cross the user-half edge): arg0 = 1, arg1 = 0.
///
/// The physical address is needed by a server that talks to a device which
/// DMA-reads or DMA-writes a buffer (the BCM283x Mailbox, which takes a
/// physical address in its write register).  The page is Normal Write-Back
/// (cached) — a device that DMA-writes to it must be coherent with the ARM's
/// cache, or the server must invalidate.
pub const SYS_ALLOC_PAGE: u64 = 27;

/// Reap a finished child: x0 = child id (0 = any), x1 = deadline in
/// counter ticks (0 = block forever).  Returns x0 = reaped child id,
/// x1 = its exit status.  On timeout x0 = [`WAIT_TIMEOUT`], x1 = 0.
/// On a bad child id x0 = [`WAIT_BAD_ID`], x1 = 0.
pub const SYS_WAIT: u64 = 28;
/// The `SYS_WAIT` timeout sentinel (returned in x0).
pub const WAIT_TIMEOUT: u64 = 0xff_ff_ff_ff;
/// The `SYS_WAIT` bad-child-id sentinel (returned in x0).
pub const WAIT_BAD_ID: u64 = 0xff_ff_ff_fe;

/// Terminate a process: x0 = target id.  The target is marked for
/// termination; it dies on the next switch (or immediately if not
/// Running).  Returns x0 = 0 on success, [`KILL_BAD_ID`] if the id is
/// not a live or zombie process.
pub const SYS_KILL: u64 = 29;
/// The `SYS_KILL` bad-id error (returned in x0).
pub const KILL_BAD_ID: u64 = 1;

/// Set a process's priority: x0 = target id ([`u64::MAX`] = self), x1 =
/// priority (0 = most urgent, 255 = idle sentinel, refused).  Returns x0 = 0
/// on success, [`SETPRIO_BAD_ID`] if the id is not a live process,
/// [`SETPRIO_BAD_PRIO`] if the priority is the idle sentinel.
pub const SYS_SETPRIO: u64 = 30;
/// The `SYS_SETPRIO` bad-id error.
pub const SETPRIO_BAD_ID: u64 = 1;
/// The `SYS_SETPRIO` bad-priority error (the idle sentinel).
pub const SETPRIO_BAD_PRIO: u64 = 2;

/// Print a string to the kernel debug console (PL011): x0 = user VA of the
/// string buffer, x1 = length in bytes (capped at 256).  Returns x0 = 0.
///
/// A debug/boot facility: the production I/O path is IPC to the console
/// server (`/dev/cons`).  Not thread-safe: bytes within a single call are
/// contiguous, but calls from different processes may interleave (the same
/// guarantee as the kernel's `iprintln!`).
pub const SYS_PRINT: u64 = 31;

// `SYS_SPAWN` result codes.  A value below `SPAWN_ERR_MIN` is a process id
// (a table index, 0..NPROCS, and NPROCS is far below the bound); at or above
// it is one of these errors.  The spawner maps them back to its own errors.
/// The minimum value that is an error rather than a process id.
pub const SPAWN_ERR_MIN: u64 = 128;
/// The image index is not in the registry (out of range, or the registry is
/// empty — it is populated at boot, before any spawn can reference an index).
pub const SPAWN_BAD_INDEX: u64 = 128;
/// The process table is full (no free slot).
pub const SPAWN_NO_SLOT: u64 = 129;
/// The child-state or priority is malformed (too many handles, or the
/// priority is the idle sentinel or above).
pub const SPAWN_BAD_STATE: u64 = 130;

/// The message opcode a `SYS_RECEIVE_AT` returns when its deadline passes
/// before a message arrives: the receive timed out.  A value reserved for the
/// kernel (the maximum `u16`); a protocol that sends a message with this
/// opcode is ambiguous with a timeout and must not.
pub const RECEIVE_TIMEOUT: u16 = 0xffff;
/// The error code a `SYCRECEIVE` / `SYS_RECEIVE_AT` returns in arg0 when the
/// channel's owner has died (the kernel closed it on process death): the
/// peer is gone, not a protocol error.  Matches the kernel's `ERR_CLOSED`.
pub const RECEIVE_CLOSED: u16 = 2;

/// The layout of the generalized `HANDLES_VA` page the spawner writes a
/// child's state into (and the child reads from its first instruction):
///
///     [n_handles:u32 LE][handle:u32 LE ...][argc:u32 LE][argv ...]
///
/// `n_handles` counts the `handle` words that follow; `argc` (immediately
/// after the last handle) is the byte length of the trailing `argv`.  The
/// common case — a server handed a channel pair — is `n_handles = 2`,
/// `argc = 0`, so the old `[in:4][out:4]` write survives as the two handles
/// under a count.  A child with no state is a zero page (`n_handles = 0`).
///
/// The bound on the handle count: the rest of the page is argv.  A spawn whose
/// `n_handles` exceeds it is refused (`SPAWN_BAD_STATE`) rather than read past
/// the page.
pub const SPAWN_MAX_HANDLES: usize = 512;
