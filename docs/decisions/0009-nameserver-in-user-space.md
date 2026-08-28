---
status: accepted
---

# 0009 — Names live in a user-space nameserver; the kernel owns only handles

- **Status**: accepted — implemented (`cmd/nameserver`, `aarch64/src/registry.rs`)
- **Date**: 2026-08-28 (record written from `tasks/plans/microkernel-nameserver.md`)
- **Context**: `tasks/plans/microkernel-nameserver.md`

## Decision

The namespace is a user-space process owning a flat `name → ChannelHandle`
map, resolved by a native-opcode message. The kernel owns the *handle* — a
validated index into its channel table — and nothing about names. One new
syscall, channel creation, is the entire kernel-side addition. The bind table
is a fixed-size array with a linear scan, in the nameserver's own memory.

## Why

The split is kernel = handle, user = name. A handle must be kernel-validated
because `send` and `receive` act on it; a name is policy, and policy in the
trusted base is how the monolith regrows. A server that cannot create its own
channel cannot be an independent server, so the one syscall is the minimum
surface, not a convenience. The names are already absolute Plan 9-style paths
(`/dev/console`), so the eventual tree is a re-organisation of the same map
rather than a rewrite.

## Alternatives rejected

- **An in-kernel name→channel table.** Lost: names are policy; "put the
  registry in the kernel because it's easier" is how a microkernel dies.
- **A 9P nameserver now.** Lost for this slice: 9P is a later stage, and
  building the fid walk to serve one native-opcode server is premature.
- **A static kernel module (the Oberon shape).** Lost: it refuses the process
  boundary that lets the nameserver die without taking the kernel with it.
- **Hand every server its handles as spawn arguments.** Lost: it hardcodes the
  wiring the namespace exists to remove.
- **A channel-pair creation verb.** Lost: two plain calls are the boring total
  form; a pair is convenience nothing needed.
- **A hash map or a tree for the bind table.** Lost: fancy machinery at
  single-digit n; the tree is the 9P `walk` structure and arrives with 9P.

**Dissent** (whole-system lens): a native-opcode `RESOLVE` returning a raw
handle leaks the mechanism through the metaphor. Accepted for this slice
because the only consumer is a test client; the 9P client will see only a
path.

**Dissent** (simplicity-and-interfaces lens): a flat map of absolute names is
a symbol table, not a name space. Agreed it is not the end state — the flat
map is the mechanism, the tree is deferred structure.

## Consequences

- Boot wiring lives in `aarch64/src/system.rs`, which spawns the nameserver
  first and hands later servers their handles.
- Every server discovers its peers by name, so adding one is a bind, not a
  kernel change.
- The `RESOLVE` opcode is a debt against the file metaphor, tracked by
  [0005](0005-opaque-kernel-message.md).
