---
id: 92
status: done
commit: b859dc8
---

# Task 92: Fault-checked user-memory access

## Status: done (b859dc8)

## Problem

`copy_from_user` (`aarch64/src/ipc.rs:224-234`) is a raw
`copy_nonoverlapping` with no fault handling — its own comment says "A
faulting pointer is a user bug the test images do not produce." That was
tolerable for trusted test images; it is not now that `SYS_PRINT`
(fd7e96c) takes an arbitrary user pointer: **any process can panic the
kernel** by passing a bogus `x0`. IPC send/receive buffers have the same
exposure, and task 90's stack walker would take a recursive fault inside
the fault handler on a garbage frame-pointer chain.

## Design

A `read_user` / `read_user_word` helper that is safe against unmapped or
kernel-only VAs:

- **Software-walk the process's TTBR0 tables** and check the leaf is
  valid, readable, and user-accessible before touching the VA; return
  `None`/`Err` otherwise. No nested-exception machinery needed — the
  walk *is* the check. (The alternative — a fixup-table user-copy like
  Linux's `uaccess` — is more code for no gain at r9x's scale.)
- Bounds: reject VAs outside the user half up front (cheap mask check
  before the walk).
- Route `sys_print`, the IPC copy paths, and (later) task 90's walker
  through it.

Precedents: seL4 sidesteps the problem entirely — `seL4_DebugPutChar`
takes a *register*, never a pointer; Zircon's `zx_debuglog_write` copies
through a fault-checked user-copy path. Plan 9's `okaddr`/`validaddr`
checks the segment map before kernel touches user memory — the same
shape as the walk here.

Optional hardening in the same area (note, decide in review): gate
`SYS_PRINT` behind a cargo feature for eventual production builds, per
seL4's `CONFIG_PRINTING`.

## Done when

- A test image passes a bad pointer to `SYS_PRINT` and to `SYS_SEND`;
  the process gets an error (or is killed), the kernel survives, and the
  image asserts it.
- All user-pointer dereferences in the syscall layer go through the
  helper (grep for `copy_nonoverlapping` in ipc.rs finds only the
  helper's own internals).
- Full `cargo xtask ci` green.

Origin: backlog audit 2026-08-27 (user-space group; consumers are
SYS_PRINT hardening and task 90).
