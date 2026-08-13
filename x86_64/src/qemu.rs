//! Leaving QEMU with a status, for integration tests.
//!
//! Uses the isa-debug-exit device, which QEMU adds with
//! `-device isa-debug-exit,iobase=0xf4,iosize=0x04`.  Writing to its port
//! makes QEMU exit with `(value << 1) | 1`, so the status is always odd
//! and never zero: a passing run cannot be reported as an exit code of 0
//! the way it is on the other architectures, and the harness has to know
//! which status this arch calls success.

use crate::pio::outl;

/// Written to leave QEMU with [`PASS_STATUS`].
pub const PASS: u32 = 0x10;
/// Written to leave QEMU with a status that is not [`PASS_STATUS`].
pub const FAIL: u32 = 0x11;

/// The process exit status QEMU produces for [`PASS`].
///
/// `(0x10 << 1) | 1`.  Kept beside the value it comes from so the two
/// cannot drift; xtask asks the arch for this rather than assuming zero.
pub const PASS_STATUS: i32 = 33;

/// Where `-device isa-debug-exit` is asked to sit.
pub const IOBASE: u16 = 0xf4;

/// Stop the machine, handing `code` back to QEMU's own exit status.
///
/// Returns only if the device is absent, in which case there is no way to
/// stop and the caller's timeout is what ends the run.
#[cfg(target_os = "none")]
pub fn exit(code: u32) -> ! {
    unsafe { outl(IOBASE, code) };
    loop {
        unsafe { core::arch::asm!("hlt", options(nomem, nostack)) }
    }
}

#[cfg(not(target_os = "none"))]
pub fn exit(_code: u32) -> ! {
    unimplemented!("exit is only meaningful on the bare metal target")
}
