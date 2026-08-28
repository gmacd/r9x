---
status: accepted
---

# 0008 — IRQs become messages: `try_send`, no inheritance, no retry

- **Status**: accepted — implemented (`aarch64/src/ipc.rs`, `IRQ_ROUTES` at line 97, `NIRQS = 16` at line 59)
- **Date**: 2026-08-28 (record written from `tasks/plans/microkernel-irq-message.md`)
- **Context**: `tasks/plans/microkernel-irq-message.md`

## Decision

A user process claims an INTID and receives that interrupt as a message. The
IRQ handler's whole job is lookup, enqueue, wake:

- Delivery uses `try_send`, a variant of `send` that returns `Err(Full)`
  instead of blocking — the same fast/slow paths, only the full path differs.
- No priority inheritance on delivery: the sender is the kernel, so there is
  no client priority to inherit; a server sets its own priority.
- The routing table is a linear scan over 16 entries, not a hash table.
- The message is the INTID as opcode, with no payload.
- A lost interrupt is not retried.

## Why

The interrupt-context budget is three things — lookup, enqueue, wake — and
every clause above exists to keep it there. The kernel cannot block, so
delivery must be total and non-blocking, which the bounded pre-allocated queue
of [0004](0004-blocking-send-bounded-channels.md) makes possible without
allocating. Sixteen comparisons is cheap against an interrupt; a hash table
buys asymptotics the table size never reaches. The server already owns the
device's registers ([0007](0007-device-dumb-kernel.md)), so it can read device
state directly rather than have the kernel marshal it. And a dropped display
refresh is survivable — the next frame's interrupt arrives 16.7 ms later.

## Alternatives rejected

- **A separate `IrqMessage` type** to make the no-allocation invariant
  explicit. Lost: the code is shared with `send`; the invariant is stated in
  the doc comment instead.
- **Inherit the client's priority for urgent interrupts.** Lost: inheritance
  is a request/reply mechanism; IRQ delivery has no requesting client.
- **A hash table for routes.** Lost: fancy machinery at n = 16.
- **A device-specific payload.** Lost: the server can read the device.
- **Retry on a full queue.** Lost: it needs a per-IRQ state machine; the
  server re-reads the device instead.

## Consequences

- Interrupt latency is bounded by the three-thing budget; anything added to
  the handler must be argued against this record.
- Input devices, where a lost interrupt is *not* acceptable, will need the
  retry question re-opened — the display case is what justified "no retry".
- `NIRQS = 16` is a static table, in keeping with the kernel's four fixed
  tables ([0002](0002-qnx-mechanism-plan9-interface.md)).
