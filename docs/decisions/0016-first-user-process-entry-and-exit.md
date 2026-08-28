---
status: accepted
---

# 0016 — EL0 entry and return for the first user process

- **Status**: accepted — implemented (`aarch64/src/swtch.S`, `aarch64/src/process.rs`, `aarch64/src/trap.rs`)
- **Date**: 2026-08-28 (record written from `tasks/plans/first-user-process.md`)
- **Context**: `tasks/plans/first-user-process.md`

## Decision

The mechanics of running a process at EL0 and getting back:

- **The exit trap returns via `swtch` to the saved kernel context**, not via
  `eret` to a hand-built EL1 state.
- **`swtch` takes the resume state as a caller-supplied argument**, not from a
  register.
- **No `Process` struct at one user** — two module statics instead.
- **Syscall encoding stays in `aarch64`**, not `port`.
- **An unimplemented syscall kills the process** (status = the syscall number).
- **The first process starts with interrupts enabled, with no knob to vary it.**

## Why

Each clause is forced by the hardware or by the count of users, not by taste.
`eret` restores no general registers, so the kernel caller's x19–x30 would be
whatever the trap left behind. AArch64 has no CPSR read — the plan originally
specified one and the build gate caught it — and `spsr_el1` is written by the
hardware on every trap to EL1, so an invariant over it would be false by
construction; the DAIF half depends on the caller's interrupt state, which only
the caller knows. A `Process` struct at one user is form without content: every
field would have exactly one writer and one reader. Killing on an unimplemented
syscall is the only outcome that reports: spinning hangs the test image, and
returning livelocks a program that re-issues.

Entering with interrupts enabled is a policy choice with a testing rationale:
masked entry would leave the EL0→EL1 interrupt vector unexercised and silently
stop the timer during the process, hiding exactly the class of bug — kernel
state reachable from EL0 — this milestone existed to flush out.

## Alternatives rejected

- **`eret` to a hand-built EL1 state.** Lost to the register rules.
- **Read CPSR, or maintain an `spsr_el1` invariant, or use a constant.** Lost:
  impossible, false by construction, and caller-dependent respectively.
- **A `Process { entry, context, status }` struct.** Lost at one user; the
  second process earns it, and that refactor is expected.
- **`port::syscall` with per-arch encoding.** Lost until a second architecture
  needs it — `port` changes cost a gate trip on three arches.
- **A `run(…, irq_on: bool)` parameter.** Lost twice over: a tunable is a
  decision refused, and a boolean parameter makes one function do two things.

**Dissent** (hardware-truth lens): least privilege at first contact argues for
masked entry. Held: the vector preamble masks, the interrupt stack is proven,
and the integration test surfaces a failure immediately.

## Consequences

- The second user process is the trigger to revisit the struct decision — it
  is expected, not a regression.
- Interrupt-enabled EL0 entry means every syscall path must be reentrancy-safe
  from the first process onward.
- The syscall ABI's home moves to `port` (or to `r9x_abi`, see
  [0014](0014-curated-r9x-std.md)) when a second architecture grows userspace.
