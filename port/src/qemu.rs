//! QEMU exit status constants.

/// Written to leave QEMU with [`PASS_STATUS`].
pub const PASS: u32 = 0x10;
/// Written to leave QEMU with a status that is not [`PASS_STATUS`].
pub const FAIL: u32 = 0x11;

/// The process exit status QEMU produces for [`PASS`].
///
/// `(0x10 << 1) | 1`.
pub const PASS_STATUS: i32 = 33;

/// Where `-device isa-debug-exit` is asked to sit.
pub const IOBASE: u16 = 0xf4;
