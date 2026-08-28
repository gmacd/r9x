---
status: stub — not yet read
---

# An Architectural Overview of QNX

```
citation: "An Architectural Overview of QNX", USENIX Workshop on Micro-kernels and Other Kernel Architectures, 1992
doi:
local: /Volumes/Code/repos/papers/qnx-architectural-overview-1992.pdf
informs: docs/decisions/0002-qnx-mechanism-plan9-interface.md, docs/decisions/0008-irq-to-message-routing.md, .agents/skills/references/lenses/microkernel-and-firmware.md
verified: not yet read
```

## Why it is here

[0002](../decisions/0002-qnx-mechanism-plan9-interface.md) makes QNX the
mechanism half of the whole system, and the `microkernel-and-firmware` lens
takes its rules from QNX doctrine — but both source that doctrine to vendor
documentation and to the system's observable behaviour. There is very little
peer-reviewed QNX literature, and this paper is the main exception.

That scarcity is itself a finding worth recording: the most load-bearing
decision in the project rests largely on a manual. This note should either
firm that foundation up or state plainly that it cannot be firmed up, so the
weakness is visible rather than implied.

What to look for specifically: the send/receive/reply state machine as
originally described, how interrupt handlers are bounded and what they are
permitted to do (against
[0008](../decisions/0008-irq-to-message-routing.md)'s three-thing budget), and
the resource-manager model that
[0007](../decisions/0007-device-dumb-kernel.md) claims as its shape.

## Claims worth keeping

*Empty until read.*

## What r9x takes

*Empty until read.*

## What r9x rejects and why

*Empty until read.*

## Follow-ups

*Empty until read.*
