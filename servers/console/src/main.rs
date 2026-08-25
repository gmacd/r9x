//! The console server: the first real r9 user-space server, built as a
//! separate bare-metal Rust executable (the user-binary-loading plan).
//!
//! It does what the hand-assembled `SERVER_TEXT` in the old
//! `console_server` image did, written in Rust: map the PL011 UART's
//! physical register page into this process's own address space via
//! `SYSMAPMMIO`, write `'A'` to the data register, and exit.  From stage
//! 6 it also takes a name: it creates its own channel pair and publishes
//! it under `/dev/console` in the nameserver, so a client can find it by
//! name instead of by a hardcoded handle.
//!
//! `core` plus a syscall shim, nothing else: no std, no alloc, no link-time
//! dependency on the kernel.  The syscall numbers and the nameserver
//! protocol are the r9 ABI, mirrored here so the server stands alone; when
//! a second server needs the same shim the arc extracts an `r9-sys` crate.
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
const SYCSEND: u64 = 16;
const SYCRECEIVE: u64 = 17;
const SYCREPLY: u64 = 18;
const SYS_MAP_MMIO: u64 = 20;
const SYCCREATECHAN: u64 = 21;

/// The nameserver protocol the server speaks (mirrors
/// `servers/nameserver`): `BIND` is the verb the server sends to publish its
/// name; `R_OK` / `R_EFULL` are the results the nameserver replies.
const OP_BIND: u16 = 0;
const R_OK: u16 = 0;
const R_EFULL: u16 = 2;

/// The name the server publishes under.  Absolute, the way a file server's
/// path is — the client resolves this, not a raw channel handle.
const NAME: &[u8] = b"/dev/console";

/// The VA the spawner writes the nameserver's channel pair into before this
/// server's first instruction (the user-VA convention both ends read from
/// `port::user::HANDLES_VA`).  This server is a *client* of the nameserver
/// (it sends it a `BIND`), so the pair it reads is the nameserver's — the
/// spawner hands it the nameserver's handles, as it hands the nameserver its
/// own.  Mirrored here so the server stands alone.
const HANDLES_VA: u64 = 0x100_0000;

/// The r9 message-syscall shim: the number in `x8`, the arguments in
/// `x0`–`x4`, the result back in `x0` (and, for a receive, the byte count and
/// tag in `x3` / `x4`).  The receive uses `x3` / `x4` as inputs the kernel
/// does not write and as its outputs, so all three are `inout`.
#[inline(always)]
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

/// End the process.  The kernel records the svc number (here always
/// `SYS_EXIT`) as the exit status, so the first argument is carried for the
/// ABI's shape and is not a distinguishable code.
#[inline(never)]
unsafe fn exit(code: u64) -> ! {
    // SAFETY: same as `sys`; the exit syscall transfers to the kernel and the
    // process does not resume, so the trailing spin is never reached.
    let _ = unsafe { sys(SYS_EXIT, code, 0, 0, 0, 0) };
    loop {
        core::hint::spin_loop();
    }
}

/// Write one byte to the PL011 data register (the kernel console): the
/// server's output to the terminal.
unsafe fn trace(dr: *mut u32, b: u8) {
    // SAFETY: `dr` is the PL011 data register (a Device-memory page); a
    // 32-bit volatile write is the register access the hardware expects.
    unsafe { core::ptr::write_volatile(dr, b as u32) };
}

/// Read the nameserver's channel pair from `HANDLES_VA`: the inbound channel
/// this server sends its `BIND` to and the outbound channel it receives the
/// result on.  The spawner wrote `[in:4 LE][out:4 LE]` here before the
/// server's first instruction.
unsafe fn read_nameserver_pair() -> (u64, u64) {
    let p = HANDLES_VA as *const u32;
    // SAFETY: the spawner wrote the pair to this page before the server's
    // first instruction; a 32-bit read is the access it expects.
    let in_h = unsafe { core::ptr::read_volatile(p) } as u64;
    let out_h = unsafe { core::ptr::read_volatile(p.add(1)) } as u64;
    (in_h, out_h)
}

/// The server's entry point: where the loader sets `e_entry`.  Maps the PL011
/// into this process's own address space, writes `'A'` to the data register
/// (offset 0x00; the FIFO is empty on a fresh boot, so no DR-ready poll),
/// creates its own channel pair, publishes it under `/dev/console` in the
/// nameserver, and exits.
#[inline(never)]
#[unsafe(no_mangle)]
pub extern "C" fn start() -> ! {
    // If the map fails, the write below faults and the kernel's EL0 fault path
    // kills this process — the failure policy — so the result is not checked.
    let _ = unsafe { sys(SYS_MAP_MMIO, PL011_PHYS, MMIO_VA, 0, 0, 0) };
    let dr = MMIO_VA as *mut u32;
    // SAFETY: `dr` is the PL011 data register, mapped by the `SYSMAPMMIO`
    // above as a Device-memory page; a 32-bit volatile write is the register
    // access the hardware expects.
    unsafe { trace(dr, b'A') };

    // Create this server's own channel pair: the inbound channel clients
    // send to and the outbound channel it replies on.  Two `SYCCREATECHAN`s
    // — the boring, total form (a connected-pair convenience is not needed).
    let in_h = unsafe { sys(SYCCREATECHAN, 0, 0, 0, 0, 0) }.0;
    let out_h = unsafe { sys(SYCCREATECHAN, 0, 0, 0, 0, 0) }.0;

    // Read the nameserver's pair: the one to send the `BIND` to.
    let (ns_in, ns_out) = unsafe { read_nameserver_pair() };

    // Build the `BIND` request: `[name][in:4 LE][out:4 LE]`, the nameserver's
    // envelope — the NUL-free name followed by the server's own pair.
    // `ptr::copy_nonoverlapping` rather than `copy_from_slice`: the latter is
    // a non-inlined call in this `no_std` build and its stack frame exceeds
    // the mapped user stack; the direct copy is inlined and uses no extra frame.
    let mut req = [0u8; NAME.len() + 8];
    {
        let n = NAME.len();
        // SAFETY: `req[..n]` and `NAME` are the same length and do not
        // overlap; the 4-byte LE halves are disjoint from each other and
        // from the name.
        unsafe {
            core::ptr::copy_nonoverlapping(NAME.as_ptr(), req.as_mut_ptr(), n);
            let ib = in_h.to_le_bytes();
            core::ptr::copy_nonoverlapping(ib.as_ptr(), req.as_mut_ptr().add(n), 4);
            let ob = out_h.to_le_bytes();
            core::ptr::copy_nonoverlapping(ob.as_ptr(), req.as_mut_ptr().add(n + 4), 4);
        };
    }

    // Publish: send the `BIND` to the nameserver's inbound channel.
    let _ =
        unsafe { sys(SYCSEND, ns_in, req.as_ptr() as u64, req.len() as u64, OP_BIND as u64, 0) };

    // Receive the result on the nameserver's outbound channel.  It is
    // `R_OK` or `R_EFULL`; on a non-`OK` the server still proceeds — binding
    // is the namespace's concern and the image asserts the bind landed, so
    // a failure here is reported by the image, not the server.
    let mut reply = [0u8; 8];
    let (op, _, _) =
        unsafe { sys(SYCRECEIVE, ns_out, reply.as_mut_ptr() as u64, reply.len() as u64, 0, 0) };
    let _ = (op == R_OK as u64) || (op == R_EFULL as u64);

    // One-shot client service: receive a byte on this server's own inbound
    // channel, echo it back on the outbound channel, then exit.  A
    // persistent loop is stage 7's concern (9P servers serve many clients);
    // this arc needs only one round-trip to prove the namespace works end to
    // end.
    let mut req_buf = [0u8; 16];
    let (_, bytes, tag) =
        unsafe { sys(SYCRECEIVE, in_h, req_buf.as_mut_ptr() as u64, req_buf.len() as u64, 0, 0) };
    let n = bytes as usize;
    // Reply with the received byte echoed: opcode `R_OK`, payload is the
    // first `n` bytes of the request (the client sends one byte).
    let reply_len = n.min(16);
    let _ = unsafe {
        sys(SYCREPLY, out_h, req_buf.as_ptr() as u64, reply_len as u64, R_OK as u64, tag)
    };

    // SAFETY: `exit` issues the exit syscall and never returns.
    unsafe { exit(0) };
}

/// Exists so the `no_std` link succeeds.  This server's body cannot panic —
/// no indexing, no arithmetic that overflows — so this is a last-resort spin
/// the kernel is not expected to observe, not a path it handles.
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    // A spin is a valid `-> !` body and makes no `unreachable_unchecked` claim;
    // the kernel is not expected to observe it because this server cannot
    // panic (no indexing, no overflowing arithmetic).
    loop {
        core::hint::spin_loop();
    }
}
