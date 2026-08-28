---
status: accepted (not yet implemented)
---

# 0010 — `SYS_MAP_MMIO` becomes a capability, not an open syscall

- **Status**: accepted, **not yet implemented** — supersedes the single-tenant clause of [0007](0007-device-dumb-kernel.md)
- **Date**: 2026-08-28 (ruling taken during the 2026-08 architecture review)
- **Context**: `tasks/plans/architecture-review-2026-08.md`, ruling 1; tasks 99 (PA validation) and 120 (device capabilities)

## Decision

Physical-memory access becomes a capability. `SYS_MAP_MMIO` is to be gated and
permissioned: physical-address validation is the first half, per-process
device authorization the second.

## Why

Any process can currently map any physical page — the GIC's, the timer's,
another server's registers. The isolation the substrate claims is therefore
not enforced, and the comment at `aarch64/src/ipc.rs:392` asserting the
device-dumb property states a property the code does not provide. That makes
it wrong rather than merely incomplete, which is the distinction that turned a
deferral into a ruling.

## Alternatives rejected

- **Leave it open, single-tenant.** This was [0007](0007-device-dumb-kernel.md)'s
  accepted position and is what the ruling overturns: "the processes are all
  trusted" stops being true the moment anything untrusted runs, and the
  syscall's contract is easier to tighten before there are many callers than
  after.

## Consequences

- Until the gate lands, treat the device-dumb comment in `aarch64/src/ipc.rs`
  as aspirational, and do not cite it as an isolation guarantee.
- Every existing `SYS_MAP_MMIO` caller (`cmd/console`, `cmd/display`,
  `cmd/mailbox`) becomes a capability holder and must be granted its range.
- The grant mechanism is a design question in its own right — whoever builds
  it supersedes this record with the shape that lands.
