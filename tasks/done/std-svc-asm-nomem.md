---
id: 103
status: done
commit: 3c5f3b0
---

# Task 103: the `svc` inline asm declares `nomem`, which is false

## Status: done (3c5f3b0)

## Problem

`std/src/sys.rs:50` passes `options(nomem, nostack)` to the syscall
`asm!`.  `nomem` is a promise to the compiler that the assembly neither
reads nor writes memory.  Every buffer-carrying syscall breaks it:
`SYCSEND` has the kernel read the payload through `x1`, and
`SYCRECEIVE` / `SYS_RECEIVE_AT` have the kernel *write* into the caller's
buffer through it.

LLVM is entitled to act on the promise: sink the stores that filled a
send buffer past the `svc`, or reuse a pre-syscall cached load of a
receive buffer afterwards.  It happens not to today at the current
opt-level; it is UB either way, and the failure it produces when it does
bite (a message that is half the previous message) is close to
undebuggable.

The SAFETY comment above it states the reasoning that is wrong —
"`nomem` and `nostack` hold because the syscall's memory and state
effects are the kernel's, not direct reads or writes by this code".  The
kernel's writes through a pointer this code handed it *are* memory
effects of this asm block.

## Design

- Drop `nomem`.  Keep `nostack` (true — the syscall uses no red zone).
- Correct the SAFETY comment to say why: the kernel dereferences the
  pointers passed in the argument registers, so the block must be assumed
  to touch memory.
- Audit the other `asm!` blocks in the tree for the same claim while
  here — `aarch64/src/ipc.rs:422`'s TLBI block is `nomem, nostack` and
  that one is correct (it names no pointer).

## Tests

- Host: none possible (target-only).
- Integration: existing IPC images, built at `--release` once task 102
  lands, are the observation window.

## Done when

- No syscall `asm!` block claims `nomem`.
- The SAFETY comments state the real justification.
- Full `cargo xtask ci` green.

Origin: architecture and correctness review of `f76d96a`, 2026-08-28.
