# r9 knowledge base

What is *true* about this project and its hardware, versioned with the code it
describes. Start here: this index is the routing table — read it, then read
only the page you need.

## Pages

| Page | Covers | Read it when |
|---|---|---|
| [`hardware/gicv2.md`](hardware/gicv2.md) | GICv2 / GIC-400: registers, init sequence, dispatch, Pi 4 and QEMU wiring | touching `aarch64/src/gic.rs`, `irq.rs`, or interrupt routing |
| [`subsystems/boot.md`](subsystems/boot.md) | Boot and early init — aarch64 flow in full; x86-64, riscv64, port are stub sections | changing early init, page tables, or the handoff into Rust |
| [`lessons.md`](lessons.md) | Gotchas that cost debugging time: bitstruct withers, `aarch64_cpu` registers, `LockGuard`, QEMU process hygiene | before writing register code, locking code, or running a guest |
| [`reading/`](reading/README.md) | Reading notes — this project's reading of a paper: what r9x takes from it and what it refuses | a design question has a literature behind it, or you are about to re-derive something a paper settled |
| [`decisions/`](decisions/README.md) | 16 decision records — what r9x chose, why, and what lost, indexed in `decisions/README.md` | a design question feels already-settled, or you are about to re-litigate one |

## What goes where

This repo's knowledge lives in four tiers, separated by how they are loaded:

| Tier | Home | Lifecycle |
|---|---|---|
| Always-on rules and commands | `AGENTS.md` (imported by `CLAUDE.md`) | small and stable; every line is paid for on every request |
| Procedures — how to do a thing | `.agents/skills/` | invoked on demand as `/os-plan`, `/os-build`, `/os-review` |
| Facts — what is true | **this directory** | grepped and read when relevant |
| Work in flight | `tasks/` | churns daily; never the source of truth for how something works |

Status belongs in `tasks/`, not here. A page that says "not yet implemented"
is a page that will be wrong within the month.

## Writing a page

Every page carries a header:

```markdown
---
covers: aarch64/src/gic.rs, aarch64/src/irq.rs
sources: ARM IHI 0048D §4.3, BCM2711 ARM Peripherals §6.4
verified: f76d96a (2026-08-28)
---
```

- **`covers`** — the code this page describes. It is the staleness anchor: if
  these files move underneath the page, the page is suspect.
- **`sources`** — where the claims come from: document and section, a `git`
  reference, or a dated measurement with the command that produced it.
- **`verified`** — the commit and date at which a human last confirmed the
  page against the code and the spec.

Reading notes swap `covers:` for `informs:` — a paper describes no code, so it
anchors to the knowledge it backs. Their PDFs live in
`/Volumes/Code/repos/papers/`, never in this repo.

Decision records are the exception: they carry `status:` instead. A decision
does not go stale, it gets superseded — by another record that says so.

Then the rules that keep it usable:

1. **Cite or delete.** Every non-obvious claim names its source — spec section,
   `file.rs:line`, or "measured on <date> with <command>". An uncited claim is
   indistinguishable from a guess.
2. **Link to code; don't restate it.** Anything copied out of a `.rs` file is
   now two things that must agree. Write down what the code cannot say:
   rationale, hardware behaviour, spec ambiguity, measurements, dead ends.
3. **Say what the page does not cover**, so a reader stops rather than infers.
4. **One topic per file**, stable kebab-case name, one `#` heading, stable
   `##` anchors — skills and reviews cite `docs/hardware/gicv2.md#registers`,
   not a 600-line file.
5. **Delete freely.** A wrong page is worse than a missing one.

## Keeping it true

Four write triggers, all of them a byproduct of work already happening:

- **Bringup or debugging discovers hardware behaviour** → a page, or a section
  of one, the same day. `/os-build` files these.
- **A decision gets made** → a record in `decisions/`, numbered, with the
  alternatives that lost. `/os-plan` files these when a plan is accepted.
- **A review finding recurs** → an entry in `lessons.md`, so it is caught by
  reading next time instead of by review.
- **A paper turns out to matter** → a note in `reading/`, wired to whatever it
  informs. Most papers instead belong in a page's `sources:` or in the plan
  that cites them; a note is for the ones worth re-reading.

And one read trigger: grep this directory before searching the web or
re-deriving hardware behaviour from a spec.

Planned: `cargo xtask kb --check`, which for each page diffs the `covers`
paths between `verified` and `HEAD` and reports pages whose code moved
underneath them — run in CI beside `fmt --check`, reporting rather than
blocking.
