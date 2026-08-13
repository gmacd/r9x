//! The x86-64 kernel as a library.
//!
//! Everything except the entry point lives here, so that the kernel binary
//! and each QEMU integration test can link the same code and supply their
//! own `main`: a test runs exactly the initialisation it needs and no more.
#![cfg_attr(target_os = "none", no_std)]
#![cfg_attr(target_os = "none", feature(alloc_error_handler))]
#![feature(fn_align)]
#![feature(sync_unsafe_cell)]
#![allow(clippy::upper_case_acronyms)]
#![forbid(unsafe_op_in_unsafe_fn)]

pub mod allocator;
pub mod cpu;
pub mod dat;
pub mod devcons;
pub mod node0;
pub mod pio;
pub mod proc;
pub mod qemu;
pub mod runtime;
pub mod syscall;
pub mod trap;
pub mod uart16550;
pub mod vsvm;

extern crate alloc;

// Modules reach for `crate::println`, which resolved through the binary's
// own import before the library existed.
use port::println;

// The boot code, which ends in a call to main.  `ENTRY(start)` in kernel.ld
// names its entry symbol, which is what keeps this object from being
// dropped when the library is linked into a binary that never names it.
#[cfg(target_os = "none")]
core::arch::global_asm!(include_str!("l.S"), options(att_syntax));
