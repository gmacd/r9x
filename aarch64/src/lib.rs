//! The aarch64 kernel as a library.
//!
//! Everything except the entry point lives here, so that the kernel binary
//! and each QEMU integration test can link the same code and supply their
//! own `main9`: a test runs exactly the initialisation it needs and no more.
#![allow(clippy::too_many_arguments)]
#![allow(clippy::upper_case_acronyms)]
#![allow(internal_features)]
#![cfg_attr(target_os = "none", no_std)]
#![cfg_attr(target_os = "none", feature(alloc_error_handler))]
#![feature(core_intrinsics)]
#![forbid(unsafe_op_in_unsafe_fn)]

pub mod allocator;
pub mod aspace;
pub mod boot;
pub mod devcons;
pub mod deviceutil;
pub mod gic;
pub mod io;
pub mod ipc;
pub mod irq;
pub mod kmem;
pub mod mailbox;

pub mod pagealloc;
pub mod param;
pub mod pre_mmu;
pub mod process;
pub mod qemu;
pub mod reg;
pub mod registers;
pub mod registry;
pub mod runtime;
pub mod swtch;
pub mod system;
pub mod timer;
pub mod trap;
pub mod uartmini;
pub mod uartpl011;
pub mod vm;
pub mod vmdebug;

extern crate alloc;

// The boot code, which ends in `bl main9`.  `ENTRY(start)` in kernel.ld
// names its entry symbol, which is what keeps this object from being
// dropped when the library is linked into a binary that never names it.
#[cfg(target_os = "none")]
core::arch::global_asm!(include_str!("l.S"));
