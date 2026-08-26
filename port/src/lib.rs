#![allow(clippy::upper_case_acronyms)]
#![allow(clippy::too_long_first_doc_paragraph)]
#![cfg_attr(not(any(test)), no_std)]
#![feature(allocator_api)]
#![forbid(unsafe_op_in_unsafe_fn)]

extern crate alloc;

pub mod allocator;
pub mod bitmapalloc;
pub mod dat;
pub mod devcons;
pub mod elf;
pub mod ipc;
pub mod irq;
pub mod maths;
pub mod mcslock;
pub mod mem;
pub mod once;
pub mod pagealloc;
pub mod qemu;
pub mod user;

pub type Result<T> = core::result::Result<T, &'static str>;

#[cfg(test)]
mod tests {
    //! Decision 3 fallback: the kernel's re-exports of the shared ABI constants
    //! must equal [`r9x_abi`], the single source both the kernel and the
    //! `r9x_std` target read.  The re-exports make this hold by construction;
    //! the test guards against a divergent re-hardcode.

    #[test]
    fn abi_constants_match_r9x_abi() {
        assert_eq!(crate::ipc::MSG_MAX, r9x_abi::MSG_MAX);
        assert_eq!(crate::user::HANDLES_VA, r9x_abi::HANDLES_VA);
        assert_eq!(crate::user::IMAGE_BASE, r9x_abi::IMAGE_BASE);
    }
}
