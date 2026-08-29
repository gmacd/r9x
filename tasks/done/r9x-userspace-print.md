---
status: done
commit: fd7e96c
---

# Task: User-space print facility (SYS_PRINT)

## Status: done (fd7e96c)

## Context

User-space processes (servers) have no way to print diagnostic output. The
kernel has `iprintln!` (writes via `iputb` → PL011), but user-space has
nothing equivalent. The console server requires an IPC round-trip and must
already be running — useless when debugging the console server itself or any
server that starts before it.

This gap is actively blocking debugging: the mailbox server cannot report
what physical address the firmware returned, forcing kernel-side debug prints
as a workaround.

## Design

### `SYS_PRINT` syscall (number 31)

A minimal kernel-side print for debug:

- **Syscall number**: `SYS_PRINT = 31` (next free after `SYS_SETPRIO` = 30)
- **ABI**: `x0 = user VA of the string buffer`, `x1 = length in bytes`
  (no NUL terminator — the caller knows the length, matching the existing
  syscall convention)
- **Cap**: kernel truncates at 256 bytes (a debug facility, not a streaming
  I/O path)
- **Kernel path**: `copy_from_user` (already exists in `ipc.rs`) into a
  kernel stack buffer, then write byte-by-byte via `devcons::iputb` (the
  same path `iprintln!` uses)
- **Return**: `x0 = 0` on success
- **Concurrency**: `iputb` is a byte-at-a-time TXE spin; `iprintln!` is
  already non-atomic (interleaved across cores). For a debug facility this
  is acceptable. A spinlock can be added later if interleaving becomes a
  problem.
- **Not for production I/O**: this is a debug/boot facility. The production
  path is IPC to the console server (`/dev/cons`, task 88).

### `r9x_std::println!` macro

A `println!`-style macro in `r9x_std`:

```rust
// std/src/print.rs
#[macro_export]
macro_rules! println {
    () => { $crate::print::print_str("\n") };
    ($($t:tt)*) => { $crate::print::print_fmt(core::format_args!($($t)*)) };
}

pub mod print {
    use core::fmt::Write;
    use crate::sys;

    /// Write a string to the kernel debug console (SYS_PRINT).
    pub fn print_str(s: &str) {
        let _ = sys::syscall2(sys::SYS_PRINT, s.as_ptr() as u64, s.len() as u64);
    }

    /// Format into a 256-byte stack buffer, then SYS_PRINT. Truncated with
    /// `...` if the formatted output exceeds the buffer.
    pub fn print_fmt(args: core::fmt::Arguments<'_>) {
        let mut buf = [b' '; 256];
        let mut w = BufWriter(&mut buf);
        let n = match args.write(&mut w) {
            Ok(()) => w.len(),
            Err(_) => 256, // truncated
        };
        buf[n] = b'\n';
        n += 1;
        let _ = sys::syscall2(sys::SYS_PRINT, buf.as_ptr() as u64, n as u64);
    }
}
```

- `#[macro_export]` so it's usable as `r9x_std::println!(...)` from any
  user-space crate.
- The format buffer is 256 bytes on the stack. Longer output is truncated
  with a `...` suffix. Sufficient for debug.
- `print_str` for the simple case (no formatting), `print_fmt` for the
  formatted case. The macro dispatches between them.

### What this is NOT

- Not a replacement for the console server. The console server (task 88) is
  the production I/O path (multiplexed, named, `RESOLVE`-able).
- Not a general `write` syscall. Just a formatted-string-to-serial debug
  path.
- Not thread-safe. Same guarantee as `iprintln!`: bytes within a single
  call are contiguous, but calls from different processes may interleave.

## Files to change

| File | Change |
|---|---|
| `abi/src/lib.rs` | Add `SYS_PRINT: u64 = 31` |
| `aarch64/src/ipc.rs` | Add `pub(crate) fn sys_print(va: u64, len: u64) -> u64` |
| `aarch64/src/trap.rs` | Add dispatch case for `SYS_PRINT` |
| `aarch64/src/trace.rs` | Add `SYS_PRINT` to the systrace match |
| `std/src/sys.rs` | Add `SYS_PRINT` constant (re-export from `r9x_abi`) |
| `std/src/print.rs` | **New**: `print_str`, `print_fmt`, `BufWriter`, `println!` macro |
| `std/src/lib.rs` | Add `pub mod print;` |

No changes to `port/` (not built for the host), no changes to riscv64/x86_64
(no process stack yet — the syscall dispatch is aarch64-only for now).

## Acceptance

- `cargo xtask ci` green (all arches, warning-free)
- A test image (or the existing `system` image) calls
  `r9x_std::println!("hello {:#x}", 0xdeadbeef)` and the output appears on
  the QEMU serial console
- The mailbox server can use `r9x_std::println!` to report the firmware's
  ALLOCATE response (unblocks task 87's investigation)

## Dependencies

- None (new syscall, new std module)
- Unblocks: task 87 (MMIO translation fault investigation), task 88 (console
  server debugging)

## Notes

- `copy_from_user` already exists in `ipc.rs` (line 224) — reads from a user
  VA via the process's TTBR0 (still installed during the SVC). No new
  mechanism needed.
- `iputb` in `devcons.rs` is the byte-level PL011 write (the `iprintln!`
  macro calls it). `SYS_PRINT` calls it directly.
- Once the console server (task 88) is the primary I/O path, `SYS_PRINT`
  remains as a low-level debug facility (like `dmesg` vs `write(1, ...)`).
