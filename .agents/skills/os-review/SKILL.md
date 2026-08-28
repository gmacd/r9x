---
name: os-review
description: Panel code review for r9 through six lenses distilled from systems, publications and principles (Plan 9 and the Unix papers, the Linux kernel review tradition, QNX and coreboot/LinuxBoot doctrine, Smalltalk and Oberon design writing, hardware-realism research). Reviews a commit, a range, files, or the working diff. Use when the user asks for /os-review, a panel review, or a lens review.
---

# os-review — six-lens panel review for r9

Review-only: never edit code during this skill. The deliverable is the report.

## Prerequisites

- The r9 repo is at `$REPO` (resolve from your current working directory or ask the user); these skills live at `$REPO/.agents/skills/`
- Shared reference corpus — **`$REFS` = `$REPO/.agents/skills/references/`** — holds `review-protocol.md`, `lenses/*.md`, `design-questions.md`, `plan-template.md`, `amiga-inspiration.md`; the index is `$REFS/README.md`. Every `$REFS/...` path below is literal: expand `$REPO` and read the file.
- Knowledge base — **`$KB` = `$REPO/docs/`** — what is true about the project and its hardware; the index is `$KB/README.md`. Grep it before searching the web or re-deriving hardware behaviour. Pages declare `covers:`, `sources:` and `verified:`.
- Work in flight — **`$TASKS` = `$REPO/tasks/`** — one file per task, `$TASKS/done/` for finished ones, `$TASKS/plans/` for design docs, `$TASKS/todo.md` as the prioritised index; conventions and front matter in `$TASKS/README.md`
- If `/Volumes/Code/repos/linux` or `/Volumes/Code/repos/plan9` exist, use them as witnesses

Lenses are distilled from systems, publications and principles, never from people. Findings cite the rule and its source, never a name.

## 1. Determine scope

From the invocation arguments:
- A commit or range (e.g. `HEAD~3..`, `46a59c9`) → review exactly that.
- File paths → review those files as they stand.
- No arguments → if the working tree is dirty, review the working diff (staged + unstaged); if clean, review `HEAD~1..HEAD`.

Run `git log --oneline -3` and `git diff --stat` for the chosen scope so the report can state what was reviewed. If the scope is empty, say so and stop.

## 2. Apply the six lenses

Read `$REFS/review-protocol.md` once — it defines how a lens pass runs, the output contract every lens uses, and the verification discipline in step 3.

Then, for each lens, read `$REFS/lenses/<lens>.md` and apply its rules to the scope:

| Lens | Asks |
|---|---|
| `hardware-truth` | Does this address the machine that exists? |
| `simplicity-and-interfaces` | Is this the simplest thing that works, shaped like the rest of the system? |
| `kernel-taste` | What does every future reader pay for this? |
| `microkernel-and-firmware` | Does this belong in the kernel, is it bounded, is it auditable? |
| `whole-system-design` | Can one person still hold the system in their head? |
| `clarity-and-composition` | Can the next reader follow it, and does it already exist? |

Each lens file carries its own sources, rules, and explicit non-business. Gather the diff with `git` and read surrounding code for context — a lens judges code in place, not a hunk in isolation.

Also read `$REFS/amiga-inspiration.md` when the diff touches interrupt routing, IPC channels, display/GPU paths, or boot sequencing. It is not a lens but a design-goal reference: r9 boots to a graphical environment, and the kernel keeps the display alive at 60 Hz while user-space servers do everything else.

## 3. Check the knowledge base against the diff

Grep `$KB/` for pages whose `covers:` list names any file in the scope. For each, read the page and ask whether the diff falsifies it — a changed register sequence, a renamed function the page cites, a decision record the diff quietly contradicts. Report those as findings of their own (`should-fix` when the page is now wrong, `nit` when it is merely stale), attributed `(knowledge base — <page>)`. A wrong page is worse than a missing one, and review is the only place staleness is actually visible.

## 4. Verify before reporting

Apply the verification section of `$REFS/review-protocol.md` in full: read the cited lines, drop pastiche and misreads, dedup convergent findings, and downgrade to `question` anything a blocker- or should-fix-grade claim cannot evidence.

## 5. Report

Structure:
1. **Verdict** — two or three sentences: what was reviewed, overall shape, whether anything blocks.
2. **Findings** — ordered blocker → should-fix → nit → question. Each: `file:line`, the issue in one or two plain sentences, the fix direction, and attribution `(lens — source)`. Omit empty severity tiers.
3. **Lens disagreements** — where lenses genuinely pull opposite directions on this diff (late binding vs. no-premature-abstraction is the standing example), present both positions and which fits r9 here, with reasoning. Do not average tensions away; this section is often the most valuable. Omit if none.
4. **What's strong** — one or two things the diff does well, only if genuinely notable.

Do not apply fixes. If the user wants fixes afterward, that's a separate request — respect the project's minimal-scoped-changes preference.
