# Tasks

Work in flight. One file per task, `done/` for finished ones, `plans/` for the
design docs that produce them, and [`todo.md`](todo.md) as the prioritised
index and audit log.

This is deliberately *not* the knowledge base. `docs/` records what is true
about the project; this directory records what we are doing about it. A task
that turns out to hold a durable fact should push that fact into `docs/` and
link to it, rather than becoming the place people look things up.

## Front matter

Each task carries machine-readable fields; only what is known is present.

```markdown
---
id: 113                       # the task number, where one was assigned
status: open | parked | done
wave: 2                       # landing order, from the plan that filed it
depends-on: 119, 122
commit: d773a37               # done tasks: what landed it
issue: 47                     # optional: a GitHub issue, when one exists
---
```

The prose `## Status:` line stays, and keeps the nuance front matter cannot
hold ("parked (spun off task 87; unpark trigger below)"). The two must agree —
that is one of the things `cargo xtask tasks --check` is for (task 129).

Older tasks are named by slug rather than number and have no `id`; the
filename is their identity. Backfill an `id:` onto a slug-named done task
only when a live task file cites its number — that is what `known_ids`
resolves against — and take the number from `done.md`; never invent one.

## The list in todo.md

The open-task list in [`todo.md`](todo.md) sits between two markers:
`<!-- xtask:tasks begin -->` and `<!-- xtask:tasks end -->`.
`cargo xtask tasks --fix` reconciles only that region — dropping entries
whose task is done or gone and appending bare entries for open tasks that
are unlisted; the narrative, section headers and per-entry notes are
hand-written, and `--fix` never writes outside the markers. `--check`
reports a `todo.md` that has lost its markers, since its list can no
longer be reconciled.

## Body

The established shape, all of it optional except the first and last:

- **Problem** — what is wrong, with evidence: `file.rs:line`, a failing
  command, a spec section.
- **Precedents** — what QNX, Plan 9, Linux, seL4 or Zircon do here.
- **Design** — the fix direction, short term and proper.
- **Tests** — what proves it, named against the integration images.
- **Done when** — the checkable exit criterion.
- **Origin** — the review, plan, or session that filed it.

## GitHub

Issues are a projection, not the store. Open one when something needs an
outside audience — a contributor could pick it up, or someone reported it —
and record its number in the task's `issue:` field. The file stays
authoritative: it survives GitHub being down, is greppable offline, and is
reviewed in the same diff as the code that closes it.
