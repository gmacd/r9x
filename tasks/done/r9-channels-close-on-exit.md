---
id: 95
status: done
commit: 4144216
---

# Task 95: Close a process's channels on exit/kill

## Status: done (4144216)

## Problem

`exit_current` (`aarch64/src/process.rs:1146-1191`) never closes the
dead process's channels — no `close` call exists anywhere in
`aarch64/src`. `port::ipc` *has* close semantics (close wakes blocked
peers, `port/src/ipc.rs:199-201`, unit-tested in ff6499a), but the
kernel never invokes them on process death.

Consequence: if a server (console, mailbox, display, nameserver) crashes
or is killed, every client blocks forever in `receive`, and senders
block forever on the full queue. The first persistent server (task 88)
makes this a live failure mode, and it bites mailbox/display/9P servers
identically.

## Design

- On `exit_current` and on the kill path, close every channel handle the
  process holds; blocked peers wake with `ERR_CLOSED`.
- `r9x_std::ipc` surfaces `ERR_CLOSED` as a distinct error so clients
  can distinguish "server gone" from a protocol error.
- This is the cheap half of the dead-server story. The full answer —
  restart — is task 98 (init supervises, the Minix 3 reincarnation-
  server shape; Zircon's analogue is peer-closed signals on channels).

## Tests

- Unit: a channel with a blocked receiver; close it; the receiver gets
  `ERR_CLOSED` (extends the existing close-semantics tests).
- Integration: an image where a client sends to a server that exits
  before replying; the client observes `ERR_CLOSED` (not a hang) and
  exits with its sentinel.

## Done when

- Process death closes its channels; both tests pass; no integration
  image hangs on a dead server.
- Full `cargo xtask ci` green.

Origin: backlog audit 2026-08-27 (user-space group — task 88's
server-death dependency).
