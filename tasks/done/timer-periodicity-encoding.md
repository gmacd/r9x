---
status: done
---

# One encoding for periodicity; fix the wrong-hazard comment

Periodicity is represented twice in timer.rs and the encodings can
disagree: the `repeat: bool` field (read only at `start`, timer.rs:59) and
the `period_ticks == 0` sentinel the interrupt handler actually tests
(timer.rs:185).

Verified divergence: a periodic timer started before `init` (TIMER_FREQ
still 0) gets `duration_to_ticks == 0`, so `period_ticks = 0` — and
silently degrades into an *immediate one-shot* (deadline = now + 0, fires
once, deactivates). Meanwhile the comment at timer.rs:152-154 predicts an
interrupt storm from exactly this case — a hazard the sentinel branch makes
impossible. The comment documents the wrong failure; the real one is
undocumented.

Fix (three parts, one change):
1. Have the handler read `timer.repeat` (or store a proper enum) and drop
   the zero-sentinel.
2. Enforce the init-order invariant instead of commenting it: assert
   `freq != 0` in `duration_to_ticks` — the commit's own allocator hunk
   establishes this exact "checked invariant, not unlucky timing"
   principle — or better, read CNTFRQ_EL0 directly (cheap sysreg read) and
   delete the runtime-initialized `TIMER_FREQ` static and its hidden
   ordering dependency entirely.
3. Rewrite or delete the 152-154 comment to match reality.

Done when: one fact has one encoding; starting a timer before init is
either impossible by construction or a loud checked failure; no comment
describes a hazard the code prevents.

## Status note

timer-rearm-clamp.md has landed, and the clamp at the re-arm site means
an always-past deadline degrades to coalesced ticks, not a storm. The
comment this task calls out (timer.rs:152-154) is now stale in a second
way: it warns of an interrupt storm the clamp already prevents, while the
real failure it should describe — the silent degrade to an immediate
one-shot via `period_ticks == 0` — still stands.

Origin: panel review of 46a59c9 (clarity + microkernel lenses, adjudicated
against the code — the storm claim was verified false, the silent-degrade
claim verified true).

## Done (b3383dc)

All three parts in one change, taking the task's "better" option for
part 2: the handler branches on `repeat` (the zero-sentinel is gone and
`period_ticks` is just the cached conversion), `TIMER_FREQ` is deleted
with `duration_to_ticks` reading CNTFRQ_EL0 directly and asserting it
non-zero, and the storm comment is rewritten to match reality.  Two
consequences surfaced and were handled: with the sentinel gone a
sub-tick periodic period would reach the re-arm division as zero, so
`start` asserts >= 1 tick for periodic timers; and `init` keeps its
boot-time CNTFRQ panic for early diagnosis.  The dead `SAMPLED_IN_TEST`
guard went too.

The unit suite grew from one test to seven, all driving the real
`start`/`cancel`/`interrupt_handler` paths against mocked counter and
frequency (`set_mock_freq` added beside `set_mock_count`), serialised
over the shared table: re-arm clamping (the old test, now via `start`),
one-shot fires-once-ignoring-return, periodic stop on false, cancel, a
callback restarting its own one-shot (which also pins that callbacks
run outside the table lock), and should_panic for the two new asserts.
Verified beyond the gates with a QEMU run of the kernel image: pc1/pc2
tick, "stopping pc1" at 5s, pc2 stops at its limit.
