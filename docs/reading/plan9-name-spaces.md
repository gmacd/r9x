---
status: stub — not yet read
---

# The Use of Name Spaces in Plan 9

```
citation: "The Use of Name Spaces in Plan 9", ACM SIGOPS Operating Systems Review 27(2), 1993 (from the 5th SIGOPS European Workshop, 1992) — verify venue against the PDF
doi:
local: /Volumes/Code/repos/papers/plan9-name-spaces-1993.pdf
informs: docs/decisions/0009-nameserver-in-user-space.md, docs/decisions/0005-opaque-kernel-message.md, .agents/skills/references/lenses/simplicity-and-interfaces.md
verified: not yet read
```

## Why it is here

[0002](../decisions/0002-qnx-mechanism-plan9-interface.md) says the interface
half of r9x is Plan 9's, and
[0009](../decisions/0009-nameserver-in-user-space.md) builds the first piece of
it — a flat `name → ChannelHandle` map in a user-space server, with the tree
explicitly deferred. That record already carries a dissent saying a flat map of
absolute paths is a symbol table, not a name space.

This paper is the authority on what the difference actually amounts to: per-
process namespaces, `bind` and `mount` semantics, union directories, and what
the namespace buys that a global name table does not. The open question it
should settle is whether r9x's deferral is a staging decision or a design
error — whether the tree can be added to the existing map later, as 0009
claims, or whether per-process namespaces have to be in the mechanism from the
start.

Second thing to extract: how much of the namespace lives in the kernel in Plan
9 versus in servers. r9x split it kernel-owns-handles, user-owns-names; the
paper is the check on whether that split holds up.

## Claims worth keeping

*Empty until read.*

## What r9x takes

*Empty until read.*

## What r9x rejects and why

*Empty until read.*

## Follow-ups

*Empty until read.*
