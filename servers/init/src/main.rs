//! The init process: the first "real" user process.
//!
//! In this arc it does nothing but exist and block: it creates a channel
//! pair and waits on a receive that no one satisfies.  Stage 7 fills it in
//! as the process manager (spawn servers, handle crashes, restart them).
//!
//! `core` plus a syscall shim, nothing else: no std, no alloc, no link-time
//! dependency on the kernel.
#![no_std]
#![no_main]

/// The r9 message-syscall numbers, mirrored from the kernel's `aarch64::
/// process`: createchan (no args; on return x0=handle), receive
/// (x0=handle, x1=buf, x2=cap; on return x0=opcode, x3=bytes, x4=tag).
const SYCCREATECHAN: u64 = 21;
const SYCRECEIVE: u64 = 17;

/// The message payload bound (mirrors the kernel's `port::ipc::MSG_MAX`).
const MSG_MAX: usize = 256;

/// The r9 message-syscall shim: the number in `x8`, the arguments in
/// `x0`–`x4`, the result back in `x0` (and, for a receive, the byte count
/// and tag in `x3` / `x4`).
unsafe fn sys(n: u64, a0: u64, a1: u64, a2: u64, a3: u64, a4: u64) -> (u64, u64, u64) {
    let mut x0 = a0;
    let mut x3 = a3;
    let mut x4 = a4;
    // SAFETY: the constraints place the ABI registers the kernel reads (the
    // number in `x8`, the arguments in `x0`–`x4`) and read the result back
    // out of `x0` (and, for a receive, the byte count and tag out of `x3` /
    // `x4`).  `clobber_abi("C")` tells the compiler the `svc` clobbers every
    // other caller-saved register (aarch64's `x0`–`x18`), so no live value the
    // compiler holds in one survives the call unmentioned; `nomem` and
    // `nostack` hold because the syscall's memory and state effects are the
    // kernel's, not direct reads or writes by this asm.
    unsafe {
        core::arch::asm!(
            "svc #0",
            in("x8") n,
            in("x1") a1,
            in("x2") a2,
            inout("x3") x3,
            inout("x4") x4,
            inout("x0") x0,
            options(nomem, nostack),
            clobber_abi("C"),
        );
    }
    (x0, x3, x4)
}

/// The init entry point: where the loader sets `e_entry`.  Creates a channel
/// pair to block on (stage 7 will send events here) and waits forever.
#[inline(never)]
#[unsafe(no_mangle)]
pub extern "C" fn start() -> ! {
    // Create a channel pair to block on.  The inbound channel is the one
    // stage 7's event delivery will target; the outbound is unused for now.
    let in_h = unsafe { sys(SYCCREATECHAN, 0, 0, 0, 0, 0) }.0;
    let _out_h = unsafe { sys(SYCCREATECHAN, 0, 0, 0, 0, 0) }.0;

    // Block forever: receive on the inbound channel (no one sends).  The
    // buffer is never read — a wakeup is a stage-7 concern.
    let mut buf = [0u8; MSG_MAX];
    loop {
        let _ = unsafe { sys(SYCRECEIVE, in_h, buf.as_mut_ptr() as u64, buf.len() as u64, 0, 0) };
    }
}

/// Exists so the `no_std` link succeeds.  This process's body cannot panic —
/// the only syscalls are `SYCCREATECHAN` (returns a handle or an error code)
/// and `SYCRECEIVE` (blocks) — so this is a last-resort spin the kernel is
/// not expected to observe, not a path it handles.
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {
        core::hint::spin_loop();
    }
}
