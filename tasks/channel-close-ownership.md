---
id: 113
status: open
wave: 2
---

# Task 113: a dying client permanently closes the server's channel

## Status: open — wave 2 (subsumed by task 119's ownership model)

## Problem

`close_all_for` (`aarch64/src/ipc.rs:187-194`) closes every channel where
the dying process is the `recv_waiter` **or** the `send_waiter`
(`Channel::is_blocked_on`, `port/src/ipc.rs:203-207`).  There is no
ownership check — `Channel::owner` is always 0 (`ipc.rs:48`), so there is
nothing to check against.

A client blocked in `send` on the nameserver's inbound channel *is* that
channel's `send_waiter`.  When it faults or exits, `close_all_for` marks
the **nameserver's own inbound channel** `closed = true`.  Channels are
never reclaimed, so the close is permanent: `receive` on it returns
`Err(Closed)` from then on, for every other client too.

One dead client therefore bricks the name service system-wide, and the
same holds for the console and mailbox servers.  Reachable whenever a
server's 8-deep queue is full and a ninth client blocks and then dies —
which is also how the console's busy-spin in task 114 is triggered.

Task 110's stale `recv_waiter` widens it further: a process that merely
*timed out* on a channel is still reported as blocked on it, so it does
not even need to be blocked to take the channel down with it.

## Precedents

- **QNX** ties channel lifetime to the owning process; a client's death
  destroys its *connection*, never the server's channel.
- **Zircon** refcounts handles: an object dies when its last handle
  closes, and a client never holds a handle to the server's receive end.
- **Plan 9** has the same asymmetry — a hung-up client closes its own fid,
  not the server's.

All three separate "who is waiting on this" from "who owns this".  r9x
currently has only the former.

## Design

- Short term: `close_all_for` closes only channels the dying process
  *created*.  That needs the `owner` field to be real, which is a small
  piece of task 119 that can land early.
- Proper: task 119's per-process handle tables with refcounts —
  a channel dies when its last handle goes, and a blocked waiter is not
  a handle.
- Either way, a client's death must wake the server's blocked peers with
  an error rather than closing the endpoint they are waiting on.

## Tests

- Integration: extend the existing `channel_close` image — a client dies
  while blocked sending to the nameserver, and the nameserver still
  serves a subsequent request from a second client.  Fails today.

## Done when

- A dying process cannot close a channel it does not own.
- The `channel_close` image asserts the nameserver survives a client
  death.
- Full `cargo xtask ci` green.

Origin: architecture and correctness review of `f76d96a`, 2026-08-28.
