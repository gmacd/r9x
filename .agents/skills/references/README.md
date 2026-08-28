# Shared reference corpus

Used by all three skills in this tree — `os-plan`, `os-build` and
`os-review`. Paths below are relative to this directory; from the repo root
they are `.agents/skills/references/…` (equivalently
`.claude/skills/references/…`, which is the same tree through a symlink).

| File | What it is | Used by |
|---|---|---|
| `review-protocol.md` | How a lens pass runs: code mode vs. plan mode, output contract, verification discipline, how lens disagreements are recorded | plan, build, review |
| `lenses/<lens>.md` | One file per lens: its sources, its review rules, and what it deliberately ignores | plan, build, review |
| `design-questions.md` | Design-time interrogation checklist, grouped by lens | plan (Phase 1), build (when a task has no plan) |
| `plan-template.md` | Design-doc skeleton | plan (output), build (reading a plan) |
| `amiga-inspiration.md` | The real-time interactive graphics design goal — a system reference, not a lens | plan (Phase 2 candidate shape), build and review (interrupt, IPC, display, boot paths) |

## The six lenses

| Lens | The question it asks |
|---|---|
| `hardware-truth` | Does this address the machine that exists, or the one the textbook described? |
| `simplicity-and-interfaces` | Is this the simplest thing that works, and is its interface shaped like the rest of the system? |
| `kernel-taste` | Would a maintainer of a long-lived kernel accept this, and what does every reader pay for it? |
| `microkernel-and-firmware` | Does this belong in the kernel, is its latency bounded, could a reviewer audit it against the manual? |
| `whole-system-design` | After this change, can one person still hold the system in their head? |
| `clarity-and-composition` | Can the next reader follow this, and does it already exist as a composition of what we have? |

## House rule on attribution

Lenses are distilled from **systems, publications and principles** — never
from people. Cite the paper, the manual, the system, or the rule; a finding
reads `principle (lens, source)`. No name appears in a lens, a finding, or a
report. The point is that the rule is checkable against a document, not that
someone respected once said it.

## Not to be confused with the knowledge base

This corpus is *how to judge* — review rules distilled from systems,
publications and principles. `$REPO/docs/` is *what is true* — this project's
hardware facts, subsystem descriptions and decision records. Lenses change
rarely and are about method; KB pages change with the code and are about r9.
