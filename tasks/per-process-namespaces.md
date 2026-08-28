---
id: 123
status: open
wave: 6
depends-on: 119, 122
---

# Task 123: per-process namespaces

## Status: open — wave 6.  Depends on 119, 122

## Problem

The nameserver holds one global bind table of eight entries with one view
for everyone.  That is Plan 9's *naming* without Plan 9's *namespace* —
and the namespace is the idea the project is named after.  My `/dev`
should be able to differ from yours; that is what makes sandboxing,
testing against fake devices, and remote resources fall out for free
instead of being features.

You cannot grow one into the other while a handle is a global integer,
which is why task 119 is a hard prerequisite rather than a nicety.

The current table also has a live defect (filed in task 114): `bind`
truncates a name to 32 bytes and returns `R_OK` while `resolve` compares
the full length, so an over-long name binds unresolvably.

## Precedents

**Plan 9** is the whole precedent: `rfork` namespace flags,
`bind`/`mount`, longest-prefix resolution, union directories.  The
distinction to copy carefully is copy-vs-share at fork — it is what makes
the namespace useful rather than merely per-process.

## Design

- Decide first, and write down why: namespace tracked by the kernel, or a
  library each process links.  Kernel-tracked is simpler for inheritance;
  library keeps the kernel smaller and is more in the r9x spirit.  This
  decision gates the rest of the task.
- Per-process ordered mount list: name prefix → server fid.
- `bind`, `mount`, `unmount` operate on the caller's own namespace.
- Inheritance at spawn: copy, share, or empty, chosen by the spawner —
  the `rfork` distinction.
- Longest-prefix resolution, walking the remainder through 9P.
- Retire `cmd/nameserver` as a distinct service, or reduce it to the root
  mount table.
- Reject an over-long bind rather than truncating.

## Tests

- Integration: two processes resolve `/dev/console` to two different
  servers, both correctly.
- Integration: a child spawned with an empty namespace cannot reach any
  server it was not explicitly given — the sandboxing property, and the
  one that proves the design.
- Integration: names longer than the old 32-byte limit either work or are
  rejected at bind, never silently truncated.

## Done when

- Namespaces are per-process and inherited by an explicit choice.
- The two-views test passes.
- Full `cargo xtask ci` green.

Origin: architecture and correctness review of `f76d96a`, 2026-08-28.
