//! Layout conventions for r9 user binaries — the static, non-PIE, fixed-base
//! ELF64s the servers are built as (the user-binary-loading plan).
//!
//! The values live in [`r9x_abi`], the single source both the kernel and the
//! `r9x_std` target read, and are re-exported here so the existing
//! `port::user::` paths keep working; a pinning test asserts they match.  Each
//! value is a stated convention, not a hardware fact.

pub use r9x_abi::{HANDLES_VA, IMAGE_BASE};
