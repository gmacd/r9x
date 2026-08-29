---
id: 131
status: open
---

# Task 131: rephrase the task-tree findings the gate still reports

## Problem

`cargo xtask tasks --check` (task 129) reports three unresolved
references. They are genuine, but each is an editorial judgment, not a
tree defect the gate can fix:

- **the plan's item 3, cited as a task** — `mark-range-check-end-flag.md:27`
  names the third item of `tasks/plans/range-by-value.md` with the task
  phrasing; it is a plan item, not a task id. Rephrase (e.g. "item 3 of
  the plan").
- **the arc's first member, cited as a task** — `r9x-std-servers.md:70,72`
  cite the arc's first member (the `r9x_std` seed) with the task
  phrasing; the id 1 has no file. Same rephrasing.

## Note

This file's first draft also carried four id-70 findings, on the premise
that task 70's file was lost. That was a misdiagnosis: the file is in the
tree — `done/stage6-init-bringup.md`, recorded in `done.md` as "task 70,
e6d9145" — it simply had no `id:` front matter. That has been added, and
the four findings resolved. The file is stage 6's bringup, not a lost
console task.

## Done when

`cargo xtask tasks --check` reports no unresolved references (the two
above rephrased).

Origin: task 129's first real run, 2026-08-28; the id-70 premise
corrected in the 2026-08-28 review of task 129's diff.
