---
status: stub — not yet read
---

# Improving IPC by Kernel Design

```
citation: "Improving IPC by Kernel Design", SOSP 1993
doi:
local: /Volumes/Code/repos/papers/improving-ipc-by-kernel-design-sosp93.pdf
informs: docs/decisions/0004-blocking-send-bounded-channels.md, docs/decisions/0008-irq-to-message-routing.md, .agents/skills/references/lenses/kernel-taste.md
verified: not yet read
```

## Why it is here

r9x's IPC decisions are argued from shape, not from numbers.
[0004](../decisions/0004-blocking-send-bounded-channels.md) chose bounded
queues with a blocking `send` because the primitive stays total;
[0008](../decisions/0008-irq-to-message-routing.md) chose a linear scan over 16
routes because "16 comparisons is cheap". Both are plausible and neither is
measured — and the `kernel-taste` lens's own rule is that a performance claim
without numbers is a finding.

This paper is where microkernel IPC stopped being argued and started being
measured: the design decisions that made L4's IPC an order of magnitude faster
than its predecessors, and the method by which they were justified. The value
here is as much the method as the result — what to measure on an IPC path, and
which costs turn out to dominate.

Concretely, it should tell us whether r9x's message copy through a bounded
queue is the right shape at all, or whether register-based transfer for small
messages is the obvious thing we have not done. `MSG_MAX = 256` bytes
(`abi/src/lib.rs:41`) was chosen as a bound, not from a distribution of actual
message sizes.

## Claims worth keeping

*Empty until read.*

## What r9x takes

*Empty until read.*

## What r9x rejects and why

*Empty until read.*

## Follow-ups

*Empty until read.*
