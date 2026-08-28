---
status: done
---

# Define the no-GIC policy: Pi 3 scope and init failure consistency

BCM2837 (Pi 3) has no GIC — the timer PPI routes through the bcm2836 local
interrupt controller — yet the tree targets Pi (uartmini, mailbox). Today on
such a board `gic::init` prints and continues, `main9` still starts timers,
CNTP is armed, and nothing ever fires: a degraded state distinguishable from
working only by absence of output.

Related inconsistency: the two subsystems disagree on failure policy —
`gic::init` (gic.rs:169-170) prints and soldiers on, while `timer::init`
(timer.rs:145) panics on CNTFRQ=0. For a kernel whose scheduler will hang off
this timer, is running without an interrupt controller ever a supported mode?

Decide:
1. Is Pi 3 (bcm2836 intc) in scope? If yes, an intc abstraction seam is
   needed; if no, say so where targets are documented.
2. One failure policy for missing/broken core bringup subsystems — loud
   (panic at boot) or an explicit, checked degraded mode. Apply it to both
   gic and timer init.

Done when: both inits follow one stated policy, and the Pi 3 decision is
recorded.

Origin: panel review of 46a59c9 (hardware-truth + microkernel + kernel-taste
lenses).

---

Resolved: one loud policy — boot panic. `gic::init` no longer returns a
Result the caller can ignore: every failure (no GIC node, GICv1, double
init, no GIC-routed timer PPI) panics at boot with the specific reason,
matching `timer::init`'s existing CNTFRQ=0 panic. `boot::interrupts` is
a straight line: timer, gic, unmask. Policy recorded in the gic module
docs.

Pi 3 decision: **out of scope**, with a correction to the task's
premise. The BCM2837 *does* have a GIC-400 (like the Pi 4's), but its
generic-timer PPIs route through the bcm2836 local interrupt
controller, not the GIC — its DT timer node is `arm,armv7-timer`
parented to the local intc (verified in the Linux tree's
bcm2837.dtsi). A Pi 3 therefore panics in `gic::init` with a message
naming the local-intc gap. Supporting it needs a second interrupt
controller (an intc driver), which is a design decision, filed under
the deferred tier, not a small task.
