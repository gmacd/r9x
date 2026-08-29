---
id: 129
status: open
depends-on: 128
---

# Task 129: `cargo xtask tasks --check` — validate the task tree and reconcile the index

## Problem

`tasks/` now carries machine-readable front matter (`id`, `status`, `wave`,
`depends-on`, `commit`, `issue`), added mechanically when the tree moved into
the repo. Nothing keeps it honest, and four things can drift silently:

- **Front matter versus prose.** The `## Status:` line is authoritative for
  humans and carries nuance the fields cannot; `status:` is what tooling reads.
  They can disagree, and today nobody would notice.
- **Cross-references.** Tasks name each other in prose ("subsumed by task
  119", "Depends on 118, 121", "successor to task 99"). A referenced id may not
  exist, or may already be in `done/`.
- **Duplicate or missing ids.** Numbers are assigned by hand in `todo.md`.
- **The index.** `todo.md` lists open tasks by hand; a task can land in `done/`
  and stay listed as open.

## Design

An xtask subcommand with two modes.

`--check` reports:

- front matter that fails to parse, or an unknown key;
- `status:` disagreeing with the prose `## Status:` line;
- a file in `done/` whose status is not `done`, or a top-level file whose
  status is `done` (it belongs in `done/`);
- a `depends-on` or in-prose task reference naming an id that does not exist;
- duplicate ids;
- a `done` task with no `commit:` where one is recoverable from its prose;
- open tasks missing from `todo.md`'s list, and listed tasks that are done.

`--fix` reconciles the marked list region of `todo.md` in place — dropping
entry blocks whose task is done or gone, and appending bare entries for open
tasks that are unlisted (a human places each into a section). It does not
regenerate the region: the sections are hand-written and thematic rather
than grouped by wave, and the per-entry notes are the most valuable part of
the file. `todo.md`'s narrative header is likewise hand-written and must be
preserved verbatim: it is a dated audit log of review passes. Mark the
generated region with explicit begin/end comments and never write outside it.

Same reporting discipline as task 128: report, do not block; non-zero exit
only for malformed front matter.

The code for the task should live in its own file - xtask/main.rs is getting huge.

## Tests

- Unit: front-matter parse and round-trip; the status/prose comparison; the
  reference extractor against the real phrasings in the tree ("Depends on 118,
  121", "subsumed by task 119", "successor to the done task 70").
- Unit: index rendering preserves the narrative header byte for byte.
- Integration: run over the real tree; ids unique, references resolve.

## Done when

- `cargo xtask tasks --check` runs clean over `tasks/`, or prints a list of
  genuine inconsistencies to fix.
- `--fix` reconciles `todo.md`'s list region without touching its narrative.
- CI runs `--check` in the `checks` job; `cargo xtask ci` runs it locally.

Origin: moving `tasks/` into the repo, 2026-08-28. The front matter was
derived mechanically and has never been validated.
