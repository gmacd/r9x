---
id: 132
status: open
---

# Task 132: the check classes the review found that task 129's spec omits

## Problem

The review of task 129's diff (`cargo xtask tasks`) found three check
classes the gate could have, that its spec does not carry. None fires
today — they are coverage gaps, not live defects:

- **H1 versus `id:`** — nothing compares a heading's `# Task N:` prefix
  with the front matter `id:`. The exact bug has happened once already
  (the "Task 101" collision recorded in `todo.md`, since corrected to
  task 13), and a wrong H1 number that happens to exist elsewhere
  resolves silently. The check must fire only when the H1 carries a
  `Task <n>:` form and `<n> != id` — the slug-form H1s (`# r9: ...`,
  `# stage6-...`) must not fire.
- **the same file listed under two numbers** — two `todo.md` entries
  linking the same file under different numbers pass every check (number
  collisions are checked; filename collisions are not).
- **top-level / `done/` basename collision** — tasks are keyed by
  basename, so a same-named `x.md` and `done/x.md` shadow each other in
  the index lookups (first scanned wins). No such pair exists today;
  report it, or make the keying explicit.

## Done when

The three checks exist in `xtask/src/tasks.rs`, each with a fixture test,
and the tree passes with them on.

Origin: the 2026-08-28 review of task 129's diff.
