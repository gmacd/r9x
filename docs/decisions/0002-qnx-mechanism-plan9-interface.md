---
status: accepted
---

# 0002 — QNX mechanism under a Plan 9 interface

- **Status**: accepted — implemented as the substrate (`port/src/ipc.rs`, `aarch64/src/aspace.rs`, `cmd/*`)
- **Date**: 2026-08-28 (record written from `tasks/plans/microkernel-substrate.md`)
- **Context**: `tasks/plans/microkernel-substrate.md`

## Decision

r9x is a microkernel in the QNX mould wearing Plan 9 clothes. The kernel owns
message passing, scheduling and per-process address spaces, and as little else
as it can; drivers and services are ordinary user processes; names, not
handles, are how one process finds another. Plan 9's 9P protocol, server model
and namespace are the *interface* built on that substrate, not a replacement
for it.

## Why

The two traditions answer different questions. QNX answers "what does the
kernel do" — send/receive/reply, priority inheritance, drivers as processes.
Plan 9 answers "what does the system look like" — files, servers, a per-process
namespace. Taking mechanism from one and interface from the other keeps
bounded interfaces, isolation and determinism without regrowing a monolith,
and leaves the file metaphor intact for everything above the kernel.

## Alternatives rejected

- **Plan 9 as shipped** — in-kernel 9P server, shared address space, no IPC
  primitive. Forfeits isolation and determinism and grows the trusted base
  back into a monolith. Its 9P protocol, Fid/Req server model and namespace
  are grafted on top instead, because that half it got right.
- **Oberon shape** — one address space of lean modules whose narrow `pub`
  interfaces *are* the message passing. Refuses processes and address-space
  isolation, which are the entire payoff. Its concept-lean discipline is
  grafted: the kernel's own modules (`ipc`, `sched`, `aspace`) stay Oberon-lean
  and a module's `pub` surface is its specification.
- **Exokernel / user-space IPC** — the kernel handles traps, a userspace
  library does IPC. Removes the kernel but not the coordination problem:
  someone still arbitrates priorities and delivers IRQs, and distributing that
  across userspace is a larger, less auditable trusted base for a system one
  person must hold in their head. Auditable-small beat empty-but-distributed.

**Dissent** (whole-system lens): two message ideas are in flight — the kernel
message and the 9P message. Resolved by layering rather than averaging: the
kernel message is the mechanism, 9P is a payload protocol over it. One
mechanism, one namespace, and the metaphor does not fork.

**Dissent** (hardware-truth lens): a small kernel is still a kernel, and the
multikernel argument for pushing coordination out remains real. If r9x ever
scales to that shape, this record is what gets superseded — the choice was
made at this project's scale, not for all scales.

## Consequences

- Every later decision inherits "does this belong in the kernel?" as its first
  question; `docs/decisions/` is where residence arguments get settled.
- The kernel stays four fixed static tables — process, channel, message, IRQ
  route — with no allocation and no reclamation.
- 9P is deferred, not abandoned: native opcodes are the documented exception
  (see [0005](0005-opaque-kernel-message.md)), and the client-visible path
  survives the switch when 9P lands.
