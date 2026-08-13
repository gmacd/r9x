//! Leaving QEMU with a status, for integration tests.
//!
//! Uses the sifive_test finisher the virt machine maps at 0x100000, whose
//! writes QEMU turns into its own exit status.  SBI's system reset cannot
//! serve here: it carries no code, so QEMU would exit zero whether the run
//! passed or failed.
//!
//! The kernel runs with paging off, so the finisher's physical address is
//! the address to write.

/// Exit code QEMU returns when every test in an image passed.
pub const PASS: u32 = 0;
/// Exit code QEMU returns when a test failed or the kernel panicked.
pub const FAIL: u32 = 1;

/// Stop the machine, handing `code` back to QEMU's own exit status.
#[cfg(target_os = "none")]
pub fn exit(code: u32) -> ! {
    /// Where the virt machine maps the finisher.
    const FINISHER: *mut u32 = 0x10_0000 as *mut u32;
    /// Exits QEMU with zero, whatever is in the high half.
    const FINISHER_PASS: u32 = 0x5555;
    /// Exits QEMU with the code in the high half.
    const FINISHER_FAIL: u32 = 0x3333;

    // A failure carrying zero would exit zero as surely as a pass, so
    // anything that is not a pass reports at least one.
    let status = if code == PASS { FINISHER_PASS } else { FINISHER_FAIL | (code.max(1) << 16) };

    unsafe { core::ptr::write_volatile(FINISHER, status) };

    // Only reached if the machine has no finisher, in which case there is
    // no way to stop and the caller's timeout is what ends the run.
    loop {
        unsafe { core::arch::asm!("wfi", options(nomem, nostack)) }
    }
}

#[cfg(not(target_os = "none"))]
pub fn exit(_code: u32) -> ! {
    unimplemented!("exit is only meaningful on the bare metal target")
}
