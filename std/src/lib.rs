//! r9's user-space facade: the thin `core`-based library every r9 executable
//! except the kernel links, instead of the platform `std`.
//!
//! This is not a fork of `std`.  The r9 kernel is a QNX-shaped message-passing
//! broker — an 8-syscall core with no filesystem, no networking, no threads —
//! so a whole platform library cannot be backed by it.  `r9x_std` is a curated
//! layer instead: process control and the message primitives are thin wrappers
//! over the syscall core (the [`sys`] module); everything else — the heap,
//! the file services, naming — is reached by message-passing to a user-space
//! server (the [`ipc`] and [`mem`] modules; file and network services grow
//! here as the corresponding servers do).
//!
//! Like `std`, it is selected automatically: it is the only `std` for the r9
//! targets (their `os` is `"r9"`), so a server written against `r9x_std` is
//! built with `-Z build-std=core,alloc` and the crate is swapped in for `std`
//! by target, with no source change.  See the design in
//! `_tasks/plans/r9x-target-std-backend.md`.

#![no_std]

extern crate alloc;

/// The compiler's memory builtins (`memcpy` / `memset` / `memcmp`), which a
/// target whose `os` is not `"none"` must provide itself.
pub mod builtin;
/// The thin syscall core and the message wrappers over it: the only part of
/// r9 that needs a per-architecture shim.
pub mod ipc;
pub mod mem;
pub mod process;
/// The runtime glue a r9 executable needs that the platform `std` would
/// otherwise provide: the entry point, the panic handler, and the runtime
/// facts the loader passes in (the DTB VA, the spawner-passed handles).
///
/// `r9x_std` is only ever built for the bare-metal r9 targets (and the kernel
/// target's check step), all of which this runtime is valid for, so it is not
/// gated on `target_os`.
pub mod rt;
/// The thin syscall core: the only per-architecture part of `r9x_std`.
pub mod sys;
