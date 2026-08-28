---
status: accepted
---

# 0004 — Channels are bounded and `send` blocks; there is no drop mode

- **Status**: accepted — implemented (`port/src/ipc.rs`; `MSG_MAX` at `abi/src/lib.rs:41`)
- **Date**: 2026-08-28 (record written from `tasks/plans/microkernel-substrate.md`)
- **Context**: `tasks/plans/microkernel-substrate.md`

## Decision

A channel is a bounded queue of pre-allocated message slots (`MSG_MAX = 256`
payload bytes, queue depth 8). `send` to a full channel blocks the sender.
There is no drop mode, no non-blocking flag, and no per-channel policy knob.

## Why

The primitive stays total: one behaviour, always. A flag would make one
function do two things, and a per-channel knob is a design decision refused
and exported as permanent interface. Dropping silently hides a stuck server —
the failure that is hardest to diagnose from the outside. Bounded and
pre-allocated is also what lets the IRQ path send without allocating.

## Alternatives rejected

- **Drop-on-full via a non-blocking flag.** Lost: two behaviours behind one
  entry point, and silent loss of the signal that a server has stalled.
- **A per-channel policy knob.** Lost: a tunable nobody could name a varying
  caller for.

**Dissent** (whole-system lens): a future server may genuinely want
drop-on-full. It gets it in user space by retrying with a short block
timeout over the primitive; the primitive itself stays total.

## Consequences

- The kernel-side IRQ path needs a non-blocking variant, since the kernel
  cannot block — that is `try_send`, a variant of `send` sharing its paths,
  not a second mechanism (see [0008](0008-irq-to-message-routing.md)).
- Queue depth and payload bound are ABI facts in `r9x_abi`, shared by kernel
  and servers (see [0014](0014-curated-r9x-std.md)).
- A stuck server manifests as a blocked sender, which is observable, rather
  than as lost messages, which are not.
