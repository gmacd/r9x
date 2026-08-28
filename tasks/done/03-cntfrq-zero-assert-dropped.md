---
status: done
---

# Dropped CNTFRQ_EL0 != 0 assert → potential interrupt storm

**Severity: should fix**

**Status: done** (2026-07-24)

The old `TimerSubsystem::init` asserted the counter frequency was nonzero. The
new `timer::init` (`aarch64/src/timer.rs:180-182`) silently stores whatever
`CNTFRQ_EL0` reads, including 0.

With freq 0 — or if any timer is created before `timer::init()` runs —
`duration_to_ticks` returns 0, so a periodic timer gets `period_ticks == 0`
and `when_ticks == now`. On each interrupt it re-arms with an unchanged,
always-in-the-past deadline and refires at maximum rate forever: an interrupt
storm with callbacks firing back-to-back.

## Fix

Restore the assert in `timer::init` (panic on zero frequency), and consider
asserting/handling `TIMER_FREQ == 0` in `duration_to_ticks` rather than
returning 0.
