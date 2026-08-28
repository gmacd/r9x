---
status: done
---

# Close the GIC bringup window: publish before enabling delivery ✅ DONE

Three compounding orderings in 46a59c9 leave a window where an interrupt
can fire before the kernel can handle it:

1. IRQs are unmasked at the CPU (`DAIFClr #2` in `trap::init`, run from
   main9 well before `gic::init`).
2. `Gic::new` (gic.rs:168-190) enables the distributor, the timer INTID,
   and the CPU interface (GICC_CTLR) — and only _after_ it returns does
   `init` (gic.rs:148-160) take the IrqGuard and publish the driver into
   the `GIC` static.
3. Nothing forces CNTP_CTL_EL0 to a known state: its ENABLE bit is
   architecturally UNKNOWN at reset (ARM DDI 0487) and firmware may leave
   the timer enabled with a stale CVAL — a level-triggered PPI asserting
   the moment the CPU interface comes up.

An interrupt taken in the window finds `GIC == None`, `try_ack_interrupt`
returns None, and the trap falls into the unhandled spin loop (interacts
with the trap-dispatch blocker, but is a bug independently).

Fix direction: publish the driver (or hold the IRQ mask) across enabling
GICC_CTLR — e.g. split "map + configure distributor" from "enable CPU
interface" and do the latter post-publish; have `timer::init` write
CNTP_CTL_EL0 to disabled _before_ the GIC enables the timer line, which
also means the current gic-before-timer order in main9 is load-bearing and
backwards — reorder or make the dependency explicit.

Done when: no interrupt can be taken between delivery-enable and the driver
being reachable; CNTP_CTL is in a known state before its INTID is enabled;
the init order is either forced by structure or documented as load-bearing.

Origin: panel review of 46a59c9 (microkernel + hardware-truth lenses).

Implementation:
- `trap::init()`: removed `DAIFClr #2` — IRQs no longer unmasked at trap init
- `gic::init()`: split into `configure` (distributor + timer INTID + PMR), publish (IrqGuard + AtomicPtr store), `enable_cpu_interface()` (GICC_CTLR.ENABLE post-publish)
- `timer::init()`: explicit `timer_disable()` — CNTP_CTL_EL0 in known state before GIC enables timer line
- `main9()`: reordered to call `timer::init()` before `gic::init()`
- `// SAFETY:` comment on the `unsafe` block in init

Panel review (hardware-truth + microkernel-and-firmware + kernel-taste):
- 1 should-fix fixed: `// SAFETY:` annotation on unsafe block
- 1 nit fixed: test gate comment in timer::init()
- No blockers. Timer→gic init order is documented as load-bearing in main9.

All gates pass: clippy × 3 arches, dist, test (10/10).
