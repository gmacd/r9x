//! The riscv64 kernel as a library.
//!
//! Everything except the entry point lives here, so that the kernel binary
//! and each QEMU integration test can link the same code and supply their
//! own `main9`: a test runs exactly the initialisation it needs and no more.
#![cfg_attr(target_os = "none", no_std)]
#![cfg_attr(target_os = "none", feature(alloc_error_handler))]
#![feature(sync_unsafe_cell)]
#![allow(clippy::upper_case_acronyms)]
#![forbid(unsafe_op_in_unsafe_fn)]

pub mod allocator;
pub mod platform;
pub mod qemu;
pub mod runtime;
pub mod sbi;
pub mod uart16550;

extern crate alloc;

// The boot code, which ends in a jump to main9.  `ENTRY(start)` in
// kernel.ld names its entry symbol, which is what keeps this object from
// being dropped when the library is linked into a binary that never names
// it.
#[cfg(target_os = "none")]
core::arch::global_asm!(include_str!("l.S"));
