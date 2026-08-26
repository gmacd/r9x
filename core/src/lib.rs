//! Shared *code* — pure, no syscalls, no privilege — that both the kernel and
//! the r9x user-space link.  Currently the FDT (flattened device tree) parser:
//! the kernel uses it for pre-server bringup, and servers use it to look up
//! their MMIO bases for `SYS_MAP_MMIO` requests (the DTB VA arrives as the
//! first `main9(dtb_va)` entry argument).
//!
//! Because it is pure (no privileged operations, no service calls) it is safe
//! for the unprivileged servers to link, exactly as `r9x_abi` (the shared
//! *constants*) is.  The kernel's `port` keeps its kernel-only half and
//! depends on this crate for the shared code.

#![no_std]
#![feature(step_trait)]

#[cfg(test)]
extern crate alloc;

pub mod addr;
pub mod fdt;
