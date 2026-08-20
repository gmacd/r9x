//! State for the first user process (plans/first-user-process.md).
//!
//! One process, one caller, so the whole state is two words: where the
//! kernel that started the process resumes, and the status the process
//! exited with.  `run` lands in a following task; the exit trap
//! consumes it.

// Only the accessors below use it; on host builds they are cfg'd away
// with the rest of the module's machinery.
#[cfg(target_os = "none")]
use crate::swtch::Context;

/// `svc #0`: the process asks to be killed.  The syscall number is the
/// svc immediate, held in ESR_EL1.ISS for EC 0x15 (Arm ARM DDI 0487).
pub const SYSEXIT: u64 = 0;

// Single-core by the l.S gate (non-zero MPIDR affinity hangs at
// boot), so the statics need no synchronisation.  They exist only
// where the accessors below do: host builds (unit tests) have no
// process machinery at all.
#[cfg(target_os = "none")]
static mut KERNEL_RETURN: *const Context = core::ptr::null();
#[cfg(target_os = "none")]
static mut EXIT_STATUS: u64 = 0;

/// Record where the kernel that is about to start a process resumes.
///
/// # Safety
///
/// Single core.  `ctx` must be a context `swtch` saved on the
/// caller's stack, and the caller's frame must stay live until the
/// exit trap has consumed it.
// process-run lands the first caller; the expectation fails as a
// prompt to drop it.
#[cfg(target_os = "none")]
#[expect(dead_code)]
pub(crate) unsafe fn set_kernel_return(ctx: *const Context) {
    unsafe { KERNEL_RETURN = ctx };
}

/// The kernel context a running process exits to, or null.
///
/// # Safety
///
/// Single core.
#[cfg(target_os = "none")]
pub(crate) unsafe fn kernel_return() -> *const Context {
    unsafe { KERNEL_RETURN }
}

/// Record a process's exit status.
///
/// # Safety
///
/// Single core.  Called only from the exit trap, which cannot run
/// unless `set_kernel_return` has run.
#[cfg(target_os = "none")]
pub(crate) unsafe fn set_exit_status(status: u64) {
    unsafe { EXIT_STATUS = status };
}

/// The status the last process exited with.
///
/// # Safety
///
/// Single core.
// process-run lands the first caller; the expectation fails as a
// prompt to drop it.
#[cfg(target_os = "none")]
#[expect(dead_code)]
pub(crate) unsafe fn exit_status() -> u64 {
    unsafe { EXIT_STATUS }
}
