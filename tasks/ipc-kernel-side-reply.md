---
id: 118
status: open
wave: 4
---

# Task 118: make reply a kernel concept (receive ids)

## Status: open — wave 4.  Design: plans/architecture-review-2026-08.md

## Problem

A channel is a one-way queue, so reply is implemented *above* the kernel:
the client creates its own channel and passes the handle inside the
message payload.  Each server chose a different place to put it —
nameserver at the last four bytes (`cmd/nameserver/src/main.rs:67`),
console at the first four (`cmd/console/src/main.rs:134`), mailbox not at
all, replying on a shared outbound channel two clients race on
(`cmd/mailbox/src/main.rs:290`).  The `tag` field already exists for
correlation and now duplicates the job.

The costs compound with every server added:

- A request/reply pairing costs three channels instead of one.
  `aarch64/tests/display.rs:26-28` documents the budget as "exact:
  nameserver 2, mailbox 5, display 5, console server 3, client 1 —
  sixteen, the `NCHANNELS` limit."  There is no headroom left.
- The reply handle is client-supplied data a server replies through
  blind, which is a capability hole task 119 cannot fully close while the
  convention exists.
- It is why `close_all_for` can kill a server when a client dies (task
  113): the client's reply channel and the server's inbound channel are
  both "channels the client was blocked on".

## Precedents

**QNX Neutrino** is the direct model and the one r9x already cites:
`MsgSend` blocks the sender, `MsgReceive` returns an `rcvid` naming it,
`MsgReply` takes that `rcvid`.  The reply path is kernel state, never
payload.  **seL4** does the same with reply objects/capabilities;
**Zircon** with `zx_channel_call`.  All three converge: the kernel, which
already knows who is blocked, is the right place to remember it.

## Design

- A `Reply` table in `port/src/ipc.rs`: a fixed array of
  `(ProcId, tag, channel)` slots.  The receive id is index-plus-
  generation, so a stale or forged reply is rejected rather than
  misdelivered.
- **Concurrent from the first commit** (the multi-core ruling): slot
  allocation is a CAS or a held lock, and the generation counter is what
  makes a reply arriving from another core safe to validate.
- `send` blocks the sender after enqueue and allocates a reply slot.
  `try_send` stays non-blocking — the IRQ path has no sender to block.
- `receive`/`receive_at` return the receive id, replacing the current
  `tag` passthrough in the third result register.
- `SYCREPLY` takes a receive id, looks up the blocked sender, copies the
  payload **straight into its buffer**, and wakes it.  One copy, not two
  — which is also half of task 121.
- Lifetime: replying twice is an error (the generation check makes it
  detectable across cores); a server that exits with unreplied ids wakes
  those senders with `ERR_CLOSED`.
- Do **not** rebuild task 110's lost-wakeup window: the sender must be
  committed to blocking before its receive id is visible to a replier.
- Delete the reply-channel field from all four server protocols and their
  clients: `cmd/nameserver`, `cmd/console`, `cmd/mailbox`, `cmd/display`,
  `std/src/console.rs`.

## Tests

- Integration: the `display` image runs with its channel count at or
  below 8 (16 today).
- Integration: a new `two_clients` case — two clients issue overlapping
  requests to one server, each receives its own reply, and no per-client
  reply channel exists anywhere.
- Integration: a server exiting mid-request wakes its blocked clients
  with `ERR_CLOSED` rather than hanging them.
- Host: the reply table's generation logic — stale id rejected, reused
  slot not confused with its predecessor.

## Done when

- No server protocol carries a reply handle in its payload.
- Channel usage in the display image is at least halved.
- Full `cargo xtask ci` green.

Origin: architecture and correctness review of `f76d96a`, 2026-08-28.
This is the highest-leverage item on the list: it rewrites every server's
wire format, and the cost is linear in a server count that only grows.
