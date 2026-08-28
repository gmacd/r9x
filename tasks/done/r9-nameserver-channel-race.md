---
status: done
---

# Task: Nameserver shared-outbound channel race

## Status: done (channel race fixed; Mailbox MMIO on QEMU is a separate issue)

**Channel race fix committed.** Per-client reply channels implemented:
- Client creates a reply channel, includes it in the request
- Nameserver replies on the client's channel, not its own outbound
- All four clients updated (console, mailbox, display, namespace test)
- MAIR Attr1 fixed: `0x00` (Normal NC) → `0x04` (Device nGnRnE)
- QEMU machine: `virt` → `raspi4b` (the kernel targets BCM283x)

**Remaining:** the display test still fails on QEMU because the mailbox
server's MMIO access to `0xFE000000` faults. QEMU's `raspi4b` machine
may not expose the Mailbox to user space (or the page table mapping for
Device memory is incomplete). This is a separate issue from the channel
race. The channel routing is now correct (confirmed by systrace).

## Problem

The nameserver uses a single outbound channel (`ns_out`) for all replies.
Multiple clients (mailbox, display, future servers) call `receive(ns_out)`
concurrently. The channel has only one `recv_waiter` slot — a second
`receive` overwrites the first, so the wrong client gets the next message.

**Observed:** the display server received the mailbox's BIND ack (0 bytes)
instead of its own RESOLVE reply (8 bytes). The display then read
`mbox_in = 0` from the zeroed buffer and sent the configure request to
channel 0 (the nameserver's inbound) instead of channel 2 (the mailbox's
inbound).

## Root cause

`port/src/ipc.rs` `Channel::inner.recv_waiter` is a single `Option<ProcId>`.
When two processes call `receive` on the same channel, the second overwrites
the first's waiter slot. The kernel's `send` fast path wakes only the current
`recv_waiter`, so the first process is silently dropped.

This is a **design limitation**, not a code bug: the shared-outbound model
assumes one receiver per channel, but the nameserver has many clients.

## Fix options

### Option 1: Per-client reply channels (Plan 9 / QNX style) — **preferred**

The client creates its own channel pair and includes the reply handle in the
request payload. The nameserver sends the reply on the client's channel, not
its own outbound.

```
Request:  [verb:2][name:..][reply_chan:4]
Reply:    sent on reply_chan, not ns_out
```

- Each client has its own inbound channel → no sharing, no race.
- The nameserver's outbound channel is eliminated (it only has an inbound).
- Matches Plan 9's `IO_RECV` pattern: the server writes to the client's pipe.
- The `create_pair` call moves from the server to the client.

**Changes:**
- `cmd/nameserver/src/main.rs`: parse `reply_chan` from the request; reply
  on `reply_chan` instead of `pair.out_h`.
- `cmd/mailbox/src/main.rs`, `cmd/display/src/main.rs` (and any future
  client): create a local channel, include it in the request, receive on
  the local channel.
- The `rt::handle_at(2)` / `handle_at(3)` mechanism for passing the
  nameserver's handles becomes unnecessary for clients that create their
  own reply channel.

### Option 2: Serialize access (timing hack)

The init process ensures only one client talks to the nameserver at a time
(e.g. spawn mailbox, wait for it to register, then spawn display).

- No protocol change needed.
- Fragile: any new server that resolves a name concurrently reintroduces
  the race.
- Reject: papering over a design flaw.

### Option 3: Tag-filtered receive

`receive` takes an expected tag; the channel only delivers messages with
that tag. Non-matching messages stay in the queue.

- Requires a per-receiver tag filter in the channel (more state).
- The queue can fill with unmatched messages (memory pressure).
- More complex than Option 1.

## Decision

**Option 1.** It is the Plan 9 shape, eliminates the shared channel entirely,
and scales to any number of concurrent clients.

## Files to change

- `cmd/nameserver/src/main.rs` — parse reply_chan, reply on it
- `cmd/nameserver/src/bind_table.rs` — no change (the table is fine)
- `cmd/mailbox/src/main.rs` — create local reply channel, include in BIND
- `cmd/display/src/main.rs` — create local reply channel, include in RESOLVE
- `std/src/ipc.rs` — no change (the API is already general enough)
- `port/src/ipc.rs` — no change (the channel model is correct; the bug is
  in how the nameserver uses it)

## Tests

- The `namespace` integration image already tests BIND/RESOLVE with a single
  client. Extend it to test **concurrent** clients (two processes resolving
  at the same time).
- The `display` integration image exercises the race in practice (mailbox
  BIND + display RESOLVE). It should pass after the fix.

## Related

- Discovered via systrace (task 83) on the display image.
- The mailbox ALLOCATE "garbage value" bug was this race, not a QEMU bug.
