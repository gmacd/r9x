---
name: os-plan
description: Design/planning skill for r9 using the six review lenses generatively. Interrogates a proposed change with design-time questions from the lenses, optionally drafts competing candidate designs by system shape (Plan 9, QNX, Oberon, Amiga), then lens-critiques the draft plan before any code is written. Produces a design doc with decision records and files implementation tasks. Use when the user asks for /os-plan, a design, or a plan for r9 work.
---

# os-plan — design before code, interrogated by the lenses

Planning skill for r9 (`$REPO`). The output is a design doc and task files — never code. Escalate phases with stakes; a small task may stop after Phase 1. If the user says "quick plan", Phase 1 only.

## Prerequisites

- The r9 repo is at `$REPO` (resolve from your current working directory or ask the user); these skills live at `$REPO/.agents/skills/`
- Shared reference corpus — **`$REFS` = `$REPO/.agents/skills/references/`** — holds `review-protocol.md`, `lenses/*.md`, `design-questions.md`, `plan-template.md`, `amiga-inspiration.md`; the index is `$REFS/README.md`. Every `$REFS/...` path below is literal: expand `$REPO` and read the file.
- Knowledge base — **`$KB` = `$REPO/docs/`** — what is true about the project and its hardware; the index is `$KB/README.md`. Grep it before searching the web or re-deriving hardware behaviour. Pages declare `covers:`, `sources:` and `verified:`.
- Work in flight — **`$TASKS` = `$REPO/tasks/`** — one file per task, `$TASKS/done/` for finished ones, `$TASKS/plans/` for design docs, `$TASKS/todo.md` as the prioritised index; conventions and front matter in `$TASKS/README.md`
- If `/Volumes/Code/repos/linux` or `/Volumes/Code/repos/plan9` exist, use them as witnesses

Lenses are distilled from systems, publications and principles, never from people. Decision records cite the lens and its source, never a name.

## Phase 0 — gather intent

Read the input: a `$TASKS/` file, a goal statement, or a problem description. Read the code it touches, then `$KB/README.md` and any KB page whose `covers:` names that code — the hardware facts and the decisions already made are prior art, and a plan that contradicts a decision record must supersede it explicitly rather than ignore it. Then prior art in the reference repos (`/Volumes/Code/repos/{plan9,linux}`). Restate the problem in two or three sentences, plus the standing constraints: warning-free across aarch64, x86-64, riscv64 (`cargo xtask` gates); minimal scoped changes; Plan 9 shape; the real-time interactive graphics goal (r9 boots to a graphical environment by default; the kernel's job is to keep the display alive at 60 Hz while user-space servers do everything else — read `$REFS/amiga-inspiration.md` for the full design questions). If the goal is ambiguous, ask now — a plan for the wrong problem is waste.

## Phase 1 — interrogation

Work through `$REFS/design-questions.md`. Every question gets an answer in the plan or an explicit "N/A because…". The hardware-assumptions section is not optional: name what the design assumes per target, even when the answer is "nothing new". For small changes, write the mini-plan now using `$REFS/plan-template.md` (sections may be collapsed) and skip to Phase 3 or straight to output.

**Never write code. Never implement. Never start editing files.** If the change is small, the mini-plan *is* the plan — produce it, then stop and ask the user before implementing. The `os-plan` skill produces plans and design docs only; implementation is `os-build`'s job.

## Phase 2 — competing candidates (large designs only)

For designs that are wide — a new subsystem, a cross-arch interface, new public API — draft competing candidates. Consider four system shapes, each with a reference to read before sketching:

1. **Plan 9 shape**: resources as file servers, namespace-first, small uniform interfaces. Read `$REFS/lenses/simplicity-and-interfaces.md`.
2. **QNX shape**: message passing, explicit ownership, determinism and bounded latency first. Read `$REFS/lenses/microkernel-and-firmware.md`.
3. **Oberon shape**: minimal, static, lean; the fewest concepts that solve the problem; explicit module boundaries. Read `$REFS/lenses/whole-system-design.md`.
4. **Amiga shape**: interrupt-driven I/O as the heartbeat, bounded message ports, hardware acceleration as the norm, boot to a graphical environment by default. Read `$REFS/amiga-inspiration.md`.

For each shape, sketch: data structures, interfaces, init order, failure policy, and — required — what it refuses to build. Then judge: score each sketch against the Phase 1 answers, pick a winner, graft the runners-up's best ideas, and record *why the losers lost* (that reasoning is the most durable part of the doc).

## Phase 3 — lens critique of the draft

Write the draft plan to a file, then review it through the six lenses in **plan mode** — read `$REFS/review-protocol.md` for what plan mode changes (judge committed decisions, added concepts and assumptions rather than lines; cite plan sections; prefer `question` where the plan is silent rather than wrong).

For each lens: read `$REFS/lenses/<lens>.md`, apply its rules to the draft plan with the existing code as context, and emit findings in the protocol's output contract.

Then verify as the protocol describes — check findings against the plan and the code, drop pastiche, dedup — and fold the survivors into the plan. Where lenses genuinely disagree (late binding vs. no-premature-abstraction is the standing example), do not average: make the decision and record it with the dissent — "the whole-system lens argued X; we chose Y because Z".

## Output

1. The design doc, per `$REFS/plan-template.md`, written to `$TASKS/plans/<kebab-name>.md`.
2. Implementation tasks filed as `$TASKS/*.md` files (the established format: problem, evidence, fix direction, done-when, origin) and linked from `$TASKS/todo.md`, sequenced if order matters.
3. A decision record in `$KB/decisions/` for each contested choice the plan settles — numbered, following `$KB/decisions/0000-template.md`, carrying the alternatives that lost and why. The plan doc holds the reasoning; the record is what a future reader finds when the question resurfaces.
4. A summary to the user: the decision, the dissents, and the task list.
5. If the change is small, also include: an explicit proposal with scope and estimated effort, followed by a question: "Shall I proceed with implementation?"

**Strict rule: `os-plan` never implements.** Do not create files for code, do not edit source files, do not run `cargo` commands that build. Only produce the design doc, task files, and the summary. After producing output, stop and let the user decide when to invoke `/os-build`.
