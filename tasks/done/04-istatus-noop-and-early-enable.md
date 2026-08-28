---
status: done
---

# clear_timer_status is a no-op; trap::init enable causes guaranteed spurious IRQ

**Severity: should fix**
**Status: done** (verified 2026-07-24)
Originally: **FIXED (uncommitted)** — swept in full: removed the `trap::init`
CNTP enable, the `trap()` ISTATUS fallback branch, `clear_timer_status` and
its handler call, and the unused `CntpCvalEl0::read`/`value`/`Debug`;
`timer::now`/`duration_to_ticks` are private now. The GIC is the sole timer
delivery path and `arm_hardware` the sole owner of CNTP_CTL.ENABLE (also
deasserting the level-triggered interrupt, documented on the handler).
Verified: all-arch clippy/dist/test/fmt green; QEMU demo output unchanged.

Two related issues around CNTP_CTL_EL0 handling:

1. `clear_timer_status` (`aarch64/src/timer.rs:116-120`) writes CNTP_CTL_EL0
   back with the ISTATUS bit set, but ISTATUS is **read-only** — the write does
   nothing. The interrupt is actually deasserted by step 4's `arm_hardware`
   (future CVAL, or disable), so the handler works, but the function and its
   "write 1 to ISTATUS bit" comment are misleading. Delete the function, or
   deassert properly via `IMASK` if masking is wanted.

2. `trap::init` (`aarch64/src/trap.rs:14-15`) now sets `CNTP_CTL_EL0.ENABLE = 1`
   with a stale `CVAL` (0), so ISTATUS asserts immediately at boot. The moment
   `Gic::new` enables the distributor + CPU interface, a timer interrupt fires —
   before `timer::init()` has run and with an empty queue. It resolves (empty
   drain → `timer_disable`), but the enable serves no purpose:
   `arm_hardware` already enables/disables the timer on demand. It also opens
   the `gic::init` lock window described in task 01.

## Fix

Remove the ENABLE write from `trap::init` and delete `clear_timer_status`
(or replace it with correct IMASK-based masking).

Update (post caller-owned timer redesign): the scope grew. The ISTATUS
fallback branch in `trap()` is now dead too — timers can only be armed via
`Timer::start` after gic/timer init, and `arm_hardware` is the sole owner of
CNTP_CTL.ENABLE, so pre-GIC timer interrupts cannot occur. Delete all three
together: the `trap::init` enable, the `trap()` ISTATUS fallback, and
`clear_timer_status` (+ its handler call). `CntpCvalEl0::read`/`value` in
cnt_el0.rs are also unused now and can go in the same sweep, and
`timer::now`/`timer::duration_to_ticks` can drop their `pub`.
