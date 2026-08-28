---
status: done
---

# Timer::start returns Result instead of panicking on a full table

`register` (timer.rs:115) panics when the 8-slot table is full, so the 9th
concurrent `Timer::start` anywhere in the kernel is a kernel panic on a
reachable runtime path. The panic is documented at `start`, but
documentation doesn't make it init-only — timer users will accrete
(scheduler tick, timeouts, watchdogs), and callers, not the table, know
whether failure is fatal for them.

Fix: `register` returns `Result<(), Error>`; `start` propagates it
(`pub fn start(&'static self) -> Result<()>`). Callers that genuinely
cannot proceed may unwrap loudly at their own site — the policy moves to
the caller, matching r9's panic-freedom practice for kernel paths.
Note the slot-reuse path (inactive timers are reclaimed, timer.rs:110)
means full-table is genuinely "8 *active* timers", worth stating in the
error/docs.

Done when: no panic in the registration path; existing call sites updated;
warning-free on all three arches.

Origin: panel review of 46a59c9 (microkernel lens — panics are not
error handling on reachable kernel paths).
