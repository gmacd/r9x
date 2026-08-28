---
status: done
---

# Pi 3 support: the bcm2836 local interrupt controller

The Pi 3's BCM2837 has a GIC-400 (SoC interrupts: UART, SD, …), but its
Arm generic-timer PPIs route through the bcm2836 *local* interrupt
controller, not the GIC — its DT timer node is `arm,armv7-timer` with
`interrupt-parent = <&local_intc>` and `interrupts = <0..3>` as
local-intc lines (verified against the Linux tree's bcm2837.dtsi). r9
therefore panics in `gic::init` on a Pi 3 with a message naming this gap
(the no-gic-target-policy resolution).

Supporting the Pi 3 means a second interrupt controller: the local intc
(2 registers: pending at 0x4, enable at 0x0) that owns INTID 0-3 for the
timer PPIs. Design questions: does r9 get an intc abstraction (a second
`IrqOps`-style seam, a second ack source in `trap.rs`'s IRQ path), or is
the Pi 3 declared out of scope for the aarch64 port entirely?

Done when: either the Pi 3 boots r9 with working timers (local intc
driver + DT-derived timer routing + a QEMU or hardware test plan), or
the scope decision is recorded where targets are documented
(AGENTS.md) and the `gic::init` panic message points at it.

Origin: no-gic-target-policy resolution, 2026-08-21.
