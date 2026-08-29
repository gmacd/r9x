---
id: 133
status: done
commit: b11022f
---

# Task 133: the `timers` integration test asserts a host-load-dependent fire count

## Status: done (b11022f)

## Problem

The `timers` integration image proves periodic re-arm by fire *count*:
`check!(fast >= 2, …)` and `check!(limited >= 2, …)`.  Fire counts are
proportional to the vCPU service rate, not to the timer's correctness.

Under heavy host CPU load (the suite runs four QEMU VMs in parallel; a
loaded runner starves a vCPU for tens of milliseconds), the 5 ms timer's
first interrupt sits unhandled until the vCPU finally runs.  The handler
then fires the periodic **once** and re-arms to the *clamped* future
(`timer.rs:341`, `next = deadline + ((now-deadline)/period + 1)*period`)
— far past the 40 ms one-shot, which has by then already cancelled it.
So `fast == 1` and `check!(fast >= 2)` fails:

```
Aarch64 timers: FAILED (exit 1)
FAIL  fast periodic re-armed before cancel, 1 fires
```

Reproduced on the dev host by saturating the cores (`yes > /dev/null` ×10)
while running `cargo xtask integration-test --arch aarch64`; the failure
is intermittent (~1 in 5–10 loaded runs).

Under load a *working* periodic and a *broken* (fires-once-then-stops)
periodic are indistinguishable by count, so the assertion cannot tell the
two apart.  It is a false failure, not a caught defect.

## Why the count is redundant

The re-arm and self-stop *logic* is already proven deterministically by
the unit tests against a mocked counter, with no host scheduling in the
loop:

- `periodic_rearm_clamps_missed_deadlines` — re-arms to the clamped future.
- `periodic_stops_when_callback_returns_false` — a periodic stops at its limit.
- `cancelled_timer_does_not_fire` — cancel deactivates.

The integration test's unique contribution is the *hardware* path — CVAL
arming, the timer PPI through the GIC, trap dispatch, and the
level-triggered deassert on re-arm.  A single fire proves that path for a
periodic; the quiescence check (already load-robust) proves the deassert
(no interrupt storm after cancel).

## Approach

Assert the fire *path*, not a count:

- `check!(fast >= 1, …)` — a single fire is guaranteed: the handler fires
  all due timers in table order (`FAST` before `STOP_FAST`), so by the
  time `STOP_FAST.fires == 1` the fast periodic has fired at least once.
- `check!(limited >= 1, …)` — same for the limit timer.
- Keep the `fired` (one-shot) and quiescence checks unchanged; both are
  load-robust, and the quiescence check is what proves the deassert on
  hardware.

## Done when

- `aarch64/tests/timers.rs` asserts `>= 1` (fire path) rather than `>= 2`
  (count), with the comment explaining why a count must not be asserted.
- `cargo xtask ci` is green.
- The `timers` image holds under a saturated host: repeated
  `cargo xtask integration-test --arch aarch64` with the cores loaded
  passes every run.
- The "timing tests must not assert host-scheduling-dependent counts"
  lesson is in `docs/lessons.md`.
