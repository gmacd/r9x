---
status: open
---

# State the TimerCallback execution-context contract

`TimerCallback::fire` (aarch64/src/timer.rs:27) runs arbitrary registered
code in interrupt context, so interrupt latency is the sum of whatever
callbacks anyone registers — unbounded by construction. The trait is
public API and users are growing on it.

Premise update (2026-08-27 audit): the original "there is no scheduler to
defer to" is now false — the scheduler landed, and its tick is a
*production* `TimerCallback` (aarch64/src/process.rs:409-424) whose own
comment answers the old question 1: "the tick sets a flag, the trap tail
schedules. This is the Plan 9 / Linux shape." **The contract has been
decided by usage; it just isn't written on the trait** (the trait doc
still says only "Fired in interrupt context"). This makes the task more
urgent, not less: write it down before more `fire` implementors appear.

What the doc should state, from the actual dispatch path
(`interrupt_handler`, timer.rs:312-367):

- callbacks run in hard-IRQ context with IRQs masked, **outside** the
  `TIMERS` lock — starting or cancelling timers from `fire` is
  explicitly legal (timer.rs:95, :308-309);
- bounded work (interrupt latency is the sum of all callbacks); no
  blocking or spinning on locks that non-IRQ code holds without masking
  (the `with_timers` doc at timer.rs:119-122 explains the deadlock);
- no context switch from inside `fire` (the Tick comment at
  process.rs:411-420 explains why); defer real work by setting a flag
  for the trap tail;
- this corresponds to Linux `HRTIMER_MODE_HARD`
  (include/linux/hrtimer.h:31-34 documents context per mode — the
  precedent for stating it on the type). QNX-style thread-context
  delivery (pulses) remains the plausible end state for *user-visible*
  timers now that IPC exists; the kernel tick stays IRQ-context. The
  doc can say both.

Done when: the trait docs state the context, its constraints, and the
end state. One paragraph suffices. Docs-only.

Origin: panel review of 46a59c9 (microkernel lens — bounded
handlers); premise refreshed by the 2026-08-27 backlog audit.
