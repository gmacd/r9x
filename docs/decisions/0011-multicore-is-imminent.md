---
status: accepted
---

# 0011 — Multi-core is imminent, so every race is a live defect

- **Status**: accepted — a standing ruling on how defects are triaged
- **Date**: 2026-08-28 (ruling taken during the 2026-08 architecture review)
- **Context**: `tasks/plans/architecture-review-2026-08.md`, ruling 2; task 124 (bring the secondaries up)

## Decision

Secondary cores are coming sooner rather than later. Every race the review
found is filed at its true severity as a live defect, not as an SMP-latent
deferral, and bringing the secondaries up moves ahead of the IPC rework rather
than behind it.

## Why

"Correct on one core, racy on many" is a deferral that expires on a known date
— and defects filed at understated severity do not get re-triaged when it
does. Filing at true severity now means the backlog is already sorted for the
world that is arriving, rather than needing a second pass through 100+ tasks
to find what SMP just made urgent.

This ruling is also the project's standing instruction to reviewers: judge
concurrency as if the secondaries were already up.

## Alternatives rejected

- **Keep triaging races as latent until SMP lands.** Lost: it defers the
  triage cost to the moment when the code is hardest to reason about, and it
  systematically understates the backlog.

## Consequences

- No new single-core assumption may be added without stating it as such; an
  unstated one is a defect.
- Static tables, lock ordering and interrupt-context paths all get reviewed
  under the multi-core reading — see the `kernel-taste` and
  `microkernel-and-firmware` lenses in `.agents/skills/references/lenses/`.
- Task ordering follows: secondaries first, then the IPC rework.
