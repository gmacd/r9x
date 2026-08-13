//! Leaving QEMU with a status, for integration tests.
//!
//! Uses the Arm semihosting SYS_EXIT call, which QEMU implements when run
//! with `-semihosting`.  Unlike PSCI SYSTEM_OFF this carries an exit code,
//! so a test binary can report pass or fail to the process that spawned it.

/// Exit code QEMU returns when every test in an image passed.
pub const PASS: u32 = 0;
/// Exit code QEMU returns when a test failed or the kernel panicked.
pub const FAIL: u32 = 1;

/// Stop the machine, handing `code` back to QEMU's own exit status.
///
/// With semihosting enabled this does not return.  Without it QEMU takes
/// `hlt #0xf000` for an unallocated instruction and traps, so the loop
/// below is reached only by way of a handler that returns here.  Either
/// way the machine stops making progress and the caller's timeout is what
/// ends the run.
#[cfg(target_os = "none")]
pub fn exit(code: u32) -> ! {
    const SYS_EXIT: u64 = 0x18;
    const APPLICATION_EXIT: u64 = 0x2_0026;

    // SYS_EXIT takes a two word block on aarch64: passing the reason code
    // directly is the aarch32 form and loses the exit status.
    let block = [APPLICATION_EXIT, code as u64];
    unsafe {
        core::arch::asm!(
            "hlt #0xf000",
            // Semihosting hands its result back in x0, so the operation
            // number goes in as an output that is thrown away: `in` would
            // promise the register comes back holding SYS_EXIT.
            inout("x0") SYS_EXIT => _,
            in("x1") block.as_ptr(),
            options(nostack),
        );
    }
    loop {
        unsafe { core::arch::asm!("wfe", options(nomem, nostack)) }
    }
}

#[cfg(not(target_os = "none"))]
pub fn exit(_code: u32) -> ! {
    unimplemented!("exit is only meaningful on the bare metal target")
}
