//! Process control: the syscalls a process issues about itself.

use r9x_abi::{SYSEXIT, SYSYIELD};

use crate::sys::sys;

/// End this process.  The kernel records the svc number as the exit status,
/// so `code` is carried for the ABI's shape and is not a distinguishable
/// code.
#[inline(never)]
pub fn exit(code: u64) -> ! {
    let _ = unsafe { sys(SYSEXIT, code, 0, 0, 0, 0) };
    loop {
        core::hint::spin_loop();
    }
}

/// Voluntarily yield the CPU to other ready processes.
pub fn yield_now() {
    let _ = unsafe { sys(SYSYIELD, 0, 0, 0, 0, 0) };
}
