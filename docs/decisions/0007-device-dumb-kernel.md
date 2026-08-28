---
status: accepted
---

# 0007 — The kernel is device-dumb: servers map their own MMIO

- **Status**: accepted — implemented (`aarch64/src/ipc.rs` `sys_map_mmio`, `aarch64/src/aspace.rs:216`, `cmd/console`); authorization deferred, see [0008](0008-irq-to-message-routing.md) and [0010](0010-map-mmio-becomes-a-capability.md)
- **Date**: 2026-08-28 (record written from `tasks/plans/microkernel-console-server.md`)
- **Context**: `tasks/plans/microkernel-console-server.md`

## Decision

A device server maps its own registers by calling `SYS_MAP_MMIO`; the kernel
provides the capability and knows nothing about which device belongs to whom.
The mapping lands in TTBR0 only, with Device memory attributes, so the server
owns the register page exclusively and the kernel cannot reach it.

## Why

The alternative makes the kernel device-smart: it would parse the device tree,
find the PL011, and decide which server receives it — putting device policy in
the trusted base, which is what the substrate exists to avoid. The QNX model
is the correct shape: a resource manager maps its own devices via a syscall.
TTBR0-only is the isolation property of [0006](0006-aspace-shape-and-fault-policy.md)
applied to devices: mapping TTBR1 as well would hand the kernel back the
access the design just removed.

## Alternatives rejected

- **The kernel maps at spawn time** (parses the DT, hands the range over).
  Lost: device knowledge and device policy re-enter the kernel.
- **A spawn parameter.** Lost: same, with the wiring hardcoded in the spawner.
- **Map both TTBR0 and TTBR1.** Lost: defeats exclusive ownership. A
  diagnostic `map_mmio_kernel` variant is deferred until a need exists.

**Dissent** (microkernel-and-firmware lens): with no permission check, any
process can map any physical page, including the GIC's or the timer's. This
was accepted for a single-tenant system of trusted processes — and has since
been overturned; see [0010](0010-map-mmio-becomes-a-capability.md).

## Consequences

- `aarch64/src/ipc.rs:392` carries a comment asserting the device-dumb
  property. Until 0010 lands, that comment claims an isolation the code does
  not enforce.
- Each new device server is a `SYS_MAP_MMIO` caller, so the syscall's contract
  is load-bearing for every driver that follows.
- The stage-5 proof was deliberately TX-only: it proves MMIO ownership, not a
  full device server. RX over the interrupt path was filed as the next step.
