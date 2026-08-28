---
status: stub — not yet read
---

# Priority Inheritance Protocols

```
citation: "Priority Inheritance Protocols: An Approach to Real-Time Synchronization", IEEE Transactions on Computers 39(9), 1990
doi:
local: /Volumes/Code/repos/papers/priority-inheritance-protocols-1990.pdf
informs: docs/decisions/0003-priority-scheduling-with-inheritance.md
verified: not yet read
```

## Why it is here

[0003](../decisions/0003-priority-scheduling-with-inheritance.md) asserts that
round-robin cannot bound priority inversion and that inheritance is what makes
the IPC path's latency claim honest. It asserts this without a citation: the
reasoning is sound but the *bound* is the paper's, not ours.

This paper is the source for the protocol r9x implements (`port/src/ipc.rs`
boosts a receiver to a blocked sender's priority for the duration of the
request). What it should give us: the actual blocking bound, the conditions
under which it holds, and the failure modes — chained inheritance and deadlock
among them — that a naive implementation misses. Whether r9x's single-boost
form satisfies those conditions is the open question, and it is exactly the
sort of claim the `microkernel-and-firmware` lens should be checking against a
document rather than against intuition.

Also worth extracting: the distinction between inheritance and priority
ceiling. r9x chose inheritance; the record does not say why not ceiling, which
is a gap this note should close or hand to a new decision record.

## Claims worth keeping

*Empty until read.*

## What r9x takes

*Empty until read.*

## What r9x rejects and why

*Empty until read.*

## Follow-ups

*Empty until read.*
