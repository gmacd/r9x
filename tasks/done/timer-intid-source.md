---
status: done
---

# Decide the timer interrupt source: DT-derived INTID or CNTV

`aarch64/src/gic.rs:48` hardcodes `TIMER_INTID = 30` (CNTP, non-secure physical),
and the comment concedes that which PPI CNTP raises depends on the security
state we boot in. If we come up secure, INTID 29 fires instead, hits the
unhandled path in `trap.rs`, gets disabled at the distributor, and the timer
subsystem is permanently dead with only a console line as evidence.

Options:
1. Parse the PPIs from the devicetree timer node (`arm,armv8-timer`,
   `interrupts` property) — we already parse the DT for the GIC itself.
2. Switch to the virtual timer (CNTV, INTID 27), which EL1 kernels
   conventionally use precisely to sidestep the secure/non-secure ambiguity.

Either way, state which boot state r9 guarantees across QEMU virt and Pi.

Done when: the INTID is established from data (DT) or by construction (CNTV),
not guessed; the choice and the guaranteed boot state are documented in the
timer or gic module docs.

Origin: panel review of 46a59c9 (kernel-taste + hardware-truth lenses).

---

Resolved: parsed from the devicetree (option 1), with a documented
positional assumption. `gic::timer_intid_from_dt` takes the
`arm,armv8-timer` node's `interrupts` entry [1] (EL1 non-secure
physical per `arm,arch_timer.yaml`: [0] secure, [1] non-secure, [2]
virtual, [3] EL2 phys) and maps PPI + 16 to the INTID. Both supported
machines verified: QEMU virt's live DTB and bcm2711's DT list PPI 14 at
entry [1] (INTID 30), and on QEMU virt arming CNTP empirically
delivers INTID 30. The guaranteed boot state (non-secure EL1 handoff on
QEMU virt and the BCM firmware) is documented at the parser. Guards:
GIC_PPI type cell checked, `interrupt-names` lists refused (a different
convention, e.g. hypervisor-generated DTs), PPI range checked. The
const `TIMER_INTID` is gone; the value lives in the `Gic` and trap.rs
compares against `gic::timer_intid()`.
