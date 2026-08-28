---
status: done
---

# Claim a timer before firing it

`interrupt_handler` (aarch64/src/timer.rs:170) copies the timer table under
the lock and then runs the due-check, `fire()` and deadline-advance with
the lock released — deliberately, so a callback may start or cancel timers.
That is correct on one core and wrong on several, because `TIMERS` is
global while `CNTP_CVAL_EL0` is banked per core.

Once two cores have both run `arm_hardware()` (timer.rs:123), each has the
same global minimum deadline in its own banked CVAL. At expiry both take
the PPI. Both copy the table (line 176), both read the same `&'static
Timer` and see `deadline <= now` (lines 184-185), and both call
`timer.callback.fire()` (lines 192/193): one logical expiry, two callback
invocations. The periodic path then has both cores storing
`deadline + period` (line 196) over each other, so the timer can also skip
a period or lose one entirely depending on which store lands last.

Dropping the lock is not the problem and re-taking it across `fire()` would
reintroduce the deadlock 01-timer-lock-irq-deadlock.md fixed. What is
missing is a claim: exactly one core must win the right to fire a given
expiry.

Fix direction: make the due-check a claim rather than a load — e.g.
`deadline_ticks.compare_exchange(deadline, deadline + period)` for periodic
timers, so the core that advances the deadline is the one that fires and
the loser sees a deadline in the future and skips. One-shots can claim via
the existing `active` flag with a swap instead of a store. Whichever
primitive, the property to state at the site is that a single expiry
produces a single `fire()`.

timer-rearm-clamp.md has landed: the advance is now the clamp at
timer.rs:203, `deadline + ((now - deadline) / period + 1) * period`.
The claim must be built on it — a compare-exchange from the claimed
deadline to the clamped next value, so the one core that advances the
deadline is the one that fires, and the clamp is computed from the
claimed value, not a separately-loaded one.

Note: gic-timer-review-nits.md #6 asks for the per-core banking to be
documented. Documenting it does not fix this.

Done when: a periodic timer fires its callback once per expiry with
secondaries running, and the deadline advances exactly once per fire.

Origin: code review of the qemu-integration-tests branch (main...HEAD).
