---
id: 100
status: done
commit: 4144216
---

# Task 100: SYSEXIT reports the svc number as the exit status, not the exit code

## Status: done (4144216)

## Problem

`r9x_std::process::exit(code)` issues `sys(SYSEXIT, code, 0, 0, 0, 0)`,
which puts the **syscall number in `x8`** and the **exit code in `x0`**
(the aarch64 trap convention, `std/src/sys.rs`). But the kernel's SYSEXIT
arm passes the *svc number* as the status:

```rust
// aarch64/src/trap.rs, the svc dispatch
// Exit and an unimplemented number both end the
// process, with the svc number as status.
_ => process::exit_current(syscall),   // `syscall = frame.x8`
```

`frame.x8` is `SYSEXIT = 0`, so **every clean exit is reported as status 0**,
regardless of the code the process actually passed. `process::status(id)`
therefore always returns `0` for a process that exited via `SYSEXIT` — the
distinction between `exit(1)`, `exit(2)`, and a panic (which also exits 0)
is lost. The kernel's own print reads
`process {id} exited, status {status}` (process.rs:1170) and shows `0` for
all of them.

This was invisible until task 87: chasing the display server's `exit(2)`
produced `process 2 exited, status 0`, which sent the diagnosis down the
wrong process (the mailbox, not the display — see the SYSEXIT/panic
ambiguity). The `println!` reasons added to the exit branches (task 87's
convention) are what made the real reason visible; this task makes the
*code* itself honest.

## Fix

Give `SYSEXIT` its own arm and pass the exit code (`x0`), keeping the
unimplemented-number arm on the svc number:

```rust
process::SYSEXIT => process::exit_current(frame.x0),
_ => process::exit_current(syscall),
```

Check the riscv64 / x86-64 equivalents (`trap.rs` per arch) for the same
conflation; they are `cfg`'d out of the process stack today but should not
re-introduce it when the other arches gain the process/IPC path.

## Verification

- A test image (or the `procctl` image) that spawns a child calling
  `exit(2)` observes `process::status(child) == 2`, and a child that
  panics is still distinguishable (or, at minimum, the two non-zero codes
  are distinguishable from each other and from `0`).
- Full `cargo xtask ci` green.

## Related

- Discovered during task 87 (mailbox MMIO). Filed separately: it is a
  kernel trap-handler bug, not part of the mailbox server fix.
- The silent `#[panic_handler] { exit(0) }` (`std/src/rt.rs`) means a panic
  and a clean `exit(0)` are *still* indistinguishable by status alone; a
  dedicated non-zero panic status (or a fault-style `FAULT_STATUS` for
  panics) would close that gap too. Noted here, not required by this fix.
