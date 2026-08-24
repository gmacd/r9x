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

/// The VA a server reads its spawner-passed state from: the page the
/// spawner writes a server's own channel pair (and, later, its parent
/// handles) into before the server's first instruction.  The nameserver is
/// the first consumer (stage 6): it is the first server, so nothing exists
/// yet that a client could ask to find it — its spawner must hand it the pair
/// directly.  A server's own pair is not a constant the server knows (unlike
/// the console server's PL011 base), so it is *passed*: the spawner writes
/// `[in:4 LE][out:4 LE]` here and the server reads it.  The VA is a stated
/// convention the spawner and the server both read from here, so they cannot
/// drift; task 4 (the `namespace` image) is what maps the page and writes the
/// pair.  It sits in the user half, clear of the image (`IMAGE_BASE`) and its
/// stack by a wide margin.
pub const HANDLES_VA: usize = 0x100_0000;
