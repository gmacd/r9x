//! The console server: the first real r9 user-space server, built as a
//! separate bare-metal Rust executable (the user-binary-loading plan).
//!
//! It does the same thing the hand-assembled `SERVER_TEXT` in the old
//! `console_server` image did, written in Rust: map the PL011 UART's physical
//! register page into this process's own address space via `SYSMAPMMIO`, write
//! `'A'` to the data register, and exit.  The kernel is device-dumb — it does
//! not parse the DT for the PL011 or map it into the server; the server knows
//! its own platform and requests the mapping itself (the QNX model).
//!
//! `core` plus a syscall shim, nothing else: no std, no alloc, no link-time
//! dependency on the kernel.  The syscall numbers are the r9 ABI, mirrored
//! here so the server stands alone; when a second server needs the same shim
//! the arc extracts an `r9-sys` crate.
#![no_std]
#![no_main]

/// The PL011 UART's physical base on the BCM2711 (QEMU `raspi4b`); a constant
/// the server knows, not something the kernel looks up for it.
const PL011_PHYS: u64 = 0xfe20_1000;
/// The VA the server chooses for the PL011 mapping: in the user (TTBR0) half,
/// far above the image (linked at `IMAGE_BASE`) and its 64 KiB stack, so it is
/// clear of both by a wide margin.  `SYSMAPMMIO` maps one 4 KiB Device page
/// here; the data register is the first word of that page.
const MMIO_VA: u64 = 0x8000_0000;

/// The r9 syscall numbers this server uses, mirrored from the kernel's
/// `aarch64::process` so the server is self-contained for the arc.
const SYS_EXIT: u64 = 0;
const SYS_MAP_MMIO: u64 = 20;

/// A syscall: the number in `x8`, the first two arguments in `x0` and `x1`,
/// the result back in `x0`.  `x0` is both an argument (in) and the result
/// (out), so it is one in-out operand; the number and the second argument are
/// moved to `x8` and `x1` before the `svc`.
#[inline(always)]
unsafe fn sys(n: u64, a0: u64, a1: u64) -> u64 {
    // `result` is the `x0` operand: it goes in as the first argument (`a0`) and
    // comes back as the result.
    let mut result = a0;
    // SAFETY: the constraints place the ABI registers the kernel reads (the
    // number in `x8`, the first two arguments in `x0`/`x1`) and read the result
    // back out of `x0`.  `clobber_abi("C")` tells the compiler the `svc`
    // clobbers every other caller-saved register (aarch64's x0–x18), so no live
    // value the compiler holds in one survives the call unmentioned; `nomem` and
    // `nostack` hold because the syscall's memory and state effects are the
    // kernel's, not direct reads or writes by this asm.
    unsafe {
        core::arch::asm!(
            "svc #0",
            in("x8") n,
            in("x1") a1,
            inout("x0") result,
            options(nomem, nostack),
            clobber_abi("C"),
        );
    }
    result
}

/// End the process.  The kernel records the svc number (here always
/// `SYS_EXIT`) as the exit status, so the first argument is carried for the
/// ABI's shape and is not a distinguishable code.
#[inline(never)]
unsafe fn exit(code: u64) -> ! {
    // SAFETY: same as `sys`; the exit syscall transfers to the kernel and the
    // process does not resume, so the trailing spin is never reached.
    unsafe { sys(SYS_EXIT, code, 0) };
    loop {
        core::hint::spin_loop();
    }
}

/// The server's entry point: where the loader sets `e_entry`.  Maps the PL011
/// into this process's own address space, writes `'A'` to the data register
/// (offset 0x00; the FIFO is empty on a fresh boot, so no DR-ready poll), and
/// exits.
#[inline(never)]
#[unsafe(no_mangle)]
pub extern "C" fn start() -> ! {
    // If the map fails, the write below faults and the kernel's EL0 fault path
    // kills this process — the failure policy — so the result is not checked.
    unsafe { sys(SYS_MAP_MMIO, PL011_PHYS, MMIO_VA) };
    let dr = MMIO_VA as *mut u32;
    // SAFETY: `dr` is the PL011 data register, mapped by the `SYSMAPMMIO`
    // above as a Device-memory page; a 32-bit volatile write is the register
    // access the hardware expects.
    unsafe { core::ptr::write_volatile(dr, b'A' as u32) };
    // SAFETY: `exit` issues the exit syscall and never returns.
    unsafe { exit(0) };
}

/// Exists so the `no_std` link succeeds.  This server's body cannot panic — no
/// indexing, no arithmetic that overflows — so this is a last-resort spin the
/// kernel is not expected to observe, not a path it handles.
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    // A spin is a valid `-> !` body and makes no `unreachable_unchecked` claim;
    // the kernel is not expected to observe it because this server cannot
    // panic (no indexing, no overflowing arithmetic).
    loop {
        core::hint::spin_loop();
    }
}
