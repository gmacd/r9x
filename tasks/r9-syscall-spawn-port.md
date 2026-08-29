---
id: 74b
status: open
---

# Task 74b: SYS_SPAWN (and the stack under it) for riscv64 / x86-64

## Status: open (parked behind the aarch64 track)

The aarch64 half is done (`done/r9-syscall-spawn.md`, commits
4319a84/c220ac2); this file scopes the port, which todo.md previously
tracked as a dangling link to the moved file.

## Problem

`SYS_SPAWN` — and the machinery it stands on — is aarch64-only. The
other arches lack the full process/IPC stack: process table and context
switch, trap/syscall entry with the shared frame layout, per-process
address spaces, the ELF-load path, and the syscall surface itself
(`SYS_SEND`/`RECEIVE`/`REPLY`, heap, clock, proc-control). This is not a
mechanical port: riscv64's entry path is boot-only `l.S` today, and
x86-64's `Mach`/`vsvm` layer has its own context model.

## Shape (per arch)

1. Trap entry with a saved frame equivalent to aarch64's `TrapFrame`
   (gate 46's frame-offset prelude should be extended rather than
   re-hand-maintained — its "Not in scope" defers the other arches to
   exactly this moment).
2. Process table + context switch + user/kernel address-space split.
3. The syscall dispatch and the allowed-syscall set.
4. `spawn_elf` from the shared `port::elf` loader + image registry.
5. The integration images (`aspace`, `two_process`, namespace) ported as
   the acceptance gates.

Related deferrals that trigger with this task:
- task 80 (timer table → `port/`) — `SYS_RECEIVE_AT`/`SYS_CLOCK` on a
  second arch need the shared table;
- task 90's riscv64/x86-64 half (the backtrace walk is per-arch frame
  layout).

## Done when

- A second architecture boots to init spawning a child via `SYS_SPAWN`,
  with the namespace round-trip image passing on it.
- Full `cargo xtask ci` green (the ported images run in that arch's
  QEMU job).

Origin: split from the completed aarch64 task 74/74a (2026-08-27 audit —
todo.md's 74b entry pointed at the moved done file).
