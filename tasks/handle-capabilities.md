---
id: 119
status: open
wave: 4
---

# Task 119: handles become per-process capabilities

## Status: open — wave 4.  Design: plans/architecture-review-2026-08.md

## Problem

`CHANNELS` is a global static array and `channel()`
(`aarch64/src/ipc.rs:167`) accepts any index below `NUSED` from any
process.  `Channel::owner` exists and is always 0 (`:48`).  So a handle
is a global integer, not a capability:

- Any process can `receive` on the console's or nameserver's inbound
  channel and steal its requests, or `send`/`reply` on a channel it was
  never given.
- `sys_spawn` (`aarch64/src/process.rs:890-910`) copies a child's handle
  words verbatim out of user memory with no check that the spawner holds
  them.
- Servers reply to a client-supplied handle
  (`cmd/console/src/main.rs:134`, `cmd/nameserver/src/main.rs:108`),
  compounding it.
- `close_all_for` has nothing to check ownership against (task 113).
- Channels are never reclaimed, so the 16-slot table is a boot-time
  budget rather than a runtime resource.

It also blocks the Plan 9 half of the project outright.  Per-process
namespaces (task 123) are impossible while a handle is a global integer —
that is the ordering constraint, and it is currently invisible in the
code.

## Precedents

**Zircon** is the closest fit: per-process handle tables, rights bits per
handle, handles transferred explicitly in messages and translated by the
kernel between tables.  **seL4** CSpaces are the same idea with a
capability-derivation tree.  **Plan 9** fids are per-process by
construction.  QNX is the outlier — connection ids are per-process but
coids are guessable — and is the one model not to copy here.

## Design

- `HandleTable` on `Process`: a fixed array of
  `Option<(ObjectRef, Rights)>` indexed by the process-local handle.
  Rights: send, receive, reply, transfer.
- Every handle-taking syscall goes through
  `current_process().resolve(handle, required_rights)` instead of the
  global `channel()`.
- Refcount channels **with atomics**; free the slot at zero.  Reclamation
  under concurrent lookup needs the same generation-tag treatment as task
  118's reply slots, or a stale handle races a reused index.
- Explicit handle transfer: a message carries handles in a descriptor
  array the kernel translates between sender and receiver tables.  This
  replaces stuffing raw handle integers into payload bytes — which is
  what every server does today.
- `sys_spawn` validates the child-state page against the spawner's own
  table: a spawner may grant only handles it holds.
- Rework `close_all_for` to close handles the dying process *owns*, never
  channels it merely blocked on (task 113).
- The error space must not alias the handle space — see task 126, which
  should land first.

## Tests

- Integration: a process guessing handle integers 0–15 gets
  `ERR_BAD_HANDLE` for every one it was not granted.
- Integration: the `channel_close` image extended — a client dying while
  blocked sending to the nameserver leaves the nameserver serving.
- Integration: a spawner cannot grant a handle it does not hold.
- Host: rights checks, refcount transitions, generation reuse.

## Done when

- No syscall accepts a handle by global index.
- Channels are reclaimed when their last handle closes.
- Full `cargo xtask ci` green.

Origin: architecture and correctness review of `f76d96a`, 2026-08-28.
