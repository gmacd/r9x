//! The thin syscall core: the only per-architecture part of `r9x_std`.
//!
//! The kernel's syscall ABI — the numbers and the register conventions — is
//! owned by [`r9x_abi`], the single source both the kernel and this target
//! read.  [`sys`] is the raw trap: it places the number and the arguments in
//! the registers the kernel reads and returns the result.  The named service
//! wrappers live in their semantic modules ([`crate::ipc`], [`crate::mem`],
//! [`crate::process`]).

/// Issue a raw r9 syscall.  The number goes in the OS register, the arguments
/// in the argument registers; the result comes back in the first, and for a
/// receive the byte count and the tag come back in the third and fourth.
///
/// # Safety
///
/// The caller places the arguments the kernel reads for `n` (for a message
/// syscall, the buffer pointers and their lengths) and guarantees those
/// buffers are valid for the kernel's access.
#[inline(always)]
pub unsafe fn sys(n: u64, a0: u64, a1: u64, a2: u64, a3: u64, a4: u64) -> (u64, u64, u64) {
    unsafe { trap(n, a0, a1, a2, a3, a4) }
}

/// The aarch64 trap: the message-syscall convention the kernel's EL0 path
/// reads.  The number in `x8`, the arguments in `x0`–`x4`, the result back in
/// `x0` (and, for a receive, the byte count and tag in `x3` / `x4`).  The
/// receive uses `x3` / `x4` as inputs the kernel does not write and as its
/// outputs, so all three are `inout`.
#[cfg(target_arch = "aarch64")]
unsafe fn trap(n: u64, a0: u64, a1: u64, a2: u64, a3: u64, a4: u64) -> (u64, u64, u64) {
    let mut x0 = a0;
    let mut x3 = a3;
    let mut x4 = a4;
    // SAFETY: the constraints place the ABI registers the kernel reads and
    // read the result back out of `x0` (and, for a receive, `x3` / `x4`).
    // `clobber_abi("C")` tells the compiler the `svc` clobbers every other
    // caller-saved register (aarch64's `x0`–`x18`), so no live value held in
    // one survives the call unmentioned.  `nostack` holds — the syscall uses
    // no red zone.  There is deliberately no `nomem`: the kernel dereferences
    // the buffer pointers passed in the argument registers, so the block
    // must be assumed to touch memory, and the compiler is left to treat the
    // `svc` as a full memory barrier.
    unsafe {
        core::arch::asm!(
            "svc #0",
            in("x8") n,
            in("x1") a1,
            in("x2") a2,
            inout("x3") x3,
            inout("x4") x4,
            inout("x0") x0,
            options(nostack),
            clobber_abi("C"),
        );
    }
    (x0, x3, x4)
}

/// The non-aarch64 trap: not exercised (Aspace, the user-space MMU, is
/// aarch64-only, so no riscv64 or x86-64 r9 server builds), but a valid
/// function so the crate compiles for every r9 target.
#[cfg(not(target_arch = "aarch64"))]
unsafe fn trap(n: u64, a0: u64, a1: u64, a2: u64, a3: u64, a4: u64) -> (u64, u64, u64) {
    let _ = (n, a0, a1, a2, a3, a4);
    core::unreachable!("r9 syscalls are aarch64-only in this arc");
}
