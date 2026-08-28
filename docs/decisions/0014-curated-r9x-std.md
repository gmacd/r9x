---
status: accepted
---

# 0014 — A curated `r9x_std` on `core` + `alloc`, not a fork of `std`

- **Status**: accepted — implemented (`std/` = `r9x-std`, `abi/` = `r9x-abi`; static allocator at `std/src/mem.rs:65`)
- **Date**: 2026-08-28 (record written from `tasks/plans/r9x-target-std-backend.md`)
- **Context**: `tasks/plans/r9x-target-std-backend.md`

## Decision

"The std backend" means a curated `r9x_std` layer over `core` + `alloc` plus a
runtime — not a build of the `std` crate. With it:

- **Threads are processes.** `r9x_std::thread::spawn`, when it lands, spawns a
  process; r9x processes stay single-context.
- **ABI constants live in a neutral `r9x_abi` crate** that both the kernel and
  user-space depend on — `IMAGE_BASE`, `HANDLES_VA`, `MSG_MAX`, the syscall
  numbers.
- **A static allocator now**, a kernel-backed heap later.
- **One repository**: kernel, user-space crates and servers in `r9x`, one CI,
  one toolchain pin, atomic ABI bumps.

## Why

r9x's ABI is a *subset* of what std's platform layer presumes: there are no
file, process, network or thread syscalls, by design. Forking std means owning
a fast-moving half-million-line tree and implementing or stubbing every
platform trait, most of them unimplementable or actively wrong against a
QNX/Plan 9-shaped microkernel. Setting `os = "linux"` and reusing that platform
layer would issue Linux syscall numbers against r9x's `svc` ABI — a silent lie.

Threads are processes because the scheduler, fault isolation and determinism
stories are all per-process; an M:N runtime collides with one kernel stack and
one trap frame per process. The neutral ABI crate exists because the constants
are *format* facts shared by both sides, and mirroring them is drift waiting to
happen.

## Alternatives rejected

- **Fork `std` with an r9x platform layer.** Lost: an unbounded maintenance
  obligation for a surface the kernel cannot back.
- **`build-std` with `os` set to an existing OS.** Lost: wrong syscalls,
  silently.
- **Green threads on one context.** Lost: collides with the per-process model.
- **The kernel keeps owning the constants and user-space mirrors them.** Lost:
  drift guarded only by a placement check. A pinning test is the fallback if
  the shared-crate layering is ever rejected.
- **A bump-only allocator, or blocking on the heap syscall first.** Lost:
  leaky for a long-running server; and blocking the whole layer on a kernel
  change nothing yet needs.

**Dissent** (kernel-taste lens): a hand-rolled standard library is
reimplementing std, and the plain reading of "its own std backend" is the real
crate. Overridden: "std" here means the standard base library r9x binaries link
against.

**Dissent** (microkernel-and-firmware lens): the kernel depending on a crate
that also serves user-space muddies the trust boundary. Accepted because the
constants are format facts, not kernel state.

**Dissent** (whole-system lens): shipping a static allocator you intend to
replace is a stopgap. Accepted because it is honest today — the current servers
allocate within it — and the replacement is one scoped task.

## Consequences

- Anything std-shaped that r9x cannot back must be absent rather than stubbed;
  a stub that returns a plausible lie is worse than a missing API.
- `r9x_abi` is the single source for ABI constants; changing one is an atomic
  edit across kernel and servers, which is exactly why they share a repo.
- The static heap size is a stated per-server constant, like the stack size,
  until the kernel-backed heap lands.
