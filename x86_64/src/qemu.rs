//! Leaving QEMU with a status, for integration tests.
//!
//! Uses the isa-debug-exit device, which QEMU adds with
//! `-device isa-debug-exit,iobase=0xf4,iosize=0x04`.  Writing to its port
//! makes QEMU exit with `(value << 1) | 1`, so the status is always odd
//! and never zero: a passing run cannot be reported as an exit code of 0
//! the way it is on the other architectures, and the harness has to know
//! which status this arch calls success.

use crate::pio::outl;
use port::qemu::*;

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
