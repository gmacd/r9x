---
status: stub — not yet read
---

# seL4: Formal Verification of an OS Kernel

```
citation: "seL4: Formal Verification of an OS Kernel", SOSP 2009
doi:
local: /Volumes/Code/repos/papers/sel4-formal-verification-sosp09.pdf
informs: docs/decisions/0007-device-dumb-kernel.md, docs/decisions/0010-map-mmio-becomes-a-capability.md, docs/decisions/0002-qnx-mechanism-plan9-interface.md
verified: not yet read
```

## Why it is here

This is the stub most likely to change something.
[0010](../decisions/0010-map-mmio-becomes-a-capability.md) accepted that
`SYS_MAP_MMIO` must become a capability and left the grant mechanism
explicitly undesigned — "whoever builds it supersedes this record with the
shape that lands". seL4's capability model is the most thoroughly worked
answer to that question in the literature: capability derivation, revocation,
and untyped memory as the thing device pages are carved from.

What to extract: the capability model itself, and the cost of it — how much
machinery a derivation tree implies for a kernel that today has four fixed
static tables and no allocation
([0002](../decisions/0002-qnx-mechanism-plan9-interface.md)). r9x needs the
authorization property, not necessarily the full model, and the note's job is
to say which parts transfer at this scale.

The other half is the verification argument, and here the answer is likely
"no": full functional correctness is a cost this project cannot pay, and
pretending otherwise would be worse than not reading the paper. The **What r9x
rejects and why** section is the point of this note — a clear statement of
what is being given up, so that "we should verify the kernel" arrives as a
considered refusal rather than an open question.

## Claims worth keeping

*Empty until read.*

## What r9x takes

*Empty until read.*

## What r9x rejects and why

*Empty until read.*

## Follow-ups

*Empty until read.*
