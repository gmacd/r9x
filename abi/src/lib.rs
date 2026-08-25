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
