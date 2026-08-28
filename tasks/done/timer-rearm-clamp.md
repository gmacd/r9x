---
status: done
---

# Clamp the periodic re-arm past now

The handler advances a periodic timer with `deadline + period`
(timer.rs:192) with no clamp against the current count. A callback that
runs longer than its period — or a period shorter than the handler path
itself — leaves the deadline permanently in the past: every EOI immediately
re-raises the level-triggered interrupt, back-to-back, starving thread
context forever. The kernel's scheduler tick will eventually hang off this
path, so "don't register slow callbacks" is not an enforceable defense.

Fix direction (pick one, document the semantics):
- Clamp: advance by `period` repeatedly (or compute) until the new deadline
  is > now — accepts tick coalescing under overload, keeps phase.
- Or re-base: `now + period` — simpler, drifts phase under load.
- Either way, consider stating the callback-must-be-≪-period expectation in
  the TimerCallback docs (see timer-callback-context-contract.md).

Done when: a slow callback degrades to missed/coalesced ticks, not an
interrupt storm; the chosen overload semantics are documented at the
re-arm site.

Origin: panel review of 46a59c9 (microkernel lens — no unbounded
retry/starvation in interrupt paths).

## Status: done

- Clamp chosen: the handler advances a due periodic deadline to
  `deadline + ((now - deadline) / period + 1) * period` — the first
  multiple of the period strictly past `now` — so a slow callback
  degrades to coalesced ticks. The overload semantics are commented at
  the re-arm site, and the behaviour is covered by
  `test_periodic_timer_clamping`.
