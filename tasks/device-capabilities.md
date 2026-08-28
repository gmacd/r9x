---
id: 120
status: open
wave: 5
---

# Task 120: device capabilities — gate MMIO and move discovery to user space

## Status: open — wave 5.  Successor to task 99 (unpark it with this)

## Problem

**Ruled during the 2026-08-28 review: `SYS_MAP_MMIO` is to be gated /
permissioned.**  This task is the authorization half that task 99
explicitly deferred ("Per-server device *capabilities* (who may map the
mailbox?) are a separate, later decision; don't conflate the two here").
Task 99 remains the validation half — is this PA a real device at all —
and the two should land together, 99 first.

Today `sys_map_mmio` (`aarch64/src/ipc.rs:398`) checks only that `pa` is
page-aligned and `size != 0`, and `trap.rs:334` dispatches it for every
process.  `Aspace::map_mmio` maps with `Entry::rw_user_mmio()` =
`AllRw`.  So any process can map kernel text, the page-allocator bitmap,
the process table, or another process's pages read-write.  The
microkernel is paying IPC costs without buying isolation: a driver bug is
whole-system memory corruption, which is the thing the architecture
exists to prevent.

The doc comment at `:392` calls this "device-dumb: it provides the
capability, the server decides which MMIO to map (the QNX model)".  QNX
gates `mmap_device_memory` behind `PROCMGR_AID_MEM_PHYS`.  The comment
asserts an isolation property the code does not provide and is now
**wrong**, not merely incomplete — correcting it is part of this task.

Separately, the split is in the wrong place: the kernel parses the FDT
and the servers never see it, so each driver hardcodes its device address
(`PL011_PHYS = 0xfe20_1000` in `cmd/console`, `MBOX_PAGE_PHYS` in
`cmd/mailbox`).  Every new board is a source edit in every server.

## Precedents

Same three as task 99, now applied to authorization rather than
validation: **Plan 9** `Physseg` — user device memory comes only from
kernel-registered segments; **Zircon** — `zx_vmo_create_physical` gated
on an MMIO *resource capability*; **seL4** — device frames handed out
only from platform-described device untypeds.  All three make "which
process may map which device" an explicit grant.

## Design

- A `Device` kernel object: a physical range, an FDT node offset, and an
  optional INTID set.  Minted at boot from the device tree, one per
  relevant node.  Depends on task 119 for handles to name it.
- `SYSMAPMMIO` becomes `(device_handle, offset, size, va)`, bounds-checked
  against the capability's range.  Fold in task 116's neighbours while
  here: reject unaligned `va`, cap `size` (an unbounded `size` today
  spins the core unpreemptably — DAIF.I is masked for the whole trap),
  and use checked arithmetic for `pa + offset` and `va + offset`.
- Fold `SYSIRQCLAIM` into the same capability: a driver may claim only
  INTIDs its device names.  That also closes the unauthenticated global
  claim and gives task 127's EOI fix somewhere to hang the per-INTID
  mask/unmask.
- Grant device capabilities at spawn from a per-image manifest, so
  who-drives-what is declared in one place instead of implied by
  constants scattered through `cmd/`.
- Expose the granted FDT subtree through `r9x_std`; delete the hardcoded
  physical addresses from `cmd/console` and `cmd/mailbox`.
- Unmap on process death — an exited driver's device mapping currently
  leaks with its address space.

## Tests

- Integration: a process with no device capability is refused for every
  address, including valid device addresses.
- Integration: a driver granted the PL011 cannot map the mailbox page,
  and cannot map one byte past its own range.
- Integration: `map_mmio` with a huge `size` returns an error promptly
  instead of wedging the core.
- Grep gate: no physical-address constant remains in `cmd/`.

## Done when

- MMIO is reachable only through a granted device capability.
- The `:392` comment describes what the code does.
- Task 99 landed alongside.
- Full `cargo xtask ci` green.

Origin: architecture and correctness review of `f76d96a`, 2026-08-28.
Ruling recorded in plans/architecture-review-2026-08.md.
