# Reading notes

One note per paper that earns standing room: something we will re-read, that
informs more than one page or decision. Most papers never need a note — a
hardware document belongs in a page's `sources:`, and prior art for a design in
flight belongs in the plan and then in the decision record it settles.

A note is *this project's reading* of a paper, not a summary of it. Summaries
are freely available; what is not available anywhere else is what r9x takes
from the paper, and what it deliberately refuses.

## The papers themselves

PDFs live in `/Volumes/Code/repos/papers/`, beside the `linux` and `plan9`
mirrors — outside this repo. Do not commit PDFs: bloat, licensing, and the
paper is not r9x's knowledge. The note is.

## Notes

| Note | Paper | Informs | Read? |
|---|---|---|---|
| [`priority-inheritance-protocols.md`](priority-inheritance-protocols.md) | Priority Inheritance Protocols, 1990 | [0003](../decisions/0003-priority-scheduling-with-inheritance.md) | not yet |
| [`qnx-architectural-overview.md`](qnx-architectural-overview.md) | An Architectural Overview of QNX, 1992 | [0002](../decisions/0002-qnx-mechanism-plan9-interface.md), [0008](../decisions/0008-irq-to-message-routing.md) | not yet |
| [`plan9-name-spaces.md`](plan9-name-spaces.md) | The Use of Name Spaces in Plan 9, 1993 | [0009](../decisions/0009-nameserver-in-user-space.md), [0005](../decisions/0005-opaque-kernel-message.md) | not yet |
| [`sel4-formal-verification.md`](sel4-formal-verification.md) | seL4: Formal Verification of an OS Kernel, SOSP 2009 | [0007](../decisions/0007-device-dumb-kernel.md), [0010](../decisions/0010-map-mmio-becomes-a-capability.md) | not yet |
| [`improving-ipc-by-kernel-design.md`](improving-ipc-by-kernel-design.md) | Improving IPC by Kernel Design, SOSP 1993 | [0004](../decisions/0004-blocking-send-bounded-channels.md), [0008](../decisions/0008-irq-to-message-routing.md) | not yet |

## Writing a note

Start from [`TEMPLATE.md`](TEMPLATE.md). The header:

```markdown
---
citation: "<Title>", <venue>, <year>
doi:
local: /Volumes/Code/repos/papers/<file>.pdf
informs: docs/decisions/0003-….md, .agents/skills/references/lenses/kernel-taste.md
verified: not yet read
---
```

- **`citation` carries no author list.** This is deliberate and consistent with
  the project's rule that sources are systems, publications and principles, not
  people — the DOI makes the paper findable without naming anyone. Do not
  "fix" this by adding names.
- **`doi`** goes in when the PDF is in hand. Venue and year come off the paper
  itself: a remembered citation is a guess, and the KB's cite-or-delete rule
  applies to its own citations first.
- **`informs`** lists the pages, decisions and lenses this note feeds. It is
  the reverse of `covers:` — a paper describes no code, so it anchors to the
  knowledge it backs. Wire both directions: whatever a note informs should
  name the note in its own `sources:`.
- **`verified`** becomes a date and what was actually read (`2026-09-04, §3–§5`).
  A note claiming the whole paper when only the intro was read is worse than
  no note.

The three body sections carry the weight:

1. **Claims worth keeping** — each with a page or section number. A claim
   cited at whole-document granularity is barely better than uncited.
2. **What r9x takes** — named against the decision or page it changes. If it
   changes nothing, say so; a paper can be worth reading and change nothing.
3. **What r9x rejects and why** — the section people skip, and the one with
   the most value in six months. A note without it is a summary.

If a paper overturns a decision, write a new decision record that supersedes
the old one. Never edit the old record into agreement, and never let a note
become the place a decision quietly changed.

## A note on stubs

A stub carries the citation and the *hypothesis* — why we expect the paper to
matter — and nothing else. A stub's citation *may* carry a remembered venue
and year — that is what makes the paper findable before the PDF is in hand —
but it remains a guess until the paper is read, and the stub's `status:` /
`verified: not yet read` lines say exactly that. The writing rule above
applies from the moment the note stops being a stub. Its claims sections stay
empty until someone reads the paper. Filling them from memory or reputation
is exactly the failure the provenance rule exists to prevent.
