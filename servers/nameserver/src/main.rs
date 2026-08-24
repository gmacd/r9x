//! The nameserver: the process the whole file metaphor resolves through.
//!
//! It owns the bind table — a fixed `name -> (in, out) channel-pair` map — and
//! serves three verbs over the message syscalls:
//!
//! - `BIND`: a file server publishes its channel pair under a name.
//! - `RESOLVE`: a client asks for a name and gets the pair back.
//! - `UNBIND`: a file server withdraws its name.
//!
//! A channel is unidirectional, so a request/reply runs over a *pair*: the
//! client sends on the server's inbound channel and receives on its outbound
//! channel.  The bind entry therefore stores the pair, not one handle — a name
//! that resolved to only the inbound channel would let a client send but never
//! receive the reply.
//!
//! It is the first server: its own pair is passed to it by its spawner (a page
//! at `HANDLES_VA`), not created by it — nothing exists yet that a client could
//! ask to find the nameserver, so the spawner hands it the pair directly.
//!
//! `core` plus a syscall shim, nothing else: no std, no alloc, no link-time
//! dependency on the kernel.  The syscall numbers and the `HANDLES_VA`
//! convention are mirrored from the kernel so the server stands alone; when a
//! second server needs the same shim the arc extracts an `r9-sys` crate.
#![no_std]
#![no_main]

/// The bind table and the result codes it replies with (a pure-`core` module,
/// unit-tested on the host).
mod bind_table;
use bind_table::{BindTable, Pair, R_ENOENT, R_OK};

/// The VA the spawner writes this server's own channel pair into before its
/// first instruction (the user-VA convention both ends read from
/// `port::user::HANDLES_VA`); mirrored here so the server stands alone.
const HANDLES_VA: u64 = 0x100_0000;

/// The r9 message-syscall numbers, mirrored from the kernel's `aarch64::
/// process`: send (x0=handle, x1=buf, x2=len, x3=opcode, x4=tag), receive
/// (x0=handle, x1=buf, x2=cap; on return x0=opcode, x3=bytes, x4=tag), reply
/// (x0=handle, x1=buf, x2=len, x3=opcode, x4=tag).
const SYCRECEIVE: u64 = 17;
const SYCREPLY: u64 = 18;

/// The message payload bound (mirrors the kernel's `port::ipc::MSG_MAX`).
const MSG_MAX: usize = 256;

/// The request verbs (the message opcode a client sends).
const OP_BIND: u16 = 0;
const OP_RESOLVE: u16 = 1;
const OP_UNBIND: u16 = 2;

/// The r9 message-syscall shim: the number in `x8`, the arguments in
/// `x0`–`x4`, the result back in `x0` (and, for a receive, the byte count and
/// tag in `x3` / `x4`).  The receive and reply forms use `x3` / `x4` as
/// inputs (the opcode / tag) that the kernel does not write, and the receive
/// uses them as outputs, so all three are `inout`.
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

/// Receive a message on `handle` into `buf` (at most `buf.len()` bytes).
/// Blocks until one arrives.  Returns `(opcode, bytes, tag)`.
unsafe fn receive(handle: u64, buf: &mut [u8]) -> (u16, usize, u32) {
    let (op, bytes, tag) =
        unsafe { sys(SYCRECEIVE, handle, buf.as_mut_ptr() as u64, buf.len() as u64, 0, 0) };
    (op as u16, bytes as usize, tag as u32)
}

/// Reply on `handle` with `result` as the opcode and `payload` as the bytes,
/// correlated to `tag`.
unsafe fn reply(handle: u64, result: u16, tag: u32, payload: &[u8]) {
    let n = payload.len().min(MSG_MAX);
    let _ = unsafe {
        sys(SYCREPLY, handle, payload.as_ptr() as u64, n as u64, result as u64, tag as u64)
    };
}

/// Read this server's own channel pair from the spawner-passed page at
/// `HANDLES_VA`: `[in:4 LE][out:4 LE]`.
unsafe fn read_pair() -> Pair {
    let p = HANDLES_VA as *const u32;
    // SAFETY: the spawner mapped the page at `HANDLES_VA` into this process's
    // address space and wrote `[in:4 LE][out:4 LE]` before this process's
    // first instruction (the user-VA convention); the two `u32` reads are
    // in-bounds of that page.
    let in_h = unsafe { core::ptr::read_volatile(p) };
    let out_h = unsafe { core::ptr::read_volatile(p.add(1)) };
    Pair { in_h, out_h }
}

/// The server's entry point: where the loader sets `e_entry`.  Reads its own
/// pair and serves the bind table.  A server runs until it is killed; there is
/// no clean exit this arc, so the loop is unbounded.
#[inline(never)]
#[unsafe(no_mangle)]
pub extern "C" fn start() -> ! {
    let pair = unsafe { read_pair() };
    let mut table = BindTable::new();
    let mut buf = [0u8; MSG_MAX];
    loop {
        let (op, bytes, tag) = unsafe { receive(pair.in_h as u64, &mut buf) };
        let payload = &buf[..bytes.min(MSG_MAX)];
        // Dispatch the verb.  The payload layout is the stated convention the
        // client mirrors: a request carries the NUL-free `name`; a BIND
        // carries the name followed by the bound pair (`[in:4 LE][out:4 LE]`),
        // so the name is the first `len - 8` bytes of a BIND and the whole
        // payload of a RESOLVE / UNBIND.
        let (result, found) = match op {
            OP_BIND => {
                // A well-formed BIND is at least 8 bytes (a zero-length name
                // plus the pair); the kernel bounds `payload` to `MSG_MAX`, and
                // a shorter payload is a client bug the arc does not produce —
                // the slice below is in-bounds for every payload it does.
                let name_len = payload.len() - 8;
                let name = &payload[..name_len];
                let in_h = u32::from_le_bytes(payload[name_len..name_len + 4].try_into().unwrap());
                let out_h =
                    u32::from_le_bytes(payload[name_len + 4..name_len + 8].try_into().unwrap());
                (table.bind(name, Pair { in_h, out_h }), None)
            }
            OP_RESOLVE => match table.resolve(payload) {
                Some(p) => (R_OK, Some(p)),
                None => (R_ENOENT, None),
            },
            OP_UNBIND => (table.unbind(payload), None),
            // An unknown verb is answered like an unknown name.
            _ => (R_ENOENT, None),
        };
        // A RESOLVE that found a name replies with the pair; every other reply
        // is empty.
        let mut out = [0u8; 8];
        let out_slice: &[u8] = match found {
            Some(p) => {
                out[..4].copy_from_slice(&p.in_h.to_le_bytes());
                out[4..8].copy_from_slice(&p.out_h.to_le_bytes());
                &out[..8]
            }
            None => &[],
        };
        unsafe { reply(pair.out_h as u64, result, tag, out_slice) };
    }
}

/// Exists so the `no_std` link succeeds.  This server's body cannot panic —
/// the only indexing is by lengths the kernel already bounds to `MSG_MAX` (and
/// a well-formed BIND is at least 8 bytes), and the pair read is two in-bounds
/// words — so this is a last-resort spin the kernel is not expected to
/// observe, not a path it handles.
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {
        core::hint::spin_loop();
    }
}
