---
name: os-build
description: Implementation skill for r9 — one author, review lenses as gates. Takes a task file, a plan doc, or a described change; implements the minimal scoped change; runs the cargo xtask gates across all three architectures; then routes the working diff to the relevant lenses and loops until findings are dry. Use when the user asks for /os-build, to implement a task, or to build from a plan.
---

# os-build — author, gate, routed lenses, loop

Implementation skill for r9 (`$REPO`). One author — the main loop, not a cast of reviewer agents; lens judgment enters as gates, never as a code-writing voice. Findings shape the diff; the diff's style stays the repo's.

## Prerequisites

- The r9 repo is at `$REPO` (resolve from your current working directory or ask the user); these skills live at `$REPO/.agents/skills/`
- Shared reference corpus — **`$REFS` = `$REPO/.agents/skills/references/`** — holds `review-protocol.md`, `lenses/*.md`, `design-questions.md`, `plan-template.md`, `amiga-inspiration.md`; the index is `$REFS/README.md`. Every `$REFS/...` path below is literal: expand `$REPO` and read the file.
- Knowledge base — **`$KB` = `$REPO/docs/`** — what is true about the project and its hardware; the index is `$KB/README.md`. Grep it before searching the web or re-deriving hardware behaviour. Pages declare `covers:`, `sources:` and `verified:`.
- Work in flight — **`$TASKS` = `$REPO/tasks/`** — one file per task, `$TASKS/done/` for finished ones, `$TASKS/plans/` for design docs, `$TASKS/todo.md` as the prioritised index; conventions and front matter in `$TASKS/README.md`
- The repo uses `cargo xtask` for all build/test/clippy gates
- The repo targets aarch64, x86-64, and riscv64

Lenses are distilled from systems, publications and principles, never from people. Findings cite the rule and its source, never a name.

## 1. Scope

Input: a `$TASKS/*.md` file path, a plan doc from `$TASKS/plans/`, or a described change. No input → list open items from `$TASKS/todo.md` and ask which one. Read the task/plan and everything it links; restate the scope and the done-when criterion before writing code. If HEAD is `main`, create a branch first.

Read `$KB/README.md` and any KB page whose `covers:` names the code in scope before writing code — `$KB/lessons.md` in particular, which exists to stop repeat debugging.

If the input is a described change with no plan behind it, run the relevant sections of `$REFS/design-questions.md` before writing code — at minimum the hardware-assumptions section. A change big enough to need the whole checklist is a change that wants `/os-plan` first; say so rather than designing inside the build loop.

Honor the task's boundary: this skill implements the minimal scoped change. Adjacent problems discovered mid-build are filed as new `$TASKS/` entries (problem, evidence, fix direction, done-when, origin), not fixed by cascade.

## 2. Implement

Write the change in the repo's existing style and conventions (check neighboring code and git history when unsure). New invariants get why-comments; new `unsafe` gets `// SAFETY:`; hardware constants cite document and section.

## 3. Hard gates (deterministic, before any lens)

From the repo's AGENTS.md, all must pass warning-free:
- `cargo xtask clippy --arch aarch64`
- `cargo xtask clippy --arch riscv64`
- `cargo xtask clippy --arch x86-64`
- `cargo xtask dist`
- `cargo xtask test`

Fix failures before spending tokens on review. Report gate output honestly.

## 4. Routed lenses on the working diff

A full six-lens review per iteration is too expensive; route by what changed. Read `$REFS/review-protocol.md` once for the output contract and verification rules, then read the diff and pick every matching row below — cap at 3 lenses, preferring the more specific rows. If the diff spans 3+ subsystems or the user asks for thoroughness, run the full panel via `/os-review` instead.

| Change touches | Lenses |
|---|---|
| MMIO, sysregs, barriers, `unsafe`, trap/vector code, DT parsing, drivers (`gic.rs`, `trap.rs`/`trap.S`, `timer.rs`, `reg/`, `mailbox.rs`, `uart*`, `vm.rs`) | hardware-truth + microkernel-and-firmware |
| Locking, interrupt-context paths, hot paths, allocator | kernel-taste + microkernel-and-firmware |
| Interrupt routing, IPC channels, display/GPU paths, boot sequencing | hardware-truth + microkernel-and-firmware + `$REFS/amiga-inspiration.md` |
| New/changed `pub` API, new module, new trait | simplicity-and-interfaces + whole-system-design |
| `port/` or cross-arch interfaces | whole-system-design + microkernel-and-firmware |
| Refactor/cleanup with no behavior change | clarity-and-composition + kernel-taste |
| Anything else | kernel-taste + clarity-and-composition |

For each chosen lens: read `$REFS/lenses/<lens>.md`, read the working diff and the surrounding code, apply the lens's rules, and emit findings in the protocol's output contract.

## 5. Verify, fix, loop

Verify as `$REFS/review-protocol.md` describes: read every cited line before believing a finding; drop pastiche and misreads. Then:
- **blocker / should-fix, in scope** → fix now, rerun step 3 gates, rerun the routed lenses on the changed portion.
- **out of scope** → file a `$TASKS/` entry + `$TASKS/todo.md` link; do not cascade.
- **needs the user's decision** (a genuine design fork) → write the question as a `$TASKS/` file, stop, and surface it.

Loop until a pass produces no in-scope blockers or should-fixes. Nits: fix if trivial and in-scope, otherwise file them.

Before reporting, feed the KB — this is where it gets written, not in a separate documentation pass:
- **A durable fact was discovered** (hardware behaviour, a spec quirk, a measurement) → a section in the relevant `$KB/` page, or a new page from the header format in `$KB/README.md`.
- **Something cost real debugging time** → an entry in `$KB/lessons.md`, with the code that witnesses it.
- **The diff changed code a page's `covers:` list names** → re-read that page, fix what the diff falsified, and update its `verified:` to the new commit and date.
Cite sources for every claim added. Add nothing that the code already says plainly.

## 6. Report

What changed and why (tied to the task's done-when), gate results (actual output, not "passed" hand-waves), lens findings fixed vs. filed, files touched. Do not commit or push unless asked. If the task's done-when is met, say so plainly; if not, say exactly what remains.
