//! Layout conventions for r9 user binaries — the static, non-PIE, fixed-base
//! ELF64s the servers are built as (the user-binary-loading plan).
//!
//! These live here, in `port`, so the two ends that must agree on them read
//! the same constants and cannot drift: the *build* (xtask links a server at
//! `IMAGE_BASE` via `--image-base`) and the *loader* (the per-arch
//! `spawn_elf` placement check rejects any segment below it).  Each value is
//! a stated convention, not a hardware fact.

/// The base a user binary is linked at (`--image-base`): page-aligned, in the
/// TTBR0/user half.  The build links at it and the loader rejects any segment
/// placed below it, so the two agree by construction.  It sits clear of the
/// very top of the user half (where an MMIO a server maps lives) and clear of
/// the kernel's low mappings.
pub const IMAGE_BASE: usize = 0x10_0000;
