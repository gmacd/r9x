---
id: 99
status: parked
---

# Task 99: Validate SYS_MAP_MMIO physical addresses against device ranges

## Status: parked (spun off task 87; unpark trigger below)

## Problem

`sys_map_mmio` (`aarch64/src/ipc.rs:355-385`) is device-dumb: it will
map **any** physical address a server asks for — including bus holes.
Task 87 was the incident: the mailbox server mapped `0xFE00_0000` (an
unassigned gap in the raspi4b memory map), the map call succeeded, and
the process died later on a synchronous external abort that took a full
misdiagnosed task file to attribute. The kernel had the information to
refuse the mapping at the syscall, where the error names itself.

The device-dumb model itself (the QNX shape — the kernel is a broker,
drivers live in user space, `ipc.rs:349-353`) is a deliberate design and
stays. This task bounds it: "any PA a *server* names" becomes "any PA
inside a *device region the platform actually has*".

## Precedents

- **Plan 9** makes the bug class structurally impossible: user device
  memory comes only from kernel-registered `Physseg`s
  (`port/segment.c`, `addphysseg`; mapped in the fault path,
  `port/fault.c:154-161`) — a user cannot name an arbitrary PA.
- **Zircon** gates `zx_vmo_create_physical` on an MMIO resource
  capability checked against the kernel's known MMIO ranges.
- **seL4** hands out device frames only from platform-described device
  untypeds.
All three converge on the same idea: the platform description, not the
caller, is the authority on what physical addresses are devices.

## Design

- At boot, build a small table of device `PhysRange`s from the DT — the
  kernel already parses the nodes it cares about (`brcm,bcm2835-mbox`,
  the PL011, the GIC); start with a walk that collects every node's
  `reg` into ranges (the FDT iterators exist, `core/src/fdt.rs`), or —
  smaller first cut — just the ranges of the drivers the kernel already
  probes.
- `sys_map_mmio` checks the requested `PhysRange` is contained in a
  device range; otherwise returns a named error (`ERR_BADPA` or
  similar) instead of succeeding. The server's failure moves from a
  delayed SEA kill (task 93's decoder makes that *legible*; this makes
  it *immediate*).
- Log the PA→VA grant under `systrace` while here — the audit's other
  suggestion for making MMIO grants auditable.
- Policy note to record in the resolution: this is validation, not
  authorization — any server can still claim any real device. Per-server
  device *capabilities* (who may map the mailbox?) are a separate,
  later decision; don't conflate the two here.

## Tests

- Host: containment check unit tests (inside / straddling / outside a
  device range; zero-length).
- Integration: a program calls `SYS_MAP_MMIO` on a hole PA and gets the
  error immediately (asserted exit code), while the console/mailbox
  servers still come up (their real ranges pass).

## Unpark when

Task 87's fix has landed (so the mailbox server maps the real page) and
the DT-range table has a second consumer, or the next time a bad-PA
incident costs a debugging session — whichever comes first. Task 93
(DFSC decode) is the cheap half and is not gated on this.

## Done when

- A hole PA is refused at the syscall with a named error; the
  integration image asserts it; existing MMIO clients still work.
- Full `cargo xtask ci` green.

Origin: backlog audit 2026-08-27 — spun off the task 87 rediagnosis
(`r9-mailbox-mmio-fix.md`), Plan 9 physseg / Zircon MMIO-resource
cross-check.
