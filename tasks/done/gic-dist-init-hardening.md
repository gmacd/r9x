---
status: done
---

# Harden distributor init against inherited firmware state

`Gic::new` (gic.rs:168-190) trusts whatever state boot stages left in the
GIC: no disable-all sweep of GICD_ICENABLER, no clearing of pending
interrupts, no priority programming — just CTLR enable plus one ISENABLER
bit and a PMR write.

Consequences on real boards (boot firmware has already run against this
GIC):
- Inherited enables surface as "Unhandled GIC IRQ" noise (contained today
  only by the disable-on-unhandled path).
- An inherited priority of 0xff against PMR=0xff is silently undeliverable
  forever — an interrupt that can never fire and never explains itself.

Witness: Linux `gic_dist_init`
(/Volumes/Code/repos/linux/drivers/irqchip/irq-gic.c) disables everything
and programs default priorities before enabling the distributor.

Fix: in `Gic::new`, before enabling — sweep ICENABLER (all banks), clear
pending (ICPENDR), program a default priority for the INTIDs in use (or
all), then enable. Cite the GICv2 spec section at each step (see the
citations item in gic-timer-review-nits.md).

Done when: distributor state after init is fully kernel-established,
independent of what firmware left behind.

Origin: panel review of 46a59c9 (hardware-truth lens — firmware
state is a claim, not truth).

## Also the per-core half (gic.rs:226)

98f5104 split bringup into `init_distributor` and `init_cpu` after this was
written, and the priority gap above now has a second site that the `Gic::new`
fix does not reach. GICD_IPRIORITYR for INTIDs 0..32 is banked per core, so
only the core itself can program its own — and `init_cpu` does not. It
clears ICACTIVER and ICENABLER, sets `GICC_PMR` to 0xff, enables the timer
PPI, then enables the CPU interface. No IPRIORITYR write appears anywhere in
the file.

This is exactly the 0xff-against-PMR-0xff case above, on the one INTID the
kernel currently depends on: an interrupt is forwarded only when its
priority is numerically lower than PMR, so a timer PPI left at 0xff by
firmware is never delivered. The timer never fires, `interrupt_handler` is
never entered, and nothing reports why.

It is invisible in CI by construction — QEMU resets priorities to 0, so the
boot images pass — and shows up only on hardware whose firmware left the
banked range alone. The comments in `init_cpu` already cite Linux's
`gic_cpu_config()` (drivers/irqchip/irq-gic-common.c) for the
clear-inherited-state sweep; the same function writes 0xa0 to the banked
IPRIORITYR for this reason, so the citation is there but half-applied.

Fix: program the banked IPRIORITYR range in `init_cpu`, on every core, with
the same default the distributor half uses — one policy, applied at both
sites.

Done when: no INTID the kernel enables depends on firmware having left its
priority below PMR, on the boot core or a secondary.

Origin (this section): code review of the qemu-integration-tests branch
(main...HEAD).

## Status

The opening paragraph predates the init split and describes the state
before it: `Gic::new` now programs no registers at all; the distributor
half is `init_distributor` (CTLR enable only) and the ISENABLER/PMR
writes moved to `init_cpu`, which does clear inherited ICACTIVER and
ICENABLER (banked INTIDs 0..32) before enabling. What is still missing,
in both halves:
- no pending-interrupt clear (ICPENDR) anywhere;
- no IPRIORITYR programming anywhere — the per-core half above stands
  in full (the timer PPI left at 0xff by firmware is still never
  delivered, still invisible in CI);
- the ICENABLER sweep covers only the banked 0..32 range, not all banks.
