---
status: done
---

# Add the missing ISBs around timer system registers

timer.rs issues MSRs to CNTP_CVAL_EL0/CNTP_CTL_EL0 (timer.rs:131-132,
timer_enable/timer_disable) with no ISB after, and reads CNTPCT_EL0
(`now()`, timer.rs:148) with no ISB before. These registers are not
self-synchronizing.

Concrete hazards:
- The interrupt handler re-arms (new CVAL / CTL write) and then writes
  GICC_EOIR; without an ISB the timer output may not have deasserted when
  EOI lands — a spurious re-take of the level-triggered PPI, the exact
  event the commit message says re-arming prevents.
- `now()` can read CNTPCT speculatively early and under-read against a
  deadline.

Witness: Linux inserts `isb()` after `write_sysreg(val, cntp_ctl_el0)` and
uses `isb; mrs %0, cntpct_el0` for counter reads
(/Volumes/Code/repos/linux/arch/arm64/include/asm/arch_timer.h).

Fix: add the ISBs at the reg-wrapper layer (cnt_el0.rs) so callers can't
forget, with a comment per barrier stating which reordering it prevents —
an unexplained barrier is a lie waiting to be believed.

Done when: CVAL/CTL writes are followed by ISB, CNTPCT reads are preceded
by one, each with its one-line justification.

Origin: panel review of 46a59c9 (hardware-truth lens).

## Status: done

- ISBs added at the reg-wrapper layer in `reg/cnt_el0.rs`, exactly as
  proposed: after `CntpCvalEl0::write`, after `CntpCtlEl0::write`, and
  before `CntPctEl0::read`. Each carries the one-line justification of
  the reordering it prevents (EOI racing the deassert of the
  level-triggered PPI; a speculatively-early counter read).
