---
status: accepted
---

# 0005 — The kernel message is opaque; 9P rides on it; native opcodes are the documented exception

- **Status**: accepted — implemented for native opcodes (`abi/src/lib.rs`, `cmd/console`, `cmd/nameserver`); 9P not yet built
- **Date**: 2026-08-28 (record written from `tasks/plans/microkernel-substrate.md`, `tasks/plans/microkernel-console-server.md`)
- **Context**: `tasks/plans/microkernel-substrate.md`

## Decision

The kernel treats a message payload as opaque bytes. 9P is a user-space
protocol carried inside those bytes. A narrow native opcode API is the
documented exception for servers that are genuinely not files — the raw
console during bringup being the case that forced it.

## Why

Making 9P the kernel's type would force the file model onto things that are
not files and put 9P semantics inside the trusted base. Making everything a
native opcode would fork the uniform file metaphor the whole design exists to
preserve. Opaque payload keeps the kernel small and lets the interface layer
evolve — including the eventual switch to 9P — without a kernel change.

## Alternatives rejected

- **9P as the kernel message type.** Lost: file semantics in the trusted base,
  imposed on non-file servers.
- **Native opcodes everywhere.** Lost: the system would have two ways to say
  one thing, permanently.

**Dissent** (simplicity-and-interfaces lens): only files, no native opcode at
all. The exception is allowed because the raw console genuinely is not a file
— it is a polled MMIO character device during early bringup — and even it is
name-addressable, so the client still sees a path. Recorded, not averaged
away.

## Consequences

- Every native-opcode server is a standing debt against the file metaphor, and
  each one must justify itself here rather than by precedent.
- When 9P lands, client-visible paths are unchanged; only the protocol beneath
  them changes.
- The kernel never parses payloads, so a malformed message is a server's
  problem, not a kernel fault.
