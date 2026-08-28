---
status: accepted
---

# 0001 — Require a GIC-routed generic timer PPI on aarch64

- **Status**: accepted
- **Date**: 2026-08-28 (record written; the decision predates it — see `git log aarch64/src/timer.rs`)
- **Context**: `AGENTS.md` (targets), `aarch64/src/timer.rs` `init`, verified at `f76d96a`

## Decision

On aarch64, r9 supports only machines whose generic-timer PPI is routed
through the GIC — the BCM2711 (Raspberry Pi 4) and QEMU's `raspi4b` machine.
`timer::init` reads the `arm,armv8-timer` devicetree node, requires the
interrupt specifier to be in the 3-cell GIC PPI form, and panics with an
explanatory message rather than guessing when either is absent
(`aarch64/src/timer.rs`, the `refusing to boot without a GIC-routed timer`
path).

## Why

The Pi 3's BCM2837 routes its timer PPIs through the bcm2836 local interrupt
controller rather than its GIC-400, so a kernel that assumes GIC routing will
silently take no timer interrupts there. The devicetree makes the difference
observable — a machine with local-intc routing has no `arm,armv8-timer` node
of the expected shape — so the check is cheap and exact. Failing loudly at
boot converts a silent hang into a one-line diagnosis.

Register and routing conventions follow the Server Base System Architecture
(SBSA) PPI assignment, which the GIC-400 TRM agrees with; see the comment
block at `aarch64/src/timer.rs:191`.

## Alternatives rejected

- **Support the Pi 3 via the bcm2836 local interrupt controller.** A second
  interrupt-routing path in the kernel for one out-of-scope board; the
  microkernel-and-firmware lens's "does this belong in the kernel" test and
  the hardware-truth lens's component economy both say no.
- **Probe and guess the routing at runtime.** Guessing hardware topology is
  what the hardware-truth lens exists to prevent; the devicetree is the
  claim to check, not a hint to work around.
- **Fail silently and run without a timer.** A kernel that boots into a
  timerless state fails far from the cause.

## Consequences

- `aarch64/src/timer.rs` owns the check; anything that changes devicetree
  parsing or PPI discovery must keep the loud-failure path intact.
- Adding a new aarch64 target means confirming GIC routing first; if a future
  board needs local-intc routing, this record is what must be superseded.
- Test images and QEMU invocations use `raspi4b`, not `raspi3b`.
